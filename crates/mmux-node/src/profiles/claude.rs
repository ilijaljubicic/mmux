use mmux_shared::CliProfile;

pub(crate) fn config() -> CliProfile {
    CliProfile {
        name: "claude".into(),
        cmd: Some("claude".into()),
        permission_bypass_cmd: Some("claude --dangerously-skip-permissions".into()),
        launch_strategy: None,
        text_mode: "literal-keys".into(),
        submit_keys: "Enter".into(),
        submit_after_text: true,
        prompt_indicator: "❯".into(),
        busy_indicators: vec!["Thinking".into(), "Working".into(), "Running".into()],
        approve_keys: "y Enter".into(),
        reject_keys: "n Enter".into(),
        cancel_keys: "C-c".into(),
        escape_keys: "Escape".into(),
    }
}

pub(crate) fn startup_dismiss_key(active_region: &str) -> Option<&'static str> {
    [
        "Update available",
        "Claude Code Update Available",
        "A new version of Claude Code is available",
    ]
    .iter()
    .any(|trigger| active_region.contains(trigger))
    .then_some("Escape")
}

pub(crate) fn has_blocking_confirmation(active_region: &str) -> bool {
    let lower = active_region.to_ascii_lowercase();
    lower.contains("bypass permissions")
        || (lower.contains("dangerously skip permissions")
            && (lower.contains("accept") || lower.contains("confirm")))
}

pub(crate) fn is_noise_line(line: &str, lower: &str) -> bool {
    let after_prompt = lower.trim_start_matches('❯').trim();
    lower == "interval"
        || lower.contains("claude code v")
        || lower.contains("welcome back")
        || lower.contains("tips for getting")
        || lower.contains("feature of the week")
        || lower.contains("what's new")
        || lower.contains("organization")
        || lower.contains("/release-notes")
        || lower.contains("for shortcuts")
        || lower.contains("← for agents")
        || lower.contains("worked for")
        || lower.contains("brewed for")
        || lower.contains("cooked for")
        || lower.contains("cogitated for")
        || lower.contains("do not run commands. do not edit files.")
        || lower.contains("not run commands. do not edit files.")
        || lower.starts_with("opus ")
        || (line.contains(" · /") && lower.contains("opus"))
        || line.contains("▝▜█████▛▘")
        || line.contains("▘▘ ▝▝")
        || after_prompt.starts_with("try \"edit <filepath>")
        || after_prompt.is_empty()
        || after_prompt.starts_with("try \"")
        || after_prompt.starts_with("reply with exactly")
        || after_prompt.starts_with("ask claude to create")
}
