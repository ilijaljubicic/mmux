mod claude;
mod codex;
mod kimi;
mod opencode;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use mmux_shared::CliProfile;

pub type ProfileRegistry = Arc<HashMap<String, CliProfile>>;

const BUSY_SCAN_TRAILING_LINES: usize = 30;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuiltinProfile {
    Codex,
    Kimi,
    Claude,
    Opencode,
}

impl BuiltinProfile {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "codex" => Some(Self::Codex),
            "kimi" => Some(Self::Kimi),
            "claude" => Some(Self::Claude),
            "opencode" => Some(Self::Opencode),
            _ => None,
        }
    }

    pub fn all() -> [Self; 4] {
        [Self::Codex, Self::Opencode, Self::Kimi, Self::Claude]
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Kimi => "kimi",
            Self::Claude => "claude",
            Self::Opencode => "opencode",
        }
    }

    pub fn config(self) -> CliProfile {
        match self {
            Self::Codex => codex::config(),
            Self::Kimi => kimi::config(),
            Self::Claude => claude::config(),
            Self::Opencode => opencode::config(),
        }
    }

    fn startup_dismiss_key(self, active_region: &str) -> Option<&'static str> {
        match self {
            Self::Codex => codex::startup_dismiss_key(active_region),
            Self::Kimi => kimi::startup_dismiss_key(active_region),
            Self::Claude => claude::startup_dismiss_key(active_region),
            Self::Opencode => None,
        }
    }

    fn has_blocking_confirmation(self, active_region: &str) -> bool {
        match self {
            Self::Claude => claude::has_blocking_confirmation(active_region),
            Self::Codex | Self::Kimi | Self::Opencode => false,
        }
    }

    fn is_noise_line(self, line: &str, lower: &str) -> bool {
        match self {
            Self::Codex => codex::is_noise_line(line, lower),
            Self::Kimi => kimi::is_noise_line(line, lower),
            Self::Claude => claude::is_noise_line(line, lower),
            Self::Opencode => opencode::is_noise_line(line, lower),
        }
    }
}

pub fn default_profiles() -> ProfileRegistry {
    Arc::new(
        BuiltinProfile::all()
            .into_iter()
            .map(|profile| (profile.name().to_owned(), profile.config()))
            .collect(),
    )
}

pub fn get_profile(registry: &ProfileRegistry, name: &str) -> Option<CliProfile> {
    registry.get(name).cloned()
}

pub fn is_builtin_profile(name: &str) -> bool {
    BuiltinProfile::from_name(name).is_some()
}

pub fn launch_command(profile: &CliProfile, bypass_permissions: bool) -> Result<&str, String> {
    ensure_profile_runtime_supported(profile)?;
    if bypass_permissions {
        return profile.permission_bypass_cmd.as_deref().ok_or_else(|| {
            format!(
                "profile '{}' does not define permission_bypass_cmd; bypass_permissions=true is not supported for this built-in profile",
                profile.name
            )
        });
    }
    profile
        .cmd
        .as_deref()
        .ok_or_else(|| format!("profile '{}' does not define cmd", profile.name))
}

pub fn launch_strategy(profile: &CliProfile) -> Result<&str, String> {
    ensure_profile_runtime_supported(profile)?;
    match profile.launch_strategy.as_deref().unwrap_or("direct") {
        "direct" => Ok("direct"),
        "shell_send" => Ok("shell_send"),
        other => Err(format!(
            "profile '{}' has unsupported launch_strategy '{}'",
            profile.name, other
        )),
    }
}

pub fn text_mode(profile: &CliProfile) -> Result<&str, String> {
    ensure_profile_runtime_supported(profile)?;
    match profile.text_mode.as_str() {
        "paste-buffer" => Ok("paste-buffer"),
        "literal-keys" => Ok("literal-keys"),
        other => Err(format!(
            "profile '{}' has unsupported text_mode '{}'",
            profile.name, other
        )),
    }
}

pub fn is_busy(output: &str, profile: &CliProfile) -> bool {
    let active_region = output_active_region(output);
    if has_blocking_confirmation(&active_region, profile) {
        return true;
    }
    if startup_dismiss_key(output, profile).is_some() {
        return true;
    }
    profile
        .busy_indicators
        .iter()
        .any(|marker| !marker.is_empty() && active_region.contains(marker))
}

pub fn has_prompt(output: &str, profile: &CliProfile) -> bool {
    if has_blocking_confirmation(&output_active_region(output), profile) {
        return false;
    }
    !profile.prompt_indicator.is_empty() && output.contains(&profile.prompt_indicator)
}

pub fn turn_idle(output: &str, profile: &CliProfile) -> bool {
    has_prompt(output, profile) && !is_busy(output, profile)
}

