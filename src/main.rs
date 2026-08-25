use std::collections::HashSet;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let mut args: Vec<OsString> = std::env::args_os().collect();
    if args.len() == 1
        || matches!(
            args.get(1).and_then(|arg| arg.to_str()),
            Some("-h" | "--help")
        )
    {
        print_root_help();
        return;
    }

    match args.get(1).and_then(|arg| arg.to_str()) {
        Some("tmux") => {
            std::process::exit(run_tmux_proxy(&args[2..]));
        }
        Some("attach") => {
            std::process::exit(run_attach_proxy(&args[2..]));
        }
        Some("list-projects") => {
            std::process::exit(run_list_projects(None));
        }
        Some("create-project") => {
            std::process::exit(run_create_project(None, &args[2..]));
        }
        Some("delete-project") => {
            std::process::exit(run_delete_project(None, &args[2..]));
        }
        Some("prune") => {
            std::process::exit(run_prune(None, &args[2..]));
        }
        Some("--store-path") if args.get(3).and_then(|arg| arg.to_str()) == Some("tmux") => {
            std::process::exit(run_tmux_proxy_with_store(
                args.get(2).map(PathBuf::from),
                &args[4..],
            ));
        }
        Some("--store-path") if args.get(3).and_then(|arg| arg.to_str()) == Some("attach") => {
            std::process::exit(run_attach_proxy_with_store(
                args.get(2).map(PathBuf::from),
                &args[4..],
            ));
        }
        Some("--store-path")
            if args.get(3).and_then(|arg| arg.to_str()) == Some("list-projects") =>
        {
            std::process::exit(run_list_projects(args.get(2).map(PathBuf::from)));
        }
        Some("--store-path")
            if args.get(3).and_then(|arg| arg.to_str()) == Some("create-project") =>
        {
            std::process::exit(run_create_project(
                args.get(2).map(PathBuf::from),
                &args[4..],
            ));
        }
        Some("--store-path")
            if args.get(3).and_then(|arg| arg.to_str()) == Some("delete-project") =>
        {
            std::process::exit(run_delete_project(
                args.get(2).map(PathBuf::from),
                &args[4..],
            ));
        }
        Some("--store-path") if args.get(3).and_then(|arg| arg.to_str()) == Some("prune") => {
            std::process::exit(run_prune(args.get(2).map(PathBuf::from), &args[4..]));
        }
        Some(value) if value.starts_with("--store-path=") => {
            let store_path = PathBuf::from(value.trim_start_matches("--store-path="));
            match args.get(2).and_then(|arg| arg.to_str()) {
                Some("tmux") => {
                    std::process::exit(run_tmux_proxy_with_store(Some(store_path), &args[3..]))
                }
                Some("attach") => {
                    std::process::exit(run_attach_proxy_with_store(Some(store_path), &args[3..]))
                }
                Some("list-projects") => std::process::exit(run_list_projects(Some(store_path))),
                Some("create-project") => {
                    std::process::exit(run_create_project(Some(store_path), &args[3..]))
                }
                Some("delete-project") => {
                    std::process::exit(run_delete_project(Some(store_path), &args[3..]))
                }
                Some("prune") => std::process::exit(run_prune(Some(store_path), &args[3..])),
                _ => {}
            }
        }
        Some("controller") => {
            args.remove(1);
            mmux_controller::main_entry_from(args);
        }
        Some("node") => {
            args.remove(1);
            mmux_node::main_entry_from(args);
        }
        _ => {
            mmux_controller::main_entry_from(args);
        }
    }
}

