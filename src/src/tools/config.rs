use std::path::Path;

use serde_json::Value;

use crate::{error::Result, project_config};

pub fn get_config(repo_path: &str) -> Result<Value> {
    project_config::get_raw_config(Path::new(repo_path))
}

pub fn set_config(repo_path: &str, key: &str, value: Value) -> Result<Value> {
    project_config::set_raw_config_value(Path::new(repo_path), key, value)
}
