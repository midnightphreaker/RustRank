use std::path::{Path, PathBuf};

use regex::Regex;
use walkdir::WalkDir;

use crate::{
    context::{Language, supported_source_files},
    error::{AppError, Result},
    fmt::{ApiUsageRow, SearchRow},
    project_config,
};

use super::code_rank;

pub fn contextual_search(
    path: &str,
    pattern: &str,
    file_type: Option<&str>,
    is_regex: bool,
    num_context_lines: usize,
) -> Result<Vec<SearchRow>> {
    let root = Path::new(path);
    validate_dir(root)?;
    let regex = if is_regex {
        Regex::new(pattern).map_err(|err| AppError::Validation(err.to_string()))?
    } else {
        Regex::new(&regex::escape(pattern)).map_err(|err| AppError::Validation(err.to_string()))?
    };
    let files = source_files(root, file_type)?;
    let mut rows = Vec::new();

    for file in files {
        let source = std::fs::read_to_string(&file)?;
        let lines = source.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
        for (idx, line) in lines.iter().enumerate() {
            if regex.is_match(line) {
                rows.push(SearchRow {
                    file: rel(root, &file),
                    line: idx + 1,
                    snippet: line.trim().to_string(),
                    context_before: context_before(&lines, idx, num_context_lines),
                    context_after: context_after(&lines, idx, num_context_lines),
                    score: 0.0,
                });
            }
        }
    }

    Ok(rows)
}

pub fn smart_code_search(
    repo_path: &str,
    pattern: &str,
    context_lines: usize,
    num_context_lines: usize,
) -> Result<Vec<SearchRow>> {
    let mut rows = contextual_search(repo_path, pattern, None, false, context_lines)?
        .into_iter()
        .filter(|row| Language::from_path(Path::new(&row.file)).is_some())
        .collect::<Vec<_>>();
    let ranks = code_rank::coderank_map(repo_path)?;
    for row in &mut rows {
        let module = module_from_file(&row.file);
        row.score = ranks.get(&module).copied().unwrap_or(0.0);
    }
    rows.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
    });
    rows.truncate(num_context_lines.max(1));
    Ok(rows)
}

pub fn api_usage(
    repo_path: &str,
    api_name: &str,
    max_examples: usize,
    group_by_pattern: bool,
) -> Result<Vec<ApiUsageRow>> {
    let rows = contextual_search(repo_path, api_name, None, false, 0)?
        .into_iter()
        .filter(|row| Language::from_path(Path::new(&row.file)).is_some())
        .collect::<Vec<_>>();
    let mut usage = rows
        .into_iter()
        .map(|row| {
            let pattern_key = if !group_by_pattern {
                "usage".to_string()
            } else if row.snippet.contains(&format!("{api_name}(")) {
                "call".to_string()
            } else if row.snippet.trim_start().starts_with("from ")
                || row.snippet.trim_start().starts_with("import ")
            {
                "import".to_string()
            } else if row.snippet.contains('=') {
                "assignment".to_string()
            } else {
                "reference".to_string()
            };
            ApiUsageRow {
                file: row.file,
                line: row.line,
                snippet: row.snippet,
                pattern_key,
            }
        })
        .collect::<Vec<_>>();
    usage.truncate(max_examples.max(1));
    Ok(usage)
}

pub(crate) fn source_files(root: &Path, file_type: Option<&str>) -> Result<Vec<PathBuf>> {
    if file_type.is_none() {
        return Ok(supported_source_files(root)?
            .into_iter()
            .map(|(path, _)| path)
            .collect());
    }

    let mut files = Vec::new();
    let normalized_ext = file_type.map(|ext| ext.trim_start_matches('.'));
    let excludes = project_config::configured_excludes(root)?;
    for entry in WalkDir::new(root) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if excludes.is_excluded(root, path) {
            continue;
        }
        if let Some(ext) = normalized_ext
            && path.extension().is_none_or(|candidate| candidate != ext)
        {
            continue;
        }
        files.push(path.to_path_buf());
    }
    files.sort();
    Ok(files)
}

pub(crate) fn validate_dir(path: &Path) -> Result<()> {
    if !path.is_dir() {
        return Err(AppError::Validation(format!(
            "repository path is not a directory: {}",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn rel(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
}

pub(crate) fn module_from_file(file: &str) -> String {
    let without_ext = [
        ".py", ".rs", ".cs", ".tsx", ".ts", ".jsx", ".js", ".mjs", ".cjs", ".c", ".h", ".h++",
        ".hh", ".hpp", ".hxx", ".c++", ".cc", ".cpp", ".cxx", ".go",
    ]
    .iter()
    .find_map(|ext| file.strip_suffix(ext))
    .unwrap_or(file);
    without_ext
        .trim_end_matches("/__init__")
        .replace(['/', '\\'], ".")
}

fn context_before(lines: &[String], idx: usize, count: usize) -> Vec<String> {
    let start = idx.saturating_sub(count);
    lines[start..idx].to_vec()
}

fn context_after(lines: &[String], idx: usize, count: usize) -> Vec<String> {
    let end = (idx + 1 + count).min(lines.len());
    lines[idx + 1..end].to_vec()
}