fn print_root_help() {
    if let Err(error) = mmux_controller::print_help() {
        eprintln!("failed to print help: {error}");
        std::process::exit(1);
    }
    println!("Root commands:");
    println!("  controller              Run the MCP controller explicitly");
    println!("  node                    Run a standalone execution node");
    println!("  tmux -- <args>          Run tmux against mmux's private local-node socket");
    println!("  attach [--read-only] <session>  Attach to a private local-node tmux session");
    println!(
        "  create-project <title> --description <text>  Create a durable orchestration project in mmux.db"
    );
    println!(
        "  delete-project <id-or-slug>  Delete an orchestration project and its plans/tasks from mmux.db"
    );
    println!("  list-projects           List durable orchestration projects from mmux.db");
    println!("  prune                   Prune orchestration-owned live sessions, stale session records, and finished plans");
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CreateProjectArgs {
    title: String,
    description: String,
    slug: Option<String>,
}

fn run_create_project(store_path: Option<PathBuf>, raw_args: &[OsString]) -> i32 {
    if raw_args
        .iter()
        .any(|arg| matches!(arg.to_str(), Some("-h" | "--help")))
    {
        print_create_project_help();
        return 0;
    }
    let args = match parse_create_project_args(raw_args) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("mmux create-project: {error}");
            return 2;
        }
    };
    let store_path = match mmux_node::resolve_store_path(store_path.as_deref()) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("mmux create-project: {error}");
            return 2;
        }
    };
    let project = match mmux_controller::local_create_project(
        Some(&store_path),
        args.title,
        args.description,
        args.slug,
    ) {
        Ok(project) => project,
        Err(error) => {
            eprintln!("mmux create-project: {error}");
            return 1;
        }
    };
    println!(
        "{}\t{}\t{}\t{}/{}\t{}",
        project.id,
        project.slug,
        project.status,
        project.active_task_count,
        project.task_count,
        project.title
    );
    0
}

fn print_create_project_help() {
    println!("usage: mmux create-project <title> --description <text> [--slug <slug>]");
    println!();
    println!("Creates a durable orchestration project in mmux.db.");
}

fn parse_create_project_args(raw_args: &[OsString]) -> Result<CreateProjectArgs, String> {
    let mut title = None;
    let mut description = None;
    let mut slug = None;
    let mut index = 0;

    while index < raw_args.len() {
        let text = raw_args[index]
            .to_str()
            .ok_or_else(|| "arguments must be valid UTF-8".to_owned())?;
        match text {
            "--description" => {
                let value = raw_args
                    .get(index + 1)
                    .ok_or_else(|| "--description requires a value".to_owned())?
                    .to_str()
                    .ok_or_else(|| "--description value must be valid UTF-8".to_owned())?;
                description = Some(value.to_owned());
                index += 2;
            }
            "--slug" => {
                let value = raw_args
                    .get(index + 1)
                    .ok_or_else(|| "--slug requires a value".to_owned())?
                    .to_str()
                    .ok_or_else(|| "--slug value must be valid UTF-8".to_owned())?;
                slug = Some(value.to_owned());
                index += 2;
            }
            _ => {
                if let Some(value) = text.strip_prefix("--description=") {
                    description = Some(value.to_owned());
                    index += 1;
                } else if let Some(value) = text.strip_prefix("--slug=") {
                    slug = Some(value.to_owned());
                    index += 1;
                } else if text.starts_with('-') {
                    return Err(format!("unknown argument '{text}'"));
                } else if title.is_some() {
                    return Err("project title may only be provided once".into());
                } else {
                    title = Some(text.to_owned());
                    index += 1;
                }
            }
        }
    }

    let title = title.ok_or_else(|| "project title is required".to_owned())?;
    if title.trim().is_empty() {
        return Err("project title must not be empty".into());
    }
    let description = description.ok_or_else(|| "project description is required".to_owned())?;
    if description.trim().is_empty() {
        return Err("project description must not be empty".into());
    }
    Ok(CreateProjectArgs {
        title,
        description,
        slug,
    })
}

fn run_list_projects(store_path: Option<PathBuf>) -> i32 {
    let store_path = match mmux_node::resolve_store_path(store_path.as_deref()) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("mmux list-projects: {error}");
            return 2;
        }
    };
    let projects = match mmux_controller::local_projects(Some(&store_path)) {
        Ok(projects) => projects,
        Err(error) => {
            eprintln!("mmux list-projects: {error}");
            return 1;
        }
    };
    for project in projects {
        println!(
            "{}\t{}\t{}\t{}/{}\t{}",
            project.id,
            project.slug,
            project.status,
            project.active_task_count,
            project.task_count,
            project.title
        );
    }
    0
}

