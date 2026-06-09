use mmux_shared::CliProfile;

pub(crate) fn config() -> CliProfile {
    CliProfile {
        name: "kimi".into(),
        cmd: Some("kimi".into()),
        permission_bypass_cmd: Some("kimi --yolo".into()),
        launch_strategy: None,
        text_mode: "paste-buffer".into(),
        submit_keys: "Enter".into(),
        submit_after_text: true,
        prompt_indicator: ">".into(),
        busy_indicators: vec![
            "Working".into(),
            "Running".into(),
            "ctrl+c: cancel".into(),
            "ctrl-s to steer".into(),
            "ctrl+s: steer".into(),
            "to edit".into(),
        ],
        approve_keys: "y Enter".into(),
        reject_keys: "n Enter".into(),
        cancel_keys: "C-c".into(),
        escape_keys: "Escape".into(),
    }
}

pub(crate) fn startup_dismiss_key(active_region: &str) -> Option<&'static str> {
    active_region
        .contains("Kimi Code Update Available")
        .then_some("Down Enter")
}

pub(crate) fn is_noise_line(line: &str, lower: &str) -> bool {
    lower.contains("code update available")
        || lower.contains("the user wants me to")
        || lower.contains("the user wants exactly")
        || lower.contains("no tool calls")
        || lower.contains("text response")
        || lower.contains("tools, just reply")
        || lower.contains("commands, no file edits")
        || lower.starts_with("mmux_context_bench_")
        || lower == "done"
        || lower.contains("do not run commands")
        || lower.contains("has a newer release ready")
        || lower.contains("view changelog:")
        || lower == "g.html"
        || lower.starts_with("current  ")
        || lower.starts_with("target   ")
        || lower.starts_with("source   ")
        || lower.starts_with("command  ")
        || lower.contains("choose · enter confirm")
        || lower.contains("ctrl+o to expand")
        || lower.contains("install update now")
        || lower.contains("continue with current version")
        || lower.starts_with("==> detected target:")
        || lower.starts_with("==> resolving latest version")
        || lower.starts_with("==> latest version:")
        || lower.starts_with("==> fetching manifest")
        || lower.starts_with("==> downloading")
        || lower.chars().filter(|ch| *ch == '#').count() >= 3
        || lower.contains("shift+enter: newline")
        || lower.starts_with("context:")
        || lower.starts_with("kimi-")
        || line.starts_with('✨')
        || lower.starts_with("tmux extended-keys-format ")
        || lower.contains("kimi code works best with csi-u")
        || lower.contains("set -g extended-keys-format csi-u")
}
