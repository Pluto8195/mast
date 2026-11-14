//! Validated, data-driven planning sheets shared by the engine and UI.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionSheet {
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub models: Vec<Model>,
    #[serde(default)]
    pub endpoints: Vec<Endpoint>,
    #[serde(default)]
    pub rules: Vec<Rules>,
    #[serde(default)]
    pub validations: Vec<Validation>,
    #[serde(default)]
    pub permissions: Vec<Permission>,
    #[serde(default)]
    pub flows: Vec<Flow>,
    #[serde(default)]
    pub ui_states: Vec<UiState>,
    #[serde(default)]
    pub out_of_scope: Vec<OutOfScope>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub state: String,
    pub date: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    #[serde(default)]
    pub optional: bool,
    pub id: String,
    pub name: String,
    pub status: Status,
    pub recommendation: Option<String>,
    pub question: Option<String>,
    #[serde(default)]
    pub options: Vec<DecisionOption>,
    #[serde(default, rename = "recommendedOption")]
    pub recommended_option: Option<String>,
    pub fields: Vec<Field>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub id: String,
    pub name: String,
    pub marker: String,
    #[serde(rename = "type")]
    pub field_type: String,
    pub null: bool,
    pub default: Option<String>,
    pub constraint: Option<String>,
    pub status: Option<Status>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    #[serde(default)]
    pub optional: bool,
    pub id: String,
    pub name: String,
    pub request: Vec<DiffLine>,
    pub response: Vec<DiffLine>,
    #[serde(default)]
    pub callers: Vec<Caller>,
    pub status: Status,
    pub recommendation: Option<String>,
    pub question: Option<String>,
    #[serde(default)]
    pub options: Vec<DecisionOption>,
    #[serde(default, rename = "recommendedOption")]
    pub recommended_option: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiffLine {
    pub id: String,
    pub marker: String,
    pub text: String,
    pub status: Option<Status>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Caller {
    pub id: String,
    pub name: String,
    pub effect: String,
    pub status: Option<Status>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    pub id: String,
    pub columns: Vec<String>,
    pub rows: Vec<RuleRow>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleRow {
    #[serde(default)]
    pub optional: bool,
    pub id: String,
    pub state: String,
    #[serde(default)]
    pub action: Option<String>,
    pub cells: Vec<String>,
    pub status: Status,
    pub recommendation: Option<String>,
    pub question: Option<String>,
    #[serde(default)]
    pub options: Vec<DecisionOption>,
    #[serde(default, rename = "recommendedOption")]
    pub recommended_option: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Validation {
    #[serde(default)]
    pub optional: bool,
    pub id: String,
    pub rule: String,
    pub enforced: Vec<String>,
    pub result: String,
    pub status: Status,
    pub recommendation: Option<String>,
    pub question: Option<String>,
    #[serde(default)]
    pub options: Vec<DecisionOption>,
    #[serde(default, rename = "recommendedOption")]
    pub recommended_option: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permission {
    #[serde(default)]
    pub optional: bool,
    pub id: String,
    pub subject: String,
    pub access: BTreeMap<String, String>,
    pub status: Status,
    pub recommendation: Option<String>,
    pub question: Option<String>,
    #[serde(default)]
    pub options: Vec<DecisionOption>,
    #[serde(default, rename = "recommendedOption")]
    pub recommended_option: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Flow {
    #[serde(default)]
    pub optional: bool,
    pub id: String,
    pub title: String,
    pub steps: Vec<Step>,
    pub status: Status,
    pub recommendation: Option<String>,
    pub question: Option<String>,
    #[serde(default)]
    pub options: Vec<DecisionOption>,
    #[serde(default, rename = "recommendedOption")]
    pub recommended_option: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub params: Vec<String>,
    #[serde(default)]
    pub parallel: Vec<ParallelStep>,
    pub status: Option<Status>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParallelStep {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub params: Vec<String>,
    pub status: Option<Status>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiState {
    #[serde(default)]
    pub optional: bool,
    pub id: String,
    pub label: String,
    pub image: Option<String>,
    pub status: Status,
    pub recommendation: Option<String>,
    pub question: Option<String>,
    #[serde(default)]
    pub options: Vec<DecisionOption>,
    #[serde(default, rename = "recommendedOption")]
    pub recommended_option: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionOption {
    pub id: String,
    pub label: String,
    pub detail: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutOfScope {
    pub id: String,
    pub label: String,
    pub status: Status,
}

/// A decision staged or sent for a sheet row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetDecision {
    pub row_id: String,
    pub outcome: DecisionOutcome,
    #[serde(default)]
    pub override_recommendation: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Epoch milliseconds at which the batch was sent.
    pub at: i64,
    pub revision: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DecisionOutcome {
    Decided,
    Rejected,
}

/// A decision awaiting submission from the local draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetDecisionDraft {
    pub row_id: String,
    pub outcome: DecisionOutcome,
    #[serde(default)]
    pub override_recommendation: bool,
    #[serde(default)]
    pub note: Option<String>,
}

/// Device-local decisions staged against a particular revision.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetDecisionDrafts {
    pub revision: u32,
    pub decisions: Vec<SheetDecisionDraft>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitSheetDecisionsParams {
    pub artifact_id: String,
    pub revision: u32,
    pub decisions: Vec<SheetDecisionDraft>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitSheetDecisionsReply {
    pub artifact_id: String,
    pub decisions: Vec<SheetDecision>,
}

/// Classify recorded decisions after a sheet revision, preserving both sets.
pub fn partition_decisions<'a>(
    sheet: &DecisionSheet,
    decisions: &'a [SheetDecision],
) -> (Vec<&'a SheetDecision>, Vec<&'a SheetDecision>) {
    let ids: HashSet<&str> = sheet
        .decision_rows()
        .into_iter()
        .map(|row| row.id)
        .collect();
    decisions
        .iter()
        .partition(|decision| ids.contains(decision.row_id.as_str()))
}

/// Format one user message for a sent batch in sheet order.
pub fn decision_batch_message(title: &str, decisions: &[SheetDecision]) -> String {
    let entries = decisions
        .iter()
        .map(|decision| {
            if decision.override_recommendation {
                return format!("{} answered: {}", decision.row_id, decision.note.as_deref().unwrap_or("").trim());
            }
            let outcome = match decision.outcome {
                DecisionOutcome::Decided => "accepted",
                DecisionOutcome::Rejected => "rejected",
            };
            let note = decision
                .note
                .as_deref()
                .map(str::trim)
                .filter(|note| !note.is_empty())
                .map(|note| format!(" ({note})"))
                .unwrap_or_default();
            format!("{} {outcome}{note}", decision.row_id)
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("Decisions for {title}: {entries}")
}

/// A top-level decision shown in the sheet summary.
#[derive(Debug)]
pub struct DecisionRow<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub status: &'a Status,
    pub recommendation: Option<&'a str>,
    pub question: Option<&'a str>,
    pub optional: bool,
    pub options: &'a [DecisionOption],
    pub recommended_option: Option<&'a str>,
}

/// Render the sheet and its recorded answers for sharing as Markdown.
pub fn markdown(sheet: &DecisionSheet, decisions: &[SheetDecision]) -> String {
    use std::fmt::Write;
    let mut out = format!("# {}\n", sheet.title);
    for row in sheet.decision_rows() {
        let decision = decisions.iter().rev().find(|d| d.row_id == row.id);
        let state = decision
            .map(|d| match d.outcome {
                DecisionOutcome::Decided => "decided",
                DecisionOutcome::Rejected => "rejected",
            })
            .unwrap_or(&row.status.state);
        let _ = write!(out, "\n## {}: {}\n\nStatus: {}\n", row.id, row.title, state);
        if let Some(question) = row.question {
            let _ = writeln!(out, "\nDecision: {question}");
        }
        if let Some(recommendation) = row.recommendation {
            let _ = writeln!(out, "\nRecommendation: {recommendation}");
        }
        if let Some(note) = decision.and_then(|d| d.note.as_deref()) {
            let _ = writeln!(out, "\nAnswer: {note}");
        }
    }
    out
}

impl DecisionSheet {
    /// Apply sent decisions for presentation without changing the published JSON.
    pub fn apply_decisions(&mut self, decisions: &[SheetDecision]) {
        fn apply(id: &str, status: &mut Status, decisions: &[SheetDecision]) {
            if let Some(decision) = decisions
                .iter()
                .rev()
                .find(|decision| decision.row_id == id)
            {
                status.state = match decision.outcome {
                    DecisionOutcome::Decided => "decided",
                    DecisionOutcome::Rejected => "rejected",
                }
                .into();
                status.date = chrono::DateTime::from_timestamp_millis(decision.at)
                    .map(|at| at.format("%Y-%m-%d").to_string());
            }
        }
        for row in &mut self.flows {
            apply(&row.id, &mut row.status, decisions);
        }
        for row in &mut self.models {
            apply(&row.id, &mut row.status, decisions);
        }
        for row in &mut self.endpoints {
            apply(&row.id, &mut row.status, decisions);
        }
        for group in &mut self.rules {
            for row in &mut group.rows {
                apply(&row.id, &mut row.status, decisions);
            }
        }
        for row in &mut self.validations {
            apply(&row.id, &mut row.status, decisions);
        }
        for row in &mut self.permissions {
            apply(&row.id, &mut row.status, decisions);
        }
        for row in &mut self.ui_states {
            apply(&row.id, &mut row.status, decisions);
        }
    }

    /// The decisions displayed in the summary, in display order.
    pub fn decision_rows(&self) -> Vec<DecisionRow<'_>> {
        let mut rows = Vec::new();
        macro_rules! add {
            ($row:expr, $title:expr) => {
                rows.push(DecisionRow {
                    id: &$row.id,
                    title: $title,
                    status: &$row.status,
                    recommendation: $row.recommendation.as_deref(),
                    question: $row.question.as_deref(),
                    optional: $row.optional,
                    options: &$row.options,
                    recommended_option: $row.recommended_option.as_deref(),
                });
            };
        }
        for row in &self.flows {
            add!(row, &row.title);
        }
        for row in &self.models {
            add!(row, &row.name);
        }
        for row in &self.endpoints {
            add!(row, &row.name);
        }
        for group in &self.rules {
            for row in &group.rows {
                add!(row, &row.state);
            }
        }
        for row in &self.validations {
            add!(row, &row.rule);
        }
        for row in &self.permissions {
            add!(row, &row.subject);
        }
        for row in &self.ui_states {
            add!(row, &row.label);
        }
        rows
    }

    pub fn parse(source: &str) -> Result<Self, String> {
        let sheet: Self = serde_json::from_str(source).map_err(|e| e.to_string())?;
        if sheet.kind != "decision-sheet" {
            return Err("kind must be decision-sheet".into());
        }
        if sheet.title.trim().is_empty() {
            return Err("title must not be empty".into());
        }
        for decision in sheet.decision_rows() {
            if !decision.options.is_empty() {
                if decision.status.state != "open" || !(2..=3).contains(&decision.options.len()) {
                    return Err(format!("options for {:?} require an open row and 2–3 entries", decision.id));
                }
                let mut option_ids = HashSet::new();
                if decision.options.iter().any(|o| o.id.trim().is_empty() || o.label.trim().is_empty() || !option_ids.insert(&o.id)) {
                    return Err(format!("invalid or duplicate option for {:?}", decision.id));
                }
                if !decision.recommended_option.is_some_and(|id| option_ids.contains(&id.to_owned())) || decision.recommendation.as_deref().is_none_or(|r| r.trim().is_empty()) {
                    return Err(format!("recommendation and valid recommendedOption required for {:?}", decision.id));
                }
            } else if decision.recommended_option.is_some() {
                return Err(format!("recommendedOption requires options for {:?}", decision.id));
            }
        }
        let mut ids = HashSet::new();
        fn row(ids: &mut HashSet<String>, id: &str, status: &Status) -> Result<(), String> {
            if id.trim().is_empty() {
                return Err("row id must not be empty".into());
            }
            if !ids.insert(id.to_owned()) {
                return Err(format!("duplicate id {id:?}"));
            }
            match status.state.as_str() {
                "open" if status.date.is_none() => {}
                "decided" | "rejected"
                    if status.date.as_ref().is_some_and(|d| {
                        chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok() && d.len() == 10
                    }) => {}
                _ => return Err(format!("invalid status or date for {id:?}")),
            }
            Ok(())
        }
        for m in &sheet.models {
            row(&mut ids, &m.id, &m.status)?;
            for f in &m.fields {
                row(&mut ids, &f.id, f.status.as_ref().unwrap_or(&m.status))?;
                if !["+", "~", "−", ""].contains(&f.marker.as_str()) {
                    return Err(format!("invalid marker for {:?}", f.id));
                }
            }
        }
        for e in &sheet.endpoints {
            row(&mut ids, &e.id, &e.status)?;
            for c in &e.callers {
                row(&mut ids, &c.id, c.status.as_ref().unwrap_or(&e.status))?;
            }
            for l in e.request.iter().chain(&e.response) {
                row(&mut ids, &l.id, l.status.as_ref().unwrap_or(&e.status))?;
                if !["+", "−", ""].contains(&l.marker.as_str()) {
                    return Err(format!("invalid marker for {:?}", l.id));
                }
            }
        }
        for r in &sheet.rules {
            if r.id.trim().is_empty() || !ids.insert(r.id.clone()) {
                return Err(format!("missing or duplicate id {:?}", r.id));
            }
            for x in &r.rows {
                row(&mut ids, &x.id, &x.status)?;
                if x.cells.len() != r.columns.len()
                    || x.cells
                        .iter()
                        .any(|c| !["✓", "—", "?"].contains(&c.as_str()))
                {
                    return Err(format!("invalid cells for {:?}", x.id));
                }
            }
        }
        for v in &sheet.validations {
            row(&mut ids, &v.id, &v.status)?;
        }
        for p in &sheet.permissions {
            row(&mut ids, &p.id, &p.status)?;
        }
        for f in &sheet.flows {
            row(&mut ids, &f.id, &f.status)?;
            for s in &f.steps {
                row(&mut ids, &s.id, s.status.as_ref().unwrap_or(&f.status))?;
                for p in &s.parallel {
                    row(
                        &mut ids,
                        &p.id,
                        p.status.as_ref().or(s.status.as_ref()).unwrap_or(&f.status),
                    )?;
                    if !["ui", "api", "db", "event", "effect", "none"].contains(&p.kind.as_str()) {
                        return Err(format!("invalid flow kind for {:?}", p.id));
                    }
                }
                if !["ui", "api", "db", "event", "effect", "none"].contains(&s.kind.as_str()) {
                    return Err(format!("invalid flow kind for {:?}", s.id));
                }
            }
        }
        for s in &sheet.ui_states {
            row(&mut ids, &s.id, &s.status)?;
            if s.image.as_ref().is_some_and(|image| {
                !image.starts_with("mast-artifact:") && !image.starts_with("zeron-artifact:")
            }) {
                return Err(format!(
                    "image for {:?} must be an artifact reference",
                    s.id
                ));
            }
        }
        for s in &sheet.out_of_scope {
            row(&mut ids, &s.id, &s.status)?;
        }
        Ok(sheet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn answer_override_is_distinct_from_an_acceptance_note() {
        let decision = SheetDecision { row_id: "D-3".into(), outcome: DecisionOutcome::Decided,
            override_recommendation: true, note: Some("Use UTC".into()), at: 1, revision: 1 };
        assert_eq!(decision_batch_message("T", &[decision]), "Decisions for T: D-3 answered: Use UTC");
    }

    #[test]
    fn optional_rows_default_to_required() {
        let source = r#"{"kind":"decision-sheet","title":"T","flows":[{"id":"a","title":"A","status":{"state":"open"},"steps":[]},{"id":"b","title":"B","optional":true,"status":{"state":"open"},"steps":[]}]}"#;
        let sheet = DecisionSheet::parse(source).unwrap();
        let rows = sheet.decision_rows();
        assert!(!rows[0].optional);
        assert!(rows[1].optional);
        let html = crate::decision_sheet_render::render(&sheet);
        assert_eq!(html.matches("data-sheet-reject").count(), 1);
        assert_eq!(html.matches("data-sheet-answer").count(), 2);
    }

    #[test]
    fn decisions_survive_republication_and_report_missing_rows() {
        let source = r#"{"kind":"decision-sheet","title":"T","flows":[{"id":"D-2","title":"Flow","status":{"state":"open"},"recommendation":"Proceed","question":"Proceed?","steps":[]}]}"#;
        let sheet = DecisionSheet::parse(source).unwrap();
        let decisions = vec![
            SheetDecision {
                row_id: "D-2".into(),
                outcome: DecisionOutcome::Decided,
                override_recommendation: false,
                note: None,
                at: 20,
                revision: 1,
            },
            SheetDecision {
                row_id: "D-4".into(),
                outcome: DecisionOutcome::Rejected,
                override_recommendation: false,
                note: Some("too slow".into()),
                at: 20,
                revision: 1,
            },
        ];
        let (surviving, orphaned) = partition_decisions(&sheet, &decisions);
        assert_eq!(surviving[0].row_id, "D-2");
        assert_eq!(orphaned[0].row_id, "D-4");
        assert_eq!(
            decision_batch_message("T", &decisions),
            "Decisions for T: D-2 accepted, D-4 rejected (too slow)"
        );
        assert_eq!(
            serde_json::from_str::<SheetDecision>(&serde_json::to_string(&decisions[0]).unwrap())
                .unwrap(),
            decisions[0]
        );
    }

    #[test]
    fn markdown_shows_recorded_answers() {
        let sheet = DecisionSheet::parse(include_str!("../../../docs/artifact-kit/decision-sheet-example.json")).unwrap();
        let row_id = sheet.decision_rows()[0].id.to_owned();
        let decision = SheetDecision { row_id, outcome: DecisionOutcome::Decided, override_recommendation: true, note: Some("Use UTC".into()), at: 1, revision: 2 };
        let text = markdown(&sheet, &[decision]);
        assert!(text.contains("Status: decided"));
        assert!(text.contains("Answer: Use UTC"));
    }

    #[test]
    fn markdown_includes_each_decision_row() {
        let sheet = DecisionSheet::parse(include_str!("../../../docs/artifact-kit/decision-sheet-example.json")).unwrap();
        let output = markdown(&sheet, &[]);
        assert_eq!(output, include_str!("snapshots/decision-sheet-markdown.md"));
        for row in sheet.decision_rows() {
            assert!(output.contains(&format!("## {}: {}", row.id, row.title)));
            assert!(output.contains(&format!("Status: {}", row.status.state)));
        }
    }
    #[test]
    fn decision_rows_match_summary_order_and_exclude_children() {
        let source = r#"{"kind":"decision-sheet","title":"T","models":[{"id":"model","name":"Model","status":{"state":"open"},"recommendation":null,"question":null,"fields":[{"id":"field","name":"Field","marker":"","type":"text","null":false,"default":null,"constraint":null}]}],"flows":[{"id":"flow","title":"Flow","status":{"state":"open"},"recommendation":"Proceed","question":"Proceed?","steps":[{"id":"step","kind":"ui","label":"Open","params":[]}]}]}"#;
        let sheet = DecisionSheet::parse(source).unwrap();
        let rows = sheet.decision_rows();
        assert_eq!(
            rows.iter().map(|row| row.id).collect::<Vec<_>>(),
            ["flow", "model"]
        );
        assert_eq!(rows[0].question, Some("Proceed?"));
        assert_eq!(rows[0].recommendation, Some("Proceed"));
    }

    #[test]
    fn children_can_inherit_parent_status() {
        let source = r#"{"kind":"decision-sheet","title":"T","flows":[{"id":"flow","title":"Flow","status":{"state":"open"},"recommendation":"Proceed","question":"Proceed?","steps":[{"id":"step","kind":"ui","label":"Open","params":[]}]}]}"#;
        let sheet = DecisionSheet::parse(source).unwrap();
        assert!(sheet.flows[0].steps[0].status.is_none());
        assert_eq!(sheet.flows[0].recommendation.as_deref(), Some("Proceed"));
    }
    #[test]
    fn rejects_unknown_keys_and_invalid_rows() {
        assert!(
            DecisionSheet::parse(r#"{"kind":"decision-sheet","title":"T","extra":1}"#)
                .unwrap_err()
                .contains("unknown field")
        );
        assert!(DecisionSheet::parse(r#"{"kind":"decision-sheet","title":"T","ui_states":[{"label":"x","image":null,"status":{"state":"open","date":null}}]}"#).unwrap_err().contains("missing field `id`"));
        assert!(DecisionSheet::parse(r#"{"kind":"decision-sheet","title":"T","ui_states":[{"id":"x","label":"x","image":null,"status":{"state":"maybe","date":null}}]}"#).unwrap_err().contains("invalid status"));
        assert!(DecisionSheet::parse(r#"{"kind":"decision-sheet","title":"T","ui_states":[{"id":"x","label":"x","image":null,"status":{"state":"decided","date":"2026-02-30"}}]}"#).is_err());
    }
}
