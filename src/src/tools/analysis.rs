use std::{
    collections::HashSet,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    context::{Context, DefKind},
    error::{AppError, Result},
    fmt::AnalysisRow,
};

use super::search::{rel, source_files, validate_dir};

pub fn error_patterns(
    repo_path: &str,
    include_antipatterns: bool,
    show_evolution: bool,
    days_back: Option<u32>,
) -> Result<Vec<AnalysisRow>> {
    let mut rows = scan_py(repo_path, |file, line_no, line| {
        let trimmed = line.trim();
        let pattern = if trimmed.starts_with("try:") {
            Some(("try", "info"))
        } else if trimmed.starts_with("except ") || trimmed.starts_with("except:") {
            Some(("except", "info"))
        } else if trimmed.starts_with("raise ") {
            Some(("raise", "warning"))
        } else if include_antipatterns
            && (trimmed.contains("unwrap(") || trimmed.contains("panic!("))
        {
            Some(("panic_or_unwrap", "high"))
        } else {
            None
        };
        pattern.map(|(pattern, severity)| AnalysisRow {
            file,
            line: line_no,
            snippet: trimmed.to_string(),
            pattern: pattern.to_string(),
            severity: severity.to_string(),
            kind: "error_pattern".to_string(),
        })
    })?;

    if show_evolution && let Ok(mut evolution_rows) = git_evolution_rows(repo_path, days_back) {
        rows.append(&mut evolution_rows);
    }

    Ok(rows)
}

pub fn perf_bottleneck(
    repo_path: &str,
    focus_areas: &[&str],
    include_utility: bool,
) -> Result<Vec<AnalysisRow>> {
    let focus = if focus_areas.is_empty() {
        vec!["sleep", "range", "append", "push"]
    } else {
        focus_areas.to_vec()
    };
    scan_py(repo_path, |file, line_no, line| {
        let trimmed = line.trim();
        if !include_utility && file.contains("util") {
            return None;
        }
        let lower = trimmed.to_lowercase();
        focus
            .iter()
            .find(|pattern| lower.contains(&pattern.to_lowercase()))
            .map(|pattern| AnalysisRow {
                file,
                line: line_no,
                snippet: trimmed.to_string(),
                pattern: (*pattern).to_string(),
                severity: if *pattern == "sleep" {
                    "high"
                } else {
                    "medium"
                }
                .to_string(),
                kind: "perf_bottleneck".to_string(),
            })
    })
}

pub fn exec_paths(
    repo_path: &str,
    function_name: &str,
    max_depth: usize,
    include_call_contexts: bool,
) -> Result<Vec<AnalysisRow>> {
    let root = Path::new(repo_path);
    validate_dir(root)?;
    let ctx = Context::new(root.to_path_buf());
    let modules = ctx.parse_all()?;
    let mut rows = Vec::new();
    for module in modules {
        for def in module
            .defs
            .iter()
            .filter(|def| def.kind == DefKind::Func && def.name == function_name)
        {
            let mut depth = 0;
            let start = def.line.min(module.source_lines.len());
            let end = def.end_line.min(module.source_lines.len());
            for (idx, line) in module.source_lines[start..end].iter().enumerate() {
                let line_no = start + idx + 1;
                let trimmed = line.trim();
                let kind = if trimmed.starts_with("if ") || trimmed.starts_with("elif ") {
                    Some("branch")
                } else if trimmed.starts_with("for ") || trimmed.starts_with("while ") {
                    Some("loop")
                } else if trimmed.starts_with("try:") || trimmed.starts_with("except ") {
                    Some("error_path")
                } else if include_call_contexts && trimmed.contains('(') && trimmed.contains(')') {
                    Some("call")
                } else {
                    None
                };
                if let Some(kind) = kind {
                    if depth >= max_depth {
                        continue;
                    }
                    depth += 1;
                    rows.push(AnalysisRow {
                        file: module.path.to_string_lossy().replace('\\', "/"),
                        line: line_no,
                        snippet: trimmed.to_string(),
                        pattern: function_name.to_string(),
                        severity: "info".to_string(),
                        kind: kind.to_string(),
                    });
                }
            }
        }
    }
    Ok(rows)
}

pub fn execute_paths(
    repo_path: &str,
    function_name: &str,
    max_depth: usize,
    include_call_contexts: bool,
) -> Result<Vec<AnalysisRow>> {
    exec_paths(repo_path, function_name, max_depth, include_call_contexts)
}

fn scan_py(
    repo_path: &str,
    mut f: impl FnMut(String, usize, &str) -> Option<AnalysisRow>,
) -> Result<Vec<AnalysisRow>> {
    let root = Path::new(repo_path);
    validate_dir(root)?;
    let mut rows = Vec::new();
    for file in source_files(root, Some("py"))? {
        let source = std::fs::read_to_string(&file)?;
        for (idx, line) in source.lines().enumerate() {
            if let Some(row) = f(rel(root, &file), idx + 1, line) {
                rows.push(row);
            }
        }
    }
    Ok(rows)
}

fn git_evolution_rows(repo_path: &str, days_back: Option<u32>) -> Result<Vec<AnalysisRow>> {
    let root = Path::new(repo_path);
    let repo = git2::Repository::discover(root)?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| AppError::NotAGit(root.to_path_buf()))?;
    let since = days_back.map(|days| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or_default()
            - i64::from(days) * 86_400
    });
    let mut rows = Vec::new();

    for file in source_files(root, Some("py"))? {
        let rel_workdir = file
            .strip_prefix(workdir)
            .map_err(|err| AppError::Context(err.to_string()))?;
        let blame = repo.blame_file(rel_workdir, None)?;
        let mut commit_ids = HashSet::new();
        for hunk in blame.iter() {
            let oid = hunk.final_commit_id();
            if !oid.is_zero() {
                commit_ids.insert(oid);
            }
        }

        let mut commits = 0;
        for oid in commit_ids {
            let commit = repo.find_commit(oid)?;
            if since.is_none_or(|minimum| commit.time().seconds() >= minimum) {
                commits += 1;
            }
        }
        if commits > 0 {
            rows.push(AnalysisRow {
                file: rel(root, &file),
                line: 1,
                snippet: format!("{commits} blame commits in analysis window"),
                pattern: "evolution".to_string(),
                severity: "info".to_string(),
                kind: "error_pattern_evolution".to_string(),
            });
        }
    }

    Ok(rows)
}