pub fn startup_dismiss_key(output: &str, profile: &CliProfile) -> Option<String> {
    let profile_kind = BuiltinProfile::from_name(&profile.name)?;
    let active_region = readiness_scan_region(output, profile);
    profile_kind
        .startup_dismiss_key(&active_region)
        .map(str::to_owned)
}

pub fn has_blocking_confirmation(output: &str, profile: &CliProfile) -> bool {
    BuiltinProfile::from_name(&profile.name)
        .map(|profile_kind| profile_kind.has_blocking_confirmation(output))
        .unwrap_or(false)
}

pub fn compact_output(output: &str, profile: &CliProfile) -> String {
    let profile_kind = BuiltinProfile::from_name(&profile.name);
    let mut compact = Vec::new();
    let mut seen = HashSet::new();
    for raw_line in output.lines() {
        let line = normalize_line(raw_line);
        let trimmed = line.trim();
        if trimmed.is_empty() || is_noise_line(trimmed, profile_kind) {
            continue;
        }
        if !seen.insert(trimmed.to_owned()) {
            continue;
        }
        compact.push(trimmed.to_owned());
    }
    compact.join("\n")
}

fn ensure_profile_runtime_supported(profile: &CliProfile) -> Result<(), String> {
    if is_builtin_profile(&profile.name) {
        Ok(())
    } else {
        Err(format!(
            "profile '{}' is not a built-in mmux profile; runtime profile loading is not supported",
            profile.name
        ))
    }
}

fn readiness_scan_region(output: &str, profile: &CliProfile) -> String {
    let active_region = output_active_region(output);
    if profile.prompt_indicator.is_empty() {
        return active_region;
    }
    active_region
        .rfind(&profile.prompt_indicator)
        .map(|index| active_region[index..].to_owned())
        .unwrap_or(active_region)
}

