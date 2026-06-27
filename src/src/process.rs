use std::collections::{HashMap, HashSet, VecDeque};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::context::{Definition, LocalModuleResolver, ModuleDef};

const MAX_PROCESS_DEPTH: usize = 6;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessFlow {
    pub name: String,
    pub entry_symbol: String,
    pub module: String,
    pub file: String,
    pub line: usize,
    pub call_chain: Vec<ProcessStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessStep {
    pub symbol: String,
    pub module: String,
    pub file: String,
    pub line: usize,
    pub depth: usize,
    pub via: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallEdge {
    pub source_module: String,
    pub source_file: String,
    pub source_symbol: String,
    pub source_line: usize,
    pub target_module: String,
    pub target_file: String,
    pub target_symbol: String,
    pub target_line: usize,
    pub call_line: usize,
}

pub fn derive_processes(modules: &[ModuleDef], resolver: &LocalModuleResolver) -> Vec<ProcessFlow> {
    let calls = call_edges(modules, resolver);
    let calls_by_source = calls.into_iter().fold(
        HashMap::<String, Vec<CallEdge>>::new(),
        |mut grouped, edge| {
            grouped
                .entry(symbol_key(&edge.source_module, &edge.source_symbol))
                .or_default()
                .push(edge);
            grouped
        },
    );

    let mut processes = modules
        .iter()
        .flat_map(|module| {
            module
                .defs
                .iter()
                .filter(|def| is_process_entry(&def.name))
                .map(|def| process_from_entry(module, def, resolver, &calls_by_source))
        })
        .collect::<Vec<_>>();

    processes.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.module.cmp(&b.module))
            .then_with(|| a.file.cmp(&b.file))
    });
    processes
}

pub fn call_edges(modules: &[ModuleDef], resolver: &LocalModuleResolver) -> Vec<CallEdge> {
    let all_symbols = modules
        .iter()
        .flat_map(|module| module.defs.iter().map(|def| def.name.clone()))
        .collect::<HashSet<_>>();
    let defs_by_name = modules
        .iter()
        .flat_map(|module| {
            module
                .defs
                .iter()
                .map(move |def| (def.name.clone(), module, def))
        })
        .fold(
            HashMap::<String, Vec<(&ModuleDef, &Definition)>>::new(),
            |mut grouped, (name, module, def)| {
                grouped.entry(name).or_default().push((module, def));
                grouped
            },
        );

    let mut edges = Vec::new();
    for module in modules {
        let source_module = resolver.module_name_for(module);
        let source_file = path_string(&module.path);
        for def in &module.defs {
            for (called, call_line) in calls_in_definition(module, def, &all_symbols) {
                let Some(targets) = defs_by_name.get(&called) else {
                    continue;
                };
                for (target_module, target_def) in targets {
                    if std::ptr::eq(*target_module, module) && target_def.name == def.name {
                        continue;
                    }
                    edges.push(CallEdge {
                        source_module: source_module.clone(),
                        source_file: source_file.clone(),
                        source_symbol: def.name.clone(),
                        source_line: def.line,
                        target_module: resolver.module_name_for(target_module),
                        target_file: path_string(&target_module.path),
                        target_symbol: target_def.name.clone(),
                        target_line: target_def.line,
                        call_line,
                    });
                }
            }
        }
    }

    edges.sort_by(|a, b| {
        symbol_key(&a.source_module, &a.source_symbol)
            .cmp(&symbol_key(&b.source_module, &b.source_symbol))
            .then_with(|| {
                symbol_key(&a.target_module, &a.target_symbol)
                    .cmp(&symbol_key(&b.target_module, &b.target_symbol))
            })
            .then_with(|| a.call_line.cmp(&b.call_line))
    });
    edges.dedup_by(|a, b| {
        a.source_module == b.source_module
            && a.source_symbol == b.source_symbol
            && a.target_module == b.target_module
            && a.target_symbol == b.target_symbol
            && a.call_line == b.call_line
    });
    edges
}

pub fn process_for_symbol(processes: &[ProcessFlow], symbol: &str) -> Option<String> {
    processes
        .iter()
        .find(|process| {
            process.entry_symbol == symbol
                || process.call_chain.iter().any(|step| step.symbol == symbol)
        })
        .map(|process| process.name.clone())
}

fn process_from_entry(
    module: &ModuleDef,
    def: &Definition,
    resolver: &LocalModuleResolver,
    calls_by_source: &HashMap<String, Vec<CallEdge>>,
) -> ProcessFlow {
    let entry_module = resolver.module_name_for(module);
    let entry_file = path_string(&module.path);
    let mut call_chain = Vec::new();
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([(
        entry_module.clone(),
        entry_file.clone(),
        def.name.clone(),
        def.line,
        0usize,
        None,
    )]);

    while let Some((module_name, file, symbol, line, depth, via)) = queue.pop_front() {
        let key = symbol_key(&module_name, &symbol);
        if depth > MAX_PROCESS_DEPTH || !seen.insert(key.clone()) {
            continue;
        }
        call_chain.push(ProcessStep {
            symbol: symbol.clone(),
            module: module_name.clone(),
            file,
            line,
            depth,
            via,
        });
        if depth == MAX_PROCESS_DEPTH {
            continue;
        }
        if let Some(edges) = calls_by_source.get(&key) {
            for edge in edges {
                queue.push_back((
                    edge.target_module.clone(),
                    edge.target_file.clone(),
                    edge.target_symbol.clone(),
                    edge.target_line,
                    depth + 1,
                    Some(symbol.clone()),
                ));
            }
        }
    }

    ProcessFlow {
        name: def.name.clone(),
        entry_symbol: def.name.clone(),
        module: entry_module,
        file: entry_file,
        line: def.line,
        call_chain,
    }
}

fn calls_in_definition(
    module: &ModuleDef,
    def: &Definition,
    known_symbols: &HashSet<String>,
) -> Vec<(String, usize)> {
    let pattern = Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\(").expect("call regex");
    let start = def.line.saturating_sub(1);
    let end = def.end_line.max(def.line).min(module.source_lines.len());
    let mut calls = Vec::new();
    for (offset, line) in module.source_lines[start..end].iter().enumerate() {
        for capture in pattern.captures_iter(line) {
            let name = capture[1].to_string();
            if known_symbols.contains(&name) {
                calls.push((name, start + offset + 1));
            }
        }
    }
    calls.sort();
    calls.dedup();
    calls
}

fn is_process_entry(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == "main"
        || name.contains("login")
        || name.contains("authenticate")
        || name.contains("endpoint")
        || name.contains("handler")
}

fn symbol_key(module: &str, symbol: &str) -> String {
    format!("{module}::{symbol}")
}

fn path_string(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
