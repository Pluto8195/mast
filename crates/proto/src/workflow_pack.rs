//! Portable workflow pack types. These types contain no registry identifiers.
//!
//! This module is a format foundation only; it does not read or write packs.

use serde::{Deserialize, Serialize};

use crate::work_outline::WorkOutlineSnapshot;
use crate::work_outline_templates::{TemplateNode, template_limits};

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

/// Collect candidate content for review. This does not perform secret removal
/// or authorize writing the result to disk.
pub fn collect_workflow_pack_candidate(
    snapshot: &WorkOutlineSnapshot,
) -> Result<WorkflowPack, String> {
    if !snapshot.available || !snapshot.synced {
        return Err("The Space is not available and synchronized".into());
    }
    let mut scopes: Vec<WorkflowPackScope> = Vec::new();
    let mut scope_indices = std::collections::HashMap::new();
    for instruction in snapshot
        .instructions
        .iter()
        .filter(|instruction| !instruction.archived())
    {
        let mut path = Vec::new();
        let mut cursor = instruction.work_item_id.as_deref();
        let mut visited = std::collections::HashSet::new();
        let mut excluded = false;
        while let Some(id) = cursor {
            if !visited.insert(id) {
                return Err("An instruction scope has a cyclic item path".into());
            }
            let item = snapshot
                .item(id)
                .ok_or("An instruction scope has a missing item")?;
            if item.archived_at.is_some() {
                excluded = true;
                break;
            }
            path.push(item.title.clone());
            cursor = item.parent_work_item_id.as_deref();
        }
        if excluded {
            continue;
        }
        path.reverse();
        let index = *scope_indices
            .entry(instruction.work_item_id.clone())
            .or_insert_with(|| {
                let index = scopes.len();
                scopes.push(WorkflowPackScope {
                    item_path: path,
                    instructions: Vec::new(),
                });
                index
            });
        scopes[index].instructions.push(instruction.text.clone());
    }
    let templates = snapshot
        .templates
        .iter()
        .map(|template| WorkflowPackTemplate {
            name: template.name.clone(),
            body: template.body.clone(),
        })
        .collect();
    let pack = WorkflowPack {
        format: WORKFLOW_PACK_FORMAT.into(),
        mode: WorkflowPackMode::Full,
        scopes,
        templates,
    };
    pack.validate()?;
    Ok(pack)
}

/// A comparison of one incoming instruction scope with the destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowPackScopeChange {
    pub item_path: Vec<String>,
    pub current: Option<Vec<String>>,
    pub incoming: Vec<String>,
}

/// Compare portable scopes without making any registry writes. Both inputs must
/// have unique paths; otherwise a name match would silently select a target.
pub fn compare_instruction_scopes(
    incoming: &WorkflowPack,
    current: &WorkflowPack,
) -> Result<Vec<WorkflowPackScopeChange>, String> {
    incoming.validate()?;
    current.validate()?;
    fn normalized(path: &[String]) -> Vec<String> {
        path.iter().map(|part| part.trim().to_lowercase()).collect()
    }
    let by_path: std::collections::HashMap<_, _> = current
        .scopes
        .iter()
        .map(|scope| (normalized(&scope.item_path), scope))
        .collect();
    Ok(incoming
        .scopes
        .iter()
        .filter_map(|scope| {
            let existing = by_path.get(&normalized(&scope.item_path));
            if existing.is_some_and(|existing| existing.instructions == scope.instructions) {
                return None;
            }
            Some(WorkflowPackScopeChange {
                item_path: scope.item_path.clone(),
                current: existing.map(|existing| existing.instructions.clone()),
                incoming: scope.instructions.clone(),
            })
        })
        .collect())
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
            if name.chars().count() > template_limits::MAX_TEMPLATE_NAME_CHARS {
                return Err("A workflow pack template name is too long".into());
            }
            template.body.check_limits()?;
            let body = serde_json::to_string(&template.body)
                .map_err(|_| "A workflow pack template cannot be encoded")?;
            if body.encode_utf16().count() > template_limits::MAX_TEMPLATE_BODY_UTF16_UNITS {
                return Err("A workflow pack template body is too large".into());
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

    #[test]
    fn oversized_template_body_is_rejected() {
        let pack = WorkflowPack {
            format: WORKFLOW_PACK_FORMAT.into(),
            mode: WorkflowPackMode::Full,
            scopes: vec![],
            templates: vec![WorkflowPackTemplate {
                name: "Template".into(),
                body: TemplateNode {
                    title: "x".repeat(template_limits::MAX_TEMPLATE_BODY_UTF16_UNITS),
                    ..Default::default()
                },
            }],
        };
        assert!(pack.validate().is_err());
    }

    #[test]
    fn comparison_identifies_additions_and_conflicts_by_path() {
        let current = WorkflowPack {
            format: WORKFLOW_PACK_FORMAT.into(),
            mode: WorkflowPackMode::Full,
            scopes: vec![WorkflowPackScope {
                item_path: vec!["Tasks".into()],
                instructions: vec!["Mine".into()],
            }],
            templates: vec![],
        };
        let incoming = WorkflowPack {
            scopes: vec![
                WorkflowPackScope {
                    item_path: vec!["tasks".into()],
                    instructions: vec!["Theirs".into()],
                },
                WorkflowPackScope {
                    item_path: vec![],
                    instructions: vec!["Root".into()],
                },
            ],
            ..current.clone()
        };
        let changes = compare_instruction_scopes(&incoming, &current).unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].current, Some(vec!["Mine".into()]));
        assert_eq!(changes[1].current, None);
    }

    #[test]
    fn candidate_uses_paths_and_excludes_archived_scopes() {
        let snapshot: WorkOutlineSnapshot = serde_json::from_value(serde_json::json!({
            "spaceId": "space", "available": true, "synced": true,
            "items": [
                {"id":"parent", "spaceId":"space", "title":"Parent"},
                {"id":"child", "spaceId":"space", "title":"Child", "parentWorkItemId":"parent"},
                {"id":"archived", "spaceId":"space", "title":"Archived", "archivedAt":1}
            ],
            "instructions": [
                {"id":"one", "text":"first"},
                {"id":"two", "workItemId":"child", "text":"second"},
                {"id":"three", "workItemId":"archived", "text":"hidden"}
            ]
        }))
        .unwrap();
        let pack = collect_workflow_pack_candidate(&snapshot).unwrap();
        assert_eq!(pack.scopes.len(), 2);
        assert_eq!(pack.scopes[0].item_path, Vec::<String>::new());
        assert_eq!(pack.scopes[1].item_path, vec!["Parent", "Child"]);
        let json = serde_json::to_string(&pack).unwrap();
        assert!(!json.contains("archived"));
        assert!(!json.contains("workItemId"));
    }
}
