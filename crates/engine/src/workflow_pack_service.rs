//! Workflow pack import sessions and receipts.
//!
//! An import is preview, then a confirmed apply, then optionally undo. Previews
//! are held in memory; every apply writes one receipt file (the applied
//! operations and their inverses) under the host-local receipt directory, so an
//! undo works after a restart. Authorization of the actor is the caller's job.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use mast_proto::workflow_pack::{
    WorkflowPack, WorkflowPackMode, WorkflowPackRedaction, WorkflowPackScopePreview,
    WorkflowPackTemplatePreview,
};
use mast_proto::{WorkOutlineActor, WorkOutlineMutation, WorkOutlineSnapshot, derive_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::workflow_pack::{WorkflowPackMutationPair, WorkflowPackResolutions};
use crate::workspace_host::WorkspaceHost;

/// Previews kept in memory; the oldest is dropped first.
const MAX_PREVIEWS: usize = 16;
/// Receipts returned by a listing.
const MAX_LISTED_RECEIPTS: usize = 50;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowPackReceiptSummary {
    pub items_created: usize,
    pub instructions_added: usize,
    pub instructions_archived: usize,
    pub templates_added: usize,
    pub templates_replaced: usize,
}

impl WorkflowPackReceiptSummary {
    fn of(operations: &[WorkflowPackMutationPair]) -> Self {
        let mut summary = Self::default();
        for pair in operations {
            match &pair.forward {
                WorkOutlineMutation::CreateWorkItem(_) => summary.items_created += 1,
                WorkOutlineMutation::CreateWorkItemInstruction(_) => summary.instructions_added += 1,
                WorkOutlineMutation::ArchiveWorkItemInstruction(_) => summary.instructions_archived += 1,
                WorkOutlineMutation::ImportWorkItemTemplate(p) if p.template_id.is_some() => {
                    summary.templates_replaced += 1
                }
                WorkOutlineMutation::ImportWorkItemTemplate(_) => summary.templates_added += 1,
                _ => {}
            }
        }
        summary
    }
}

/// The stored record of one apply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowPackReceipt {
    pub id: String,
    pub space_id: String,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undone_at: Option<i64>,
    pub summary: WorkflowPackReceiptSummary,
    pub operations: Vec<WorkflowPackMutationPair>,
}

/// A receipt as listed: everything except the stored operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowPackReceiptView {
    pub id: String,
    pub space_id: String,
    pub created_at: i64,
    pub undone_at: Option<i64>,
    pub summary: WorkflowPackReceiptSummary,
}

