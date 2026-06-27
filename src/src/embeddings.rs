use std::{
    collections::HashMap,
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

#[derive(Clone, Default, PartialEq, Eq)]
pub struct EmbeddingOptions {
    pub enabled: Option<bool>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub dimensions: Option<usize>,
    pub api_key: Option<String>,
}

impl std::fmt::Debug for EmbeddingOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddingOptions")
            .field("enabled", &self.enabled)
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("dimensions", &self.dimensions)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
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
}

impl std::fmt::Debug for EmbeddingConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddingConfig")
            .field("enabled", &self.enabled)
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("dimensions", &self.dimensions)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct EmbeddingSource {
    pub path: String,
    pub content_hash: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CachedEmbedding {
    pub schema: String,
    pub path: String,
    pub content_hash: String,
    pub model: String,
    pub dimensions: usize,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmbeddingIndexStats {
    pub indexed: usize,
    pub cache_hits: usize,
    pub warnings: Vec<String>,
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
    })
}

pub fn index_embeddings(
    root: &Path,
    config: &EmbeddingConfig,
    sources: &[EmbeddingSource],
) -> Result<EmbeddingIndexStats> {
    let mut stats = EmbeddingIndexStats::default();
    if !config.enabled {
        return Ok(stats);
    }

    let cache_dir = cache_dir(root);
    std::fs::create_dir_all(&cache_dir)?;
    for source in sources {
        let cache_path = cache_file(&cache_dir, &source.content_hash);
        if let Some(cached) = read_cached_embedding(
            &cache_path,
            config,
            &source.content_hash,
            &mut stats.warnings,
        )? && cached.path == source.path
        {
            stats.cache_hits += 1;
            continue;
        }

        match fetch_embedding(config, &source.content) {
            Ok(embedding) if embedding.len() == config.dimensions => {
                let cached = CachedEmbedding {
                    schema: "rustrank_embedding".to_string(),
                    path: source.path.clone(),
                    content_hash: source.content_hash.clone(),
                    model: config.model.clone(),
                    dimensions: config.dimensions,
                    embedding,
                };
                write_json_atomic(&cache_path, &cached)?;
                stats.indexed += 1;
            }
            Ok(embedding) => stats.warnings.push(format!(
                "embedding dimension mismatch for {}: expected {}, got {}",
                source.path,
                config.dimensions,
                embedding.len()
            )),
            Err(err) => stats.warnings.push(format!(
                "embedding request failed for {}: {}",
                source.path, err
            )),
        }
    }
    Ok(stats)
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
        if cached.dimensions != expected_dimensions || cached.embedding.len() != expected_dimensions
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
) -> Result<(HashMap<String, f64>, Vec<String>)> {
    if !config.enabled {
        return Ok((HashMap::new(), Vec::new()));
    }
    let (cached, mut warnings) = cached_embeddings(root, config.dimensions)?;
    if cached.is_empty() {
        return Ok((HashMap::new(), warnings));
    }
    let query_embedding = match fetch_embedding(config, query) {
        Ok(embedding) if embedding.len() == config.dimensions => embedding,
        Ok(embedding) => {
            warnings.push(format!(
                "query embedding dimension mismatch: expected {}, got {}",
                config.dimensions,
                embedding.len()
            ));
            return Ok((HashMap::new(), warnings));
        }
        Err(err) => {
            warnings.push(format!("query embedding request failed: {err}"));
            return Ok((HashMap::new(), warnings));
        }
    };

    let scores = cached
        .into_iter()
        .map(|cached| {
            (
                cached.path,
                cosine_similarity(&query_embedding, &cached.embedding).max(0.0),
            )
        })
        .collect();
    Ok((scores, warnings))
}

pub fn fetch_embedding(config: &EmbeddingConfig, input: &str) -> Result<Vec<f32>> {
    let config = config.clone();
    let input = input.to_string();
    std::thread::spawn(move || fetch_embedding_blocking(&config, &input))
        .join()
        .map_err(|_| AppError::Context("embedding request thread panicked".to_string()))?
}

fn fetch_embedding_blocking(config: &EmbeddingConfig, input: &str) -> Result<Vec<f32>> {
    let url = format!("{}/embeddings", config.base_url.trim_end_matches('/'));
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|err| AppError::Context(err.to_string()))?;
    let mut request = client.post(url).json(&serde_json::json!({
        "model": config.model,
        "input": input,
        "dimensions": config.dimensions,
    }));
    if let Some(api_key) = config.api_key.as_deref().filter(|value| !value.is_empty()) {
        request = request.bearer_auth(api_key);
    }
    let response = request
        .send()
        .map_err(|err| AppError::Context(err.to_string()))?
        .error_for_status()
        .map_err(|err| AppError::Context(err.to_string()))?;
    let body = response
        .json::<EmbeddingResponse>()
        .map_err(|err| AppError::Context(err.to_string()))?;
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
    if cached.content_hash == content_hash
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

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}
