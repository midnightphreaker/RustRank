use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
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

pub struct ConfiguredExcludes {
    path_globs: GlobSet,
    extensions: HashSet<String>,
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
    set_nested_value(&mut config, key, value);
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

pub fn configured_excludes(repo_path: &Path) -> Result<ConfiguredExcludes> {
    let raw = get_raw_config(repo_path)?;
    let mut path_patterns = default_excluded_paths()
        .into_iter()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let mut extensions = default_excluded_extensions()
        .into_iter()
        .map(ToOwned::to_owned)
        .collect::<HashSet<_>>();

    if let Some(values) = raw
        .get("excludes")
        .and_then(|excludes| excludes.get("paths"))
        .and_then(Value::as_array)
    {
        path_patterns.extend(
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned),
        );
    }

    if let Some(values) = raw
        .get("excludes")
        .and_then(|excludes| excludes.get("extensions"))
        .and_then(Value::as_array)
    {
        extensions.extend(
            values
                .iter()
                .filter_map(Value::as_str)
                .map(normalize_extension)
                .filter(|ext| !ext.is_empty()),
        );
    }

    let mut builder = GlobSetBuilder::new();
    for pattern in path_patterns {
        builder.add(
            GlobBuilder::new(&pattern)
                .literal_separator(true)
                .build()
                .map_err(|err| crate::error::AppError::Validation(err.to_string()))?,
        );
    }

    Ok(ConfiguredExcludes {
        path_globs: builder
            .build()
            .map_err(|err| crate::error::AppError::Validation(err.to_string()))?,
        extensions,
    })
}

impl ConfiguredExcludes {
    pub fn is_excluded(&self, root: &Path, path: &Path) -> bool {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        if self.path_globs.is_match(&rel) {
            return true;
        }
        path.extension()
            .map(|ext| normalize_extension(&ext.to_string_lossy()))
            .is_some_and(|ext| self.extensions.contains(&ext))
    }
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

fn set_nested_value(config: &mut Map<String, Value>, key: &str, value: Value) {
    let parts = key
        .split('.')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() <= 1 {
        config.insert(key.to_string(), value);
        return;
    }

    let mut current = config;
    for part in &parts[..parts.len() - 1] {
        let entry = current
            .entry((*part).to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        current = entry.as_object_mut().expect("object inserted above");
    }
    current.insert(parts[parts.len() - 1].to_string(), value);
}

fn default_excluded_paths() -> Vec<&'static str> {
    vec![
        ".git/**",
        ".rustrank/**",
        "target/**",
        "node_modules/**",
        "dist/**",
        "build/**",
        ".venv/**",
        "venv/**",
        "**/__pycache__/**",
        ".pytest_cache/**",
    ]
}

fn default_excluded_extensions() -> Vec<&'static str> {
    vec![
        "pyc", "pyo", "so", "dll", "dylib", "o", "a", "rlib", "png", "jpg", "jpeg", "gif", "webp",
        "ico", "mp3", "mp4", "mov", "pdf", "zip", "tar", "gz",
    ]
}

fn normalize_extension(value: &str) -> String {
    value.trim().trim_start_matches('.').to_ascii_lowercase()
}
