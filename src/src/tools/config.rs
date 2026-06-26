use std::path::Path;

use serde_json::{Map, Value};

use crate::error::Result;

pub fn get_config(repo_path: &str) -> Result<Value> {
    let path = config_path(repo_path);
    if !path.exists() {
        return Ok(Value::Object(Map::new()));
    }
    let source = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&source)?)
}

pub fn set_config(repo_path: &str, key: &str, value: Value) -> Result<Value> {
    let path = config_path(repo_path);
    let mut config = match get_config(repo_path)? {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    config.insert(key.to_string(), value);
    let updated = Value::Object(config);
    std::fs::write(path, serde_json::to_string_pretty(&updated)?)?;
    Ok(updated)
}

fn config_path(repo_path: &str) -> std::path::PathBuf {
    Path::new(repo_path).join(".rustrank_config.json")
}
