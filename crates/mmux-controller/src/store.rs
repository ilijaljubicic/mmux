#![allow(dead_code)]

use std::path::PathBuf;

use mmux_controller_core::orchestration::OrchestrationState;
use rusqlite::{params, Connection, OptionalExtension};

const DB_FILE_NAME: &str = "mmux.db";
const SNAPSHOT_VERSION: i64 = 1;

#[derive(Clone)]
pub(crate) struct OrchestrationStore {
    db_path: PathBuf,
}

impl OrchestrationStore {
    pub(crate) fn open(store_path: PathBuf) -> Result<Self, String> {
        let store_path = mmux_node::resolve_store_path(Some(&store_path))?;
        mmux_node::ensure_store_dir(&store_path)?;
        let db_path = store_path.join(DB_FILE_NAME);
        let store = Self { db_path };
        store.initialize()?;
        Ok(store)
    }

    pub(crate) fn load(&self) -> Result<Option<OrchestrationState>, String> {
        let connection = self.connect()?;
        let row = connection
            .query_row(
                "SELECT version, state_json FROM orchestration_snapshots WHERE id = 1",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| {
                format!(
                    "failed to load orchestration snapshot from '{}': {}",
                    self.db_path.display(),
                    error
                )
            })?;

        let Some((version, state_json)) = row else {
            return Ok(None);
        };
        if version != SNAPSHOT_VERSION {
            return Err(format!(
                "unsupported orchestration snapshot version {} in '{}'",
                version,
                self.db_path.display()
            ));
        }

        serde_json::from_str(&state_json)
            .map(Some)
            .map_err(|error| {
                format!(
                    "failed to deserialize orchestration snapshot from '{}': {}",
                    self.db_path.display(),
                    error
                )
            })
    }

    pub(crate) fn save(&self, state: &OrchestrationState, now_ms: u64) -> Result<(), String> {
        let updated_at_ms = i64::try_from(now_ms).map_err(|_| {
            format!(
                "orchestration snapshot timestamp {} exceeds SQLite INTEGER range",
                now_ms
            )
        })?;
        let state_json = serde_json::to_string(state).map_err(|error| {
            format!(
                "failed to serialize orchestration snapshot for '{}': {}",
                self.db_path.display(),
                error
            )
        })?;

        let connection = self.connect()?;
        connection
            .execute(
                "INSERT INTO orchestration_snapshots (id, version, state_json, updated_at_ms)
                 VALUES (1, ?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET
                    version = excluded.version,
                    state_json = excluded.state_json,
                    updated_at_ms = excluded.updated_at_ms",
                params![SNAPSHOT_VERSION, state_json, updated_at_ms],
            )
            .map_err(|error| {
                format!(
                    "failed to save orchestration snapshot to '{}': {}",
                    self.db_path.display(),
                    error
                )
            })?;
        Ok(())
    }

    fn initialize(&self) -> Result<(), String> {
        let connection = self.connect()?;
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS orchestration_snapshots (
                    id INTEGER PRIMARY KEY CHECK (id = 1),
                    version INTEGER NOT NULL,
                    state_json TEXT NOT NULL,
                    updated_at_ms INTEGER NOT NULL
                );",
            )
            .map_err(|error| {
                format!(
                    "failed to initialize orchestration store '{}': {}",
                    self.db_path.display(),
                    error
                )
            })?;
        Ok(())
    }

