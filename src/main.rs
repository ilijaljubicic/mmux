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
        Some("prune-store") => {
            std::process::exit(run_prune_store(None, &args[2..]));
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
        Some("--store-path") if args.get(3).and_then(|arg| arg.to_str()) == Some("prune-store") => {
            std::process::exit(run_prune_store(args.get(2).map(PathBuf::from), &args[4..]));
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
                Some("prune-store") => {
                    std::process::exit(run_prune_store(Some(store_path), &args[3..]))
                }
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
    println!("  attach <session>        Attach to a private local-node tmux session");
    println!(
        "  create-project <title> --description <text>  Create a durable orchestration project in mmux.db"
    );
    println!("  list-projects           List durable orchestration projects from mmux.db");
    println!("  prune-store             Prune stale task sessions and finished plans from mmux.db");
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct PruneStoreArgs {
    dry_run: bool,
    sessions_only: bool,
    older_than_days: Option<u64>,
}

fn run_prune_store(store_path: Option<PathBuf>, raw_args: &[OsString]) -> i32 {
    if raw_args
        .iter()
        .any(|arg| matches!(arg.to_str(), Some("-h" | "--help")))
    {
        print_prune_store_help();
        return 0;
    }
    let args = match parse_prune_store_args(raw_args) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("mmux prune-store: {error}");
            return 2;
        }
    };
    let store_path = match mmux_node::resolve_store_path(store_path.as_deref()) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("mmux prune-store: {error}");
            return 2;
        }
    };
    let live_local_sessions = match live_local_tmux_sessions(&store_path) {
        Ok(sessions) => sessions,
        Err(error) => {
            eprintln!("mmux prune-store: {error}");
            return 1;
        }
    };
    let report = match mmux_controller::local_prune_store(
        Some(&store_path),
        &live_local_sessions,
        args.dry_run,
        args.sessions_only,
        args.older_than_days,
    ) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("mmux prune-store: {error}");
            return 1;
        }
    };
    if report.dry_run {
        println!(
            "dry-run: would prune {} stale session record(s) and {} finished plan(s)",
            report.pruned_session_count, report.pruned_plan_count
        );
    } else {
        println!(
            "pruned {} stale session record(s) and {} finished plan(s)",
            report.pruned_session_count, report.pruned_plan_count
        );
    }
    for candidate in report.candidates {
        println!(
            "{}\tlast_seen_ms={}\ttask={}\t{}",
            candidate.session, candidate.last_seen_ms, candidate.task_id, candidate.reason
        );
    }
    0
}

fn print_prune_store_help() {
    println!("usage: mmux prune-store [--dry-run] [--sessions-only] [--older-than-days <days>]");
    println!();
    println!("Prunes stale durable task sessions and finished plans from mmux.db.");
    println!("Only missing local sessions attached exclusively to finished tasks are eligible.");
    println!("Finished plans are pruned only when all contained tasks are finished.");
    println!("Use --sessions-only to skip finished plan pruning.");
}

fn parse_prune_store_args(raw_args: &[OsString]) -> Result<PruneStoreArgs, String> {
    let mut dry_run = false;
    let mut sessions_only = false;
    let mut older_than_days = None;
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
            "--sessions-only" => {
                sessions_only = true;
                index += 1;
            }
            "--older-than-days" => {
                let value = raw_args
                    .get(index + 1)
                    .ok_or_else(|| "--older-than-days requires a value".to_owned())?
                    .to_str()
                    .ok_or_else(|| "--older-than-days value must be valid UTF-8".to_owned())?;
                older_than_days = Some(parse_days(value)?);
                index += 2;
            }
            _ => {
                if let Some(value) = text.strip_prefix("--older-than-days=") {
                    older_than_days = Some(parse_days(value)?);
                    index += 1;
                } else {
                    return Err(format!("unknown argument '{text}'"));
                }
            }
        }
    }

    Ok(PruneStoreArgs {
        dry_run,
        sessions_only,
        older_than_days,
    })
}

fn parse_days(value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("invalid --older-than-days value '{value}'"))
}

fn live_local_tmux_sessions(store_path: &PathBuf) -> Result<HashSet<String>, String> {
    let socket = mmux_node::local_tmux_socket_path(store_path);
    let output = Command::new("tmux")
        .arg("-S")
        .arg(socket)
        .args(["list-sessions", "-F", "#{session_name}"])
        .output()
        .map_err(|error| format!("tmux failed to execute: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("no server running") || stderr.contains("failed to connect to server") {
            return Ok(HashSet::new());
        }
        return Err(format!("tmux list-sessions failed: {}", stderr.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn run_attach_proxy(raw_args: &[OsString]) -> i32 {
    run_attach_proxy_with_store(None, raw_args)
}

fn run_attach_proxy_with_store(store_path: Option<PathBuf>, raw_args: &[OsString]) -> i32 {
    let (store_path, mut args) = match parse_proxy_args(store_path, raw_args) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("mmux attach: {error}");
            return 2;
        }
    };
    if args.len() != 1 {
        eprintln!("usage: mmux attach <session>");
        return 2;
    }
    let session = args.remove(0);
    run_tmux_proxy_with_store(
        store_path,
        &[OsString::from("attach"), OsString::from("-t"), session],
    )
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
    fn prune_store_args_parse_dry_run_sessions_and_age() {
        let args = parse_prune_store_args(&os_args(&[
            "--dry-run",
            "--sessions-only",
            "--older-than-days",
            "7",
        ]))
        .unwrap();

        assert_eq!(
            args,
            PruneStoreArgs {
                dry_run: true,
                sessions_only: true,
                older_than_days: Some(7),
            }
        );
    }

    #[test]
    fn prune_store_args_accept_equals_age() {
        let args = parse_prune_store_args(&os_args(&["--older-than-days=30"])).unwrap();

        assert_eq!(
            args,
            PruneStoreArgs {
                dry_run: false,
                sessions_only: false,
                older_than_days: Some(30),
            }
        );
    }

    #[test]
    fn prune_store_args_reject_unknown_flags() {
        let error = parse_prune_store_args(&os_args(&["--all"])).unwrap_err();

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
