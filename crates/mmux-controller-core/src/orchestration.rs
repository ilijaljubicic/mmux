use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ProjectId(pub String);

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct TaskId(pub String);

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub String);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub slug: String,
    pub title: String,
    pub description: Option<String>,
    pub status: ProjectStatus,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum ProjectStatus {
    Active,
    Archived,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub project_id: ProjectId,
    pub slug: String,
    pub title: String,
    pub objective: String,
    pub scope: TaskScope,
    pub owner: Option<TaskParticipant>,
    pub status: TaskStatus,
    pub agents: Vec<TaskAgent>,
    pub gates: Vec<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub completed_at_ms: Option<u64>,
    pub summary: Option<String>,
    pub blockers: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskAgent {
    pub kind: String,
    pub role: String,
    pub skills: Vec<String>,
    pub workspace_path: Option<String>,
    pub objective: Option<String>,
    pub prompt: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskScope {
    pub include_paths: Vec<String>,
    pub exclude_paths: Vec<String>,
    pub notes: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash, Serialize, Deserialize)]
pub enum TaskStatus {
    Backlog,
    Planned,
    Running,
    WaitingForValidation,
    Blocked,
    Failed,
    Passed,
    Delivered,
    Canceled,
}

impl TaskStatus {
    pub const ALL: [Self; 9] = [
        Self::Backlog,
        Self::Planned,
        Self::Running,
        Self::WaitingForValidation,
        Self::Blocked,
        Self::Failed,
        Self::Passed,
        Self::Delivered,
        Self::Canceled,
    ];

    pub fn is_finished(self) -> bool {
        matches!(self, Self::Delivered | Self::Canceled | Self::Failed)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskParticipant {
    pub node_id: NodeId,
    pub session: SessionId,
    pub profile: String,
    pub role: String,
    pub kind: String,
    pub skills: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TaskEdge {
    pub from: TaskId,
    pub to: TaskId,
    pub kind: TaskEdgeKind,
    pub created_at_ms: u64,
    pub note: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum TaskEdgeKind {
    ParentOf,
    DependsOn,
    Blocks,
    Validates,
    Audits,
    Refines,
    Supersedes,
    Related,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionRecord {
    pub node_id: NodeId,
    pub session: SessionId,
    pub profile: String,
    pub workspace_path: Option<String>,
    pub bypass_permissions: bool,
    pub task_ids: Vec<TaskId>,
    pub role: String,
    pub kind: String,
    pub skills: Vec<String>,
    pub objective: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub last_seen_ms: u64,
}

impl SessionRecord {
    pub fn key(&self) -> String {
        session_key(&self.node_id, &self.session)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrchestrationStatus {
    pub projects: Vec<ProjectSummary>,
    pub tasks: Vec<TaskSummary>,
    pub task_edges: Vec<TaskEdgeSummary>,
    pub sessions: Vec<SessionSummary>,
    pub counts: OrchestrationCounts,
    pub cleanup_candidates: Vec<SessionCleanupCandidate>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSummary {
    pub id: ProjectId,
    pub slug: String,
    pub title: String,
    pub description: Option<String>,
    pub status: ProjectStatus,
    pub task_count: usize,
    pub active_task_count: usize,
    pub task_status_counts: BTreeMap<TaskStatus, usize>,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskSummary {
    pub id: TaskId,
    pub project_id: ProjectId,
    pub slug: String,
    pub title: String,
    pub status: TaskStatus,
    pub summary: Option<String>,
    pub owner: Option<TaskParticipant>,
    pub parent: Option<TaskId>,
    pub child_count: usize,
    pub dependency_count: usize,
    pub blocked_by: Vec<TaskId>,
    pub open_gate_count: usize,
    pub failed_gate_count: usize,
    pub blocker_count: usize,
    pub blockers: Vec<String>,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskEdgeSummary {
    pub from: TaskId,
    pub to: TaskId,
    pub kind: TaskEdgeKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSummary {
    pub node_id: String,
    pub session: String,
    pub profile: String,
    pub workspace_path: Option<String>,
    pub bypass_permissions: bool,
    pub task_ids: Vec<TaskId>,
    pub role: String,
    pub kind: String,
    pub objective: Option<String>,
    pub last_seen_ms: Option<u64>,
    pub runtime_state: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrchestrationCounts {
    pub total_projects: usize,
    pub active_projects: usize,
    pub archived_projects: usize,
    pub total_tasks: usize,
    pub active_tasks: usize,
    pub blocked_tasks: usize,
    pub waiting_for_validation_tasks: usize,
    pub passed_tasks: usize,
    pub delivered_tasks: usize,
    pub failed_tasks: usize,
    pub canceled_tasks: usize,
    pub durable_session_records: usize,
    pub cleanup_candidates: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCleanupCandidate {
    pub node_id: String,
    pub session: String,
    pub reason: String,
    pub created_at_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrchestrationState {
    pub projects: HashMap<ProjectId, Project>,
    pub tasks: HashMap<TaskId, Task>,
    pub task_edges: Vec<TaskEdge>,
    pub sessions: HashMap<String, SessionRecord>,
    pub next_project_id: u64,
    pub next_task_id: u64,
}

impl Default for OrchestrationState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateTask {
    pub project_id: ProjectId,
    pub title: String,
    pub objective: String,
    #[serde(default)]
    pub scope: TaskScope,
    #[serde(default)]
    pub agents: Vec<TaskAgent>,
    #[serde(default)]
    pub gates: Vec<String>,
    #[serde(default)]
    pub slug: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateProject {
    pub title: String,
    pub description: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateProjectStatus {
    pub project_id: ProjectId,
    pub status: ProjectStatus,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateTaskScope {
    #[serde(default)]
    pub include_paths: Option<Vec<String>>,
    #[serde(default)]
    pub exclude_paths: Option<Vec<String>>,
    #[serde(default)]
    pub notes: Option<Option<String>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateTask {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub objective: Option<String>,
    #[serde(default)]
    pub scope: UpdateTaskScope,
    #[serde(default)]
    pub agents: Option<Vec<TaskAgent>>,
    #[serde(default)]
    pub gates: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateTaskEdge {
    pub from: TaskId,
    pub to: TaskId,
    pub kind: TaskEdgeKind,
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateTaskStatus {
    pub task_id: TaskId,
    pub status: TaskStatus,
}

impl OrchestrationState {
    pub fn new() -> Self {
        Self {
            projects: HashMap::new(),
            tasks: HashMap::new(),
            task_edges: Vec::new(),
            sessions: HashMap::new(),
            next_project_id: 1,
            next_task_id: 1,
        }
    }

    pub fn orchestration_status(&self, now_ms: u64) -> OrchestrationStatus {
        let mut warnings = Vec::new();
        let mut parent_by_child: HashMap<TaskId, Vec<TaskId>> = HashMap::new();
        let mut child_counts: HashMap<TaskId, usize> = HashMap::new();
        let mut dependencies_by_task: HashMap<TaskId, Vec<TaskId>> = HashMap::new();
        let mut seen_edges = HashSet::new();

        for edge in &self.task_edges {
            if !seen_edges.insert((edge.from.clone(), edge.to.clone(), edge.kind)) {
                warnings.push(format!(
                    "duplicate {:?} edge from '{}' to '{}'",
                    edge.kind, edge.from.0, edge.to.0
                ));
            }
            if edge.from == edge.to {
                warnings.push(format!(
                    "task edge cannot point to itself for '{}'",
                    edge.from.0
                ));
            }
            if !self.tasks.contains_key(&edge.from) {
                warnings.push(format!(
                    "task edge references missing from task '{}'",
                    edge.from.0
                ));
            }
            if !self.tasks.contains_key(&edge.to) {
                warnings.push(format!(
                    "task edge references missing to task '{}'",
                    edge.to.0
                ));
            }

            match edge.kind {
                TaskEdgeKind::ParentOf => {
                    *child_counts.entry(edge.from.clone()).or_default() += 1;
                    parent_by_child
                        .entry(edge.to.clone())
                        .or_default()
                        .push(edge.from.clone());
                }
                TaskEdgeKind::DependsOn => {
                    dependencies_by_task
                        .entry(edge.from.clone())
                        .or_default()
                        .push(edge.to.clone());
                }
                _ => {}
            }
        }

        for parents in parent_by_child.values_mut() {
            parents.sort_by(|left, right| left.0.cmp(&right.0));
            parents.dedup();
        }
        for (child, parents) in &parent_by_child {
            if parents.len() > 1 {
                warnings.push(format!(
                    "task '{}' has multiple ParentOf parents: {}",
                    child.0,
                    parents
                        .iter()
                        .map(|parent| parent.0.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        for dependencies in dependencies_by_task.values_mut() {
            dependencies.sort_by(|left, right| left.0.cmp(&right.0));
            dependencies.dedup();
        }

        let mut task_count_by_project: HashMap<ProjectId, usize> = HashMap::new();
        let mut active_task_count_by_project: HashMap<ProjectId, usize> = HashMap::new();
        let mut status_counts_by_project: HashMap<ProjectId, BTreeMap<TaskStatus, usize>> =
            HashMap::new();
        for task in self.tasks.values() {
            *task_count_by_project
                .entry(task.project_id.clone())
                .or_default() += 1;
            *status_counts_by_project
                .entry(task.project_id.clone())
                .or_insert_with(empty_task_status_counts)
                .entry(task.status)
                .or_default() += 1;
            if !task.status.is_finished() {
                *active_task_count_by_project
                    .entry(task.project_id.clone())
                    .or_default() += 1;
            }
            if !self.projects.contains_key(&task.project_id) {
                warnings.push(format!(
                    "task '{}' references missing project '{}'",
                    task.id.0, task.project_id.0
                ));
            }
        }

        let mut counts = OrchestrationCounts {
            total_projects: self.projects.len(),
            total_tasks: self.tasks.len(),
            durable_session_records: self.sessions.len(),
            ..OrchestrationCounts::default()
        };
        let mut projects = self
            .projects
            .values()
            .map(|project| {
                if project.created_at_ms > now_ms {
                    warnings.push(format!(
                        "project '{}' has a future created_at_ms",
                        project.id.0
                    ));
                }
                if project.updated_at_ms > now_ms {
                    warnings.push(format!(
                        "project '{}' has a future updated_at_ms",
                        project.id.0
                    ));
                }
                match project.status {
                    ProjectStatus::Active => counts.active_projects += 1,
                    ProjectStatus::Archived => counts.archived_projects += 1,
                }
                ProjectSummary {
                    id: project.id.clone(),
                    slug: project.slug.clone(),
                    title: project.title.clone(),
                    description: project.description.clone(),
                    status: project.status,
                    task_count: task_count_by_project
                        .get(&project.id)
                        .copied()
                        .unwrap_or_default(),
                    active_task_count: active_task_count_by_project
                        .get(&project.id)
                        .copied()
                        .unwrap_or_default(),
                    task_status_counts: status_counts_by_project
                        .get(&project.id)
                        .cloned()
                        .unwrap_or_else(empty_task_status_counts),
                    updated_at_ms: project.updated_at_ms,
                }
            })
            .collect::<Vec<_>>();
        projects.sort_by(|left, right| left.id.0.cmp(&right.id.0));
        let mut tasks: Vec<TaskSummary> = self
            .tasks
            .values()
            .map(|task| {
                if task.created_at_ms > now_ms {
                    warnings.push(format!("task '{}' has a future created_at_ms", task.id.0));
                }
                if task.updated_at_ms > now_ms {
                    warnings.push(format!("task '{}' has a future updated_at_ms", task.id.0));
                }
                if task
                    .completed_at_ms
                    .is_some_and(|completed_at_ms| completed_at_ms > now_ms)
                {
                    warnings.push(format!("task '{}' has a future completed_at_ms", task.id.0));
                }

                let dependency_ids = dependencies_by_task
                    .get(&task.id)
                    .cloned()
                    .unwrap_or_default();
                let blocked_by = dependency_ids
                    .iter()
                    .filter(|dependency_id| {
                        self.tasks
                            .get(dependency_id)
                            .map(|dependency| {
                                !dependency_status_allows_readiness(dependency.status)
                            })
                            .unwrap_or(true)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                let is_blocked = task.status == TaskStatus::Blocked || !blocked_by.is_empty();

                if !task.status.is_finished() {
                    counts.active_tasks += 1;
                }
                if is_blocked {
                    counts.blocked_tasks += 1;
                }
                match task.status {
                    TaskStatus::WaitingForValidation => counts.waiting_for_validation_tasks += 1,
                    TaskStatus::Passed => counts.passed_tasks += 1,
                    TaskStatus::Delivered => counts.delivered_tasks += 1,
                    TaskStatus::Failed => counts.failed_tasks += 1,
                    TaskStatus::Canceled => counts.canceled_tasks += 1,
                    _ => {}
                }

                TaskSummary {
                    id: task.id.clone(),
                    project_id: task.project_id.clone(),
                    slug: task.slug.clone(),
                    title: task.title.clone(),
                    status: task.status,
                    summary: task.summary.clone(),
                    owner: task.owner.clone(),
                    parent: parent_by_child
                        .get(&task.id)
                        .and_then(|parents| parents.first().cloned()),
                    child_count: child_counts.get(&task.id).copied().unwrap_or_default(),
                    dependency_count: dependency_ids.len(),
                    blocked_by,
                    open_gate_count: if matches!(
                        task.status,
                        TaskStatus::Passed | TaskStatus::Delivered
                    ) {
                        0
                    } else {
                        task.gates.len()
                    },
                    failed_gate_count: if matches!(
                        task.status,
                        TaskStatus::Blocked | TaskStatus::Failed
                    ) {
                        task.blockers.len()
                    } else {
                        0
                    },
                    blocker_count: task.blockers.len(),
                    blockers: task.blockers.clone(),
                    updated_at_ms: task.updated_at_ms,
                }
            })
            .collect();
        tasks.sort_by(|left, right| left.id.0.cmp(&right.id.0));

        let mut task_edges = self
            .task_edges
            .iter()
            .map(|edge| TaskEdgeSummary {
                from: edge.from.clone(),
                to: edge.to.clone(),
                kind: edge.kind,
            })
            .collect::<Vec<_>>();
        task_edges.sort_by(|left, right| {
            left.from
                .0
                .cmp(&right.from.0)
                .then_with(|| left.to.0.cmp(&right.to.0))
                .then_with(|| edge_kind_rank(left.kind).cmp(&edge_kind_rank(right.kind)))
        });

        let mut sessions = self
            .sessions
            .iter()
            .map(|(key, session)| {
                let expected_key = session.key();
                if key != &expected_key {
                    warnings.push(format!(
                        "session record key '{}' does not match node/session '{}'",
                        key, expected_key
                    ));
                }
                if session.created_at_ms > now_ms {
                    warnings.push(format!(
                        "session '{}' has a future created_at_ms",
                        expected_key
                    ));
                }
                if session.updated_at_ms > now_ms {
                    warnings.push(format!(
                        "session '{}' has a future updated_at_ms",
                        expected_key
                    ));
                }
                if session.last_seen_ms > now_ms {
                    warnings.push(format!(
                        "session '{}' has a future last_seen_ms",
                        expected_key
                    ));
                }

                let mut task_ids = session.task_ids.clone();
                task_ids.sort_by(|left, right| left.0.cmp(&right.0));
                task_ids.dedup();
                for task_id in &task_ids {
                    if !self.tasks.contains_key(task_id) {
                        warnings.push(format!(
                            "session '{}' references missing task '{}'",
                            expected_key, task_id.0
                        ));
                    }
                }

                SessionSummary {
                    node_id: session.node_id.0.clone(),
                    session: session.session.0.clone(),
                    profile: session.profile.clone(),
                    workspace_path: session.workspace_path.clone(),
                    bypass_permissions: session.bypass_permissions,
                    task_ids,
                    role: session.role.clone(),
                    kind: session.kind.clone(),
                    objective: session.objective.clone(),
                    last_seen_ms: Some(session.last_seen_ms),
                    runtime_state: None,
                }
            })
            .collect::<Vec<_>>();
        sessions.sort_by(|left, right| {
            left.node_id
                .cmp(&right.node_id)
                .then_with(|| left.session.cmp(&right.session))
        });

        let cleanup_candidates = Vec::new();
        counts.cleanup_candidates = cleanup_candidates.len();
        warnings.sort();
        warnings.dedup();

        OrchestrationStatus {
            projects,
            tasks,
            task_edges,
            sessions,
            counts,
            cleanup_candidates,
            warnings,
        }
    }

    pub fn create_project(&mut self, input: CreateProject, now_ms: u64) -> Result<Project, String> {
        if input.title.trim().is_empty() {
            return Err("project title must not be empty".into());
        }
        let id = ProjectId(format!("project-{}", self.next_project_id));
        self.next_project_id += 1;
        let base_slug = input.slug.as_deref().unwrap_or(&input.title);
        let slug = self.unique_project_slug(base_slug);
        let project = Project {
            id: id.clone(),
            slug,
            title: input.title,
            description: input.description,
            status: ProjectStatus::Active,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };
        self.projects.insert(id, project.clone());
        Ok(project)
    }

    pub fn update_project_status(
        &mut self,
        project_id: &ProjectId,
        status: ProjectStatus,
        now_ms: u64,
    ) -> Result<Project, String> {
        let project = self
            .projects
            .get_mut(project_id)
            .ok_or_else(|| format!("project '{}' not found", project_id.0))?;
        project.status = status;
        project.updated_at_ms = now_ms;
        Ok(project.clone())
    }

    pub fn create_task(&mut self, input: CreateTask, now_ms: u64) -> Result<Task, String> {
        if !self.projects.contains_key(&input.project_id) {
            return Err(format!("project '{}' not found", input.project_id.0));
        }
        if input.title.trim().is_empty() {
            return Err("task title must not be empty".into());
        }
        if input.objective.trim().is_empty() {
            return Err("task objective must not be empty".into());
        }

        let id = TaskId(format!("task-{}", self.next_task_id));
        self.next_task_id += 1;
        let base_slug = input.slug.as_deref().unwrap_or(&input.title);
        let slug = self.unique_task_slug(&input.project_id, base_slug);
        let task = Task {
            id: id.clone(),
            project_id: input.project_id,
            slug,
            title: input.title,
            objective: input.objective,
            scope: input.scope,
            owner: None,
            status: TaskStatus::Backlog,
            agents: input.agents,
            gates: input.gates,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            completed_at_ms: None,
            summary: None,
            blockers: Vec::new(),
        };

        self.tasks.insert(id, task.clone());
        Ok(task)
    }

    pub fn update_task(
        &mut self,
        task_id: &TaskId,
        update: UpdateTask,
        now_ms: u64,
    ) -> Result<Task, String> {
        if let Some(title) = update.title.as_ref() {
            if title.trim().is_empty() {
                return Err("task title must not be empty".into());
            }
        }
        if let Some(objective) = update.objective.as_ref() {
            if objective.trim().is_empty() {
                return Err("task objective must not be empty".into());
            }
        }

        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| format!("task '{}' not found", task_id.0))?;
        if let Some(title) = update.title {
            task.title = title;
        }
        if let Some(objective) = update.objective {
            task.objective = objective;
        }
        if let Some(include_paths) = update.scope.include_paths {
            task.scope.include_paths = include_paths;
        }
        if let Some(exclude_paths) = update.scope.exclude_paths {
            task.scope.exclude_paths = exclude_paths;
        }
        if let Some(notes) = update.scope.notes {
            task.scope.notes = notes;
        }
        if let Some(agents) = update.agents {
            task.agents = agents;
        }
        if let Some(gates) = update.gates {
            task.gates = gates;
        }
        task.updated_at_ms = now_ms;
        Ok(task.clone())
    }

    pub fn assign_task(
        &mut self,
        task_id: &TaskId,
        owner: TaskParticipant,
        now_ms: u64,
    ) -> Result<Task, String> {
        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| format!("task '{}' not found", task_id.0))?;
        task.owner = Some(owner);
        task.updated_at_ms = now_ms;
        Ok(task.clone())
    }

    pub fn record_session(
        &mut self,
        mut session: SessionRecord,
        now_ms: u64,
    ) -> Result<SessionRecord, String> {
        if session.node_id.0.trim().is_empty() {
            return Err("session node_id must not be empty".into());
        }
        if session.session.0.trim().is_empty() {
            return Err("session id must not be empty".into());
        }
        if session.profile.trim().is_empty() {
            return Err("session profile must not be empty".into());
        }
        for task_id in &session.task_ids {
            if !self.tasks.contains_key(task_id) {
                return Err(format!("task '{}' not found", task_id.0));
            }
        }

        let key = session.key();
        let created_at_ms = self
            .sessions
            .get(&key)
            .map(|existing| existing.created_at_ms)
            .unwrap_or(now_ms);
        session.created_at_ms = created_at_ms;
        session.updated_at_ms = now_ms;
        session.last_seen_ms = now_ms;
        self.sessions.insert(key, session.clone());
        Ok(session)
    }

    pub fn add_task_edge(&mut self, edge: CreateTaskEdge, now_ms: u64) -> Result<TaskEdge, String> {
        self.validate_edge(&edge)?;
        let task_edge = TaskEdge {
            from: edge.from,
            to: edge.to,
            kind: edge.kind,
            created_at_ms: now_ms,
            note: edge.note,
        };
        self.task_edges.push(task_edge.clone());
        Ok(task_edge)
    }

    pub fn remove_task_edge(
        &mut self,
        from: &TaskId,
        to: &TaskId,
        kind: TaskEdgeKind,
        now_ms: u64,
    ) -> Result<(), String> {
        let _ = now_ms;
        let original_len = self.task_edges.len();
        self.task_edges
            .retain(|edge| !(&edge.from == from && &edge.to == to && edge.kind == kind));
        if self.task_edges.len() == original_len {
            return Err("task edge not found".into());
        }
        Ok(())
    }

    pub fn update_task_status(
        &mut self,
        task_id: &TaskId,
        status: TaskStatus,
        now_ms: u64,
    ) -> Result<Task, String> {
        if status == TaskStatus::Planned {
            self.ensure_dependencies_finished(task_id)?;
        }
        if status == TaskStatus::Delivered {
            self.ensure_children_delivered(task_id)?;
        }

        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| format!("task '{}' not found", task_id.0))?;
        task.status = status;
        task.updated_at_ms = now_ms;
        task.completed_at_ms = status.is_finished().then_some(now_ms);
        Ok(task.clone())
    }

    fn validate_edge(&self, edge: &CreateTaskEdge) -> Result<(), String> {
        if !self.tasks.contains_key(&edge.from) {
            return Err(format!("task '{}' not found", edge.from.0));
        }
        if !self.tasks.contains_key(&edge.to) {
            return Err(format!("task '{}' not found", edge.to.0));
        }
        if edge.from == edge.to {
            return Err("a task edge cannot point to itself".into());
        }
        let from_project = &self
            .tasks
            .get(&edge.from)
            .ok_or_else(|| format!("task '{}' not found", edge.from.0))?
            .project_id;
        let to_project = &self
            .tasks
            .get(&edge.to)
            .ok_or_else(|| format!("task '{}' not found", edge.to.0))?
            .project_id;
        if from_project != to_project {
            return Err("task edges cannot cross project boundaries in v1".into());
        }
        if self.task_edges.iter().any(|existing| {
            existing.from == edge.from && existing.to == edge.to && existing.kind == edge.kind
        }) {
            return Err("task edge already exists".into());
        }

        match edge.kind {
            TaskEdgeKind::DependsOn => {
                if self.has_path(&edge.to, &edge.from, TaskEdgeKind::DependsOn) {
                    return Err("DependsOn edge would create a cycle".into());
                }
            }
            TaskEdgeKind::ParentOf => {
                if self.task_edges.iter().any(|existing| {
                    existing.kind == TaskEdgeKind::ParentOf && existing.to == edge.to
                }) {
                    return Err("ParentOf edges allow only one parent per task in v1".into());
                }
                if self.has_path(&edge.to, &edge.from, TaskEdgeKind::ParentOf) {
                    return Err("ParentOf edge would create a cycle".into());
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn unique_project_slug(&self, value: &str) -> String {
        let base = sanitize_slug(value);
        if !self.project_slug_exists(&base) {
            return base;
        }

        let mut suffix = 2;
        loop {
            let candidate = format!("{base}-{suffix}");
            if !self.project_slug_exists(&candidate) {
                return candidate;
            }
            suffix += 1;
        }
    }

    fn project_slug_exists(&self, slug: &str) -> bool {
        self.projects.values().any(|project| project.slug == slug)
    }

    fn unique_task_slug(&self, project_id: &ProjectId, value: &str) -> String {
        let base = sanitize_slug(value);
        if !self.task_slug_exists(project_id, &base) {
            return base;
        }

        let mut suffix = 2;
        loop {
            let candidate = format!("{base}-{suffix}");
            if !self.task_slug_exists(project_id, &candidate) {
                return candidate;
            }
            suffix += 1;
        }
    }

    fn task_slug_exists(&self, project_id: &ProjectId, slug: &str) -> bool {
        self.tasks
            .values()
            .any(|task| &task.project_id == project_id && task.slug == slug)
    }

    fn has_path(&self, start: &TaskId, target: &TaskId, kind: TaskEdgeKind) -> bool {
        let mut stack = vec![start.clone()];
        let mut visited = HashSet::new();
        while let Some(current) = stack.pop() {
            if !visited.insert(current.clone()) {
                continue;
            }
            if &current == target {
                return true;
            }
            for edge in self
                .task_edges
                .iter()
                .filter(|edge| edge.kind == kind && edge.from == current)
            {
                stack.push(edge.to.clone());
            }
        }
        false
    }

    fn ensure_dependencies_finished(&self, task_id: &TaskId) -> Result<(), String> {
        self.tasks
            .get(task_id)
            .ok_or_else(|| format!("task '{}' not found", task_id.0))?;

        for edge in self
            .task_edges
            .iter()
            .filter(|edge| edge.kind == TaskEdgeKind::DependsOn && &edge.from == task_id)
        {
            let dependency = self
                .tasks
                .get(&edge.to)
                .ok_or_else(|| format!("task '{}' not found", edge.to.0))?;
            if !dependency_status_allows_readiness(dependency.status) {
                return Err(format!(
                    "task '{}' is blocked by dependency '{}' that is not ready",
                    task_id.0, edge.to.0
                ));
            }
        }
        Ok(())
    }

    fn ensure_children_delivered(&self, task_id: &TaskId) -> Result<(), String> {
        self.tasks
            .get(task_id)
            .ok_or_else(|| format!("task '{}' not found", task_id.0))?;

        for edge in self
            .task_edges
            .iter()
            .filter(|edge| edge.kind == TaskEdgeKind::ParentOf && &edge.from == task_id)
        {
            let child = self
                .tasks
                .get(&edge.to)
                .ok_or_else(|| format!("task '{}' not found", edge.to.0))?;
            if !child_status_allows_parent_delivery(child) {
                return Err(format!(
                    "task '{}' cannot be delivered before child '{}' is delivered, canceled, or failed with a summary",
                    task_id.0, edge.to.0
                ));
            }
        }
        Ok(())
    }
}

fn empty_task_status_counts() -> BTreeMap<TaskStatus, usize> {
    TaskStatus::ALL
        .into_iter()
        .map(|status| (status, 0))
        .collect()
}

fn session_key(node_id: &NodeId, session: &SessionId) -> String {
    format!("{}:{}", node_id.0, session.0)
}

fn dependency_status_allows_readiness(status: TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Passed | TaskStatus::Delivered | TaskStatus::Canceled
    )
}

fn child_status_allows_parent_delivery(child: &Task) -> bool {
    matches!(child.status, TaskStatus::Delivered | TaskStatus::Canceled)
        || (child.status == TaskStatus::Failed
            && child
                .summary
                .as_deref()
                .is_some_and(|summary| !summary.trim().is_empty()))
}

fn edge_kind_rank(kind: TaskEdgeKind) -> u8 {
    match kind {
        TaskEdgeKind::ParentOf => 0,
        TaskEdgeKind::DependsOn => 1,
        TaskEdgeKind::Blocks => 2,
        TaskEdgeKind::Validates => 3,
        TaskEdgeKind::Audits => 4,
        TaskEdgeKind::Refines => 5,
        TaskEdgeKind::Supersedes => 6,
        TaskEdgeKind::Related => 7,
    }
}

fn sanitize_slug(value: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;

    for ch in value.chars() {
        let next = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else {
            Some('-')
        };

        if let Some(ch) = next {
            if ch == '-' {
                if !last_was_dash && !slug.is_empty() {
                    slug.push(ch);
                    last_was_dash = true;
                }
            } else {
                slug.push(ch);
                last_was_dash = false;
            }
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    if slug.is_empty() {
        "task".into()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{
        de::{
            value::{Error as ValueError, MapDeserializer},
            IntoDeserializer,
        },
        ser::{Error as SerError, Impossible, SerializeStruct},
        Serialize, Serializer,
    };
    use std::{error::Error, fmt};

    fn create_task(title: &str) -> CreateTask {
        CreateTask {
            project_id: ProjectId("project-1".into()),
            title: title.into(),
            objective: format!("Implement {title}"),
            scope: TaskScope::default(),
            agents: Vec::new(),
            gates: Vec::new(),
            slug: None,
        }
    }

    fn state_with_project() -> OrchestrationState {
        let mut state = OrchestrationState::new();
        state
            .create_project(
                CreateProject {
                    title: "Project".into(),
                    description: Some("Test project".into()),
                    slug: None,
                },
                1,
            )
            .unwrap();
        state
    }

    fn create_task_with_agent(title: &str, agent: TaskAgent) -> CreateTask {
        CreateTask {
            agents: vec![agent],
            ..create_task(title)
        }
    }

    fn agent() -> TaskAgent {
        TaskAgent {
            kind: "codex".into(),
            role: "implementation-worker".into(),
            skills: vec!["rust".into()],
            workspace_path: Some("/work".into()),
            objective: Some("implement the task".into()),
            prompt: "Do the focused task.".into(),
        }
    }

    fn participant() -> TaskParticipant {
        TaskParticipant {
            node_id: NodeId("node-a".into()),
            session: SessionId("session-a".into()),
            profile: "codex".into(),
            role: "owner".into(),
            kind: "codex".into(),
            skills: vec!["rust".into()],
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

    #[derive(Debug)]
    struct FieldNameError(String);

    impl fmt::Display for FieldNameError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl Error for FieldNameError {}

    impl SerError for FieldNameError {
        fn custom<T: fmt::Display>(msg: T) -> Self {
            Self(msg.to_string())
        }
    }

    struct FieldNameSerializer;

    struct FieldNameStruct {
        fields: Vec<String>,
    }

    impl SerializeStruct for FieldNameStruct {
        type Ok = Vec<String>;
        type Error = FieldNameError;

        fn serialize_field<T: ?Sized + Serialize>(
            &mut self,
            key: &'static str,
            _value: &T,
        ) -> Result<(), Self::Error> {
            self.fields.push(key.into());
            Ok(())
        }

        fn end(self) -> Result<Self::Ok, Self::Error> {
            Ok(self.fields)
        }
    }

    macro_rules! unsupported_serializer_method {
        ($name:ident($($arg:ident: $ty:ty),*) -> $ok:ty) => {
            fn $name(self, $($arg: $ty),*) -> Result<$ok, Self::Error> {
                $(let _ = $arg;)*
                Err(FieldNameError(format!(
                    "{} is not supported by FieldNameSerializer",
                    stringify!($name)
                )))
            }
        };
    }

    impl Serializer for FieldNameSerializer {
        type Ok = Vec<String>;
        type Error = FieldNameError;
        type SerializeSeq = Impossible<Vec<String>, FieldNameError>;
        type SerializeTuple = Impossible<Vec<String>, FieldNameError>;
        type SerializeTupleStruct = Impossible<Vec<String>, FieldNameError>;
        type SerializeTupleVariant = Impossible<Vec<String>, FieldNameError>;
        type SerializeMap = Impossible<Vec<String>, FieldNameError>;
        type SerializeStruct = FieldNameStruct;
        type SerializeStructVariant = Impossible<Vec<String>, FieldNameError>;

        unsupported_serializer_method!(serialize_bool(v: bool) -> Self::Ok);
        unsupported_serializer_method!(serialize_i8(v: i8) -> Self::Ok);
        unsupported_serializer_method!(serialize_i16(v: i16) -> Self::Ok);
        unsupported_serializer_method!(serialize_i32(v: i32) -> Self::Ok);
        unsupported_serializer_method!(serialize_i64(v: i64) -> Self::Ok);
        unsupported_serializer_method!(serialize_u8(v: u8) -> Self::Ok);
        unsupported_serializer_method!(serialize_u16(v: u16) -> Self::Ok);
        unsupported_serializer_method!(serialize_u32(v: u32) -> Self::Ok);
        unsupported_serializer_method!(serialize_u64(v: u64) -> Self::Ok);
        unsupported_serializer_method!(serialize_f32(v: f32) -> Self::Ok);
        unsupported_serializer_method!(serialize_f64(v: f64) -> Self::Ok);
        unsupported_serializer_method!(serialize_char(v: char) -> Self::Ok);
        unsupported_serializer_method!(serialize_str(v: &str) -> Self::Ok);
        unsupported_serializer_method!(serialize_bytes(v: &[u8]) -> Self::Ok);
        unsupported_serializer_method!(serialize_none() -> Self::Ok);
        unsupported_serializer_method!(serialize_unit() -> Self::Ok);
        unsupported_serializer_method!(serialize_unit_struct(name: &'static str) -> Self::Ok);
        unsupported_serializer_method!(serialize_unit_variant(name: &'static str, variant_index: u32, variant: &'static str) -> Self::Ok);
        unsupported_serializer_method!(serialize_seq(len: Option<usize>) -> Self::SerializeSeq);
        unsupported_serializer_method!(serialize_tuple(len: usize) -> Self::SerializeTuple);
        unsupported_serializer_method!(serialize_tuple_struct(name: &'static str, len: usize) -> Self::SerializeTupleStruct);
        unsupported_serializer_method!(serialize_tuple_variant(name: &'static str, variant_index: u32, variant: &'static str, len: usize) -> Self::SerializeTupleVariant);
        unsupported_serializer_method!(serialize_map(len: Option<usize>) -> Self::SerializeMap);
        unsupported_serializer_method!(serialize_struct_variant(name: &'static str, variant_index: u32, variant: &'static str, len: usize) -> Self::SerializeStructVariant);

        fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Self::Ok, Self::Error> {
            let _ = value;
            Err(FieldNameError(
                "serialize_some is not supported by FieldNameSerializer".into(),
            ))
        }

        fn serialize_newtype_struct<T: ?Sized + Serialize>(
            self,
            name: &'static str,
            value: &T,
        ) -> Result<Self::Ok, Self::Error> {
            let _ = (name, value);
            Err(FieldNameError(
                "serialize_newtype_struct is not supported by FieldNameSerializer".into(),
            ))
        }

        fn serialize_newtype_variant<T: ?Sized + Serialize>(
            self,
            name: &'static str,
            variant_index: u32,
            variant: &'static str,
            value: &T,
        ) -> Result<Self::Ok, Self::Error> {
            let _ = (name, variant_index, variant, value);
            Err(FieldNameError(
                "serialize_newtype_variant is not supported by FieldNameSerializer".into(),
            ))
        }

        fn serialize_struct(
            self,
            name: &'static str,
            len: usize,
        ) -> Result<Self::SerializeStruct, Self::Error> {
            let _ = name;
            Ok(FieldNameStruct {
                fields: Vec::with_capacity(len),
            })
        }
    }

    #[test]
    fn task_creation_sanitizes_slug_and_sets_timestamps() {
        let mut state = state_with_project();

        let task = state
            .create_task(create_task("  Ship Core Orchestration!! v1  "), 100)
            .unwrap();

        assert_eq!(task.id, TaskId("task-1".into()));
        assert_eq!(task.slug, "ship-core-orchestration-v1");
        assert_eq!(task.status, TaskStatus::Backlog);
        assert_eq!(task.created_at_ms, 100);
        assert_eq!(task.updated_at_ms, 100);
        assert_eq!(task.completed_at_ms, None);
    }

    #[test]
    fn duplicate_slugs_get_short_suffixes() {
        let mut state = state_with_project();

        let first = state.create_task(create_task("Core Task"), 100).unwrap();
        let second = state.create_task(create_task("Core Task"), 101).unwrap();
        let third = state.create_task(create_task("Core---Task"), 102).unwrap();

        assert_eq!(first.slug, "core-task");
        assert_eq!(second.slug, "core-task-2");
        assert_eq!(third.slug, "core-task-3");
    }

    #[test]
    fn task_agent_serialization_shape_has_only_v1_fields() {
        let fields = agent().serialize(FieldNameSerializer).unwrap();

        assert_eq!(
            fields,
            vec![
                "kind",
                "role",
                "skills",
                "workspace_path",
                "objective",
                "prompt"
            ]
        );
        assert!(!fields.contains(&"count".into()));
        assert!(!fields.contains(&"profile".into()));
        assert!(!fields.contains(&"node".into()));
        assert!(!fields.contains(&"bypass_permissions".into()));
    }

    #[test]
    fn task_creation_rejects_runtime_placement_fields_in_agent_input() {
        for forbidden_field in ["count", "profile", "node", "node_id", "bypass_permissions"] {
            let entries = vec![
                ("kind".into_deserializer(), "codex".into_deserializer()),
                (
                    "role".into_deserializer(),
                    "implementation-worker".into_deserializer(),
                ),
                ("prompt".into_deserializer(), "Do it.".into_deserializer()),
                (
                    forbidden_field.into_deserializer(),
                    "forbidden".into_deserializer(),
                ),
            ];
            let parsed =
                TaskAgent::deserialize(MapDeserializer::<_, ValueError>::new(entries.into_iter()));
            let err = parsed.expect_err("accepted forbidden field");
            assert!(
                err.to_string().contains(forbidden_field),
                "expected error for forbidden field {forbidden_field}, got {err}"
            );
        }
    }

    #[test]
    fn task_creation_keeps_explicit_intended_agents_without_counts() {
        let mut state = state_with_project();
        let task = state
            .create_task(create_task_with_agent("Agent Task", agent()), 100)
            .unwrap();

        assert_eq!(task.agents.len(), 1);
        assert_eq!(task.agents[0].kind, "codex");
        assert_eq!(task.agents[0].role, "implementation-worker");
        assert_eq!(task.agents[0].prompt, "Do the focused task.");
    }

    #[test]
    fn task_update_mutates_metadata_without_identity_runtime_or_status_changes() {
        let mut state = state_with_project();
        let mut input = create_task("Original Task");
        input.scope = TaskScope {
            include_paths: vec!["old.rs".into()],
            exclude_paths: vec!["target".into()],
            notes: Some("old notes".into()),
        };
        input.agents = vec![agent()];
        input.gates = vec!["old gate".into()];
        let task = state.create_task(input, 100).unwrap();
        let related = state.create_task(create_task("Related"), 101).unwrap();
        state
            .add_task_edge(
                CreateTaskEdge {
                    from: task.id.clone(),
                    to: related.id.clone(),
                    kind: TaskEdgeKind::Related,
                    note: Some("keep".into()),
                },
                120,
            )
            .unwrap();
        state
            .record_session(session(vec![task.id.clone()]), 130)
            .unwrap();
        state
            .update_task_status(&task.id, TaskStatus::Failed, 140)
            .unwrap();

        let original_edges = state.task_edges.clone();
        let original_sessions = state.sessions.clone();
        let updated = state
            .update_task(
                &task.id,
                UpdateTask {
                    title: Some("Renamed Task".into()),
                    objective: Some("New objective".into()),
                    scope: UpdateTaskScope {
                        include_paths: Some(vec!["new.rs".into()]),
                        exclude_paths: Some(Vec::new()),
                        notes: Some(Some("new notes".into())),
                    },
                    agents: Some(vec![TaskAgent {
                        role: "validator".into(),
                        ..agent()
                    }]),
                    gates: Some(vec!["new gate".into(), "second gate".into()]),
                },
                200,
            )
            .unwrap();

        assert_eq!(updated.id, task.id);
        assert_eq!(updated.slug, "original-task");
        assert_eq!(updated.title, "Renamed Task");
        assert_eq!(updated.objective, "New objective");
        assert_eq!(updated.scope.include_paths, vec!["new.rs"]);
        assert!(updated.scope.exclude_paths.is_empty());
        assert_eq!(updated.scope.notes.as_deref(), Some("new notes"));
        assert_eq!(updated.agents.len(), 1);
        assert_eq!(updated.agents[0].role, "validator");
        assert_eq!(updated.gates, vec!["new gate", "second gate"]);
        assert_eq!(updated.status, TaskStatus::Failed);
        assert_eq!(updated.completed_at_ms, Some(140));
        assert_eq!(updated.updated_at_ms, 200);
        assert_eq!(state.task_edges, original_edges);
        assert_eq!(state.sessions, original_sessions);
    }

    #[test]
    fn task_update_partial_scope_and_list_semantics() {
        let mut state = state_with_project();
        let mut input = create_task("Scoped Task");
        input.scope = TaskScope {
            include_paths: vec!["a.rs".into()],
            exclude_paths: vec!["target".into()],
            notes: Some("keep notes".into()),
        };
        input.agents = vec![agent()];
        input.gates = vec!["gate".into()];
        let task = state.create_task(input, 100).unwrap();

        let updated = state
            .update_task(
                &task.id,
                UpdateTask {
                    objective: Some("Only objective changes".into()),
                    ..UpdateTask::default()
                },
                110,
            )
            .unwrap();
        assert_eq!(updated.title, "Scoped Task");
        assert_eq!(updated.scope.include_paths, vec!["a.rs"]);
        assert_eq!(updated.scope.exclude_paths, vec!["target"]);
        assert_eq!(updated.scope.notes.as_deref(), Some("keep notes"));
        assert_eq!(updated.agents.len(), 1);
        assert_eq!(updated.gates, vec!["gate"]);

        let updated = state
            .update_task(
                &task.id,
                UpdateTask {
                    agents: Some(Vec::new()),
                    gates: Some(Vec::new()),
                    ..UpdateTask::default()
                },
                120,
            )
            .unwrap();
        assert!(updated.agents.is_empty());
        assert!(updated.gates.is_empty());
    }

    #[test]
    fn task_update_gate_changes_are_reflected_in_status_counts() {
        fn gate_count(state: &OrchestrationState, task_id: &TaskId) -> usize {
            state
                .orchestration_status(200)
                .tasks
                .into_iter()
                .find(|summary| &summary.id == task_id)
                .unwrap()
                .open_gate_count
        }

        let mut state = state_with_project();
        let task = state.create_task(create_task("Gate Count"), 100).unwrap();
        assert_eq!(gate_count(&state, &task.id), 0);

        state
            .update_task(
                &task.id,
                UpdateTask {
                    gates: Some(vec!["review".into(), "test".into()]),
                    ..UpdateTask::default()
                },
                110,
            )
            .unwrap();
        assert_eq!(gate_count(&state, &task.id), 2);

        state
            .update_task_status(&task.id, TaskStatus::Passed, 120)
            .unwrap();
        assert_eq!(gate_count(&state, &task.id), 0);
    }

    #[test]
    fn task_update_rejects_empty_title_and_objective() {
        let mut state = state_with_project();
        let task = state.create_task(create_task("Validated"), 100).unwrap();

        let error = state
            .update_task(
                &task.id,
                UpdateTask {
                    title: Some(" ".into()),
                    ..UpdateTask::default()
                },
                110,
            )
            .unwrap_err();
        assert!(error.contains("title"));

        let error = state
            .update_task(
                &task.id,
                UpdateTask {
                    objective: Some(" ".into()),
                    ..UpdateTask::default()
                },
                120,
            )
            .unwrap_err();
        assert!(error.contains("objective"));
    }

    #[test]
    fn assign_task_records_participant() {
        let mut state = state_with_project();
        let task = state.create_task(create_task("Assignable"), 100).unwrap();

        let assigned = state.assign_task(&task.id, participant(), 150).unwrap();

        assert_eq!(
            assigned.owner.unwrap().session,
            SessionId("session-a".into())
        );
        assert_eq!(assigned.updated_at_ms, 150);
    }

    #[test]
    fn record_session_is_keyed_by_node_and_session() {
        let mut state = state_with_project();
        let task = state.create_task(create_task("Session Task"), 100).unwrap();
        let recorded = state.record_session(session(vec![task.id]), 200).unwrap();

        assert_eq!(recorded.created_at_ms, 200);
        assert_eq!(recorded.updated_at_ms, 200);
        assert_eq!(recorded.last_seen_ms, 200);
        assert!(state.sessions.contains_key("node-a:session-a"));
    }

    #[test]
    fn record_session_preserves_backend_owned_workspace_path_exactly() {
        let mut state = state_with_project();
        let task = state
            .create_task(create_task("Backend Workspace"), 100)
            .unwrap();
        let backend_workspace = "/__backend_only__/project/../project";
        let mut record = session(vec![task.id]);
        record.workspace_path = Some(backend_workspace.into());

        let recorded = state.record_session(record, 200).unwrap();

        assert_eq!(recorded.workspace_path.as_deref(), Some(backend_workspace));
        assert_eq!(
            state
                .sessions
                .get("node-a:session-a")
                .unwrap()
                .workspace_path
                .as_deref(),
            Some(backend_workspace)
        );
    }

    #[test]
    fn edge_creation_validates_endpoints() {
        let mut state = state_with_project();
        let task = state.create_task(create_task("Existing"), 100).unwrap();

        let err = state
            .add_task_edge(
                CreateTaskEdge {
                    from: task.id,
                    to: TaskId("missing".into()),
                    kind: TaskEdgeKind::DependsOn,
                    note: None,
                },
                200,
            )
            .unwrap_err();

        assert!(err.contains("missing"));
    }

    #[test]
    fn depends_on_edges_reject_cycles() {
        let mut state = state_with_project();
        let a = state.create_task(create_task("A"), 100).unwrap();
        let b = state.create_task(create_task("B"), 100).unwrap();
        let c = state.create_task(create_task("C"), 100).unwrap();

        state
            .add_task_edge(
                CreateTaskEdge {
                    from: a.id.clone(),
                    to: b.id.clone(),
                    kind: TaskEdgeKind::DependsOn,
                    note: None,
                },
                200,
            )
            .unwrap();
        state
            .add_task_edge(
                CreateTaskEdge {
                    from: b.id.clone(),
                    to: c.id.clone(),
                    kind: TaskEdgeKind::DependsOn,
                    note: None,
                },
                201,
            )
            .unwrap();

        let err = state
            .add_task_edge(
                CreateTaskEdge {
                    from: c.id,
                    to: a.id,
                    kind: TaskEdgeKind::DependsOn,
                    note: None,
                },
                202,
            )
            .unwrap_err();
        assert!(err.contains("cycle"));
    }

    #[test]
    fn parent_of_edges_reject_multiple_parents() {
        let mut state = state_with_project();
        let parent_a = state.create_task(create_task("Parent A"), 100).unwrap();
        let parent_b = state.create_task(create_task("Parent B"), 100).unwrap();
        let child = state.create_task(create_task("Child"), 100).unwrap();

        state
            .add_task_edge(
                CreateTaskEdge {
                    from: parent_a.id,
                    to: child.id.clone(),
                    kind: TaskEdgeKind::ParentOf,
                    note: None,
                },
                200,
            )
            .unwrap();
        let err = state
            .add_task_edge(
                CreateTaskEdge {
                    from: parent_b.id,
                    to: child.id,
                    kind: TaskEdgeKind::ParentOf,
                    note: None,
                },
                201,
            )
            .unwrap_err();

        assert!(err.contains("one parent"));
    }

    #[test]
    fn dependencies_block_ready_until_readiness_allowed_status() {
        let mut state = state_with_project();
        let task = state.create_task(create_task("Task"), 100).unwrap();
        let dependency = state.create_task(create_task("Dependency"), 100).unwrap();

        state
            .add_task_edge(
                CreateTaskEdge {
                    from: task.id.clone(),
                    to: dependency.id.clone(),
                    kind: TaskEdgeKind::DependsOn,
                    note: None,
                },
                200,
            )
            .unwrap();

        let err = state
            .update_task_status(&task.id, TaskStatus::Planned, 250)
            .unwrap_err();
        assert!(err.contains("not ready"));

        state
            .update_task_status(&dependency.id, TaskStatus::Passed, 300)
            .unwrap();
        let ready = state
            .update_task_status(&task.id, TaskStatus::Planned, 350)
            .unwrap();
        assert_eq!(ready.status, TaskStatus::Planned);
    }

    #[test]
    fn parent_children_block_delivery_but_not_running() {
        let mut state = state_with_project();
        let parent = state.create_task(create_task("Parent"), 100).unwrap();
        let child = state.create_task(create_task("Child"), 100).unwrap();

        state
            .add_task_edge(
                CreateTaskEdge {
                    from: parent.id.clone(),
                    to: child.id.clone(),
                    kind: TaskEdgeKind::ParentOf,
                    note: None,
                },
                200,
            )
            .unwrap();

        let running = state
            .update_task_status(&parent.id, TaskStatus::Running, 250)
            .unwrap();
        assert_eq!(running.status, TaskStatus::Running);

        let err = state
            .update_task_status(&parent.id, TaskStatus::Delivered, 300)
            .unwrap_err();
        assert!(err.contains("before child"));

        state
            .update_task_status(&child.id, TaskStatus::Delivered, 350)
            .unwrap();
        let delivered = state
            .update_task_status(&parent.id, TaskStatus::Delivered, 400)
            .unwrap();
        assert_eq!(delivered.status, TaskStatus::Delivered);
        assert_eq!(delivered.completed_at_ms, Some(400));
    }

    #[test]
    fn parent_delivery_allows_canceled_or_failed_child_with_summary() {
        let mut state = state_with_project();
        let canceled_parent = state
            .create_task(create_task("Canceled Parent"), 100)
            .unwrap();
        let canceled_child = state
            .create_task(create_task("Canceled Child"), 100)
            .unwrap();
        let failed_parent = state
            .create_task(create_task("Failed Parent"), 100)
            .unwrap();
        let failed_child = state.create_task(create_task("Failed Child"), 100).unwrap();

        for (parent, child) in [
            (&canceled_parent, &canceled_child),
            (&failed_parent, &failed_child),
        ] {
            state
                .add_task_edge(
                    CreateTaskEdge {
                        from: parent.id.clone(),
                        to: child.id.clone(),
                        kind: TaskEdgeKind::ParentOf,
                        note: None,
                    },
                    200,
                )
                .unwrap();
        }

        state
            .update_task_status(&canceled_child.id, TaskStatus::Canceled, 250)
            .unwrap();
        let delivered = state
            .update_task_status(&canceled_parent.id, TaskStatus::Delivered, 300)
            .unwrap();
        assert_eq!(delivered.status, TaskStatus::Delivered);

        state
            .update_task_status(&failed_child.id, TaskStatus::Failed, 350)
            .unwrap();
        let err = state
            .update_task_status(&failed_parent.id, TaskStatus::Delivered, 400)
            .unwrap_err();
        assert!(err.contains("failed with a summary"));

        state.tasks.get_mut(&failed_child.id).unwrap().summary =
            Some("Operator accepted the failed child outcome.".into());
        let delivered = state
            .update_task_status(&failed_parent.id, TaskStatus::Delivered, 450)
            .unwrap();
        assert_eq!(delivered.status, TaskStatus::Delivered);
    }

    #[test]
    fn status_updates_set_finished_timestamps() {
        let mut state = state_with_project();
        let task = state.create_task(create_task("Status Task"), 100).unwrap();

        let running = state
            .update_task_status(&task.id, TaskStatus::Running, 200)
            .unwrap();
        assert_eq!(running.updated_at_ms, 200);
        assert_eq!(running.completed_at_ms, None);

        let failed = state
            .update_task_status(&task.id, TaskStatus::Failed, 250)
            .unwrap();
        assert_eq!(failed.updated_at_ms, 250);
        assert_eq!(failed.completed_at_ms, Some(250));

        let blocked = state
            .update_task_status(&task.id, TaskStatus::Blocked, 300)
            .unwrap();
        assert_eq!(blocked.completed_at_ms, None);
    }

    #[test]
    fn orchestration_status_counts_active_and_finished_tasks() {
        let mut state = state_with_project();
        let backlog = state.create_task(create_task("Backlog"), 100).unwrap();
        let running = state.create_task(create_task("Running"), 100).unwrap();
        let waiting = state.create_task(create_task("Waiting"), 100).unwrap();
        let blocked = state.create_task(create_task("Blocked"), 100).unwrap();
        let passed = state.create_task(create_task("Passed"), 100).unwrap();
        let delivered = state.create_task(create_task("Delivered"), 100).unwrap();
        let failed = state.create_task(create_task("Failed"), 100).unwrap();
        let canceled = state.create_task(create_task("Canceled"), 100).unwrap();

        state
            .update_task_status(&running.id, TaskStatus::Running, 200)
            .unwrap();
        state
            .update_task_status(&waiting.id, TaskStatus::WaitingForValidation, 201)
            .unwrap();
        state
            .update_task_status(&blocked.id, TaskStatus::Blocked, 202)
            .unwrap();
        state
            .update_task_status(&passed.id, TaskStatus::Passed, 203)
            .unwrap();
        state
            .update_task_status(&delivered.id, TaskStatus::Delivered, 204)
            .unwrap();
        state
            .update_task_status(&failed.id, TaskStatus::Failed, 205)
            .unwrap();
        state
            .update_task_status(&canceled.id, TaskStatus::Canceled, 206)
            .unwrap();

        let status = state.orchestration_status(300);

        assert_eq!(status.counts.total_tasks, 8);
        assert_eq!(status.counts.active_tasks, 5);
        assert_eq!(status.counts.blocked_tasks, 1);
        assert_eq!(status.counts.waiting_for_validation_tasks, 1);
        assert_eq!(status.counts.passed_tasks, 1);
        assert_eq!(status.counts.delivered_tasks, 1);
        assert_eq!(status.counts.failed_tasks, 1);
        assert_eq!(status.counts.canceled_tasks, 1);
        assert!(status
            .tasks
            .iter()
            .any(|summary| summary.id == backlog.id && summary.status == TaskStatus::Backlog));
        let project = status.projects.first().expect("project summary");
        assert_eq!(project.task_count, 8);
        assert_eq!(project.active_task_count, 5);
        assert_eq!(project.task_status_counts.len(), TaskStatus::ALL.len());
        assert_eq!(project.task_status_counts[&TaskStatus::Backlog], 1);
        assert_eq!(project.task_status_counts[&TaskStatus::Planned], 0);
        assert_eq!(project.task_status_counts[&TaskStatus::Running], 1);
        assert_eq!(
            project.task_status_counts[&TaskStatus::WaitingForValidation],
            1
        );
        assert_eq!(project.task_status_counts[&TaskStatus::Blocked], 1);
        assert_eq!(project.task_status_counts[&TaskStatus::Passed], 1);
        assert_eq!(project.task_status_counts[&TaskStatus::Delivered], 1);
        assert_eq!(project.task_status_counts[&TaskStatus::Failed], 1);
        assert_eq!(project.task_status_counts[&TaskStatus::Canceled], 1);
    }

    #[test]
    fn project_summaries_include_zero_counts_for_every_task_status() {
        let state = state_with_project();

        let status = state.orchestration_status(300);

        let project = status.projects.first().expect("project summary");
        assert_eq!(project.task_count, 0);
        assert_eq!(project.active_task_count, 0);
        assert_eq!(project.task_status_counts.len(), TaskStatus::ALL.len());
        for task_status in TaskStatus::ALL {
            assert_eq!(project.task_status_counts[&task_status], 0);
        }
    }

    #[test]
    fn orchestration_status_calculates_blocked_by_from_depends_on() {
        let mut state = state_with_project();
        let task = state.create_task(create_task("Task"), 100).unwrap();
        let dependency = state.create_task(create_task("Dependency"), 100).unwrap();

        state
            .add_task_edge(
                CreateTaskEdge {
                    from: task.id.clone(),
                    to: dependency.id.clone(),
                    kind: TaskEdgeKind::DependsOn,
                    note: None,
                },
                200,
            )
            .unwrap();

        let status = state.orchestration_status(300);
        let summary = status
            .tasks
            .iter()
            .find(|summary| summary.id == task.id)
            .unwrap();
        assert_eq!(summary.dependency_count, 1);
        assert_eq!(summary.blocked_by, vec![dependency.id.clone()]);
        assert_eq!(status.counts.blocked_tasks, 1);

        state
            .update_task_status(&dependency.id, TaskStatus::Passed, 350)
            .unwrap();
        let status = state.orchestration_status(400);
        let summary = status
            .tasks
            .iter()
            .find(|summary| summary.id == task.id)
            .unwrap();
        assert!(summary.blocked_by.is_empty());
        assert_eq!(status.counts.blocked_tasks, 0);
    }

    #[test]
    fn orchestration_status_summarizes_parent_and_children() {
        let mut state = state_with_project();
        let parent = state.create_task(create_task("Parent"), 100).unwrap();
        let child_a = state.create_task(create_task("Child A"), 100).unwrap();
        let child_b = state.create_task(create_task("Child B"), 100).unwrap();

        for child in [&child_b, &child_a] {
            state
                .add_task_edge(
                    CreateTaskEdge {
                        from: parent.id.clone(),
                        to: child.id.clone(),
                        kind: TaskEdgeKind::ParentOf,
                        note: None,
                    },
                    200,
                )
                .unwrap();
        }

        let status = state.orchestration_status(300);
        let parent_summary = status
            .tasks
            .iter()
            .find(|summary| summary.id == parent.id)
            .unwrap();
        let child_summary = status
            .tasks
            .iter()
            .find(|summary| summary.id == child_a.id)
            .unwrap();

        assert_eq!(parent_summary.child_count, 2);
        assert_eq!(child_summary.parent, Some(parent.id));
    }

    #[test]
    fn orchestration_status_counts_gates_by_lifecycle_state() {
        let mut state = state_with_project();
        let open = state
            .create_task(
                CreateTask {
                    gates: vec!["review".into(), "test".into()],
                    ..create_task("Open Gates")
                },
                100,
            )
            .unwrap();
        let passed = state
            .create_task(
                CreateTask {
                    gates: vec!["review".into()],
                    ..create_task("Passed Gates")
                },
                100,
            )
            .unwrap();
        let delivered = state
            .create_task(
                CreateTask {
                    gates: vec!["review".into()],
                    ..create_task("Delivered Gates")
                },
                100,
            )
            .unwrap();
        let blocked = state
            .create_task(
                CreateTask {
                    gates: vec!["review".into(), "test".into()],
                    ..create_task("Blocked Gates")
                },
                100,
            )
            .unwrap();

        state
            .update_task_status(&passed.id, TaskStatus::Passed, 200)
            .unwrap();
        state
            .update_task_status(&delivered.id, TaskStatus::Delivered, 201)
            .unwrap();
        state
            .update_task_status(&blocked.id, TaskStatus::Blocked, 202)
            .unwrap();
        state.tasks.get_mut(&blocked.id).unwrap().blockers = vec!["tests failed".into()];

        let status = state.orchestration_status(300);
        let by_id = |task_id: &TaskId| {
            status
                .tasks
                .iter()
                .find(|summary| &summary.id == task_id)
                .unwrap()
        };

        assert_eq!(by_id(&open.id).open_gate_count, 2);
        assert_eq!(by_id(&open.id).failed_gate_count, 0);
        assert_eq!(by_id(&passed.id).open_gate_count, 0);
        assert_eq!(by_id(&delivered.id).open_gate_count, 0);
        assert_eq!(by_id(&blocked.id).open_gate_count, 2);
        assert_eq!(by_id(&blocked.id).failed_gate_count, 1);
        assert_eq!(by_id(&blocked.id).blocker_count, 1);
        assert_eq!(by_id(&blocked.id).blockers, vec!["tests failed"]);
    }

    #[test]
    fn session_summary_serialization_shape_and_values_are_compact() {
        let mut state = state_with_project();
        let task = state.create_task(create_task("Session Task"), 100).unwrap();
        let mut session = session(vec![task.id.clone()]);
        session.bypass_permissions = true;
        state.record_session(session, 200).unwrap();

        let status = state.orchestration_status(300);
        let summary = status.sessions.first().unwrap();
        let fields = summary.serialize(FieldNameSerializer).unwrap();

        assert_eq!(
            fields,
            vec![
                "node_id",
                "session",
                "profile",
                "workspace_path",
                "bypass_permissions",
                "task_ids",
                "role",
                "kind",
                "objective",
                "last_seen_ms",
                "runtime_state"
            ]
        );
        assert_eq!(summary.node_id, "node-a");
        assert_eq!(summary.session, "session-a");
        assert_eq!(summary.profile, "codex");
        assert_eq!(summary.workspace_path, Some("/workspace/project".into()));
        assert!(summary.bypass_permissions);
        assert_eq!(summary.task_ids, vec![task.id]);
        assert_eq!(summary.last_seen_ms, Some(200));
        assert_eq!(summary.runtime_state, None);
    }

    #[test]
    fn cleanup_candidate_serialization_shape_is_stable() {
        let candidate = SessionCleanupCandidate {
            node_id: "node-a".into(),
            session: "session-a".into(),
            reason: "stale".into(),
            created_at_ms: Some(100),
        };

        let fields = candidate.serialize(FieldNameSerializer).unwrap();

        assert_eq!(
            fields,
            vec!["node_id", "session", "reason", "created_at_ms"]
        );
        assert!(!fields.iter().any(|field| field == "last_seen_ms"));
    }

    #[test]
    fn orchestration_status_output_ordering_is_stable() {
        let mut state = state_with_project();
        let first = state.create_task(create_task("First"), 100).unwrap();
        let second = state.create_task(create_task("Second"), 100).unwrap();
        let third = state.create_task(create_task("Third"), 100).unwrap();

        state
            .add_task_edge(
                CreateTaskEdge {
                    from: third.id.clone(),
                    to: first.id.clone(),
                    kind: TaskEdgeKind::DependsOn,
                    note: None,
                },
                200,
            )
            .unwrap();
        state
            .add_task_edge(
                CreateTaskEdge {
                    from: second.id.clone(),
                    to: first.id.clone(),
                    kind: TaskEdgeKind::ParentOf,
                    note: None,
                },
                201,
            )
            .unwrap();

        let mut session_b = session(vec![third.id.clone(), first.id.clone()]);
        session_b.node_id = NodeId("node-b".into());
        session_b.session = SessionId("session-b".into());
        state.record_session(session_b, 250).unwrap();
        let mut session_a = session(vec![second.id.clone()]);
        session_a.node_id = NodeId("node-a".into());
        session_a.session = SessionId("session-a".into());
        state.record_session(session_a, 251).unwrap();

        let status = state.orchestration_status(300);

        assert_eq!(
            status
                .tasks
                .iter()
                .map(|summary| summary.id.0.as_str())
                .collect::<Vec<_>>(),
            vec!["task-1", "task-2", "task-3"]
        );
        assert_eq!(
            status
                .task_edges
                .iter()
                .map(|edge| (edge.from.0.as_str(), edge.to.0.as_str(), edge.kind))
                .collect::<Vec<_>>(),
            vec![
                ("task-2", "task-1", TaskEdgeKind::ParentOf),
                ("task-3", "task-1", TaskEdgeKind::DependsOn)
            ]
        );
        assert_eq!(
            status
                .sessions
                .iter()
                .map(|summary| (summary.node_id.as_str(), summary.session.as_str()))
                .collect::<Vec<_>>(),
            vec![("node-a", "session-a"), ("node-b", "session-b")]
        );
        assert!(status.cleanup_candidates.is_empty());
        assert_eq!(status.counts.cleanup_candidates, 0);
    }

    #[test]
    fn orchestration_status_reports_suspicious_deserialized_state() {
        let mut state = state_with_project();
        let task = state.create_task(create_task("Task"), 100).unwrap();
        state.task_edges.push(TaskEdge {
            from: task.id.clone(),
            to: TaskId("missing".into()),
            kind: TaskEdgeKind::DependsOn,
            created_at_ms: 200,
            note: None,
        });
        let mut stored_session = session(vec![TaskId("missing".into())]);
        stored_session.node_id = NodeId("node-real".into());
        stored_session.session = SessionId("session-real".into());
        stored_session.created_at_ms = 100;
        stored_session.updated_at_ms = 100;
        stored_session.last_seen_ms = 100;
        state.sessions.insert("wrong:key".into(), stored_session);

        let status = state.orchestration_status(300);

        assert!(status
            .warnings
            .iter()
            .any(|warning| warning.contains("missing to task 'missing'")));
        assert!(status
            .warnings
            .iter()
            .any(|warning| warning.contains("does not match node/session")));
        assert!(status
            .warnings
            .iter()
            .any(|warning| warning.contains("references missing task 'missing'")));
    }
}