fn run_delete_project(store_path: Option<PathBuf>, raw_args: &[OsString]) -> i32 {
    if raw_args
        .iter()
        .any(|arg| matches!(arg.to_str(), Some("-h" | "--help")))
    {
        print_delete_project_help();
        return 0;
    }
    let project_id_or_slug = match parse_delete_project_args(raw_args) {
        Ok(project_id_or_slug) => project_id_or_slug,
        Err(error) => {
            eprintln!("mmux delete-project: {error}");
            return 2;
        }
    };
    let store_path = match mmux_node::resolve_store_path(store_path.as_deref()) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("mmux delete-project: {error}");
            return 2;
        }
    };
    let report = match mmux_controller::local_delete_project(Some(&store_path), project_id_or_slug)
    {
        Ok(report) => report,
        Err(error) => {
            eprintln!("mmux delete-project: {error}");
            return 1;
        }
    };
    let project = report.project;
    println!(
        "deleted\t{}\t{}\t{}\t{}/{}\tplans={}\ttasks={}\tedges={}\t{}",
        project.id,
        project.slug,
        project.status,
        project.active_task_count,
        project.task_count,
        report.deleted_plan_count,
        report.deleted_task_count,
        report.deleted_edge_count,
        project.title
    );
    0
}

fn print_delete_project_help() {
    println!("usage: mmux delete-project <id-or-slug>");
    println!();
    println!("Deletes an orchestration project from mmux.db.");
    println!("Also deletes all plans, task cards, task sessions, and task edges in that project.");
}

fn parse_delete_project_args(raw_args: &[OsString]) -> Result<String, String> {
    let mut project_id_or_slug = None;
    for arg in raw_args {
        let text = arg
            .to_str()
            .ok_or_else(|| "arguments must be valid UTF-8".to_owned())?;
        if text.starts_with('-') {
            return Err(format!("unknown argument '{text}'"));
        }
        if project_id_or_slug.is_some() {
            return Err("project id or slug may only be provided once".into());
        }
        project_id_or_slug = Some(text.to_owned());
    }
    let project_id_or_slug =
        project_id_or_slug.ok_or_else(|| "project id or slug is required".to_owned())?;
    if project_id_or_slug.trim().is_empty() {
        return Err("project id or slug must not be empty".into());
    }
    Ok(project_id_or_slug)
}

const DEFAULT_PRUNE_OLDER_THAN_DAYS: u64 = 14;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PruneInclude {
    live_untracked_sessions: bool,
    stale_session_records: bool,
    finished_plans: bool,
    tracked_finished_sessions: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PruneArgs {
    dry_run: bool,
    older_than_days: u64,
    include: PruneInclude,
}

fn run_prune(store_path: Option<PathBuf>, raw_args: &[OsString]) -> i32 {
    if raw_args
        .iter()
        .any(|arg| matches!(arg.to_str(), Some("-h" | "--help")))
    {
        print_prune_help();
        return 0;
    }
    let args = match parse_prune_args(raw_args) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("mmux prune: {error}");
            return 2;
        }
    };
    let store_path = match mmux_node::resolve_store_path(store_path.as_deref()) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("mmux prune: {error}");
            return 2;
        }
    };
    let live_session_infos = match live_local_tmux_session_infos(&store_path) {
        Ok(sessions) => sessions,
        Err(error) => {
            eprintln!("mmux prune: {error}");
            return 1;
        }
    };
    let live_local_sessions = live_session_infos
        .iter()
        .map(|session| session.name.clone())
        .collect::<HashSet<_>>();
    let durable_session_names =
        match mmux_controller::local_orchestration_session_names(Some(&store_path)) {
            Ok(sessions) => sessions,
            Err(error) => {
                eprintln!("mmux prune: {error}");
                return 1;
            }
        };
    let finished_session_names = match mmux_controller::local_finished_orchestration_session_names(
        Some(&store_path),
        args.older_than_days,
    ) {
        Ok(sessions) => sessions,
        Err(error) => {
            eprintln!("mmux prune: {error}");
            return 1;
        }
    };
    let live_untracked_sessions = if args.include.live_untracked_sessions {
        prune_live_untracked_session_names(
            &live_session_infos,
            &durable_session_names,
            args.older_than_days,
        )
    } else {
        Vec::new()
    };
    let tracked_finished_sessions = if args.include.tracked_finished_sessions {
        let mut sessions = live_local_sessions
            .intersection(&finished_session_names)
            .cloned()
            .collect::<Vec<_>>();
        sessions.sort();
        sessions
    } else {
        Vec::new()
    };
    let mut killed_live_untracked_sessions = Vec::new();
    let mut killed_tracked_finished_sessions = Vec::new();
    if !args.dry_run {
        for session in &live_untracked_sessions {
            match kill_local_tmux_session(&store_path, session) {
                Ok(()) => killed_live_untracked_sessions.push(session.clone()),
                Err(error) => eprintln!("mmux prune: failed to kill '{session}': {error}"),
            }
        }
        for session in &tracked_finished_sessions {
            match kill_local_tmux_session(&store_path, session) {
                Ok(()) => killed_tracked_finished_sessions.push(session.clone()),
                Err(error) => eprintln!("mmux prune: failed to kill '{session}': {error}"),
            }
        }
    }
    let mut live_sessions_for_store = live_local_sessions;
    if args.include.tracked_finished_sessions && args.include.stale_session_records {
        for session in &tracked_finished_sessions {
            live_sessions_for_store.remove(session);
        }
    }
    let report = match mmux_controller::local_prune_store(
        Some(&store_path),
        &live_sessions_for_store,
        args.dry_run,
        args.include.stale_session_records,
        args.include.finished_plans,
        Some(args.older_than_days),
    ) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("mmux prune: {error}");
            return 1;
        }
    };
    if report.dry_run {
        println!(
            "dry-run: would kill {} untracked live session(s), kill {} tracked finished session(s), prune {} stale session record(s), and prune {} finished plan(s)",
            live_untracked_sessions.len(),
            tracked_finished_sessions.len(),
            report.pruned_session_count,
            report.pruned_plan_count
        );
    } else {
        println!(
            "killed {} untracked live session(s), killed {} tracked finished session(s), pruned {} stale session record(s), and pruned {} finished plan(s)",
            killed_live_untracked_sessions.len(),
            killed_tracked_finished_sessions.len(),
            report.pruned_session_count,
            report.pruned_plan_count
        );
    }
    for session in live_untracked_sessions {
        println!("{session}\tlive_untracked_session");
    }
    for session in tracked_finished_sessions {
        println!("{session}\ttracked_finished_session");
    }
    for candidate in report.candidates {
        println!(
            "{}\tlast_seen_ms={}\ttask={}\t{}",
            candidate.session, candidate.last_seen_ms, candidate.task_id, candidate.reason
        );
    }
    0
}

