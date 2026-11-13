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

/// One forward operation and the operation that reverses it.
#[derive(Debug, Clone)]
pub struct WorkflowPackMutationPair {
    pub forward: WorkOutlineMutation,
    pub inverse: WorkOutlineMutation,
}

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

/// Detect destination changes between a preview and a proposed application.
/// The caller must additionally serialize the check with the registry writes.
pub fn revalidate_import_preview(
    pack: &WorkflowPack,
    snapshot: &WorkOutlineSnapshot,
    scopes: &[WorkflowPackScopePreview],
    templates: &[WorkflowPackTemplatePreview],
) -> Result<(), String> {
    if preview_instruction_import(pack, snapshot)? != scopes
        || preview_template_import(pack, snapshot)? != templates
    {
        return Err("The destination changed since the import preview".into());
    }
    Ok(())
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

/// Build an inverse only when a created item is still empty and unchanged.
/// The check and write must be serialized with registry updates by the caller.
pub fn guard_created_item_undo(
    snapshot: &WorkOutlineSnapshot,
    created: &CreateWorkItemParams,
) -> Result<WorkOutlineMutation, String> {
    let id = derive_id(&created.operation_id, mast_proto::work_outline::roles::ITEM);
    let item = snapshot.item(&id).ok_or("Imported item no longer exists")?;
    let has_children = snapshot.items.iter().any(|child| {
        child.parent_work_item_id.as_deref() == Some(&id) && child.archived_at.is_none()
    });
    let has_instructions = snapshot.instructions.iter().any(|instruction| {
        instruction.work_item_id.as_deref() == Some(&id) && !instruction.archived()
    });
    let has_requests = snapshot
        .user_action_requests
        .iter()
        .any(|request| request.work_item_id == id);
    let is_destination = snapshot
        .items
        .iter()
        .any(|other| other.worker_destination_id.as_deref() == Some(&id));
    if item.archived_at.is_some()
        || item.space_id != created.space_id
        || item.parent_work_item_id != created.parent_work_item_id
        || item.title != created.title
        || item.description != created.description
        || item.status != created.status.unwrap_or_default()
        || item.updated_at != item.created_at
        || item.waiting_until.is_some()
        || item.waiting_reason.is_some()
        || item.is_project
        || item.worker_destination_id.is_some()
        || item.template_id.is_some()
        || item.container_template.is_some()
        || item.template_node_id.is_some()
        || !item.chat_ids.is_empty()
        || !item.external_reference_ids.is_empty()
        || has_children
        || has_instructions
        || has_requests
        || is_destination
    {
        return Err("Imported item changed or is no longer empty".into());
    }
    Ok(WorkOutlineMutation::ArchiveWorkItem(
        mast_proto::ArchiveWorkItemParams {
            work_item_id: id,
            descendant_archive_policy: mast_proto::DescendantArchivePolicy::ArchiveDescendants,
            operation_id: derive_id(&created.operation_id, "undo"),
        },
    ))
}

/// A reversible instruction addition. Undo must check that the instruction
/// still has its imported text before archiving it.
#[derive(Debug, Clone)]
pub struct PlannedInstructionAddition {
    pub create: WorkOutlineMutation,
    pub undo: WorkOutlineMutation,
    pub expected_text: String,
}

/// Refuse an inverse if the imported instruction changed after application.
/// The caller must check this against a fresh snapshot immediately before undo.
pub fn guard_instruction_undo(
    snapshot: &WorkOutlineSnapshot,
    addition: &PlannedInstructionAddition,
) -> Result<WorkOutlineMutation, String> {
    let WorkOutlineMutation::CreateWorkItemInstruction(created) = &addition.create else {
        return Err("Invalid instruction receipt".into());
    };
    let WorkOutlineMutation::ArchiveWorkItemInstruction(inverse) = &addition.undo else {
        return Err("Invalid instruction inverse".into());
    };
    let expected_id = derive_id(
        &created.operation_id,
        mast_proto::work_outline::roles::INSTRUCTION,
    );
    if inverse.instruction_id != expected_id {
        return Err("Instruction receipt does not match its inverse".into());
    }
    let instruction = snapshot
        .instructions
        .iter()
        .find(|item| item.id == expected_id)
        .ok_or("Imported instruction no longer exists")?;
    let same_scope = match &created.scope {
        mast_proto::InstructionScope::SpaceRoot => instruction.work_item_id.is_none(),
        mast_proto::InstructionScope::WorkItem { work_item_id } => {
            instruction.work_item_id.as_ref() == Some(work_item_id)
        }
    };
    if instruction.archived()
        || instruction.space_id != created.space_id
        || !same_scope
        || instruction.text != addition.expected_text
        || instruction.text != created.text
        || instruction.source.is_some()
        || instruction.template_instruction_id.is_some()
    {
        return Err("Imported instruction changed since import".into());
    }
    Ok(addition.undo.clone())
}

/// Plan additions to empty scopes. Existing instructions require explicit
/// conflict resolution and are deliberately refused by this operation.
pub fn plan_instruction_additions(
    snapshot: &WorkOutlineSnapshot,
    scopes: &[WorkflowPackScopePreview],
    operation_id: &str,
) -> Result<Vec<PlannedInstructionAddition>, String> {
    let mut result = Vec::new();
    for scope in scopes {
        if !scope.current.is_empty() {
            return Err("An instruction conflict requires a resolution".into());
        }
        if scope
            .missing_from
            .is_some_and(|index| index >= scope.item_path.len())
        {
            return Err("The instruction preview has an invalid missing path".into());
        }
        let item_id = if scope.missing_from.is_some() {
            let normalized: Vec<String> = scope
                .item_path
                .iter()
                .map(|part| part.trim().to_lowercase())
                .collect();
            let path = serde_json::to_string(&normalized).map_err(|error| error.to_string())?;
            let item_operation = derive_id(operation_id, &path);
            Some(derive_id(
                &item_operation,
                mast_proto::work_outline::roles::ITEM,
            ))
        } else {
            scope.work_item_id.clone()
        };
        let target = match item_id {
            Some(work_item_id) => mast_proto::InstructionScope::WorkItem { work_item_id },
            None => mast_proto::InstructionScope::SpaceRoot,
        };
        let path = serde_json::to_string(&scope.item_path).map_err(|error| error.to_string())?;
        for (index, text) in scope.incoming.iter().enumerate() {
            let op = derive_id(operation_id, &format!("instruction:{path}:{index}"));
            let instruction_id = derive_id(&op, mast_proto::work_outline::roles::INSTRUCTION);
            result.push(PlannedInstructionAddition {
                create: WorkOutlineMutation::CreateWorkItemInstruction(
                    mast_proto::CreateWorkItemInstructionParams {
                        space_id: snapshot.space_id.clone(),
                        scope: target.clone(),
                        text: text.clone(),
                        placement: None,
                        source: None,
                        operation_id: op.clone(),
                    },
                ),
                undo: WorkOutlineMutation::ArchiveWorkItemInstruction(
                    mast_proto::ArchiveWorkItemInstructionParams {
                        instruction_id,
                        operation_id: derive_id(&op, "undo"),
                    },
                ),
                expected_text: text.clone(),
            });
        }
    }
    Ok(result)
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
    fn import_preview_detects_destination_changes() {
        let mut snapshot: WorkOutlineSnapshot = serde_json::from_value(serde_json::json!({
            "spaceId":"space", "available":true, "synced":true
        }))
        .unwrap();
        let pack = WorkflowPack {
            format: mast_proto::workflow_pack::WORKFLOW_PACK_FORMAT.into(),
            mode: WorkflowPackMode::Full,
            scopes: vec![mast_proto::workflow_pack::WorkflowPackScope {
                item_path: vec![],
                instructions: vec!["Rule".into()],
            }],
            templates: vec![],
        };
        let scopes = preview_instruction_import(&pack, &snapshot).unwrap();
        let templates = preview_template_import(&pack, &snapshot).unwrap();
        assert!(revalidate_import_preview(&pack, &snapshot, &scopes, &templates).is_ok());
        snapshot.instructions.push(
            serde_json::from_value(serde_json::json!({"id":"local", "text":"Mine"})).unwrap(),
        );
        assert!(revalidate_import_preview(&pack, &snapshot, &scopes, &templates).is_err());
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

    #[test]
    fn item_undo_refuses_an_added_child() {
        let created = CreateWorkItemParams {
            space_id: "space".into(),
            parent_work_item_id: None,
            placement: WorkOutlinePlacement::Last,
            title: "Tasks".into(),
            description: None,
            status: None,
            operation_id: "import".into(),
        };
        let id = derive_id(&created.operation_id, mast_proto::work_outline::roles::ITEM);
        let mut snapshot: WorkOutlineSnapshot = serde_json::from_value(serde_json::json!({
            "spaceId":"space", "available":true, "synced":true,
            "items":[{"id":id, "spaceId":"space", "title":"Tasks"}]
        }))
        .unwrap();
        assert!(guard_created_item_undo(&snapshot, &created).is_ok());
        snapshot.items.push(
            serde_json::from_value(serde_json::json!({
                "id":"child", "spaceId":"space", "title":"Added", "parentWorkItemId":id
            }))
            .unwrap(),
        );
        assert!(guard_created_item_undo(&snapshot, &created).is_err());
    }

    #[test]
    fn instruction_plan_targets_created_item_and_includes_inverse() {
        let snapshot: WorkOutlineSnapshot = serde_json::from_value(serde_json::json!({
            "spaceId":"space", "available":true, "synced":true
        }))
        .unwrap();
        let scope = WorkflowPackScopePreview {
            item_path: vec!["Tasks".into()],
            work_item_id: None,
            missing_from: Some(0),
            current: vec![],
            incoming: vec!["Rule".into()],
        };
        let items = plan_missing_items(&snapshot, std::slice::from_ref(&scope), "import").unwrap();
        let WorkOutlineMutation::CreateWorkItem(item) = &items[0] else {
            panic!("expected item")
        };
        let additions = plan_instruction_additions(&snapshot, &[scope], "import").unwrap();
        let WorkOutlineMutation::CreateWorkItemInstruction(instruction) = &additions[0].create
        else {
            panic!("expected instruction")
        };
        assert_eq!(
            instruction.scope,
            mast_proto::InstructionScope::WorkItem {
                work_item_id: derive_id(&item.operation_id, mast_proto::work_outline::roles::ITEM)
            }
        );
        let WorkOutlineMutation::ArchiveWorkItemInstruction(undo) = &additions[0].undo else {
            panic!("expected inverse")
        };
        assert_eq!(
            undo.instruction_id,
            derive_id(
                &instruction.operation_id,
                mast_proto::work_outline::roles::INSTRUCTION
            )
        );
        let mut after: WorkOutlineSnapshot = serde_json::from_value(serde_json::json!({
            "spaceId":"space", "available":true, "synced":true,
            "instructions":[{"id":undo.instruction_id,"spaceId":"space",
                "workItemId":derive_id(&item.operation_id, mast_proto::work_outline::roles::ITEM), "text":"Rule"}]
        })).unwrap();
        assert!(guard_instruction_undo(&after, &additions[0]).is_ok());
        after.instructions[0].text = "Edited".into();
        assert!(guard_instruction_undo(&after, &additions[0]).is_err());
    }
}