    fn connect(&self) -> Result<Connection, String> {
        Connection::open(&self.db_path).map_err(|error| {
            format!(
                "failed to open orchestration store '{}': {}",
                self.db_path.display(),
                error
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mmux_controller_core::orchestration::{
        CreateProject, CreateTask, CreateTaskEdge, NodeId, ProjectId, SessionId, SessionRecord,
        TaskEdgeKind, TaskId, TaskScope,
    };
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{}-{unique}", std::process::id()))
    }

    fn create_task(title: &str) -> CreateTask {
        CreateTask {
            project_id: ProjectId("project-1".into()),
            title: title.into(),
            objective: format!("Objective for {title}"),
            scope: TaskScope {
                include_paths: vec!["src".into()],
                exclude_paths: vec!["target".into()],
                notes: Some("focused scope".into()),
            },
            agents: Vec::new(),
            gates: vec!["cargo test".into()],
            slug: None,
        }
    }

    fn create_project() -> CreateProject {
        CreateProject {
            title: "Project".into(),
            description: None,
            slug: None,
        }
    }

    fn session(task_ids: Vec<TaskId>) -> SessionRecord {
        SessionRecord {
            node_id: NodeId("node-a".into()),
            session: SessionId("session-a".into()),
            profile: "codex".into(),
            workspace_path: Some("/workspace/project".into()),
            bypass_permissions: false,
            task_ids,
            role: "implementation-worker".into(),
            kind: "codex".into(),
            skills: vec!["rust".into()],
            objective: Some("work on a task".into()),
            created_at_ms: 0,
            updated_at_ms: 0,
            last_seen_ms: 0,
        }
    }

    fn populated_state() -> OrchestrationState {
        let mut state = OrchestrationState::new();
        state.create_project(create_project(), 99).unwrap();
        let parent = state.create_task(create_task("Parent"), 100).unwrap();
        let child = state.create_task(create_task("Child"), 101).unwrap();
        state
            .add_task_edge(
                CreateTaskEdge {
                    from: parent.id.clone(),
                    to: child.id.clone(),
                    kind: TaskEdgeKind::ParentOf,
                    note: Some("breakdown".into()),
                },
                200,
            )
            .unwrap();
        state.record_session(session(vec![child.id]), 300).unwrap();
        state
    }

    #[test]
    fn open_creates_database_in_store_path() {
        let dir = unique_temp_dir("mmux-store-create");
        let db_path = dir.join(DB_FILE_NAME);

        let store = OrchestrationStore::open(dir.clone()).unwrap();

        assert_eq!(store.db_path, db_path);
        assert!(store.db_path.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn load_missing_snapshot_returns_none() {
        let dir = unique_temp_dir("mmux-store-empty");
        let store = OrchestrationStore::open(dir.clone()).unwrap();

        assert!(store.load().unwrap().is_none());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn save_and_load_round_trips_tasks_edges_and_sessions() {
        let dir = unique_temp_dir("mmux-store-roundtrip");
        let store = OrchestrationStore::open(dir.clone()).unwrap();
        let state = populated_state();

        store.save(&state, 400).unwrap();
        let loaded = store.load().unwrap().unwrap();

        assert_eq!(loaded.tasks.len(), 2);
        assert_eq!(loaded.task_edges, state.task_edges);
        assert_eq!(loaded.sessions, state.sessions);
        assert_eq!(loaded.next_task_id, state.next_task_id);
        assert_eq!(
            loaded.tasks.get(&TaskId("task-1".into())).unwrap().gates,
            vec!["cargo test"]
        );

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn save_overwrites_singleton_snapshot_row() {
        let dir = unique_temp_dir("mmux-store-overwrite");
        let store = OrchestrationStore::open(dir.clone()).unwrap();
        let first = populated_state();
        let mut second = OrchestrationState::new();
        second.create_project(create_project(), 499).unwrap();
        second.create_task(create_task("Replacement"), 500).unwrap();

        store.save(&first, 400).unwrap();
        store.save(&second, 600).unwrap();
        let loaded = store.load().unwrap().unwrap();
        let row_count: i64 = store
            .connect()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM orchestration_snapshots", [], |row| {
                row.get(0)
            })
            .unwrap();

        assert_eq!(row_count, 1);
        assert_eq!(loaded.tasks.len(), 1);
        assert!(loaded.tasks.contains_key(&TaskId("task-1".into())));
        assert!(loaded.task_edges.is_empty());
        assert!(loaded.sessions.is_empty());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_json_returns_clear_error() {
        let dir = unique_temp_dir("mmux-store-corrupt-json");
        let store = OrchestrationStore::open(dir.clone()).unwrap();
        store
            .connect()
            .unwrap()
            .execute(
                "INSERT INTO orchestration_snapshots (id, version, state_json, updated_at_ms)
                 VALUES (1, ?1, '{bad json', 700)",
                params![SNAPSHOT_VERSION],
            )
            .unwrap();

        let error = store.load().unwrap_err();

        assert!(error.contains("failed to deserialize orchestration snapshot"));
        assert!(error.contains("mmux.db"));

        fs::remove_dir_all(dir).unwrap();
    }
}