fn print_prune_help() {
    println!("usage: mmux prune [--dry-run|--execute] [--older-than-days <days>] [--include-live-untracked-sessions] [--include-stale-session-records] [--include-finished-plans] [--include-tracked-finished-sessions]");
    println!();
    println!(
        "Prunes orchestration-owned live sessions, stale session records, and finished plans."
    );
    println!("Defaults: dry-run, all include categories enabled, --older-than-days 14.");
    println!("Pass one or more --include-* flags to scope pruning to only those categories.");
    println!("Use --execute to kill sessions and mutate mmux.db.");
}

fn parse_prune_args(raw_args: &[OsString]) -> Result<PruneArgs, String> {
    let mut dry_run = true;
    let mut older_than_days = DEFAULT_PRUNE_OLDER_THAN_DAYS;
    let mut include_live_untracked_sessions = false;
    let mut include_stale_session_records = false;
    let mut include_finished_plans = false;
    let mut include_tracked_finished_sessions = false;
    let mut index = 0;

    while index < raw_args.len() {
        let text = raw_args[index]
            .to_str()
            .ok_or_else(|| "arguments must be valid UTF-8".to_owned())?;
        match text {
            "--dry-run" => {
                dry_run = true;
                index += 1;
            }
            "--execute" => {
                dry_run = false;
                index += 1;
            }
            "--include-live-untracked-sessions" => {
                include_live_untracked_sessions = true;
                index += 1;
            }
            "--include-stale-session-records" => {
                include_stale_session_records = true;
                index += 1;
            }
            "--include-finished-plans" => {
                include_finished_plans = true;
                index += 1;
            }
            "--include-tracked-finished-sessions" => {
                include_tracked_finished_sessions = true;
                index += 1;
            }
            "--older-than-days" => {
                let value = raw_args
                    .get(index + 1)
                    .ok_or_else(|| "--older-than-days requires a value".to_owned())?
                    .to_str()
                    .ok_or_else(|| "--older-than-days value must be valid UTF-8".to_owned())?;
                older_than_days = parse_days(value)?;
                index += 2;
            }
            _ => {
                if let Some(value) = text.strip_prefix("--older-than-days=") {
                    older_than_days = parse_days(value)?;
                    index += 1;
                } else {
                    return Err(format!("unknown argument '{text}'"));
                }
            }
        }
    }

    let any_include = include_live_untracked_sessions
        || include_stale_session_records
        || include_finished_plans
        || include_tracked_finished_sessions;
    let include = if any_include {
        PruneInclude {
            live_untracked_sessions: include_live_untracked_sessions,
            stale_session_records: include_stale_session_records,
            finished_plans: include_finished_plans,
            tracked_finished_sessions: include_tracked_finished_sessions,
        }
    } else {
        PruneInclude {
            live_untracked_sessions: true,
            stale_session_records: true,
            finished_plans: true,
            tracked_finished_sessions: true,
        }
    };

    Ok(PruneArgs {
        dry_run,
        older_than_days,
        include,
    })
}