impl From<&WorkflowPackReceipt> for WorkflowPackReceiptView {
    fn from(receipt: &WorkflowPackReceipt) -> Self {
        Self {
            id: receipt.id.clone(),
            space_id: receipt.space_id.clone(),
            created_at: receipt.created_at,
            undone_at: receipt.undone_at,
            summary: receipt.summary.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowPackImportPreview {
    pub preview_id: String,
    pub pack: WorkflowPack,
    pub scopes: Vec<WorkflowPackScopePreview>,
    pub templates: Vec<WorkflowPackTemplatePreview>,
}

struct StoredPreview {
    id: String,
    space_id: String,
    pack: WorkflowPack,
    scopes: Vec<WorkflowPackScopePreview>,
    templates: Vec<WorkflowPackTemplatePreview>,
}

pub struct WorkflowPackService {
    workspace: WorkspaceHost,
    receipt_dir: Mutex<Option<PathBuf>>,
    previews: Mutex<VecDeque<StoredPreview>>,
    /// Serializes apply and undo so a receipt is never undone twice at once.
    gate: Mutex<()>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

fn valid_receipt_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

impl WorkflowPackService {
    pub fn new(workspace: WorkspaceHost) -> Self {
        Self {
            workspace,
            receipt_dir: Mutex::new(None),
            previews: Mutex::new(VecDeque::new()),
            gate: Mutex::new(()),
        }
    }

    /// Where receipts are kept. Apply is refused until this is set.
    pub fn set_receipt_dir(&self, dir: PathBuf) {
        *lock(&self.receipt_dir) = Some(dir);
    }

    fn receipt_dir(&self) -> Result<PathBuf, String> {
        lock(&self.receipt_dir)
            .clone()
            .ok_or_else(|| "Workflow pack receipts are not available on this device".to_string())
    }

    fn snapshot(&self, space_id: &str) -> WorkOutlineSnapshot {
        mast_doc::materialize_work_outline(space_id, &self.workspace.work_outline_inputs(space_id))
    }

    pub fn export_preview(
        &self,
        space_id: &str,
        mode: WorkflowPackMode,
    ) -> Result<(WorkflowPack, Vec<WorkflowPackRedaction>), String> {
        crate::workflow_pack::preview_export(&self.snapshot(space_id), mode)
    }

    /// Read a pack file and remember what it would change, under a preview id.
    pub fn preview_import(&self, space_id: &str, path: &Path) -> Result<WorkflowPackImportPreview, String> {
        let (pack, scopes, templates) =
            crate::workflow_pack::preview_import(path, &self.snapshot(space_id))?;
        let digest = Sha256::digest(
            serde_json::to_vec(&(space_id, &pack, &scopes, &templates)).map_err(|e| e.to_string())?,
        );
        let id: String = digest.iter().take(8).map(|byte| format!("{byte:02x}")).collect();
        let mut previews = lock(&self.previews);
        previews.retain(|preview| preview.id != id);
        previews.push_back(StoredPreview {
            id: id.clone(),
            space_id: space_id.to_owned(),
            pack: pack.clone(),
            scopes: scopes.clone(),
            templates: templates.clone(),
        });
        while previews.len() > MAX_PREVIEWS {
            previews.pop_front();
        }
        Ok(WorkflowPackImportPreview { preview_id: id, pack, scopes, templates })
    }

    /// Apply a previewed pack with the caller's choice for every conflict.
    /// `admin_space` is the Space an admin agent holds a grant for, else `None`.
    pub fn apply(
        &self,
        space_id: &str,
        preview_id: &str,
        resolutions: &WorkflowPackResolutions,
        actor: &WorkOutlineActor,
        admin_space: Option<&str>,
    ) -> Result<WorkflowPackReceiptView, String> {
        let _gate = lock(&self.gate);
        let dir = self.receipt_dir()?;
        let (pack, scopes, templates) = {
            let previews = lock(&self.previews);
            let preview = previews
                .iter()
                .find(|preview| preview.id == preview_id && preview.space_id == space_id)
                .ok_or("Unknown or expired import preview; preview the import again")?;
            (preview.pack.clone(), preview.scopes.clone(), preview.templates.clone())
        };
        let receipt_id = uuid::Uuid::new_v4().simple().to_string();
        let operation_id = derive_id(&receipt_id, "import");
        let pairs = crate::workflow_pack::plan_pack_operations(
            &self.snapshot(space_id),
            &scopes,
            &templates,
            resolutions,
            &operation_id,
        )?;
        if pairs.is_empty() {
            return Err("Nothing to import".into());
        }
        let operations = self
            .workspace
            .apply_workflow_pack_batch(space_id, &pack, &scopes, &templates, &pairs, actor, admin_space)?;
        let receipt = WorkflowPackReceipt {
            id: receipt_id,
            space_id: space_id.to_owned(),
            created_at: chrono::Utc::now().timestamp_millis(),
            undone_at: None,
            summary: WorkflowPackReceiptSummary::of(&operations),
            operations,
        };
        if let Err(error) = save_receipt(&dir, &receipt) {
            // Without a receipt the import could never be undone: take it back.
            let undone = self.workspace.undo_workflow_pack_batch(space_id, &receipt.operations, actor, admin_space);
            return Err(match undone {
                Ok(()) => format!("The import was rolled back because its receipt could not be saved: {error}"),
                Err(undo) => format!("The import was applied but its receipt could not be saved ({error}) and rolling back failed: {undo}"),
            });
        }
        lock(&self.previews).retain(|preview| preview.id != preview_id);
        Ok(WorkflowPackReceiptView::from(&receipt))
    }

    /// Undo an apply from its stored receipt, refusing if the Space changed.
    pub fn undo(
        &self,
        space_id: &str,
        receipt_id: &str,
        actor: &WorkOutlineActor,
        admin_space: Option<&str>,
    ) -> Result<WorkflowPackReceiptView, String> {
        let _gate = lock(&self.gate);
        let dir = self.receipt_dir()?;
        let mut receipt = load_receipt(&dir, receipt_id)?;
        if receipt.space_id != space_id {
            return Err("That receipt belongs to another Space".into());
        }
        if receipt.undone_at.is_some() {
            return Err("That import was already undone".into());
        }
        self.workspace
            .undo_workflow_pack_batch(space_id, &receipt.operations, actor, admin_space)?;
        receipt.undone_at = Some(chrono::Utc::now().timestamp_millis());
        save_receipt(&dir, &receipt)
            .map_err(|error| format!("The import was undone but its receipt could not be updated: {error}"))?;
        Ok(WorkflowPackReceiptView::from(&receipt))
    }

    /// Receipts of a Space, newest first.
    pub fn list_receipts(&self, space_id: &str) -> Result<Vec<WorkflowPackReceiptView>, String> {
        let dir = self.receipt_dir()?;
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.to_string()),
        };
        let mut receipts: Vec<WorkflowPackReceipt> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|entry| std::fs::read(entry.path()).ok())
            .filter_map(|bytes| serde_json::from_slice::<WorkflowPackReceipt>(&bytes).ok())
            .filter(|receipt| receipt.space_id == space_id)
            .collect();
        receipts.sort_by(|a, b| b.created_at.cmp(&a.created_at).then_with(|| a.id.cmp(&b.id)));
        receipts.truncate(MAX_LISTED_RECEIPTS);
        Ok(receipts.iter().map(WorkflowPackReceiptView::from).collect())
    }
}

fn save_receipt(dir: &Path, receipt: &WorkflowPackReceipt) -> Result<(), String> {
    if !valid_receipt_id(&receipt.id) {
        return Err("Invalid receipt id".into());
    }
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let target = dir.join(format!("{}.json", receipt.id));
    let temp = dir.join(format!("{}.json.tmp", receipt.id));
    let bytes = serde_json::to_vec_pretty(receipt).map_err(|error| error.to_string())?;
    std::fs::write(&temp, bytes).map_err(|error| error.to_string())?;
    std::fs::rename(&temp, &target).map_err(|error| error.to_string())
}

fn load_receipt(dir: &Path, id: &str) -> Result<WorkflowPackReceipt, String> {
    if !valid_receipt_id(id) {
        return Err("Invalid receipt id".into());
    }
    let bytes = std::fs::read(dir.join(format!("{id}.json"))).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "Receipt not found".to_string()
        } else {
            error.to_string()
        }
    })?;
    serde_json::from_slice(&bytes).map_err(|error| format!("Receipt is unreadable: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_pack::{ConflictResolution, ScopeResolution};
    use mast_proto::workflow_pack::{WORKFLOW_PACK_FORMAT, WorkflowPackScope};
    use mast_proto::{UpdateWorkItemInstructionParams, WorkOutlineActorKind};

    fn host(dir: &Path) -> WorkspaceHost {
        use crate::workspace_host::WorkspaceHostConfig;
        let store = std::sync::Arc::new(mast_sync::DocsStore::open(dir).unwrap());
        let host = WorkspaceHost::open(store, WorkspaceHostConfig {
            device_id: "device".into(), device_name: "Test".into(), platform: "macos".into(),
            org_id: "org".into(), user_id: "user".into(), edge: None,
        }).unwrap();
        host.create_space("space", "device", "/tmp/example", None, false).unwrap();
        host
    }

    fn actor() -> WorkOutlineActor {
        WorkOutlineActor { kind: WorkOutlineActorKind::User, device_id: "device".into(), chat_id: None, message_id: None }
    }

    fn write_pack(dir: &Path, rules: &[&str]) -> PathBuf {
        let pack = WorkflowPack {
            format: WORKFLOW_PACK_FORMAT.into(),
            mode: WorkflowPackMode::Full,
            scopes: vec![WorkflowPackScope { item_path: vec!["Tasks".into()], instructions: rules.iter().map(|r| r.to_string()).collect() }],
            templates: vec![],
        };
        let path = dir.join("pack.json");
        std::fs::write(&path, serde_json::to_vec(&pack).unwrap()).unwrap();
        path
    }

    #[tokio::test]
    async fn receipts_survive_a_restart_and_undo_refuses_changes() {
        let dir = tempfile::tempdir().unwrap();
        let receipts = dir.path().join("receipts");
        let host = host(dir.path());
        let pack = write_pack(dir.path(), &["Rule"]);
        let service = WorkflowPackService::new(host.clone());
        let none = WorkflowPackResolutions::default();
        assert!(service.apply("space", "nope", &none, &actor(), None).unwrap_err().contains("receipts"));
        service.set_receipt_dir(receipts.clone());
        let preview = service.preview_import("space", &pack).unwrap();
        let receipt = service.apply("space", &preview.preview_id, &none, &actor(), None).unwrap();
        assert_eq!(receipt.summary.items_created, 1);
        assert_eq!(receipt.summary.instructions_added, 1);
        // A new service over the same directory, as after a restart.
        let restarted = WorkflowPackService::new(host.clone());
        restarted.set_receipt_dir(receipts);
        let listed = restarted.list_receipts("space").unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, receipt.id);
        assert!(restarted.list_receipts("other").unwrap().is_empty());
        let snapshot = || mast_doc::materialize_work_outline("space", &host.work_outline_inputs("space"));
        let id = snapshot().instructions.iter().find(|row| !row.archived()).unwrap().id.clone();
        host.work_outline_mutate(&WorkOutlineMutation::UpdateWorkItemInstruction(UpdateWorkItemInstructionParams {
            instruction_id: id.clone(), text: "Edited".into(), operation_id: "edit".into(),
        }), &actor(), &[]).unwrap();
        assert!(restarted.undo("space", &receipt.id, &actor(), None).is_err());
        assert!(restarted.undo("other", &receipt.id, &actor(), None).is_err());
        assert!(restarted.undo("space", "../escape", &actor(), None).is_err());
        host.work_outline_mutate(&WorkOutlineMutation::UpdateWorkItemInstruction(UpdateWorkItemInstructionParams {
            instruction_id: id, text: "Rule".into(), operation_id: "revert".into(),
        }), &actor(), &[]).unwrap();
        let undone = restarted.undo("space", &receipt.id, &actor(), None).unwrap();
        assert!(undone.undone_at.is_some());
        assert!(snapshot().items.iter().all(|item| item.archived_at.is_some()));
        assert!(restarted.undo("space", &receipt.id, &actor(), None).unwrap_err().contains("already"));
        // The same pack can be imported again after an undo.
        let again = restarted.preview_import("space", &pack).unwrap();
        restarted.apply("space", &again.preview_id, &none, &actor(), None).unwrap();
        assert_eq!(restarted.list_receipts("space").unwrap().len(), 2);
    }

    #[tokio::test]
    async fn conflicts_need_an_explicit_resolution() {
        let dir = tempfile::tempdir().unwrap();
        let host = host(dir.path());
        let service = WorkflowPackService::new(host.clone());
        service.set_receipt_dir(dir.path().join("receipts"));
        let pack = write_pack(dir.path(), &["Theirs"]);
        let first = service.preview_import("space", &pack).unwrap();
        service.apply("space", &first.preview_id, &WorkflowPackResolutions::default(), &actor(), None).unwrap();
        let again = service.preview_import("space", &pack).unwrap();
        assert!(again.scopes[0].current == again.scopes[0].incoming);
        assert!(service.apply("space", &again.preview_id, &WorkflowPackResolutions::default(), &actor(), None).unwrap_err().contains("Nothing"));
        let other = write_pack(dir.path(), &["Different"]);
        let conflict = service.preview_import("space", &other).unwrap();
        let err = service.apply("space", &conflict.preview_id, &WorkflowPackResolutions::default(), &actor(), None).unwrap_err();
        assert!(err.contains("Choose"));
        let choose = WorkflowPackResolutions {
            scopes: vec![ScopeResolution { item_path: vec!["Tasks".into()], resolution: ConflictResolution::Replace }],
            templates: vec![],
        };
        service.apply("space", &conflict.preview_id, &choose, &actor(), None).unwrap();
    }
}
