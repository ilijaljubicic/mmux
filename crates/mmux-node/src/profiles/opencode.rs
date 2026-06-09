use mmux_shared::CliProfile;

pub(crate) fn config() -> CliProfile {
    CliProfile {
        name: "opencode".into(),
        cmd: Some("opencode".into()),
        permission_bypass_cmd: None,
        launch_strategy: Some("shell_send".into()),
        text_mode: "paste-buffer".into(),
        submit_keys: "Enter".into(),
        submit_after_text: true,
        prompt_indicator: "ctrl+p commands".into(),
        busy_indicators: vec![
            "Thinking".into(),
            "Working".into(),
            "Running".into(),
            "Processing".into(),
            "Generating".into(),
            "esc interrupt".into(),
            "⬝⬝".into(),
            "■■".into(),
        ],
        approve_keys: "y Enter".into(),
        reject_keys: "n Enter".into(),
        cancel_keys: "C-c".into(),
        escape_keys: "Escape".into(),
    }
}

pub(crate) fn is_noise_line(_line: &str, lower: &str) -> bool {
    lower.contains("ask anything")
        || lower.contains("build ·")
        || lower.contains("tab agents")
        || lower.contains("ctrl+p commands")
        || lower.contains("esc interrupt")
        || lower.contains("queued")
        || lower.contains("reply with exactly")
        || lower.contains("mmux_context_bench")
        || lower.contains("do not run commands")
        || lower.trim_start_matches('┃').trim() == "done"
        || (lower.starts_with('/') && lower.contains(" 1."))
}
