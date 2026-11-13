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
use serde::{Deserialize, Serialize};

/// Maximum serialized size accepted from an untrusted pack file.
pub const MAX_WORKFLOW_PACK_BYTES: u64 = 4 * 1024 * 1024;

/// One forward operation and the operation that reverses it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowPackMutationPair {
    pub forward: WorkOutlineMutation,
    pub inverse: WorkOutlineMutation,
    /// Text an archived instruction must still have when its archive is undone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_text: Option<String>,
}

impl WorkflowPackMutationPair {
    pub fn new(forward: WorkOutlineMutation, inverse: WorkOutlineMutation) -> Self {
        Self { forward, inverse, expected_text: None }
    }
}

/// How an existing destination scope or template is combined with the pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictResolution {
    /// Keep what the destination has.
    Skip,
    /// Keep the destination's instructions and add the ones it lacks.
    Append,
    /// Archive the destination's instructions or template body and use the pack's.
    Replace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopeResolution {
    pub item_path: Vec<String>,
    pub resolution: ConflictResolution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemplateResolution {
    pub name: String,
    pub resolution: ConflictResolution,
}

/// The user's choice for every conflict in a preview.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowPackResolutions {
    #[serde(default)]
    pub scopes: Vec<ScopeResolution>,
    #[serde(default)]
    pub templates: Vec<TemplateResolution>,
}

