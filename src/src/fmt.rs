use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchRow {
    pub file: String,
    pub line: usize,
    pub snippet: String,
    pub context_before: Vec<String>,
    pub context_after: Vec<String>,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiUsageRow {
    pub file: String,
    pub line: usize,
    pub snippet: String,
    pub pattern_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeRankRow {
    pub module: String,
    pub score: f64,
    pub imports: usize,
    pub depth: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HotspotRow {
    pub module: String,
    pub score: f64,
    pub imports: usize,
    pub change_frequency: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceRow {
    pub file: String,
    pub line: usize,
    pub snippet: String,
    pub kind: String,
    pub layer: String,
    pub chain: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisRow {
    pub file: String,
    pub line: usize,
    pub snippet: String,
    pub pattern: String,
    pub severity: String,
    pub kind: String,
}
