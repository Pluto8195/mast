//! mast-proto — wire types shared by engine, UI, and RPC.
//!
//! Ported from zeron's `packages/control/src/wire.ts` + `packages/harness/src/types.ts`.
//! Context occupancy is replicated per chat; billing `Usage` remains a harness passthrough.

pub mod admin;
pub mod agent;
pub mod artifacts;
pub mod chat_diff_target;
pub mod diff_review;
pub mod plan;
pub mod plan_render;
pub mod decision_sheet;
pub mod decision_sheet_render;
pub mod entities;
pub mod env;
pub mod friction;
pub mod linear;
pub mod motion;
pub mod orchestrator_wakes;
pub mod pr_dash;
pub mod pr_review;
pub mod pr_review_sync;
pub mod preview;
pub mod review_sidecar;
pub mod sidebar_pins;
pub mod view;
pub mod visualize;
pub mod work_outline;
pub mod work_outline_container_templates;
pub mod work_outline_execution;
pub mod work_outline_instructions;
pub mod work_outline_launch;
pub mod work_outline_order;
pub mod work_outline_templates;
pub mod work_outline_user_actions;
pub mod workflow_pack;
pub mod message_source;
pub mod workspace;

pub use admin::*;
pub use agent::*;
pub use artifacts::{
    ArtifactAddCommentParams, ArtifactAddCommentReply, ArtifactAnswerQuestionParams,
    ArtifactAnswerQuestionReply, ArtifactComment, ArtifactContentEncoding, ArtifactContentParams,
    ArtifactContentReply, ArtifactDriftParams, ArtifactDriftReply, ArtifactError,
    ArtifactErrorCode, ArtifactKind, ArtifactLayoutWarning, ArtifactOptionPick,
    ArtifactPickOptionParams, ArtifactPickOptionReply, ArtifactPinParams, ArtifactPinReply,
    ArtifactPromoteParams, ArtifactPromoteReply, ArtifactPublishReply, ArtifactQuestionAnswer,
    ArtifactRegisterParams, ArtifactRenderCheckResult, ArtifactRenderCompanion,
    ArtifactRenderRequest, ArtifactRenderResultParams, ArtifactRenderResultReply,
    ArtifactRenderScreenshot, ArtifactRenderTheme, ArtifactRenderViewport,
    ArtifactResolveCommentParams, ArtifactResolveCommentReply, ArtifactRevision, ArtifactView,
    ArtifactViewport, ArtifactsSnapshot, CommentAnchor, CommentStatus, PlanQuestion,
    PlanQuestionStatus, WatchArtifactRenderRequestsParams, WatchArtifactsParams, extract_questions,
    pick_message,
};
pub use chat_diff_target::*;
pub use diff_review::*;
pub use entities::*;
pub use friction::*;
pub use linear::*;
pub use orchestrator_wakes::*;
pub use pr_dash::*;
pub use pr_review::*;
pub use pr_review_sync::*;
pub use preview::*;
pub use review_sidecar::*;
pub use sidebar_pins::*;
pub use work_outline::*;
pub use work_outline_container_templates::*;
pub use work_outline_execution::*;
pub use work_outline_instructions::*;
pub use work_outline_launch::*;
pub use work_outline_order::*;
pub use work_outline_templates::*;
pub use work_outline_user_actions::*;
pub use workspace::*;

/// Parse "0.2.12" (tolerating a `-suffix`/`+build` tail on the last part)
/// into a comparable triple — the fleet feature-gate primitive (device rows
/// stamp `Device::version` at boot). `None` for anything that doesn't lead
/// with three dotted integers, and gates treat `None` as "too old".
pub fn version_triple(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.trim().splitn(3, '.');
    let major: u64 = parts.next()?.parse().ok()?;
    let minor: u64 = parts.next()?.parse().ok()?;
    let patch = parts.next()?;
    let patch: u64 = patch
        .split(['-', '+'])
        .next()
        .unwrap_or(patch)
        .parse()
        .ok()?;
    Some((major, minor, patch))
}

#[cfg(test)]
#[path = "../../../test_support/isolation.rs"]
mod test_isolation;
