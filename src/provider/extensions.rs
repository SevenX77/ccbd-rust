use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionConfig {
    #[serde(default)]
    pub hooks: HashMap<String, Vec<HookGroup>>,
    #[serde(default)]
    pub plugins: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HookGroup {
    pub matcher: String,
    pub hooks: Vec<HookItem>,
}

impl<'de> Deserialize<'de> for HookGroup {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum HookGroupInput {
            Command(String),
            Group {
                #[serde(default = "default_matcher")]
                matcher: String,
                #[serde(default)]
                hooks: Vec<HookItem>,
            },
        }

        match HookGroupInput::deserialize(deserializer)? {
            HookGroupInput::Command(command) => Ok(Self {
                matcher: default_matcher(),
                hooks: vec![HookItem {
                    hook_type: default_hook_type(),
                    command,
                    timeout: None,
                }],
            }),
            HookGroupInput::Group { matcher, hooks } => Ok(Self { matcher, hooks }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookItem {
    #[serde(default = "default_hook_type", rename = "type")]
    pub hook_type: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

pub fn default_matcher() -> String {
    "*".to_string()
}

fn default_hook_type() -> String {
    "command".to_string()
}