fn normalize_path(path: &[String]) -> Vec<String> {
    path.iter().map(|part| part.trim().to_lowercase()).collect()
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyParams {
    pub space_id: String,
    pub preview_id: String,
    #[serde(default)]
    pub resolutions: WorkflowPackResolutions,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UndoParams {
    pub space_id: String,
    pub receipt_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceiptsParams {
    pub space_id: String,
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
        result.extend(additions_for_scope(snapshot, scope, &scope.incoming, operation_id)?);
    }
    Ok(result)
}

fn additions_for_scope(
    snapshot: &WorkOutlineSnapshot,
    scope: &WorkflowPackScopePreview,
    texts: &[String],
    operation_id: &str,
) -> Result<Vec<PlannedInstructionAddition>, String> {
    if scope
        .missing_from
        .is_some_and(|index| index >= scope.item_path.len())
    {
        return Err("The instruction preview has an invalid missing path".into());
    }
    let item_id = if scope.missing_from.is_some() {
        let path = serde_json::to_string(&normalize_path(&scope.item_path))
            .map_err(|error| error.to_string())?;
        let item_operation = derive_id(operation_id, &path);
        Some(derive_id(&item_operation, mast_proto::work_outline::roles::ITEM))
    } else {
        scope.work_item_id.clone()
    };
    let target = match item_id {
        Some(work_item_id) => mast_proto::InstructionScope::WorkItem { work_item_id },
        None => mast_proto::InstructionScope::SpaceRoot,
    };
    let path = serde_json::to_string(&scope.item_path).map_err(|error| error.to_string())?;
    let mut result = Vec::new();
    for (index, text) in texts.iter().enumerate() {
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
    Ok(result)
}

fn find_scope_resolution(
    resolutions: &WorkflowPackResolutions,
    path: &[String],
) -> Option<ConflictResolution> {
    let wanted = normalize_path(path);
    resolutions
        .scopes
        .iter()
        .find(|entry| normalize_path(&entry.item_path) == wanted)
        .map(|entry| entry.resolution)
}

/// Plan the whole import as reversible operation pairs, in apply order:
/// missing items, archives for replaced instructions, instruction additions,
/// then templates. Every conflict needs an entry in `resolutions`; unchanged
/// scopes and templates need nothing.
pub fn plan_pack_operations(
    snapshot: &WorkOutlineSnapshot,
    scopes: &[WorkflowPackScopePreview],
    templates: &[WorkflowPackTemplatePreview],
    resolutions: &WorkflowPackResolutions,
    operation_id: &str,
) -> Result<Vec<WorkflowPackMutationPair>, String> {
    use mast_proto::work_outline::roles;
    let mut actionable = Vec::new();
    let mut archives = Vec::new();
    let mut additions = Vec::new();
    for scope in scopes {
        if scope.incoming.is_empty() || scope.current == scope.incoming {
            continue;
        }
        let path = serde_json::to_string(&scope.item_path).map_err(|error| error.to_string())?;
        let texts: Vec<String> = if scope.current.is_empty() {
            scope.incoming.clone()
        } else {
            match find_scope_resolution(resolutions, &scope.item_path) {
                None => {
                    return Err(format!(
                        "Choose how to resolve the instructions at {}",
                        display_path(&scope.item_path)
                    ));
                }
                Some(ConflictResolution::Skip) => continue,
                Some(ConflictResolution::Append) => scope
                    .incoming
                    .iter()
                    .filter(|text| !scope.current.contains(text))
                    .cloned()
                    .collect(),
                Some(ConflictResolution::Replace) => {
                    let live: Vec<_> = snapshot
                        .instructions
                        .iter()
                        .filter(|row| {
                            !row.archived() && row.work_item_id.as_deref() == scope.work_item_id.as_deref()
                        })
                        .collect();
                    if live.iter().map(|row| &row.text).ne(scope.current.iter()) {
                        return Err("The destination changed since the import preview".into());
                    }
                    for (index, row) in live.iter().enumerate() {
                        let op = derive_id(operation_id, &format!("archive:{path}:{index}"));
                        archives.push(WorkflowPackMutationPair {
                            forward: WorkOutlineMutation::ArchiveWorkItemInstruction(
                                mast_proto::ArchiveWorkItemInstructionParams {
                                    instruction_id: row.id.clone(),
                                    operation_id: op.clone(),
                                },
                            ),
                            inverse: WorkOutlineMutation::RestoreWorkItemInstruction(
                                mast_proto::RestoreWorkItemInstructionParams {
                                    instruction_id: row.id.clone(),
                                    operation_id: derive_id(&op, "undo"),
                                },
                            ),
                            expected_text: Some(row.text.clone()),
                        });
                    }
                    scope.incoming.clone()
                }
            }
        };
        additions.extend(additions_for_scope(snapshot, scope, &texts, operation_id)?);
        if scope.missing_from.is_some() {
            actionable.push(scope.clone());
        }
    }
    let mut result: Vec<WorkflowPackMutationPair> = plan_missing_items(snapshot, &actionable, operation_id)?
        .into_iter()
        .map(|forward| {
            let WorkOutlineMutation::CreateWorkItem(created) = &forward else {
                unreachable!("plan_missing_items only creates items")
            };
            let inverse = WorkOutlineMutation::ArchiveWorkItem(mast_proto::ArchiveWorkItemParams {
                work_item_id: derive_id(&created.operation_id, roles::ITEM),
                descendant_archive_policy: mast_proto::DescendantArchivePolicy::ArchiveDescendants,
                operation_id: derive_id(&created.operation_id, "undo"),
            });
            WorkflowPackMutationPair::new(forward, inverse)
        })
        .collect();
    result.extend(archives);
    result.extend(additions.into_iter().map(|addition| WorkflowPackMutationPair {
        forward: addition.create,
        inverse: addition.undo,
        expected_text: None,
    }));
    for template in templates {
        let normalized = template.name.trim().to_lowercase();
        let op = derive_id(operation_id, &format!("template:{normalized}"));
        let existing = snapshot
            .templates
            .iter()
            .find(|row| row.name.trim().to_lowercase() == normalized);
        let imported = |template_id: Option<String>, name: &str, body: &mast_proto::work_outline_templates::TemplateNode, operation_id: String| {
            WorkOutlineMutation::ImportWorkItemTemplate(
                mast_proto::work_outline_templates::ImportWorkItemTemplateParams {
                    space_id: snapshot.space_id.clone(),
                    name: name.to_owned(),
                    body: body.clone(),
                    template_id,
                    operation_id,
                },
            )
        };
        match existing {
            None => {
                let id = derive_id(&op, roles::TEMPLATE);
                result.push(WorkflowPackMutationPair::new(
                    imported(None, &template.name, &template.incoming, op.clone()),
                    WorkOutlineMutation::DeleteWorkItemTemplate(
                        mast_proto::work_outline_templates::DeleteWorkItemTemplateParams {
                            template_id: id,
                            operation_id: derive_id(&op, "undo"),
                        },
                    ),
                ));
            }
            Some(row) => {
                let choice = resolutions
                    .templates
                    .iter()
                    .find(|entry| entry.name.trim().to_lowercase() == normalized)
                    .map(|entry| entry.resolution)
                    .ok_or_else(|| format!("Choose how to resolve the template {}", template.name))?;
                match choice {
                    ConflictResolution::Skip => {}
                    ConflictResolution::Append => {
                        return Err("A template can only be skipped or replaced".into());
                    }
                    ConflictResolution::Replace => {
                        if template.current.as_ref() != Some(&row.body) {
                            return Err("The destination changed since the import preview".into());
                        }
                        result.push(WorkflowPackMutationPair::new(
                            imported(Some(row.id.clone()), &template.name, &template.incoming, op.clone()),
                            imported(Some(row.id.clone()), &row.name, &row.body, derive_id(&op, "undo")),
                        ));
                    }
                }
            }
        }
    }
    Ok(result)
}

fn display_path(path: &[String]) -> String {
    if path.is_empty() { "the Space root".into() } else { path.join(" / ") }
}

/// Check one receipt entry still matches the destination before its inverse
/// runs. The caller serializes this with the registry write.
pub fn guard_pair_undo(
    snapshot: &WorkOutlineSnapshot,
    pair: &WorkflowPackMutationPair,
) -> Result<(), String> {
    use mast_proto::work_outline::roles;
    match &pair.forward {
        WorkOutlineMutation::CreateWorkItem(created) => {
            if guard_created_item_undo(snapshot, created)? != pair.inverse {
                return Err("Item inverse does not match receipt".into());
            }
        }
        WorkOutlineMutation::CreateWorkItemInstruction(created) => {
            guard_instruction_undo(
                snapshot,
                &PlannedInstructionAddition {
                    create: pair.forward.clone(),
                    undo: pair.inverse.clone(),
                    expected_text: created.text.clone(),
                },
            )?;
        }
        WorkOutlineMutation::ArchiveWorkItemInstruction(archived) => {
            let WorkOutlineMutation::RestoreWorkItemInstruction(restore) = &pair.inverse else {
                return Err("Invalid instruction inverse".into());
            };
            if restore.instruction_id != archived.instruction_id {
                return Err("Instruction receipt does not match its inverse".into());
            }
            let row = snapshot
                .instructions
                .iter()
                .find(|row| row.id == archived.instruction_id)
                .ok_or("Archived instruction no longer exists")?;
            if !row.archived() || Some(&row.text) != pair.expected_text.as_ref() {
                return Err("Archived instruction changed since import".into());
            }
        }
        WorkOutlineMutation::ImportWorkItemTemplate(imported) => {
            let id = imported
                .template_id
                .clone()
                .unwrap_or_else(|| derive_id(&imported.operation_id, roles::TEMPLATE));
            let row = snapshot
                .templates
                .iter()
                .find(|row| row.id == id)
                .ok_or("Imported template no longer exists")?;
            if row.space_id != imported.space_id
                || row.name.trim() != imported.name.trim()
                || row.body != imported.body
            {
                return Err("Imported template changed since import".into());
            }
            match (&imported.template_id, &pair.inverse) {
                (None, WorkOutlineMutation::DeleteWorkItemTemplate(delete)) if delete.template_id == id => {
                    if snapshot.items.iter().any(|item| item.template_id.as_deref() == Some(&id)) {
                        return Err("Imported template is now used by a project".into());
                    }
                }
                (Some(_), WorkOutlineMutation::ImportWorkItemTemplate(back))
                    if back.template_id.as_deref() == Some(&id) && back.space_id == imported.space_id => {}
                _ => return Err("Template inverse does not match receipt".into()),
            }
        }
        _ => return Err("This workflow pack operation cannot be undone".into()),
    }
    Ok(())
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

    #[test]
    fn append_adds_only_missing_instructions_and_skip_adds_nothing() {
        let snapshot: WorkOutlineSnapshot = serde_json::from_value(serde_json::json!({
            "spaceId":"space", "available":true, "synced":true,
            "instructions":[{"id":"a","spaceId":"space","text":"Keep"}]
        })).unwrap();
        let scope = WorkflowPackScopePreview {
            item_path: vec![], work_item_id: None, missing_from: None,
            current: vec!["Keep".into()], incoming: vec!["Keep".into(), "New".into()],
        };
        let choice = |resolution| WorkflowPackResolutions {
            scopes: vec![ScopeResolution { item_path: vec![], resolution }], templates: vec![],
        };
        let appended = plan_pack_operations(&snapshot, std::slice::from_ref(&scope), &[], &choice(ConflictResolution::Append), "op").unwrap();
        assert_eq!(appended.len(), 1);
        let skipped = plan_pack_operations(&snapshot, &[scope], &[], &choice(ConflictResolution::Skip), "op").unwrap();
        assert!(skipped.is_empty());
    }
}
