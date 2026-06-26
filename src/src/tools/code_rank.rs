use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use graphops::{AdjacencyMatrix, PageRankConfig, pagerank};

use crate::{
    context::{Context, LocalModuleResolver, ModuleDef, supported_source_files},
    error::{AppError, Result},
    fmt::{CodeRankRow, HotspotRow},
};

pub fn coderank_analysis(
    repo_path: &str,
    top_n: usize,
    module_prefix: Option<&str>,
    external_modules: bool,
) -> Result<Vec<CodeRankRow>> {
    let ctx = Context::new(Path::new(repo_path).to_path_buf());
    let modules = ctx.parse_all()?;
    let graph = build_graph(&modules, external_modules);
    let mut rows = graph
        .modules
        .iter()
        .enumerate()
        .map(|(idx, module)| CodeRankRow {
            module: module.clone(),
            score: graph.scores.get(idx).copied().unwrap_or(0.0),
            imports: graph.outgoing.get(idx).map_or(0, HashSet::len),
            depth: graph.incoming.get(idx).map_or(0, HashSet::len),
        })
        .filter(|row| module_prefix.is_none_or(|prefix| row.module.starts_with(prefix)))
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| b.depth.cmp(&a.depth))
            .then_with(|| a.module.cmp(&b.module))
    });
    rows.truncate(top_n.max(1));
    Ok(rows)
}

pub fn code_hotspots(
    repo_path: &str,
    top_n: usize,
    min_connections: usize,
) -> Result<Vec<HotspotRow>> {
    let rank_rows = coderank_analysis(repo_path, usize::MAX, None, false)?;
    let mut rows = rank_rows
        .into_iter()
        .filter_map(|row| {
            let connections = row.imports + row.depth;
            if connections < min_connections {
                return None;
            }
            let change_frequency = git_change_frequency(repo_path, &row.module)
                .or_else(|_| textual_frequency(repo_path, &row.module))
                .unwrap_or(connections);
            Some(HotspotRow {
                module: row.module,
                score: row.score * ((change_frequency + 1) as f64).ln_1p(),
                imports: row.imports,
                change_frequency,
            })
        })
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.module.cmp(&b.module))
    });
    rows.truncate(top_n.max(1));
    Ok(rows)
}

pub(crate) fn coderank_map(repo_path: &str) -> Result<HashMap<String, f64>> {
    Ok(coderank_analysis(repo_path, usize::MAX, None, false)?
        .into_iter()
        .map(|row| (row.module, row.score))
        .collect())
}

struct ImportGraph {
    modules: Vec<String>,
    outgoing: Vec<HashSet<usize>>,
    incoming: Vec<HashSet<usize>>,
    scores: Vec<f64>,
}

fn build_graph(modules: &[ModuleDef], include_external: bool) -> ImportGraph {
    let resolver = LocalModuleResolver::new(modules);
    let mut module_names = resolver.module_names().to_vec();

    let mut index = module_names
        .iter()
        .enumerate()
        .map(|(idx, module)| (module.clone(), idx))
        .collect::<HashMap<_, _>>();

    if include_external {
        for module in modules {
            for import in &module.imports {
                if !import.module.is_empty()
                    && resolver.resolve_import_targets(module, import).is_empty()
                    && !index.contains_key(&import.module)
                {
                    let idx = module_names.len();
                    index.insert(import.module.clone(), idx);
                    module_names.push(import.module.clone());
                }
            }
        }
    }

    let mut matrix = vec![vec![0.0; module_names.len()]; module_names.len()];
    let mut outgoing = vec![HashSet::new(); module_names.len()];
    let mut incoming = vec![HashSet::new(); module_names.len()];

    for module in modules {
        let Some(source_idx) = resolver.module_index(&resolver.module_name_for(module)) else {
            continue;
        };
        for import in &module.imports {
            let mut targets = resolver.resolve_import_targets(module, import);
            if targets.is_empty() && include_external && !import.module.is_empty() {
                targets.push(import.module.clone());
            }
            for target in targets {
                let Some(target_idx) = index.get(&target).copied() else {
                    continue;
                };
                matrix[source_idx][target_idx] = 1.0;
                outgoing[source_idx].insert(target_idx);
                incoming[target_idx].insert(source_idx);
            }
        }
    }

    let scores = if module_names.is_empty() {
        Vec::new()
    } else if matrix
        .iter()
        .all(|row| row.iter().all(|score| *score == 0.0))
    {
        vec![1.0 / module_names.len() as f64; module_names.len()]
    } else {
        pagerank(&AdjacencyMatrix(&matrix), PageRankConfig::default())
    };

    ImportGraph {
        modules: module_names,
        outgoing,
        incoming,
        scores,
    }
}

fn textual_frequency(repo_path: &str, module: &str) -> Result<usize> {
    let root = Path::new(repo_path);
    let last = module.rsplit('.').next().unwrap_or(module);
    let mut count = 0;
    for (file, _) in supported_source_files(root)? {
        let source = std::fs::read_to_string(file)?;
        count += source.matches(module).count();
        count += source.matches(last).count();
    }
    Ok(count)
}

fn git_change_frequency(repo_path: &str, module: &str) -> Result<usize> {
    let root = Path::new(repo_path);
    let repo = git2::Repository::discover(root)?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| AppError::NotAGit(root.to_path_buf()))?;
    let abs_path = module_path(root, module);
    let rel_path = abs_path
        .strip_prefix(workdir)
        .map_err(|err| AppError::Context(err.to_string()))?;
    let blame = repo.blame_file(rel_path, None)?;
    let commits = blame
        .iter()
        .map(|hunk| hunk.final_commit_id())
        .filter(|oid| !oid.is_zero())
        .collect::<HashSet<_>>();
    Ok(commits.len().max(1))
}

fn module_path(root: &Path, module: &str) -> PathBuf {
    let rel_module = module.replace('.', "/");
    let file = root.join(format!("{rel_module}.py"));
    if file.exists() {
        file
    } else {
        root.join(rel_module).join("__init__.py")
    }
}
