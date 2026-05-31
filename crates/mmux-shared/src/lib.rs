use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StartupDismiss {
    pub key: String,
    pub triggers: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CliProfile {
    pub name: String,
    pub cmd: Option<String>,
    #[serde(default)]
    pub permission_bypass_cmd: Option<String>,
    pub prompt_indicator: String,
    pub busy_indicators: Vec<String>,
    pub startup_dismiss: Option<StartupDismiss>,
    #[serde(default)]
    pub launch: Option<CoderProfileLaunch>,
    pub approve_keys: String,
    pub reject_keys: String,
    pub cancel_keys: String,
    pub escape_keys: String,
}

impl Default for CliProfile {
    fn default() -> Self {
        Self {
            name: "generic".into(),
            cmd: None,
            permission_bypass_cmd: None,
            prompt_indicator: "$".into(),
            busy_indicators: vec![],
            startup_dismiss: None,
            launch: None,
            approve_keys: "y Enter".into(),
            reject_keys: "n Enter".into(),
            cancel_keys: "C-c".into(),
            escape_keys: "Escape".into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct CoderProfileLaunch {
    pub scripts_dir: Option<String>,
    pub assets_dir: Option<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub secrets: Vec<CoderProfileSecretBinding>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CoderProfileSecretBinding {
    pub env: String,
    pub value_from: String,
    #[serde(default)]
    pub placeholder: Option<String>,
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
