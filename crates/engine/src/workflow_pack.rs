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
use mast_proto::{CreateWorkItemParams, WorkOutlineMutation, WorkOutlinePlacement, derive_id};
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

/// Plan missing path components without changing the registry. IDs derive
/// from the import operation and normalized paths so shared ancestors are
/// created once. The caller must revalidate the destination before applying.
pub fn plan_missing_items(
    snapshot: &WorkOutlineSnapshot,
    scopes: &[WorkflowPackScopePreview],
    operation_id: &str,
) -> Result<Vec<WorkOutlineMutation>, String> {
    let mut paths: std::collections::HashMap<Vec<String>, String> = Default::default();
    let mut mutations = Vec::new();
    for scope in scopes {
        let Some(missing_from) = scope.missing_from else {
            continue;
        };
        let mut parent = None;
        let mut normalized = Vec::new();
        for (index, title) in scope.item_path.iter().enumerate() {
            normalized.push(title.trim().to_lowercase());
            if let Some(id) = paths.get(&normalized) {
                parent = Some(id.clone());
                continue;
            }
            if index < missing_from {
                let matches: Vec<_> = snapshot
                    .items
                    .iter()
                    .filter(|item| {
                        item.archived_at.is_none()
                            && item.parent_work_item_id == parent
                            && item.title.trim().to_lowercase() == normalized[index]
                    })
                    .collect();
                let [item] = matches.as_slice() else {
                    return Err("Destination path changed since preview".into());
                };
                parent = Some(item.id.clone());
                paths.insert(normalized.clone(), item.id.clone());
                continue;
            }
            if parent
                .as_deref()
                .and_then(|id| snapshot.item(id))
                .is_some_and(|item| item.container_template.is_some())
            {
                return Err(
                    "Cannot create an imported item under a container with child rules".into(),
                );
            }
            let path_key = serde_json::to_string(&normalized).map_err(|error| error.to_string())?;
            let op = derive_id(operation_id, &path_key);
            let id = derive_id(&op, mast_proto::work_outline::roles::ITEM);
            mutations.push(WorkOutlineMutation::CreateWorkItem(CreateWorkItemParams {
                space_id: snapshot.space_id.clone(),
                parent_work_item_id: parent,
                placement: WorkOutlinePlacement::Last,
                title: title.clone(),
                description: None,
                status: None,
                operation_id: op,
            }));
            paths.insert(normalized.clone(), id.clone());
            parent = Some(id);
        }
    }
    Ok(mutations)
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

    #[test]
    fn missing_paths_share_their_planned_ancestors() {
        let snapshot: WorkOutlineSnapshot = serde_json::from_value(serde_json::json!({
            "spaceId":"space", "available":true, "synced":true
        }))
        .unwrap();
        let scopes = vec![
            WorkflowPackScopePreview {
                item_path: vec!["Projects".into(), "One".into()],
                work_item_id: None,
                missing_from: Some(0),
                current: vec![],
                incoming: vec![],
            },
            WorkflowPackScopePreview {
                item_path: vec!["Projects".into(), "Two".into()],
                work_item_id: None,
                missing_from: Some(0),
                current: vec![],
                incoming: vec![],
            },
        ];
        let plan = plan_missing_items(&snapshot, &scopes, "import").unwrap();
        assert_eq!(plan.len(), 3);
        let WorkOutlineMutation::CreateWorkItem(root) = &plan[0] else {
            panic!("expected create")
        };
        let WorkOutlineMutation::CreateWorkItem(child) = &plan[2] else {
            panic!("expected create")
        };
        assert_eq!(
            child.parent_work_item_id.as_deref(),
            Some(derive_id(&root.operation_id, mast_proto::work_outline::roles::ITEM).as_str())
        );
        assert!(plan.iter().all(|mutation| match mutation {
            WorkOutlineMutation::CreateWorkItem(p) => p.operation_id.len() <= 64,
            _ => false,
        }));
    }
}
