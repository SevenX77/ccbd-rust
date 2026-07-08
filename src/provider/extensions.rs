use crate::provider::fingerprint::BundleDigest;
use crate::provider::skills::ResolvedSkill;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionConfig {
    #[serde(default)]
    pub hooks: HashMap<String, Vec<HookGroup>>,
    #[serde(default)]
    pub plugins: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bundle: Vec<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub settings: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp: Vec<McpServerConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_digest: Option<BundleDigest>,
    #[serde(skip, default)]
    pub resolved_skills: Vec<ResolvedSkill>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub transport: McpTransport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    Stdio,
    Http,
    Sse,
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
            CommandObject {
                #[serde(default = "default_matcher")]
                matcher: String,
                #[serde(default = "default_hook_type", rename = "type")]
                hook_type: String,
                command: String,
                #[serde(default)]
                timeout: Option<u64>,
            },
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
            HookGroupInput::CommandObject {
                matcher,
                hook_type,
                command,
                timeout,
            } => Ok(Self {
                matcher,
                hooks: vec![HookItem {
                    hook_type,
                    command,
                    timeout,
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

#[cfg(test)]
mod tests {
    use super::HookGroup;

    #[test]
    fn hook_group_accepts_command_object_shorthand() {
        let group: HookGroup = toml::from_str(
            r#"matcher = "Bash"
command = "hooks/notify.sh"
timeout = 5000
"#,
        )
        .unwrap();

        assert_eq!(group.matcher, "Bash");
        assert_eq!(group.hooks.len(), 1);
        assert_eq!(group.hooks[0].hook_type, "command");
        assert_eq!(group.hooks[0].command, "hooks/notify.sh");
        assert_eq!(group.hooks[0].timeout, Some(5000));
    }
}
