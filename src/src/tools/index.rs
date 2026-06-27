use crate::{Result, index::IndexProjectResponse};

pub fn index_project(
    repo_path: &str,
    languages: Option<Vec<String>>,
    force_rebuild: bool,
    clean_stale: bool,
) -> Result<IndexProjectResponse> {
    crate::index::index_project(repo_path, languages, force_rebuild, clean_stale)
}
