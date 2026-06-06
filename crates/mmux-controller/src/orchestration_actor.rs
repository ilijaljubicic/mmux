#![allow(dead_code)]

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use mmux_controller_core::orchestration::{
    CreateProject, CreateTask, CreateTaskEdge, OrchestrationState, OrchestrationStatus, Project,
    ProjectId, ProjectStatus, SessionRecord, Task, TaskEdge, TaskEdgeKind, TaskId, TaskParticipant,
    TaskStatus, UpdateTask,
};

use crate::store::OrchestrationStore;
use crate::{LocalPruneSessionCandidate, LocalPruneStoreReport};

#[derive(Clone)]
pub(crate) struct OrchestrationHandle {
    inner: Arc<Mutex<OrchestrationRuntimeState>>,
}

struct OrchestrationRuntimeState {
    store: OrchestrationStore,
    state: OrchestrationState,
}

impl OrchestrationHandle {
    pub(crate) fn open(store_path: Option<&Path>) -> Result<Self, String> {
        let store_path = mmux_node::resolve_store_path(store_path)?;
        Self::from_store(OrchestrationStore::open(store_path)?)
    }

    pub(crate) fn from_store(store: OrchestrationStore) -> Result<Self, String> {
        let state = store.load()?.unwrap_or_else(OrchestrationState::new);
        Ok(Self {
            inner: Arc::new(Mutex::new(OrchestrationRuntimeState { store, state })),
        })
    }

    pub(crate) fn snapshot(&self) -> Result<OrchestrationState, String> {
        Ok(self.lock()?.state.clone())
    }

    pub(crate) fn status(&self) -> Result<OrchestrationStatus, String> {
        Ok(self.lock()?.state.orchestration_status(crate::now_ms()))
    }

    pub(crate) fn create_project(&self, input: CreateProject) -> Result<Project, String> {
        self.mutate(|state, now_ms| state.create_project(input, now_ms))
    }

    pub(crate) fn update_project_status(
        &self,
        project_id: ProjectId,
        status: ProjectStatus,
    ) -> Result<Project, String> {
        self.mutate(|state, now_ms| state.update_project_status(&project_id, status, now_ms))
    }

    pub(crate) fn create_task(&self, input: CreateTask) -> Result<Task, String> {
        self.mutate(|state, now_ms| state.create_task(input, now_ms))
    }

    pub(crate) fn update_task(&self, task_id: TaskId, update: UpdateTask) -> Result<Task, String> {
        self.mutate(|state, now_ms| state.update_task(&task_id, update, now_ms))
    }

    pub(crate) fn assign_task(
        &self,
        task_id: TaskId,
        owner: TaskParticipant,
    ) -> Result<Task, String> {
        self.mutate(|state, now_ms| state.assign_task(&task_id, owner, now_ms))
    }

    pub(crate) fn record_session(&self, session: SessionRecord) -> Result<SessionRecord, String> {
        self.mutate(|state, now_ms| state.record_session(session, now_ms))
    }

    pub(crate) fn add_task_edge(&self, input: CreateTaskEdge) -> Result<TaskEdge, String> {
        self.mutate(|state, now_ms| state.add_task_edge(input, now_ms))
    }

    pub(crate) fn remove_task_edge(
        &self,
        from: TaskId,
        to: TaskId,
        kind: TaskEdgeKind,
    ) -> Result<(), String> {
        self.mutate(|state, now_ms| state.remove_task_edge(&from, &to, kind, now_ms))
    }

    pub(crate) fn update_task_status(
        &self,
        task_id: TaskId,
        status: TaskStatus,
    ) -> Result<Task, String> {
        self.mutate(|state, now_ms| state.update_task_status(&task_id, status, now_ms))
    }

