use mmux_shared::CliProfile;

pub(crate) fn config() -> CliProfile {
    CliProfile {
        name: "codex".into(),
        cmd: Some("codex".into()),
        permission_bypass_cmd: Some("codex --dangerously-bypass-approvals-and-sandbox".into()),
        launch_strategy: None,
        text_mode: "paste-buffer".into(),
        submit_keys: "Enter".into(),
        submit_after_text: true,
        prompt_indicator: "›".into(),
        busy_indicators: vec!["• Working".into(), "◦ Working".into()],
        approve_keys: "y Enter".into(),
        reject_keys: "n Enter".into(),
        cancel_keys: "C-c".into(),
        escape_keys: "Escape".into(),
    }
}

pub(crate) fn startup_dismiss_key(active_region: &str) -> Option<&'static str> {
    active_region.contains("Update now").then_some("Down Enter")
}

pub(crate) fn is_noise_line(line: &str, lower: &str) -> bool {
    let after_prompt = lower.trim_start_matches('›').trim();
    after_prompt == "find and fix a bug in @filename"
        || after_prompt == "explain this codebase"
        || after_prompt == "improve documentation in @filename"
        || after_prompt == "implement {feature}"
        || after_prompt.starts_with("reply with exactly")
        || after_prompt == "run /review on my current changes"
        || after_prompt == "summarize recent commits"
        || lower.starts_with("mmux_context_bench_")
        || lower == "done"
        || lower.contains("do not run commands")
        || lower.starts_with("• working")
        || lower.starts_with("◦ working")
        || lower.starts_with("• messages to be submitted")
        || lower.starts_with("immediately)")
        || lower.starts_with('↳')
        || lower.contains("openai codex")
        || lower.contains("/model to change")
        || lower.contains("starting mcp servers")
        || lower.starts_with("tip:")
        || (line.contains(" · /") && lower.contains("gpt-"))
}
