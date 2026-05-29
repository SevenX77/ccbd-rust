use crate::error::CcbdError;
use crate::provider::extensions::HookGroup;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub enum ConfigRole<'a> {
    Master {
        cmd: &'a str,
    },
    Agent {
        provider: &'a str,
        env: &'a HashMap<String, String>,
    },
}

pub struct ConfigFingerprintInput<'a> {
    pub role: ConfigRole<'a>,
    pub hooks: &'a HashMap<String, Vec<HookGroup>>,
    pub plugins: &'a [String],
}

pub fn compute_config_hash(input: &ConfigFingerprintInput<'_>) -> Result<String, CcbdError> {
    let role = match &input.role {
        ConfigRole::Master { cmd } => json!({
            "kind": "master",
            "cmd": cmd,
        }),
        ConfigRole::Agent { provider, env } => json!({
            "kind": "agent",
            "provider": provider,
            "env": env,
        }),
    };
    let mut plugins = input.plugins.to_vec();
    plugins.sort();
    let value = json!({
        "role": role,
        "hooks": input.hooks,
        "plugins": plugins,
    });
    let json = deterministic_json(value)?;
    let digest = Sha256::digest(json.as_bytes());
    Ok(format!("{digest:x}"))
}

pub fn deterministic_json(value: Value) -> Result<String, CcbdError> {
    serde_json::to_string(&sort_value(value))
        .map_err(|err| CcbdError::IpcInvalidRequest(format!("serialize config fingerprint: {err}")))
}

fn sort_value(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted = Map::new();
            let mut entries = map.into_iter().collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            for (key, value) in entries {
                sorted.insert(key, sort_value(value));
            }
            Value::Object(sorted)
        }
        Value::Array(values) => Value::Array(values.into_iter().map(sort_value).collect()),
        value => value,
    }
}