fn parse_days(value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("invalid --older-than-days value '{value}'"))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LiveTmuxSession {
    name: String,
    created_at_seconds: Option<u64>,
}

fn live_local_tmux_session_infos(store_path: &PathBuf) -> Result<Vec<LiveTmuxSession>, String> {
    let socket = mmux_node::local_tmux_socket_path(store_path);
    let output = Command::new("tmux")
        .arg("-S")
        .arg(socket)
        .args(["list-sessions", "-F", "#{session_name}|#{session_created}"])
        .output()
        .map_err(|error| format!("tmux failed to execute: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("no server running") || stderr.contains("failed to connect to server") {
            return Ok(Vec::new());
        }
        return Err(format!("tmux list-sessions failed: {}", stderr.trim()));
    }
    Ok(parse_live_tmux_session_infos(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

fn parse_live_tmux_session_infos(output: &str) -> Vec<LiveTmuxSession> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line == "No tmux sessions running" {
                return None;
            }
            let fields = line.splitn(2, '|').collect::<Vec<_>>();
            let name = fields.first().copied().unwrap_or("").trim();
            if name.is_empty() {
                return None;
            }
            Some(LiveTmuxSession {
                name: name.to_owned(),
                created_at_seconds: fields
                    .get(1)
                    .and_then(|value| value.trim().parse::<u64>().ok()),
            })
        })
        .collect()
}

fn prune_live_untracked_session_names(
    live_sessions: &[LiveTmuxSession],
    durable_session_names: &HashSet<String>,
    older_than_days: u64,
) -> Vec<String> {
    let Some(older_than_seconds) = older_than_days.checked_mul(86_400) else {
        return Vec::new();
    };
    let now_seconds = unix_now_seconds();
    let mut sessions = live_sessions
        .iter()
        .filter(|session| session.name.starts_with("mmux-"))
        .filter(|session| !durable_session_names.contains(&session.name))
        .filter(|session| {
            session
                .created_at_seconds
                .and_then(|created| now_seconds.checked_sub(created))
                .is_some_and(|age| age >= older_than_seconds)
        })
        .map(|session| session.name.clone())
        .collect::<Vec<_>>();
    sessions.sort();
    sessions.dedup();
    sessions
}

fn kill_local_tmux_session(store_path: &PathBuf, session: &str) -> Result<(), String> {
    let socket = mmux_node::local_tmux_socket_path(store_path);
    let output = Command::new("tmux")
        .arg("-S")
        .arg(socket)
        .args(["kill-session", "-t", session])
        .output()
        .map_err(|error| format!("tmux failed to execute: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!("tmux kill-session failed: {}", stderr.trim()))
}

fn unix_now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_else(|_| std::time::Duration::from_secs(0))
        .as_secs()
}

fn run_attach_proxy(raw_args: &[OsString]) -> i32 {
    run_attach_proxy_with_store(None, raw_args)
}

fn run_attach_proxy_with_store(store_path: Option<PathBuf>, raw_args: &[OsString]) -> i32 {
    if let Some(code) = run_attach_help(raw_args) {
        return code;
    }
    let (store_path, args) = match parse_proxy_args(store_path, raw_args) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("mmux attach: {error}");
            return 2;
        }
    };
    let attach_args = match parse_attach_args(&args) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("mmux attach: {error}");
            eprintln!("usage: mmux attach [--read-only|-r] <session>");
            return 2;
        }
    };
    let tmux_args = attach_tmux_args(attach_args);
    run_tmux_proxy_with_store(store_path, &tmux_args)
}