    pub(crate) fn update_task_status_details(
        &self,
        task_id: TaskId,
        status: TaskStatus,
        summary: Option<String>,
        blockers: Option<Vec<String>>,
    ) -> Result<Task, String> {
        self.mutate(|state, now_ms| {
            let task = state
                .tasks
                .get(&task_id)
                .ok_or_else(|| format!("task '{}' not found", task_id.0))?;

            let summary = summary.map(|summary| summary.trim().to_owned());
            let gated_passed_or_delivered = !task.gates.is_empty()
                && matches!(status, TaskStatus::Passed | TaskStatus::Delivered);
            if gated_passed_or_delivered
                && summary.as_deref().is_some_and(|summary| summary.is_empty())
            {
                return Err(format!(
                    "task '{}' has gates; summary must not be empty before setting status to {:?}",
                    task_id.0, status
                ));
            }

            let has_operator_summary = summary
                .as_deref()
                .is_some_and(|summary| !summary.trim().is_empty());
            if gated_passed_or_delivered && !has_operator_summary {
                return Err(format!(
                    "task '{}' has gates; summary is required before setting status to {:?}",
                    task_id.0, status
                ));
            }

            state.update_task_status(&task_id, status, now_ms)?;
            let task = state
                .tasks
                .get_mut(&task_id)
                .ok_or_else(|| format!("task '{}' not found", task_id.0))?;
            if let Some(summary) = summary {
                task.summary = Some(summary.trim().to_owned());
            }
            if let Some(blockers) = blockers {
                task.blockers = blockers;
            }
            task.updated_at_ms = now_ms;
            Ok(task.clone())
        })
    }

    pub(crate) fn prune_stale_session_records(
        &self,
        live_local_sessions: &std::collections::HashSet<String>,
        dry_run: bool,
        sessions_only: bool,
        older_than_days: Option<u64>,
    ) -> Result<LocalPruneStoreReport, String> {
        let now_ms = crate::now_ms();
        let cutoff_ms = older_than_days
            .map(|days| {
                days.checked_mul(86_400_000)
                    .and_then(|duration_ms| now_ms.checked_sub(duration_ms))
                    .ok_or_else(|| format!("--older-than-days value {days} is too large"))
            })
            .transpose()?;
        if dry_run {
            let guard = self.lock()?;
            let candidates = stale_session_candidates(&guard.state, live_local_sessions, cutoff_ms);
            return Ok(LocalPruneStoreReport {
                dry_run,
                sessions_only,
                pruned_count: candidates.len(),
                candidates,
            });
        }

        self.mutate(|state, _now_ms| {
            let candidates = stale_session_candidates(state, live_local_sessions, cutoff_ms);
            for candidate in &candidates {
                state.sessions.remove(&candidate.key);
            }
            Ok(LocalPruneStoreReport {
                dry_run,
                sessions_only,
                pruned_count: candidates.len(),
                candidates,
            })
        })
    }

    fn mutate<T>(
        &self,
        apply: impl FnOnce(&mut OrchestrationState, u64) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut guard = self.lock()?;
        let now_ms = crate::now_ms();
        let mut next_state = guard.state.clone();
        let result = apply(&mut next_state, now_ms)?;
        guard.store.save(&next_state, now_ms)?;
        guard.state = next_state;
        Ok(result)
    }

    fn lock(&self) -> Result<MutexGuard<'_, OrchestrationRuntimeState>, String> {
        self.inner
            .lock()
            .map_err(|_| "orchestration state lock poisoned".to_owned())
    }
}

