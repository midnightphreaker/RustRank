use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::{
    context::{Language, all_supported_source_files},
    error::Result,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredLanguages {
    pub enabled: Vec<Language>,
    pub invalid: Vec<String>,
    pub auto_detected: bool,
}

pub fn get_raw_config(repo_path: &Path) -> Result<Value> {
    let path = config_path(repo_path);
    if !path.exists() {
        return Ok(Value::Object(Map::new()));
    }
    let source = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&source)?)
}

pub fn set_raw_config_value(repo_path: &Path, key: &str, value: Value) -> Result<Value> {
    let path = config_path(repo_path);
    let mut config = match get_raw_config(repo_path)? {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    config.insert(key.to_string(), value);
    let updated = Value::Object(config);
    std::fs::write(path, serde_json::to_string_pretty(&updated)?)?;
    Ok(updated)
}

pub fn configured_languages(repo_path: &Path) -> Result<ConfiguredLanguages> {
    let raw = get_raw_config(repo_path)?;
    let Some(values) = raw
        .get("languages")
        .and_then(|languages| languages.get("enabled"))
        .and_then(Value::as_array)
    else {
        return auto_detected_languages(repo_path);
    };

    let mut enabled = Vec::new();
    let mut invalid = Vec::new();
    for value in values {
        let Some(raw_name) = value.as_str() else {
            invalid.push(value.to_string());
            continue;
        };
        match Language::from_config_name(raw_name) {
            Some(language) if !enabled.contains(&language) => enabled.push(language),
            Some(_) => {}
            None => invalid.push(raw_name.to_string()),
        }
    }

    if enabled.is_empty() {
        let mut detected = auto_detected_languages(repo_path)?;
        detected.invalid = invalid;
        return Ok(detected);
    }

    Ok(ConfiguredLanguages {
        enabled,
        invalid,
        auto_detected: false,
    })
}

pub fn enabled_languages(repo_path: &Path) -> Result<Vec<Language>> {
    Ok(configured_languages(repo_path)?.enabled)
}

pub fn config_path(repo_path: &Path) -> PathBuf {
    repo_path.join(".rustrank_config.json")
}

fn auto_detected_languages(repo_path: &Path) -> Result<ConfiguredLanguages> {
    let mut enabled = Vec::new();
    for (_, language) in all_supported_source_files(repo_path)? {
        if !enabled.contains(&language) {
            enabled.push(language);
        }
    }
    enabled.sort_by_key(|language| language.order());
    Ok(ConfiguredLanguages {
        enabled,
        invalid: Vec::new(),
        auto_detected: true,
    })
}