fn attach_tmux_args(attach_args: AttachArgs) -> Vec<OsString> {
    let mut tmux_args = vec![OsString::from("attach")];
    if attach_args.read_only {
        tmux_args.push(OsString::from("-r"));
    }
    tmux_args.push(OsString::from("-t"));
    tmux_args.push(attach_args.session);
    tmux_args
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AttachArgs {
    session: OsString,
    read_only: bool,
}

fn parse_attach_args(raw_args: &[OsString]) -> Result<AttachArgs, String> {
    let mut session = None;
    let mut read_only = false;

    for arg in raw_args {
        match arg.to_str() {
            Some("-r" | "--read-only") => {
                read_only = true;
            }
            Some("-h" | "--help") => {
                return Err("help is not available for mmux attach yet".into());
            }
            Some(value) if value.starts_with('-') => {
                return Err(format!("unknown argument '{value}'"));
            }
            _ if session.is_some() => {
                return Err("session may only be provided once".into());
            }
            _ => {
                session = Some(arg.clone());
            }
        }
    }

    let session = session.ok_or_else(|| "session is required".to_owned())?;
    if session.as_os_str().is_empty() {
        return Err("session must not be empty".into());
    }
    Ok(AttachArgs { session, read_only })
}

fn print_attach_help() {
    println!("usage: mmux attach [--read-only|-r] <session>");
    println!();
    println!("Attaches to a private local-node tmux session.");
    println!("Use --read-only to attach without sending input to the session.");
}

fn run_attach_help(raw_args: &[OsString]) -> Option<i32> {
    if raw_args
        .iter()
        .any(|arg| matches!(arg.to_str(), Some("-h" | "--help")))
    {
        print_attach_help();
        return Some(0);
    }
    None
}

fn run_tmux_proxy(raw_args: &[OsString]) -> i32 {
    run_tmux_proxy_with_store(None, raw_args)
}

fn run_tmux_proxy_with_store(store_path: Option<PathBuf>, raw_args: &[OsString]) -> i32 {
    let (store_path, tmux_args) = match parse_proxy_args(store_path, raw_args) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("mmux tmux: {error}");
            return 2;
        }
    };
    if tmux_args.is_empty() {
        eprintln!("usage: mmux tmux [--store-path <path>] -- <tmux args...>");
        return 2;
    }
    let store_path = match mmux_node::resolve_store_path(store_path.as_deref()) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("mmux tmux: {error}");
            return 2;
        }
    };
    if let Err(error) = mmux_node::ensure_store_dir(&store_path) {
        eprintln!("mmux tmux: {error}");
        return 2;
    }
    let (tmux_args, project_filter) = match parse_tmux_project_filter(tmux_args) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("mmux tmux: {error}");
            return 2;
        }
    };
    let socket = mmux_node::local_tmux_socket_path(&store_path);
    if let Some(project_filter) = project_filter {
        let sessions = match mmux_controller::local_project_session_names(
            Some(&store_path),
            &project_filter,
        ) {
            Ok(sessions) => sessions,
            Err(error) => {
                eprintln!("mmux tmux: {error}");
                return 1;
            }
        };
        return run_filtered_tmux_list_sessions(&socket, &tmux_args, &sessions);
    }
    let status = Command::new("tmux")
        .arg("-S")
        .arg(&socket)
        .args(tmux_args)
        .status();
    match status {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => {
            eprintln!("tmux failed to execute: {error}");
            127
        }
    }
}

fn run_filtered_tmux_list_sessions(
    socket: &PathBuf,
    tmux_args: &[OsString],
    sessions: &HashSet<String>,
) -> i32 {
    let output = Command::new("tmux")
        .arg("-S")
        .arg(socket)
        .args(tmux_args)
        .output();
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            eprintln!("tmux failed to execute: {error}");
            return 127;
        }
    };
    if let Err(error) = io::stderr().write_all(&output.stderr) {
        eprintln!("failed to write tmux stderr: {error}");
        return 1;
    }
    let stdout = filter_tmux_list_sessions_stdout(&output.stdout, sessions);
    if let Err(error) = io::stdout().write_all(&stdout) {
        eprintln!("failed to write tmux stdout: {error}");
        return 1;
    }
    output.status.code().unwrap_or(1)
}

