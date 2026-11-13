//! Workflow pack tools: export a Space's Work Outline setup, preview an
//! import, apply it, undo it, and list receipts. Admin chats only: the user's
//! per-Space grant is the authorization, and every call acts on that Space.

use mast_proto::agent_tools::tools;
use mast_proto::workflow_pack::WorkflowPackMode;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{AgentToolError, AgentToolErrorCode, AgentToolsService, CallContext, parse_args};
use crate::workflow_pack::WorkflowPackResolutions;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExportArgs {
    #[serde(default = "share_mode")]
    mode: WorkflowPackMode,
}

fn share_mode() -> WorkflowPackMode {
    WorkflowPackMode::Share
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreviewArgs {
    path: std::path::PathBuf,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApplyArgs {
    preview_id: String,
    #[serde(default)]
    resolutions: WorkflowPackResolutions,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UndoArgs {
    receipt_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

fn refused(message: impl Into<String>) -> AgentToolError {
    AgentToolError::refused(AgentToolErrorCode::Forbidden, message)
}

impl AgentToolsService {
    pub(super) fn workflow_pack_tool(
        &self,
        context: &CallContext,
        tool: &str,
        arguments: &Value,
    ) -> Result<Value, AgentToolError> {
        let Some(space) = context.admin_space.clone() else {
            return Err(refused(
                "Workflow packs need an admin chat (the user grants this per Space, with an expiry, from the chat's menu: \"Grant admin for this Space…\")",
            ));
        };
        let service = self.work_outline.workflow_pack();
        let actor = self.work_outline.agent_actor(&context.grant.chat_id, None);
        let failed = |message: String| AgentToolError::invalid(message);
        match tool {
            tools::WORKFLOW_PACK_EXPORT => {
                let args: ExportArgs = parse_args(arguments)?;
                let (pack, redactions) = service.export_preview(&space, args.mode).map_err(failed)?;
                Ok(json!({ "pack": pack, "redactions": redactions }))
            }
            tools::WORKFLOW_PACK_PREVIEW_IMPORT => {
                let args: PreviewArgs = parse_args(arguments)?;
                let preview = service.preview_import(&space, &args.path).map_err(failed)?;
                serde_json::to_value(preview).map_err(|error| failed(error.to_string()))
            }
            tools::WORKFLOW_PACK_APPLY => {
                let args: ApplyArgs = parse_args(arguments)?;
                let receipt = service
                    .apply(&space, &args.preview_id, &args.resolutions, &actor, Some(&space))
                    .map_err(failed)?;
                serde_json::to_value(receipt).map_err(|error| failed(error.to_string()))
            }
            tools::WORKFLOW_PACK_UNDO => {
                let args: UndoArgs = parse_args(arguments)?;
                let receipt = service.undo(&space, &args.receipt_id, &actor, Some(&space)).map_err(failed)?;
                serde_json::to_value(receipt).map_err(|error| failed(error.to_string()))
            }
            tools::WORKFLOW_PACK_RECEIPTS => {
                let _: NoArgs = parse_args(arguments)?;
                let receipts = service.list_receipts(&space).map_err(failed)?;
                Ok(json!({ "receipts": receipts }))
            }
            other => Err(AgentToolError::invalid(format!("unknown workflow pack tool {other}"))),
        }
    }
}