fn stale_session_candidates(
    state: &OrchestrationState,
    live_local_sessions: &std::collections::HashSet<String>,
    cutoff_ms: Option<u64>,
) -> Vec<LocalPruneSessionCandidate> {
    let mut candidates = state
        .sessions
        .iter()
        .filter_map(|(key, session)| {
            if session.node_id.0 != "local" {
                return None;
            }
            if live_local_sessions.contains(&session.session.0) {
                return None;
            }
            if session.task_ids.is_empty() {
                return None;
            }
            if cutoff_ms.is_some_and(|cutoff_ms| session.last_seen_ms > cutoff_ms) {
                return None;
            }
            let all_tasks_finished = session.task_ids.iter().all(|task_id| {
                state
                    .tasks
                    .get(task_id)
                    .is_some_and(|task| task.status.is_finished())
            });
            if !all_tasks_finished {
                return None;
            }
            Some(LocalPruneSessionCandidate {
                key: key.clone(),
                session: session.session.0.clone(),
                task_ids: session
                    .task_ids
                    .iter()
                    .map(|task_id| task_id.0.clone())
                    .collect(),
                last_seen_ms: session.last_seen_ms,
                reason: "missing local tmux session attached only to finished tasks".into(),
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        left.session
            .cmp(&right.session)
            .then_with(|| left.key.cmp(&right.key))
    });
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use mmux_controller_core::orchestration::{NodeId, SessionId, TaskAgent, TaskScope};
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{}-{unique}", std::process::id()))
    }

    fn create_task(project_id: ProjectId, title: &str) -> CreateTask {
        CreateTask {
            project_id,
            title: title.into(),
            objective: format!("Objective for {title}"),
            scope: TaskScope::default(),
            agents: Vec::new(),
            gates: Vec::new(),
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

    fn create_task_with_agent(project_id: ProjectId, title: &str) -> CreateTask {
        CreateTask {
            agents: vec![TaskAgent {
                kind: "codex".into(),
                role: "implementation-worker".into(),
                skills: vec!["rust".into()],
                workspace_path: Some("/workspace".into()),
                objective: Some("implement task".into()),
                prompt: "work on this".into(),
            }],
            ..create_task(project_id, title)
        }
    }

    fn session(task_ids: Vec<TaskId>) -> SessionRecord {
        SessionRecord {
            node_id: NodeId("local".into()),
            session: SessionId("worker-a".into()),
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

    #[test]
    fn startup_with_empty_store_uses_empty_state() {
        let dir = unique_temp_dir("mmux-orchestration-empty");
        let handle = OrchestrationHandle::open(Some(&dir)).unwrap();

        let state = handle.snapshot().unwrap();

        assert!(state.tasks.is_empty());
        assert!(state.task_edges.is_empty());
        assert!(state.sessions.is_empty());
        assert_eq!(state.next_task_id, 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn startup_with_existing_snapshot_loads_previous_state() {
        let dir = unique_temp_dir("mmux-orchestration-existing");
        let store = OrchestrationStore::open(dir.clone()).unwrap();
        let mut state = OrchestrationState::new();
        let project = state.create_project(create_project(), 99).unwrap();
        let task = state
            .create_task(create_task(project.id, "Persisted"), 100)
            .unwrap();
        state
            .record_session(session(vec![task.id.clone()]), 200)
            .unwrap();
        store.save(&state, 300).unwrap();

        let handle = OrchestrationHandle::open(Some(&dir)).unwrap();
        let loaded = handle.snapshot().unwrap();

        assert_eq!(loaded.tasks.len(), 1);
        assert!(loaded.tasks.contains_key(&task.id));
        assert_eq!(loaded.sessions.len(), 1);
        assert_eq!(loaded.next_task_id, state.next_task_id);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn successful_mutation_persists_state() {
        let dir = unique_temp_dir("mmux-orchestration-persist");
        let handle = OrchestrationHandle::open(Some(&dir)).unwrap();

        let project = handle.create_project(create_project()).unwrap();
        let task = handle
            .create_task(create_task(project.id, "Saved"))
            .unwrap();
        let reloaded = OrchestrationHandle::open(Some(&dir)).unwrap();
        let state = reloaded.snapshot().unwrap();

        assert!(state.tasks.contains_key(&task.id));
        assert_eq!(state.next_task_id, 2);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn task_session_edge_and_status_mutations_persist() {
        let dir = unique_temp_dir("mmux-orchestration-mutations");
        let handle = OrchestrationHandle::open(Some(&dir)).unwrap();
        let project = handle.create_project(create_project()).unwrap();
        let parent = handle
            .create_task(create_task(project.id.clone(), "Parent"))
            .unwrap();
        let child = handle
            .create_task(create_task(project.id, "Child"))
            .unwrap();
        let owner = TaskParticipant {
            node_id: NodeId("local".into()),
            session: SessionId("worker-a".into()),
            profile: "codex".into(),
            role: "implementation-worker".into(),
            kind: "codex".into(),
            skills: vec!["rust".into()],
        };

        handle.assign_task(child.id.clone(), owner.clone()).unwrap();
        handle
            .add_task_edge(CreateTaskEdge {
                from: parent.id.clone(),
                to: child.id.clone(),
                kind: TaskEdgeKind::ParentOf,
                note: Some("breakdown".into()),
            })
            .unwrap();
        handle
            .update_task_status(child.id.clone(), TaskStatus::Running)
            .unwrap();
        handle
            .record_session(session(vec![child.id.clone()]))
            .unwrap();
        handle
            .remove_task_edge(parent.id.clone(), child.id.clone(), TaskEdgeKind::ParentOf)
            .unwrap();

        let reloaded = OrchestrationHandle::open(Some(&dir)).unwrap();
        let state = reloaded.snapshot().unwrap();
        let loaded_child = state.tasks.get(&child.id).unwrap();

        assert_eq!(loaded_child.owner, Some(owner));
        assert_eq!(loaded_child.status, TaskStatus::Running);
        assert!(state.task_edges.is_empty());
        assert_eq!(state.sessions.len(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_mutation_leaves_previous_snapshot_intact() {
        let dir = unique_temp_dir("mmux-orchestration-failed");
        let handle = OrchestrationHandle::open(Some(&dir)).unwrap();
        let project = handle.create_project(create_project()).unwrap();
        let task = handle
            .create_task(create_task(project.id, "Only Task"))
            .unwrap();

        let error = handle
            .record_session(session(vec![TaskId("missing".into())]))
            .unwrap_err();
        let reloaded = OrchestrationHandle::open(Some(&dir)).unwrap();
        let state = reloaded.snapshot().unwrap();

        assert!(error.contains("task 'missing' not found"));
        assert_eq!(state.tasks.len(), 1);
        assert!(state.tasks.contains_key(&task.id));
        assert!(state.sessions.is_empty());
        assert_eq!(state.next_task_id, 2);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn concurrent_create_calls_serialize_task_ids_and_slugs() {
        let dir = unique_temp_dir("mmux-orchestration-concurrent");
        let handle = OrchestrationHandle::open(Some(&dir)).unwrap();
        let project = handle.create_project(create_project()).unwrap();

        let threads = (0..12)
            .map(|_| {
                let handle = handle.clone();
                let project_id = project.id.clone();
                thread::spawn(move || {
                    handle
                        .create_task(create_task(project_id, "Same Title"))
                        .unwrap()
                })
            })
            .collect::<Vec<_>>();
        let tasks = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        let ids = tasks
            .iter()
            .map(|task| task.id.0.clone())
            .collect::<HashSet<_>>();
        let slugs = tasks
            .iter()
            .map(|task| task.slug.clone())
            .collect::<HashSet<_>>();
        let reloaded = OrchestrationHandle::open(Some(&dir)).unwrap();
        let state = reloaded.snapshot().unwrap();

        assert_eq!(ids.len(), 12);
        assert_eq!(slugs.len(), 12);
        assert_eq!(state.tasks.len(), 12);
        assert_eq!(state.next_task_id, 13);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn startup_with_task_agents_does_not_create_sessions() {
        let dir = unique_temp_dir("mmux-orchestration-agents");
        let store = OrchestrationStore::open(dir.clone()).unwrap();
        let mut state = OrchestrationState::new();
        let project = state.create_project(create_project(), 99).unwrap();
        let task = state
            .create_task(
                create_task_with_agent(project.id, "Task With Intended Agent"),
                100,
            )
            .unwrap();
        store.save(&state, 200).unwrap();

        let handle = OrchestrationHandle::open(Some(&dir)).unwrap();
        let loaded = handle.snapshot().unwrap();

        assert_eq!(loaded.tasks.get(&task.id).unwrap().agents.len(), 1);
        assert!(loaded.sessions.is_empty());
        assert_eq!(handle.status().unwrap().sessions.len(), 0);
        let _ = fs::remove_dir_all(dir);
    }
}
