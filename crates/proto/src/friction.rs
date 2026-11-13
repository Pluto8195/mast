//! Friction reports recorded by agents and automatic capture.

use crate::RequestLink;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrictionKind {
    Bug,
    ToolFailure,
    Permission,
    Environment,
    Workflow,
    Docs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrictionSeverity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrictionStatus {
    Open,
    Dismissed,
    Promoted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrictionSource {
    Agent,
    Auto,
    Import,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrictionApproval {
    pub command: String,
    pub why: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrictionProvenance {
    pub chat_id: String,
    pub harness: String,
    pub model: Option<String>,
    pub space_id: String,
    pub work_item_id: Option<String>,
    pub branch: Option<String>,
    pub created_at: i64,
    pub source: FrictionSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrictionOccurrence {
    pub at: i64,
    pub chat_id: String,
    pub space_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrictionReport {
    pub id: String,
    pub kind: FrictionKind,
    pub summary: String,
    pub what_happened: String,
    pub workaround: Option<String>,
    pub suggestion: Option<String>,
    pub approval: Option<FrictionApproval>,
    pub severity: Option<FrictionSeverity>,
    #[serde(default)]
    pub links: Vec<RequestLink>,
    pub provenance: FrictionProvenance,
    pub fingerprint: String,
    pub occurrences: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub occurrence_history: Vec<FrictionOccurrence>,
    pub last_seen: i64,
    pub status: FrictionStatus,
    #[serde(default)]
    pub promoted_item_id: Option<String>,
    #[serde(default)]
    pub promoted_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListFrictionReportsParams {
    pub space_id: String,
    pub kind: Option<FrictionKind>,
    pub status: Option<FrictionStatus>,
    pub harness: Option<String>,
    pub model: Option<String>,
    pub since: Option<i64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetFrictionStatusParams {
    pub space_id: String,
    pub fingerprint: String,
    pub status: FrictionStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromoteFrictionToItemParams {
    pub space_id: String,
    pub fingerprint: String,
    pub parent_item_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposeFrictionInstructionParams {
    pub space_id: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrictionSettings {
    pub auto_capture: bool,
}

impl Default for FrictionSettings {
    fn default() -> Self {
        Self { auto_capture: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportFrictionParams {
    pub space_id: String,
    pub file_name: String,
    pub markdown: String,
    #[serde(default)]
    pub preview: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrictionImportEntry {
    pub kind: FrictionKind,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrictionImportSkip {
    pub line: usize,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportFrictionResult {
    pub added: usize,
    pub merged: usize,
    pub skipped: Vec<FrictionImportSkip>,
    pub entries: Vec<FrictionImportEntry>,
}
