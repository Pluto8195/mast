//! Read-only preparation for workflow pack export and import.
//!
//! File output and registry mutations are intentionally not exposed here:
//! the redaction and conflict previews must be confirmed before either.

use std::io::Read;
use std::path::Path;

use mast_proto::WorkOutlineSnapshot;
use mast_proto::workflow_pack::{
    WorkflowPack, WorkflowPackMode, WorkflowPackRedaction, WorkflowPackScopePreview,
    WorkflowPackTemplatePreview, collect_workflow_pack_candidate, preview_instruction_import,
    preview_template_import, redact_workflow_pack,
};
use serde::Deserialize;

/// Maximum serialized size accepted from an untrusted pack file.
pub const MAX_WORKFLOW_PACK_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportPreviewParams {
    pub space_id: String,
    pub mode: WorkflowPackMode,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportPreviewParams {
    pub space_id: String,
    pub path: std::path::PathBuf,
}

/// Prepare the exact candidate and detected replacements for review.
pub fn preview_export(
    snapshot: &WorkOutlineSnapshot,
    mode: WorkflowPackMode,
) -> Result<(WorkflowPack, Vec<WorkflowPackRedaction>), String> {
    let candidate = collect_workflow_pack_candidate(snapshot)?;
    redact_workflow_pack(&candidate, mode)
}

/// Read and validate a bounded local JSON pack, without changing a Space.
pub fn read_pack(path: &Path) -> Result<WorkflowPack, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_WORKFLOW_PACK_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_WORKFLOW_PACK_BYTES {
        return Err("Workflow pack exceeds the size limit".into());
    }
    let pack: WorkflowPack = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    pack.validate()?;
    Ok(pack)
}

/// Resolve incoming instruction scopes against the current target snapshot.
pub fn preview_import(
    path: &Path,
    snapshot: &WorkOutlineSnapshot,
) -> Result<
    (
        WorkflowPack,
        Vec<WorkflowPackScopePreview>,
        Vec<WorkflowPackTemplatePreview>,
    ),
    String,
> {
    let pack = read_pack(path)?;
    let scopes = preview_instruction_import(&pack, snapshot)?;
    let templates = preview_template_import(&pack, snapshot)?;
    Ok((pack, scopes, templates))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_oversized_pack_before_parsing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oversized.json");
        std::fs::write(&path, vec![b' '; MAX_WORKFLOW_PACK_BYTES as usize + 1]).unwrap();
        assert!(read_pack(&path).unwrap_err().contains("size limit"));
    }
}
