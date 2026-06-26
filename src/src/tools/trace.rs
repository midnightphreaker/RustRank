use std::path::Path;

use regex::Regex;

use crate::{
    context::{Context, LocalModuleResolver, supported_source_files},
    error::Result,
    fmt::TraceRow,
};

use super::search::{rel, validate_dir};

pub fn trace_data_flow(
    repo_path: &str,
    identifier: &str,
    include_transformations: bool,
    include_side_effects: bool,
) -> Result<Vec<TraceRow>> {
    let root = Path::new(repo_path);
    validate_dir(root)?;
    let ctx = Context::new(root.to_path_buf());
    let modules = ctx.parse_all()?;
    let ident = Regex::new(&format!(r"\b{}\b", regex::escape(identifier)))
        .map_err(|err| crate::AppError::Validation(err.to_string()))?;
    let mut rows = Vec::new();
    for module in modules {
        for (idx, line) in module.source_lines.iter().enumerate() {
            if !ident.is_match(line) {
                continue;
            }
            let line_no = idx + 1;
            let trimmed = line.trim();
            let kind = if module
                .defs
                .iter()
                .any(|def| def.line == line_no && ident.is_match(trimmed))
            {
                "definition"
            } else if include_transformations && trimmed.contains('=') {
                "transformation"
            } else if include_side_effects
                && (trimmed.starts_with("return ") || trimmed.contains("raise "))
            {
                "side_effect"
            } else {
                "usage"
            };
            rows.push(TraceRow {
                file: module.path.to_string_lossy().replace('\\', "/"),
                line: line_no,
                snippet: trimmed.to_string(),
                kind: kind.to_string(),
                layer: layer_for_file(&module.path.to_string_lossy()),
                chain: vec![identifier.to_string()],
            });
        }
    }
    Ok(rows)
}

pub fn trace_feature_impl(repo_path: &str, feature_keywords: &[&str]) -> Result<Vec<TraceRow>> {
    let root = Path::new(repo_path);
    validate_dir(root)?;
    let mut rows = Vec::new();
    for (file, _) in supported_source_files(root)? {
        let source = std::fs::read_to_string(&file)?;
        for (idx, line) in source.lines().enumerate() {
            let lower = line.to_lowercase();
            if let Some(keyword) = feature_keywords
                .iter()
                .find(|keyword| lower.contains(&keyword.to_lowercase()))
            {
                rows.push(TraceRow {
                    file: rel(root, &file),
                    line: idx + 1,
                    snippet: line.trim().to_string(),
                    kind: "feature_match".to_string(),
                    layer: layer_for_file(&file.to_string_lossy()),
                    chain: vec![(*keyword).to_string()],
                });
            }
        }
    }
    Ok(rows)
}

pub fn trace_dep_impact(repo_path: &str, target_module: &str) -> Result<Vec<TraceRow>> {
    let root = Path::new(repo_path);
    validate_dir(root)?;
    let ctx = Context::new(root.to_path_buf());
    let modules = ctx.parse_all()?;
    let resolver = LocalModuleResolver::new(&modules);
    let mut rows = Vec::new();
    for module in &modules {
        let dependent = resolver.module_name_for(module);
        for import in &module.imports {
            if resolver.import_matches_target(module, import, target_module) {
                let snippet = module
                    .source_lines
                    .get(import.line.saturating_sub(1))
                    .map(|line| line.trim().to_string())
                    .unwrap_or_else(|| import.module.clone());
                rows.push(TraceRow {
                    file: module.path.to_string_lossy().replace('\\', "/"),
                    line: import.line,
                    snippet,
                    kind: "direct_import".to_string(),
                    layer: layer_for_file(&module.path.to_string_lossy()),
                    chain: vec![target_module.to_string(), dependent.clone()],
                });
            }
        }
    }
    Ok(rows)
}

fn layer_for_file(file: &str) -> String {
    let lower = file.to_lowercase();
    if lower.contains("api") || lower.contains("controller") || lower.contains("endpoint") {
        "api".to_string()
    } else if lower.contains("model") || lower.contains("schema") || lower.contains("entity") {
        "data".to_string()
    } else if lower.contains("test") {
        "tests".to_string()
    } else if lower.contains("view") || lower.contains("ui") || lower.contains("template") {
        "ui".to_string()
    } else {
        "business_logic".to_string()
    }
}
