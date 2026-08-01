#![allow(dead_code)]

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use mmux_controller_core::orchestration::{
    CreatePlan, CreateProject, CreateTask, CreateTaskEdge, OrchestrationState, OrchestrationStatus,
    Plan, PlanId, PlanStatus, Project, ProjectId, ProjectStatus, Task, TaskEdge, TaskEdgeKind,
    TaskId, TaskSession, TaskStatus, UpdatePlan, UpdateTask,
};

use crate::store::OrchestrationStore;
use crate::{prune_finished_plans, LocalPruneSessionCandidate, LocalPruneStoreReport};

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

    pub(crate) fn create_plan(&self, input: CreatePlan) -> Result<Plan, String> {
        self.mutate(|state, now_ms| state.create_plan(input, now_ms))
    }

    pub(crate) fn update_plan(&self, plan_id: PlanId, update: UpdatePlan) -> Result<Plan, String> {
        self.mutate(|state, now_ms| state.update_plan(&plan_id, update, now_ms))
    }

    pub(crate) fn update_plan_status(
        &self,
        plan_id: PlanId,
        status: PlanStatus,
        outcome: Option<String>,
    ) -> Result<Plan, String> {
        self.mutate(|state, now_ms| state.update_plan_status(&plan_id, status, outcome, now_ms))
    }

    pub(crate) fn create_task(&self, input: CreateTask) -> Result<Task, String> {
        self.mutate(|state, now_ms| state.create_task(input, now_ms))
    }

    pub(crate) fn update_task(&self, task_id: TaskId, update: UpdateTask) -> Result<Task, String> {
        self.mutate(|state, now_ms| state.update_task(&task_id, update, now_ms))
    }

    pub(crate) fn record_session(
        &self,
        task_id: TaskId,
        session: TaskSession,
    ) -> Result<TaskSession, String> {
        self.mutate(|state, now_ms| state.record_session(&task_id, session, now_ms))
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
        outcome: Option<String>,
        blockers: Option<Vec<String>>,
        evidence: Option<Vec<String>>,
    ) -> Result<Task, String> {
        self.mutate(|state, now_ms| {
            let task = state
                .tasks
                .get(&task_id)
                .ok_or_else(|| format!("task '{}' not found", task_id.0))?;

            let outcome = outcome.map(|outcome| outcome.trim().to_owned());
            let gated_passed_or_delivered = !task.gates.is_empty()
                && matches!(status, TaskStatus::Passed | TaskStatus::Delivered);
            if gated_passed_or_delivered
                && outcome.as_deref().is_some_and(|outcome| outcome.is_empty())
            {
                return Err(format!(
                    "task '{}' has gates; outcome must not be empty before setting status to {:?}",
                    task_id.0, status
                ));
            }

            let has_operator_outcome = outcome
                .as_deref()
                .is_some_and(|outcome| !outcome.trim().is_empty());
            if gated_passed_or_delivered && !has_operator_outcome {
                return Err(format!(
                    "task '{}' has gates; outcome is required before setting status to {:?}",
                    task_id.0, status
                ));
            }

            state.update_task_status(&task_id, status, now_ms)?;
            let task = state
                .tasks
                .get_mut(&task_id)
                .ok_or_else(|| format!("task '{}' not found", task_id.0))?;
            if let Some(outcome) = outcome {
                task.outcome = Some(outcome.trim().to_owned());
            }
            if let Some(blockers) = blockers {
                task.blockers = blockers;
            }
            if let Some(evidence) = evidence {
                task.evidence = evidence;
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
            let mut preview = guard.state.clone();
            for candidate in &candidates {
                if let Some(task) = preview.tasks.get_mut(&TaskId(candidate.task_id.clone())) {
                    task.session = None;
                }
            }
            let pruned_plan_count = prune_finished_plans(&mut preview, sessions_only, cutoff_ms);
            return Ok(LocalPruneStoreReport {
                dry_run,
                sessions_only,
                pruned_session_count: candidates.len(),
                pruned_plan_count,
                candidates,
            });
        }

        self.mutate(|state, _now_ms| {
            let candidates = stale_session_candidates(state, live_local_sessions, cutoff_ms);
            for candidate in &candidates {
                if let Some(task) = state.tasks.get_mut(&TaskId(candidate.task_id.clone())) {
                    task.session = None;
                }
            }
            let pruned_plan_count = prune_finished_plans(state, sessions_only, cutoff_ms);
            Ok(LocalPruneStoreReport {
                dry_run,
                sessions_only,
                pruned_session_count: candidates.len(),
                pruned_plan_count,
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
        .tasks
        .values()
        .filter_map(|task| {
            let session = task.session.as_ref()?;
            if session.node_id.0 != "local" {
                return None;
            }
            if live_local_sessions.contains(&session.session.0) {
                return None;
            }
            if cutoff_ms.is_some_and(|cutoff_ms| session.last_seen_ms > cutoff_ms) {
                return None;
            }
            if !task.status.is_finished() {
                return None;
            }
            Some(LocalPruneSessionCandidate {
                key: session.key(),
                session: session.session.0.clone(),
                task_id: task.id.0.clone(),
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
    use mmux_controller_core::orchestration::{NodeId, SessionId, TaskScope};
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

    fn create_plan(project_id: ProjectId) -> CreatePlan {
        CreatePlan {
            project_id,
            title: "Plan".into(),
            brief: "Detailed test plan brief.".into(),
            instructions: None,
            slug: None,
        }
    }

    fn create_task(plan_id: PlanId, title: &str) -> CreateTask {
        CreateTask {
            plan_id,
            title: title.into(),
            objective: format!("Objective for {title}"),
            scope: TaskScope::default(),
            gates: Vec::new(),
            slug: None,
            auto_schedule: false,
            run_spec: None,
        }
    }

    fn create_project() -> CreateProject {
        CreateProject {
            title: "Project".into(),
            description: "Actor test project".into(),
            slug: None,
        }
    }

    fn session(session: &str) -> TaskSession {
        TaskSession {
            node_id: NodeId("local".into()),
            session: SessionId(session.into()),
            profile: "codex".into(),
            workspace_path: "/workspace/project".into(),
            bypass_permissions: false,
            role: "implementation-worker".into(),
            kind: "codex".into(),
            skills: vec!["rust".into()],
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
        assert_eq!(state.next_task_id, 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn startup_with_existing_snapshot_loads_previous_state() {
        let dir = unique_temp_dir("mmux-orchestration-existing");
        let store = OrchestrationStore::open(dir.clone()).unwrap();
        let mut state = OrchestrationState::new();
        let project = state.create_project(create_project(), 99).unwrap();
        let plan = state.create_plan(create_plan(project.id), 100).unwrap();
        let task = state
            .create_task(create_task(plan.id, "Persisted"), 101)
            .unwrap();
        state
            .record_session(&task.id, session("worker-a"), 200)
            .unwrap();
        store.save(&state, 300).unwrap();

        let handle = OrchestrationHandle::open(Some(&dir)).unwrap();
        let loaded = handle.snapshot().unwrap();

        assert_eq!(loaded.tasks.len(), 1);
        assert!(loaded.tasks.contains_key(&task.id));
        assert!(loaded.tasks.get(&task.id).unwrap().session.is_some());
        assert_eq!(loaded.next_task_id, state.next_task_id);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn successful_mutation_persists_state() {
        let dir = unique_temp_dir("mmux-orchestration-persist");
        let handle = OrchestrationHandle::open(Some(&dir)).unwrap();

        let project = handle.create_project(create_project()).unwrap();
        let plan = handle.create_plan(create_plan(project.id)).unwrap();
        let task = handle.create_task(create_task(plan.id, "Saved")).unwrap();
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
        let plan = handle.create_plan(create_plan(project.id)).unwrap();
        let parent = handle
            .create_task(create_task(plan.id.clone(), "Parent"))
            .unwrap();
        let child = handle.create_task(create_task(plan.id, "Child")).unwrap();

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
            .record_session(child.id.clone(), session("worker-a"))
            .unwrap();
        handle
            .remove_task_edge(parent.id.clone(), child.id.clone(), TaskEdgeKind::ParentOf)
            .unwrap();

        let reloaded = OrchestrationHandle::open(Some(&dir)).unwrap();
        let state = reloaded.snapshot().unwrap();
        let loaded_child = state.tasks.get(&child.id).unwrap();

        assert_eq!(loaded_child.status, TaskStatus::Running);
        assert!(loaded_child.session.is_some());
        assert!(state.task_edges.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_mutation_leaves_previous_snapshot_intact() {
        let dir = unique_temp_dir("mmux-orchestration-failed");
        let handle = OrchestrationHandle::open(Some(&dir)).unwrap();
        let project = handle.create_project(create_project()).unwrap();
        let plan = handle.create_plan(create_plan(project.id)).unwrap();
        let task = handle
            .create_task(create_task(plan.id, "Only Task"))
            .unwrap();

        let error = handle
            .record_session(TaskId("missing".into()), session("worker-a"))
            .unwrap_err();
        let reloaded = OrchestrationHandle::open(Some(&dir)).unwrap();
        let state = reloaded.snapshot().unwrap();

        assert!(error.contains("task 'missing' not found"));
        assert_eq!(state.tasks.len(), 1);
        assert!(state.tasks.contains_key(&task.id));
        assert!(state.tasks.get(&task.id).unwrap().session.is_none());
        assert_eq!(state.next_task_id, 2);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn prune_stale_store_records_prunes_finished_plans_and_contained_tasks() {
        let dir = unique_temp_dir("mmux-orchestration-prune-plans");
        let handle = OrchestrationHandle::open(Some(&dir)).unwrap();
        let project = handle.create_project(create_project()).unwrap();
        let plan = handle.create_plan(create_plan(project.id.clone())).unwrap();
        let parent = handle
            .create_task(create_task(plan.id.clone(), "Parent"))
            .unwrap();
        let child = handle
            .create_task(create_task(plan.id.clone(), "Child"))
            .unwrap();
        handle
            .add_task_edge(CreateTaskEdge {
                from: parent.id.clone(),
                to: child.id.clone(),
                kind: TaskEdgeKind::Related,
                note: None,
            })
            .unwrap();
        handle
            .record_session(child.id.clone(), session("worker-a"))
            .unwrap();
        handle
            .update_task_status(parent.id.clone(), TaskStatus::Delivered)
            .unwrap();
        handle
            .update_task_status(child.id.clone(), TaskStatus::Delivered)
            .unwrap();
        handle
            .update_plan_status(plan.id.clone(), PlanStatus::Delivered, Some("done".into()))
            .unwrap();

        let live = HashSet::new();
        let dry_run = handle
            .prune_stale_session_records(&live, true, false, None)
            .unwrap();
        assert_eq!(dry_run.pruned_session_count, 1);
        assert_eq!(dry_run.pruned_plan_count, 1);
        assert_eq!(handle.snapshot().unwrap().plans.len(), 1);

        let pruned = handle
            .prune_stale_session_records(&live, false, false, None)
            .unwrap();
        assert_eq!(pruned.pruned_session_count, 1);
        assert_eq!(pruned.pruned_plan_count, 1);
        let state = handle.snapshot().unwrap();
        assert!(state.projects.contains_key(&project.id));
        assert!(state.plans.is_empty());
        assert!(state.tasks.is_empty());
        assert!(state.task_edges.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn concurrent_create_calls_serialize_task_ids_and_slugs() {
        let dir = unique_temp_dir("mmux-orchestration-concurrent");
        let handle = OrchestrationHandle::open(Some(&dir)).unwrap();
        let project = handle.create_project(create_project()).unwrap();
        let plan = handle.create_plan(create_plan(project.id)).unwrap();

        let threads = (0..12)
            .map(|_| {
                let handle = handle.clone();
                let plan_id = plan.id.clone();
                thread::spawn(move || {
                    handle
                        .create_task(create_task(plan_id, "Same Title"))
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
}
