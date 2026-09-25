use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::{AppError, Result},
    project_config,
};

pub const DEFAULT_BASE_URL: &str = "https://api.phrk.org/v1";
pub const DEFAULT_MODEL: &str = "text-image-embedding";
pub const DEFAULT_DIMENSIONS: usize = 1536;
pub const DEFAULT_MAX_INPUT_TOKENS: usize = 512;
const TOKEN_OVERHEAD: usize = 8;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct EmbeddingOptions {
    pub enabled: Option<bool>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub dimensions: Option<usize>,
    pub api_key: Option<String>,
    pub max_input_tokens: Option<usize>,
}

impl std::fmt::Debug for EmbeddingOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddingOptions")
            .field("enabled", &self.enabled)
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("dimensions", &self.dimensions)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("max_input_tokens", &self.max_input_tokens)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct EmbeddingConfig {
    pub enabled: bool,
    pub base_url: String,
    pub model: String,
    pub dimensions: usize,
    pub api_key: Option<String>,
    pub max_input_tokens: usize,
    pub metadata_warning: Option<String>,
}

/// Resolve server-owned settings without repository defaults, then test a real embedding.
pub fn validated_server_config() -> std::result::Result<EmbeddingConfig, String> {
    let required = |name: &str| {
        std::env::var(name)
            .ok()
            .and_then(|value| non_empty(Some(value)))
            .ok_or_else(|| format!("{name} is missing or empty"))
    };
    let base_url = required("RUSTRANK_EMBEDDING_BASE_URL")?;
    let model = required("RUSTRANK_EMBEDDING_MODEL")?;
    let dimensions = required("RUSTRANK_EMBEDDING_DIMS")?
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or("RUSTRANK_EMBEDDING_DIMS must be a positive integer")?;
    let url = reqwest::Url::parse(&base_url)
        .map_err(|_| "RUSTRANK_EMBEDDING_BASE_URL must be a valid HTTP(S) URL")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("RUSTRANK_EMBEDDING_BASE_URL must be an HTTP(S) API base URL without credentials, query or fragment".into());
    }
    let configured_limit = std::env::var("RUSTRANK_EMBEDDING_MAX_INPUT_TOKENS").ok();
    let explicit_limit = configured_limit
        .map(|value| configured_max_input_tokens(Some(value)))
        .transpose()?;
    let detected_limit = detect_max_input_tokens(
        &base_url,
        &model,
        non_empty(std::env::var("RUSTRANK_EMBEDDING_API_KEY").ok()).as_deref(),
    );
    let max_input_tokens = match (explicit_limit, detected_limit) {
        (Some(configured), Some(detected)) => configured.min(detected),
        (Some(configured), None) => configured,
        (None, Some(detected)) => detected,
        (None, None) => DEFAULT_MAX_INPUT_TOKENS,
    };
    let metadata_warning = match (explicit_limit, detected_limit) {
        (Some(configured), Some(detected)) if configured != detected => Some(format!(
            "WARNING: RUSTRANK_EMBEDDING_MAX_INPUT_TOKENS={configured} differs from endpoint max_input_tokens={detected}; RustRank uses {max_input_tokens}."
        )),
        _ => None,
    };
    let config = EmbeddingConfig {
        enabled: true,
        base_url,
        model,
        dimensions,
        api_key: non_empty(std::env::var("RUSTRANK_EMBEDDING_API_KEY").ok()),
        max_input_tokens,
        metadata_warning,
    };
    let vector = fetch_embedding(&config, "RustRank startup embedding check")
        .map_err(|err| err.to_string())?;
    if vector.len() != dimensions {
        return Err(format!(
            "embedding dimension mismatch: RUSTRANK_EMBEDDING_DIMS specifies {dimensions}, endpoint returned {}",
            vector.len()
        ));
    }
    if vector.iter().any(|value| !value.is_finite()) {
        return Err("embedding endpoint returned non-finite vector values".into());
    }
    Ok(config)
}