fn output_active_region(output: &str) -> String {
    output
        .lines()
        .rev()
        .take(BUSY_SCAN_TRAILING_LINES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_line(line: &str) -> String {
    line.replace('\u{a0}', " ")
}

fn is_noise_line(line: &str, profile_kind: Option<BuiltinProfile>) -> bool {
    let lower = line.to_ascii_lowercase();
    is_box_drawing_line(line)
        || is_common_noise_line(line, &lower)
        || profile_kind
            .map(|profile_kind| profile_kind.is_noise_line(line, &lower))
            .unwrap_or(false)
}

fn is_box_drawing_line(line: &str) -> bool {
    line.chars().all(|ch| {
        ch.is_whitespace()
            || matches!(
                ch,
                '─' | '━'
                    | '▄'
                    | '▀'
                    | '╹'
                    | '│'
                    | '┃'
                    | '╭'
                    | '╮'
                    | '╰'
                    | '╯'
                    | '┌'
                    | '┐'
                    | '└'
                    | '┘'
                    | '├'
                    | '┤'
                    | '┬'
                    | '┴'
                    | '┼'
                    | '═'
                    | '║'
                    | '╔'
                    | '╗'
                    | '╚'
                    | '╝'
            )
    }) || (line.starts_with('│') && line.ends_with('│'))
}

fn is_common_noise_line(_line: &str, lower: &str) -> bool {
    lower == "…" || lower.starts_with("model:") || lower.starts_with("directory:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_have_stable_metadata() {
        let profiles = default_profiles();
        for name in ["codex", "opencode", "kimi", "claude"] {
            assert!(profiles.contains_key(name), "missing {name}");
        }
        assert_eq!(profiles["codex"].cmd.as_deref(), Some("codex"));
        assert_eq!(
            profiles["kimi"].permission_bypass_cmd.as_deref(),
            Some("kimi --yolo")
        );
        assert_eq!(profiles["claude"].text_mode, "literal-keys");
        assert_eq!(
            profiles["opencode"].launch_strategy.as_deref(),
            Some("shell_send")
        );
    }

    #[test]
    fn non_builtin_profiles_are_rejected() {
        let profile = CliProfile {
            name: "custom".into(),
            cmd: Some("custom-cli".into()),
            ..CliProfile::default()
        };

        assert!(launch_command(&profile, false)
            .unwrap_err()
            .contains("is not a built-in mmux profile"));
    }

    #[test]
    fn startup_dismiss_is_owned_by_builtin_behavior() {
        let profiles = default_profiles();
        assert_eq!(
            startup_dismiss_key("✨ Update available!\n› 1. Update now", &profiles["codex"]),
            Some("Down Enter".into())
        );
        assert_eq!(
            startup_dismiss_key(
                "Kimi Code Update Available\n ❯ Install update now",
                &profiles["kimi"]
            ),
            Some("Down Enter".into())
        );
        assert_eq!(
            startup_dismiss_key("Claude Code Update Available", &profiles["claude"]),
            Some("Escape".into())
        );
    }

    #[test]
    fn startup_chrome_compacts_to_empty_when_no_useful_content_exists() {
        let profiles = default_profiles();
        let cases = [
            (
                "codex",
                r#"
╭──────────────────────────────────────────────╮
│ >_ OpenAI Codex (v0.137.0)                   │
│ model:     loading   /model to change        │
│ directory: /tmp/mmux-profile-smoke-workspace │
╰──────────────────────────────────────────────╯

› Explain this codebase

  gpt-5.5 default fast · /tmp/mmux-profile-smoke-workspace
"#,
            ),
            (
                "kimi",
                r#"
Kimi Code Update Available
Kimi Code has a newer release ready.
Current  0.6.0
Target   0.11.0
↑↓ choose · Enter confirm · Esc continue
 ❯ Continue with current version
Kimi-k2.6 thinking  /tmp/mmux-profile-smoke-workspace
                                                       context: 0.0% (0/262.1k)
"#,
            ),
            (
                "claude",
                r#"
 ▐▛███▜▌   Claude Code v2.1.168
▝▜█████▛▘  Opus 4.8 · Claude Pro
  ▘▘ ▝▝    /tmp/mmux-profile-smoke-workspace
❯ Try "how does <filepath> work?"
  ? for shortcuts · ← for agents
"#,
            ),
            (
                "opencode",
                r#"
                                                      ▄
                     ▀▀▀▀ ▀▀▀▀ ▀▀▀▀ ▀▀▀▄ ▀▀▀▀ ▀▀▀▀ ▀▀▀▀ ▀▀▀▀
   ┃  Ask anything... "Fix broken tests"
   ┃  Build · qwen3-6-35b (aizone) aizone
                                                   tab agents  ctrl+p commands
  /tmp/mmux-profile-smoke-workspace                                    1.15.13
"#,
            ),
        ];

        for (profile_name, output) in cases {
            assert_eq!(
                compact_output(output, &profiles[profile_name]),
                "",
                "{profile_name} startup chrome leaked"
            );
        }
    }

    #[test]
    fn live_busy_markers_keep_profiles_non_idle() {
        let profiles = default_profiles();
        let cases = [
            (
                "codex",
                "› Run /review on my current changes\n◦ Working (1m 16s • esc to interrupt)",
            ),
            (
                "kimi",
                "> \nKimi-k2.6 thinking  /tmp/workspace   ctrl+s: steer mid-turn",
            ),
            (
                "opencode",
                "tab agents  ctrl+p commands\n⬝⬝⬝⬝⬝⬝■■  esc interrupt",
            ),
        ];

        for (profile_name, output) in cases {
            let profile = &profiles[profile_name];
            assert!(is_busy(output, profile), "{profile_name} should be busy");
            assert!(
                !turn_idle(output, profile),
                "{profile_name} should not be turn-idle"
            );
        }
    }

    #[test]
    fn compact_output_strips_live_prompt_turn_chrome() {
        let profiles = default_profiles();
        let cases = [
            (
                "codex",
                r#"
⚠ The cormiloDev MCP server is not logged in. Run `codex mcp login cormiloDev`.
› Reply with exactly two lines and nothing else:
MMUX_CONTEXT_BENCH_CODEX
DONE
Do not run commands. Do not edit files.
• Working (5m 15s • esc to interrupt)
• Messages to be submitted after next tool call (press esc to interrupt and send
immediately)
↳ Reply with exactly two lines and nothing else:
…
"#,
                "⚠ The cormiloDev MCP server is not logged in. Run `codex mcp login cormiloDev`.",
            ),
            (
                "kimi",
                r#"
✨ Reply with exactly two lines and nothing else:
commands, no file edits.
● The user wants exactly two lines and nothing else. No tool calls. Just the
text response.
● MMUX_CONTEXT_BENCH_KIMI
"#,
                "● MMUX_CONTEXT_BENCH_KIMI",
            ),
            (
                "claude",
                r#"
not run commands. Do not edit files.
● MMUX_CONTEXT_BENCH_CLAUDE
DONE
✻ Cooked for 1s
❯
"#,
                "● MMUX_CONTEXT_BENCH_CLAUDE\nDONE",
            ),
            (
                "opencode",
                r#"
┃  Reply with exactly two lines and nothing else:
┃  MMUX_CONTEXT_BENCH_OPENCODE
┃  DONE
┃  Do not run commands. Do not edit files.
┃   QUEUED
"#,
                "",
            ),
        ];

        for (profile_name, output, expected) in cases {
            assert_eq!(
                compact_output(output, &profiles[profile_name]),
                expected,
                "{profile_name} live prompt chrome leaked"
            );
        }
    }
}