fn parse_tmux_project_filter(
    tmux_args: Vec<OsString>,
) -> Result<(Vec<OsString>, Option<String>), String> {
    let is_list_sessions = tmux_args
        .first()
        .and_then(|arg| arg.to_str())
        .is_some_and(|arg| matches!(arg, "list-sessions" | "ls"));
    let mut project = None;
    let mut filtered = Vec::with_capacity(tmux_args.len());
    let mut index = 0;

    while index < tmux_args.len() {
        let arg = &tmux_args[index];
        if let Some(text) = arg.to_str() {
            if text == "--project" {
                if !is_list_sessions {
                    return Err("--project is only supported with list-sessions".into());
                }
                let value = tmux_args
                    .get(index + 1)
                    .ok_or_else(|| "--project requires a value".to_owned())?
                    .to_str()
                    .ok_or_else(|| "--project value must be valid UTF-8".to_owned())?
                    .to_owned();
                set_single_project_filter(&mut project, value)?;
                index += 2;
                continue;
            }
            if let Some(value) = text.strip_prefix("--project=") {
                if !is_list_sessions {
                    return Err("--project is only supported with list-sessions".into());
                }
                set_single_project_filter(&mut project, value.to_owned())?;
                index += 1;
                continue;
            }
        }
        filtered.push(arg.clone());
        index += 1;
    }

    Ok((filtered, project))
}

fn set_single_project_filter(project: &mut Option<String>, value: String) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err("--project must not be empty".into());
    }
    if project.is_some() {
        return Err("--project may only be provided once".into());
    }
    *project = Some(value);
    Ok(())
}

fn filter_tmux_list_sessions_stdout(stdout: &[u8], sessions: &HashSet<String>) -> Vec<u8> {
    let text = String::from_utf8_lossy(stdout);
    let mut filtered = String::new();
    for line in text.lines() {
        if tmux_list_session_line_name(line).is_some_and(|name| sessions.contains(name)) {
            filtered.push_str(line);
            filtered.push('\n');
        }
    }
    filtered.into_bytes()
}

fn tmux_list_session_line_name(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    trimmed
        .split_once(':')
        .map(|(name, _)| name)
        .or_else(|| trimmed.split_whitespace().next())
        .filter(|name| !name.is_empty())
}