impl std::fmt::Debug for EmbeddingConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddingConfig")
            .field("enabled", &self.enabled)
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("dimensions", &self.dimensions)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("max_input_tokens", &self.max_input_tokens)
            .field("metadata_warning", &self.metadata_warning)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct EmbeddingSource {
    pub path: String,
    pub content_hash: String,
    pub content: String,
    pub start_line: usize,
    pub end_line: usize,
    pub symbol: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CachedEmbedding {
    pub schema: String,
    pub path: String,
    pub content_hash: String,
    pub model: String,
    pub dimensions: usize,
    pub embedding: Vec<f32>,
    #[serde(default)]
    pub start_line: usize,
    #[serde(default)]
    pub end_line: usize,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub provider_hash: String,
}

#[derive(Debug)]
pub struct SemanticMatch {
    pub path: String,
    pub line: usize,
    pub symbol: Option<String>,
    pub score: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmbeddingIndexStats {
    pub total: usize,
    pub indexed: usize,
    pub cache_hits: usize,
    pub failed: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingCoverage {
    pub enabled: bool,
    pub total_chunks: usize,
    pub cached_chunks: usize,
    pub indexed_chunks: usize,
    pub failed_chunks: usize,
    pub percent: usize,
}

impl EmbeddingIndexStats {
    pub fn coverage(&self, enabled: bool) -> EmbeddingCoverage {
        let ready = self.cache_hits + self.indexed;
        EmbeddingCoverage {
            enabled,
            total_chunks: self.total,
            cached_chunks: self.cache_hits,
            indexed_chunks: self.indexed,
            failed_chunks: self.failed,
            percent: if enabled {
                ready
                    .saturating_mul(100)
                    .checked_div(self.total)
                    .unwrap_or(100)
            } else {
                0
            },
        }
    }
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

pub fn config_for_repo(repo_path: &Path, overrides: EmbeddingOptions) -> Result<EmbeddingConfig> {
    let raw = project_config::get_raw_config(repo_path)?;
    let embeddings = raw.get("embeddings").and_then(Value::as_object);
    let config_enabled = embeddings
        .and_then(|value| value.get("enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let config_base_url = embeddings
        .and_then(|value| value.get("base_url"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let config_model = embeddings
        .and_then(|value| value.get("model"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let config_dimensions = embeddings
        .and_then(|value| value.get("dimensions"))
        .and_then(Value::as_u64)
        .map(|value| value as usize);
    let config_max_input_tokens = embeddings
        .and_then(|value| value.get("max_input_tokens"))
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok());
    let max_input_tokens = overrides
        .max_input_tokens
        .or(config_max_input_tokens)
        .unwrap_or(DEFAULT_MAX_INPUT_TOKENS);
    if max_input_tokens < 16 {
        return Err(AppError::Validation(
            "embedding max_input_tokens must be at least 16".to_string(),
        ));
    }

    Ok(EmbeddingConfig {
        enabled: overrides.enabled.unwrap_or(config_enabled),
        base_url: non_empty(overrides.base_url)
            .or(config_base_url)
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
        model: non_empty(overrides.model)
            .or(config_model)
            .unwrap_or_else(|| DEFAULT_MODEL.to_string()),
        dimensions: overrides
            .dimensions
            .or(config_dimensions)
            .unwrap_or(DEFAULT_DIMENSIONS),
        api_key: non_empty(overrides.api_key),
        max_input_tokens,
        metadata_warning: None,
    })
}

pub fn index_embeddings(
    root: &Path,
    config: &EmbeddingConfig,
    sources: &[EmbeddingSource],
) -> Result<EmbeddingIndexStats> {
    let mut stats = EmbeddingIndexStats {
        total: sources.len(),
        ..Default::default()
    };
    persist_coverage(root, &stats, config.enabled)?;
    if !config.enabled {
        return Ok(stats);
    }

    let cache_dir = cache_dir(root);
    std::fs::create_dir_all(&cache_dir)?;
    let mut last_reported_decile = 0;
    for source in sources {
        let cache_key = blake3::hash(&serde_json::to_vec(&(
            "rustrank_embedding_chunk_v2",
            &source.path,
            &source.content_hash,
            source.start_line,
            source.end_line,
            &source.symbol,
            &source.content,
            &config.model,
            config.dimensions,
            provider_hash(config),
        ))?)
        .to_hex()
        .to_string();
        let cache_path = cache_file(&cache_dir, &cache_key);
        if let Some(cached) = read_cached_embedding(
            &cache_path,
            config,
            &source.content_hash,
            &mut stats.warnings,
        )? && cached.path == source.path
        {
            stats.cache_hits += 1;
            persist_coverage(root, &stats, true)?;
            report_progress(&stats, &mut last_reported_decile);
            continue;
        }

        match fetch_embedding(config, &source.content) {
            Ok(embedding)
                if embedding.len() == config.dimensions
                    && embedding.iter().all(|v| v.is_finite()) =>
            {
                let cached = CachedEmbedding {
                    schema: "rustrank_embedding_chunk_v2".to_string(),
                    path: source.path.clone(),
                    content_hash: source.content_hash.clone(),
                    model: config.model.clone(),
                    dimensions: config.dimensions,
                    start_line: source.start_line,
                    end_line: source.end_line,
                    symbol: source.symbol.clone(),
                    provider_hash: provider_hash(config),
                    embedding,
                };
                write_json_atomic(&cache_path, &cached)?;
                stats.indexed += 1;
            }
            Ok(embedding) => {
                stats.failed += 1;
                stats.warnings.push(format!(
                    "embedding dimension mismatch for {}: expected {} finite values, got {}",
                    source.path,
                    config.dimensions,
                    embedding.len()
                ));
            }
            Err(err) => {
                stats.failed += 1;
                stats.warnings.push(format!(
                    "embedding request failed for {}:{}-{}: {}",
                    source.path, source.start_line, source.end_line, err
                ));
            }
        }
        persist_coverage(root, &stats, true)?;
        report_progress(&stats, &mut last_reported_decile);
    }
    Ok(stats)
}

fn persist_coverage(root: &Path, stats: &EmbeddingIndexStats, enabled: bool) -> Result<()> {
    write_json_atomic(
        &root.join(".rustrank/index/v1/embedding_coverage.json"),
        &stats.coverage(enabled),
    )
}

fn report_progress(stats: &EmbeddingIndexStats, last_reported_decile: &mut usize) {
    if stats.total == 0 {
        return;
    }
    let ready = stats.cache_hits + stats.indexed;
    let decile = ready * 10 / stats.total;
    if decile > *last_reported_decile {
        *last_reported_decile = decile;
        eprintln!(
            "RustRank: embedding cache coverage {}% ({ready}/{} current chunks)",
            ready * 100 / stats.total,
            stats.total
        );
    }
}

pub fn cached_embeddings(
    root: &Path,
    expected_dimensions: usize,
) -> Result<(Vec<CachedEmbedding>, Vec<String>)> {
    let mut rows = Vec::new();
    let mut warnings = Vec::new();
    let dir = cache_dir(root);
    if !dir.exists() {
        return Ok((rows, warnings));
    }
    for entry in walkdir::WalkDir::new(&dir).min_depth(1).max_depth(1) {
        let entry = entry?;
        if !entry.file_type().is_file() || entry.path().extension().is_none_or(|ext| ext != "json")
        {
            continue;
        }
        let text = match std::fs::read_to_string(entry.path()) {
            Ok(text) => text,
            Err(err) => {
                warnings.push(format!(
                    "embedding cache read failed for {}: {}",
                    entry.path().display(),
                    err
                ));
                continue;
            }
        };
        let cached = match serde_json::from_str::<CachedEmbedding>(&text) {
            Ok(cached) => cached,
            Err(err) => {
                warnings.push(format!(
                    "embedding cache parse failed for {}: {}",
                    entry.path().display(),
                    err
                ));
                continue;
            }
        };
        if cached.schema != "rustrank_embedding_chunk_v2"
            || cached.start_line == 0
            || cached.end_line < cached.start_line
        {
            continue;
        }
        if cached.dimensions != expected_dimensions
            || cached.embedding.len() != expected_dimensions
            || cached.embedding.iter().any(|v| !v.is_finite())
        {
            warnings.push(format!(
                "embedding dimension mismatch for {}: expected {}, got {}",
                cached.path,
                expected_dimensions,
                cached.embedding.len()
            ));
            continue;
        }
        rows.push(cached);
    }
    Ok((rows, warnings))
}

pub fn semantic_scores(
    root: &Path,
    query: &str,
    config: &EmbeddingConfig,
) -> Result<(Vec<SemanticMatch>, Vec<String>)> {
    if !config.enabled {
        return Ok((Vec::new(), Vec::new()));
    }
    let (cached, mut warnings) = cached_embeddings(root, config.dimensions)?;
    let mut file_hashes = HashMap::new();
    let provider = provider_hash(config);
    let cached = cached
        .into_iter()
        .filter(|row| {
            if row.model != config.model || row.provider_hash != provider {
                return false;
            }
            // Old and deleted source must not keep contributing semantic matches.
            let current_hash = file_hashes.entry(row.path.clone()).or_insert_with(|| {
                let relative = Path::new(&row.path);
                if relative.is_absolute()
                    || relative
                        .components()
                        .any(|c| !matches!(c, std::path::Component::Normal(_)))
                {
                    return None;
                }
                std::fs::read(root.join(relative))
                    .ok()
                    .map(|bytes| blake3::hash(&bytes).to_hex().to_string())
            });
            current_hash.as_deref() == Some(row.content_hash.as_str())
        })
        .collect::<Vec<_>>();
    if cached.is_empty() {
        return Ok((Vec::new(), warnings));
    }
    let query_embedding = match fetch_embedding(config, query) {
        Ok(embedding)
            if embedding.len() == config.dimensions && embedding.iter().all(|v| v.is_finite()) =>
        {
            embedding
        }
        Ok(embedding) => {
            warnings.push(format!(
                "invalid query embedding: expected {} finite values, got {} values",
                config.dimensions,
                embedding.len()
            ));
            return Ok((Vec::new(), warnings));
        }
        Err(err) => {
            warnings.push(format!("query embedding request failed: {err}"));
            return Ok((Vec::new(), warnings));
        }
    };
    // Several byte-bounded chunks can occupy one long source line. Keep its
    // strongest match instead of rewarding the location for having more chunks.
    let mut locations = HashMap::new();
    for row in cached {
        let score = cosine_similarity(&query_embedding, &row.embedding).max(0.0);
        locations
            .entry((row.path, row.start_line, row.symbol))
            .and_modify(|best: &mut f64| *best = best.max(score))
            .or_insert(score);
    }
    let scores = locations
        .into_iter()
        .map(|((path, line, symbol), score)| SemanticMatch {
            path,
            line,
            symbol,
            score,
        })
        .collect();
    Ok((scores, warnings))
}

fn provider_hash(config: &EmbeddingConfig) -> String {
    blake3::hash(
        format!(
            "{}:{}",
            config.base_url.trim_end_matches('/'),
            config.max_input_tokens
        )
        .as_bytes(),
    )
    .to_hex()
    .to_string()
}

pub fn fetch_embedding(config: &EmbeddingConfig, input: &str) -> Result<Vec<f32>> {
    let config = config.clone();
    let input = input.to_string();
    std::thread::spawn(move || fetch_embedding_blocking(&config, &input))
        .join()
        .map_err(|_| AppError::Context("embedding request thread panicked".to_string()))?
}

fn fetch_embedding_blocking(config: &EmbeddingConfig, input: &str) -> Result<Vec<f32>> {
    if config.max_input_tokens < 16 {
        return Err(AppError::Validation(
            "embedding max_input_tokens must be at least 16".to_string(),
        ));
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|err| AppError::Context(err.to_string()))?;
    let pieces = split_input(input, config.max_input_tokens - TOKEN_OVERHEAD);
    if pieces.len() == 1 {
        return fetch_embedding_once(&client, config, pieces[0]);
    }
    let mut combined = vec![0.0f64; config.dimensions];
    let mut weight_sum = 0usize;
    for (index, piece) in pieces.iter().enumerate() {
        let vector = fetch_embedding_once(&client, config, piece).map_err(|err| {
            AppError::Context(format!(
                "embedding part {}/{} failed: {err}",
                index + 1,
                pieces.len()
            ))
        })?;
        if vector.len() != config.dimensions || vector.iter().any(|value| !value.is_finite()) {
            return Err(AppError::Validation(format!(
                "embedding part {}/{} returned an invalid vector (expected {} finite values)",
                index + 1,
                pieces.len(),
                config.dimensions
            )));
        }
        let weight = piece.len();
        weight_sum += weight;
        for (sum, value) in combined.iter_mut().zip(vector) {
            *sum += f64::from(value) * weight as f64;
        }
    }
    Ok(combined
        .into_iter()
        .map(|value| (value / weight_sum as f64) as f32)
        .collect())
}

fn split_input(input: &str, max_bytes: usize) -> Vec<&str> {
    if input.is_empty() {
        return vec![input];
    }
    let mut pieces = Vec::new();
    let mut start = 0;
    while start < input.len() {
        let mut end = start.saturating_add(max_bytes).min(input.len());
        while !input.is_char_boundary(end) {
            end -= 1;
        }
        pieces.push(&input[start..end]);
        start = end;
    }
    pieces
}

fn fetch_embedding_once(
    client: &reqwest::blocking::Client,
    config: &EmbeddingConfig,
    input: &str,
) -> Result<Vec<f32>> {
    let url = format!("{}/embeddings", config.base_url.trim_end_matches('/'));
    let mut request = client.post(url).json(&serde_json::json!({
        "model": config.model,
        "input": input,
        "dimensions": config.dimensions,
    }));
    if let Some(api_key) = config.api_key.as_deref().filter(|value| !value.is_empty()) {
        request = request.bearer_auth(api_key);
    }
    let response = request.send().map_err(|err| {
        let problem = if err.is_timeout() {
            "embedding endpoint timed out after 5 seconds"
        } else if err.is_connect() {
            "cannot connect to embedding endpoint (check address, DNS, TLS and reachability)"
        } else {
            "embedding request failed (check endpoint and authentication settings)"
        };
        AppError::Context(problem.into())
    })?;
    if !response.status().is_success() {
        return Err(AppError::Context(format!(
            "embedding endpoint returned HTTP {} (check endpoint, model and authentication)",
            response.status()
        )));
    }
    let body = response
        .json::<EmbeddingResponse>()
        .map_err(|_| AppError::Validation("invalid embedding response: expected JSON data[].embedding containing numeric vector values".into()))?;
    body.data
        .into_iter()
        .next()
        .map(|data| data.embedding)
        .ok_or_else(|| AppError::Validation("embedding response contained no data".to_string()))
}

pub fn cosine_similarity(left: &[f32], right: &[f32]) -> f64 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut left_norm = 0.0f64;
    let mut right_norm = 0.0f64;
    for (&left, &right) in left.iter().zip(right) {
        let left = left as f64;
        let right = right as f64;
        dot += left * right;
        left_norm += left * left;
        right_norm += right * right;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm.sqrt() * right_norm.sqrt())
    }
}

fn read_cached_embedding(
    path: &Path,
    config: &EmbeddingConfig,
    content_hash: &str,
    warnings: &mut Vec<String>,
) -> Result<Option<CachedEmbedding>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            warnings.push(format!(
                "embedding cache read failed for {}: {}",
                path.display(),
                err
            ));
            return Ok(None);
        }
    };
    let cached = match serde_json::from_str::<CachedEmbedding>(&text) {
        Ok(cached) => cached,
        Err(err) => {
            warnings.push(format!(
                "embedding cache parse failed for {}: {}",
                path.display(),
                err
            ));
            return Ok(None);
        }
    };
    if cached.schema == "rustrank_embedding_chunk_v2"
        && cached.provider_hash == provider_hash(config)
        && cached.embedding.iter().all(|value| value.is_finite())
        && cached.content_hash == content_hash
        && cached.model == config.model
        && cached.dimensions == config.dimensions
        && cached.embedding.len() == config.dimensions
    {
        Ok(Some(cached))
    } else {
        Ok(None)
    }
}

fn cache_dir(root: &Path) -> PathBuf {
    root.join(".rustrank/index/v1/embeddings")
}

fn cache_file(cache_dir: &Path, content_hash: &str) -> PathBuf {
    cache_dir.join(format!("{content_hash}.json"))
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn configured_max_input_tokens(value: Option<String>) -> std::result::Result<usize, String> {
    match value {
        Some(value) => value
            .parse::<usize>()
            .ok()
            .filter(|n| *n >= 16)
            .ok_or_else(|| {
                "RUSTRANK_EMBEDDING_MAX_INPUT_TOKENS must be an integer of at least 16".to_string()
            }),
        None => Ok(DEFAULT_MAX_INPUT_TOKENS),
    }
}

fn detect_max_input_tokens(base_url: &str, model: &str, api_key: Option<&str>) -> Option<usize> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;
    for path in ["model/info", "models"] {
        let url = format!("{}/{path}", base_url.trim_end_matches('/'));
        let mut request = client.get(url);
        if let Some(key) = api_key {
            request = request.bearer_auth(key);
        }
        let response = match request.send() {
            Ok(response) if response.status().is_success() => response,
            _ => continue,
        };
        let value: Value = match serde_json::from_reader(response.take(2 * 1024 * 1024)) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let detected = value
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|row| {
                row.get("model_name").and_then(Value::as_str) == Some(model)
                    || row.get("id").and_then(Value::as_str) == Some(model)
            })
            .filter_map(|row| {
                row.get("model_info")
                    .and_then(|info| info.get("max_input_tokens"))
                    .or_else(|| row.get("max_input_tokens"))
                    .and_then(Value::as_u64)
                    .and_then(|limit| usize::try_from(limit).ok())
                    .filter(|limit| *limit >= 16)
            })
            .min();
        if detected.is_some() {
            return detected;
        }
    }
    None
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}
