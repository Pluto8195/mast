//! Portable workflow pack types. These types contain no registry identifiers.
//!
//! This module is a format foundation only; it does not read or write packs.

use serde::{Deserialize, Serialize};

use crate::work_outline_templates::TemplateNode;

pub const WORKFLOW_PACK_FORMAT: &str = "mast.workflow-pack/v1";

/// An explicitly selected export mode. Both modes exclude credential stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowPackMode {
    Full,
    Share,
}

/// A portable instruction scope, expressed as item titles from the Space root.
/// An empty path denotes the Space root itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowPackScope {
    pub item_path: Vec<String>,
    pub instructions: Vec<String>,
}

/// A frozen project template, without source registry ids or revision history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowPackTemplate {
    pub name: String,
    pub body: TemplateNode,
}

/// The initial format covers instruction scopes and frozen project templates.
/// Content must be selected explicitly; serializing an outline snapshot or UI
/// settings object directly would also serialize device and chat identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowPack {
    pub format: String,
    pub mode: WorkflowPackMode,
    pub scopes: Vec<WorkflowPackScope>,
    pub templates: Vec<WorkflowPackTemplate>,
}

impl WorkflowPack {
    /// Reject unsupported versions and ambiguous scope or template names.
    pub fn validate(&self) -> Result<(), String> {
        if self.format != WORKFLOW_PACK_FORMAT {
            return Err("Unsupported workflow pack format".into());
        }
        let mut scopes = std::collections::HashSet::new();
        for scope in &self.scopes {
            if scope.item_path.iter().any(|part| part.trim().is_empty()) {
                return Err("An item path contains an empty title".into());
            }
            let key: Vec<String> = scope
                .item_path
                .iter()
                .map(|part| part.trim().to_lowercase())
                .collect();
            if !scopes.insert(key) {
                return Err("A workflow pack contains duplicate instruction scopes".into());
            }
        }
        let mut names = std::collections::HashSet::new();
        for template in &self.templates {
            let name = template.name.trim();
            if name.is_empty() || !names.insert(name.to_lowercase()) {
                return Err("A workflow pack contains an empty or duplicate template name".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_round_trip_and_version_check() {
        let mut pack = WorkflowPack {
            format: WORKFLOW_PACK_FORMAT.into(),
            mode: WorkflowPackMode::Full,
            scopes: vec![WorkflowPackScope {
                item_path: vec![],
                instructions: vec!["Use tests".into()],
            }],
            templates: vec![],
        };
        let json = serde_json::to_string(&pack).unwrap();
        assert_eq!(serde_json::from_str::<WorkflowPack>(&json).unwrap(), pack);
        assert!(pack.validate().is_ok());
        pack.format = "mast.workflow-pack/v2".into();
        assert!(pack.validate().is_err());
    }

    #[test]
    fn duplicate_paths_are_rejected_case_insensitively() {
        let pack = WorkflowPack {
            format: WORKFLOW_PACK_FORMAT.into(),
            mode: WorkflowPackMode::Share,
            scopes: vec![
                WorkflowPackScope {
                    item_path: vec!["Tasks".into()],
                    instructions: vec![],
                },
                WorkflowPackScope {
                    item_path: vec![" tasks ".into()],
                    instructions: vec![],
                },
            ],
            templates: vec![],
        };
        assert!(pack.validate().is_err());
    }
}