fn parse_proxy_args(
    initial_store_path: Option<PathBuf>,
    raw_args: &[OsString],
) -> Result<(Option<PathBuf>, Vec<OsString>), String> {
    let mut store_path = initial_store_path;
    let mut passthrough = false;
    let mut tmux_args = Vec::new();
    let mut index = 0;

    while index < raw_args.len() {
        let arg = &raw_args[index];
        if !passthrough {
            if arg == "--" {
                passthrough = true;
                index += 1;
                continue;
            }
            if arg == "--store-path" {
                let value = raw_args
                    .get(index + 1)
                    .ok_or_else(|| "--store-path requires a value".to_owned())?;
                store_path = Some(PathBuf::from(value));
                index += 2;
                continue;
            }
            if let Some(text) = arg.to_str() {
                if let Some(value) = text.strip_prefix("--store-path=") {
                    store_path = Some(PathBuf::from(value));
                    index += 1;
                    continue;
                }
            }
        }
        tmux_args.push(arg.clone());
        index += 1;
    }

    Ok((store_path, tmux_args))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os_args(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn attach_args_accept_session_and_read_only_flags() {
        let args = parse_attach_args(&os_args(&["codex"])).unwrap();
        assert_eq!(
            args,
            AttachArgs {
                session: OsString::from("codex"),
                read_only: false,
            }
        );

        let args = parse_attach_args(&os_args(&["--read-only", "codex"])).unwrap();
        assert_eq!(
            args,
            AttachArgs {
                session: OsString::from("codex"),
                read_only: true,
            }
        );

        let args = parse_attach_args(&os_args(&["codex", "-r"])).unwrap();
        assert!(args.read_only);
    }

    #[test]
    fn attach_args_reject_missing_duplicate_and_unknown() {
        let error = parse_attach_args(&os_args(&[])).unwrap_err();
        assert_eq!(error, "session is required");

        let error = parse_attach_args(&os_args(&["one", "two"])).unwrap_err();
        assert_eq!(error, "session may only be provided once");

        let error = parse_attach_args(&os_args(&["--write", "codex"])).unwrap_err();
        assert!(error.contains("unknown argument"));
    }

    #[test]
    fn attach_tmux_args_include_read_only_flag_before_target() {
        let args = attach_tmux_args(AttachArgs {
            session: OsString::from("codex"),
            read_only: true,
        });

        assert_eq!(args, os_args(&["attach", "-r", "-t", "codex"]));
    }

    #[test]
    fn create_project_args_accept_title_description_and_optional_slug() {
        let args = parse_create_project_args(&os_args(&[
            "My Project",
            "--description",
            "Long-running work",
            "--slug=custom-project",
        ]))
        .expect("create project args");

        assert_eq!(
            args,
            CreateProjectArgs {
                title: "My Project".into(),
                description: "Long-running work".into(),
                slug: Some("custom-project".into()),
            }
        );
    }

    #[test]
    fn create_project_args_require_title_and_description() {
        let error = parse_create_project_args(&os_args(&[])).expect_err("missing title");
        assert_eq!(error, "project title is required");

        let error =
            parse_create_project_args(&os_args(&["My Project"])).expect_err("missing description");
        assert_eq!(error, "project description is required");

        let error =
            parse_create_project_args(&os_args(&["One", "Two"])).expect_err("duplicate title");
        assert_eq!(error, "project title may only be provided once");
    }

    #[test]
    fn project_filter_is_removed_from_list_sessions_args() {
        let (args, project) =
            parse_tmux_project_filter(os_args(&["list-sessions", "--project", "mmux", "-F", "#S"]))
                .unwrap();

        assert_eq!(args, os_args(&["list-sessions", "-F", "#S"]));
        assert_eq!(project.as_deref(), Some("mmux"));
    }

    #[test]
    fn project_filter_accepts_equals_form() {
        let (args, project) =
            parse_tmux_project_filter(os_args(&["ls", "--project=project-1"])).unwrap();

        assert_eq!(args, os_args(&["ls"]));
        assert_eq!(project.as_deref(), Some("project-1"));
    }

    #[test]
    fn project_filter_is_rejected_for_non_list_sessions_commands() {
        let error =
            parse_tmux_project_filter(os_args(&["attach", "--project", "mmux"])).unwrap_err();

        assert!(error.contains("only supported with list-sessions"));
    }

    #[test]
    fn prune_args_parse_dry_run_include_and_age() {
        let args = parse_prune_args(&os_args(&[
            "--dry-run",
            "--include-stale-session-records",
            "--older-than-days",
            "7",
        ]))
        .unwrap();

        assert_eq!(
            args,
            PruneArgs {
                dry_run: true,
                older_than_days: 7,
                include: PruneInclude {
                    live_untracked_sessions: false,
                    stale_session_records: true,
                    finished_plans: false,
                    tracked_finished_sessions: false,
                },
            }
        );
    }

    #[test]
    fn prune_args_default_to_dry_run_all_categories_and_fourteen_days() {
        let args = parse_prune_args(&os_args(&[])).unwrap();

        assert_eq!(
            args,
            PruneArgs {
                dry_run: true,
                older_than_days: 14,
                include: PruneInclude {
                    live_untracked_sessions: true,
                    stale_session_records: true,
                    finished_plans: true,
                    tracked_finished_sessions: true,
                },
            }
        );
    }

    #[test]
    fn prune_args_accept_execute_and_equals_age() {
        let args = parse_prune_args(&os_args(&["--execute", "--older-than-days=30"])).unwrap();

        assert_eq!(
            args,
            PruneArgs {
                dry_run: false,
                older_than_days: 30,
                include: PruneInclude {
                    live_untracked_sessions: true,
                    stale_session_records: true,
                    finished_plans: true,
                    tracked_finished_sessions: true,
                },
            }
        );
    }

    #[test]
    fn prune_args_reject_unknown_flags() {
        let error = parse_prune_args(&os_args(&["--all"])).unwrap_err();

        assert!(error.contains("unknown argument"));
    }

    #[test]
    fn list_sessions_output_filters_by_session_name() {
        let sessions = HashSet::from(["codex".to_owned(), "worker-a".to_owned()]);
        let stdout = b"codex: 1 windows\nother: 1 windows\nworker-a: 2 windows\n";

        let filtered = filter_tmux_list_sessions_stdout(stdout, &sessions);

        assert_eq!(
            String::from_utf8(filtered).unwrap(),
            "codex: 1 windows\nworker-a: 2 windows\n"
        );
    }
}
