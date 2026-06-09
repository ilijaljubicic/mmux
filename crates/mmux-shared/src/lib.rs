use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CliProfile {
    pub name: String,
    pub cmd: Option<String>,
    #[serde(default)]
    pub permission_bypass_cmd: Option<String>,
    #[serde(default)]
    pub launch_strategy: Option<String>,
    #[serde(default = "default_text_mode")]
    pub text_mode: String,
    #[serde(default = "default_submit_keys")]
    pub submit_keys: String,
    #[serde(default = "default_submit_after_text")]
    pub submit_after_text: bool,
    pub prompt_indicator: String,
    pub busy_indicators: Vec<String>,
    pub approve_keys: String,
    pub reject_keys: String,
    pub cancel_keys: String,
    pub escape_keys: String,
}

pub fn default_text_mode() -> String {
    "paste-buffer".into()
}

pub fn default_submit_keys() -> String {
    "Enter".into()
}

pub fn default_submit_after_text() -> bool {
    true
}

impl Default for CliProfile {
    fn default() -> Self {
        Self {
            name: "generic".into(),
            cmd: None,
            permission_bypass_cmd: None,
            launch_strategy: None,
            text_mode: default_text_mode(),
            submit_keys: default_submit_keys(),
            submit_after_text: default_submit_after_text(),
            prompt_indicator: "$".into(),
            busy_indicators: vec![],
            approve_keys: "y Enter".into(),
            reject_keys: "n Enter".into(),
            cancel_keys: "C-c".into(),
            escape_keys: "Escape".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadFileResult {
    pub path: String,
    pub content: String,
    pub encoding: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub read_bytes: usize,
    pub compression: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SaveFileResult {
    pub path: String,
    pub bytes_written: usize,
    pub mime_type: Option<String>,
}
