//! Structured plans and their validation.
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

macro_rules! types { ($($name:ident { $($field:ident : $ty:ty),* $(,)? })*) => {$ (
    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(deny_unknown_fields, rename_all = "camelCase")]
    pub struct $name { $(pub $field: $ty,)* }
)*}; }
types! {
 Plan { kind: String, title: String, summary: String, sheet: Option<Sheet>, scope: Option<Scope>, findings: Option<Vec<Finding>>, decisions: Option<Vec<Decision>>, tickets: Option<Vec<Ticket>>, contracts: Option<Vec<Contract>>, questions: Option<Vec<Question>>, blast_radius: Option<Vec<Blast>>, verification: Option<Vec<Verification>>, superseded: Option<Vec<Superseded>> }
 Sheet { r#ref: String, rev: u64 }
 Scope { constraints: Vec<String>, r#in: Vec<String>, out: Vec<ScopeOut>, optional: Vec<ScopeOptional> }
 ScopeOut { item: String, lives_in: String }
 ScopeOptional { item: String, question: String }
 Citation { path: Option<String>, line: Option<u64>, url: Option<String> }
 Finding { id: String, claim: String, cite: Option<Citation>, status: String, section: Option<String> }
 Decision { id: String, title: String, text: String, rows: Vec<String>, origin: String }
 Ticket { id: String, title: String, goal: String, linear: Option<Linear>, behavior: Vec<Behavior>, acceptance: Vec<String>, out_of_scope: Vec<TicketOut>, contract: Option<TicketContract>, r#where: Vec<Location>, depends_on: Vec<Dependency>, external: Option<Vec<External>>, status: TicketStatus, size: Size }
 Linear { id: String, url: String }
 Behavior { text: String, row: Option<String>, rows: Option<Vec<String>>, questions: Option<Vec<String>> }
 TicketOut { text: String, lives_in: String }
 TicketContract { r#ref: String, role: String }
 Location { path: String, line: Option<u64>, status: Option<String> }
 Dependency { id: String, mode: String }
 External { label: String, url: String, state: String }
 TicketStatus { state: String, questions: Vec<String> }
 Size { label: String, lines: [u64; 2] }
 Contract { id: String, name: String, summary: String, produced_by: Vec<String>, consumed_by: Vec<String>, example: Option<String> }
 Question { id: String, text: String, row: String, blocking: bool, blocks: Vec<String>, example: Option<String> }
 Blast { surface: String, kind: String, audience: String, change: String, gate: String }
 Verification { ticket: Option<String>, text: String }
 Superseded { item: String, reason: String, date: String }
}
impl Plan {
    pub fn parse(source: &str) -> Result<Self, String> {
        let mut deserializer = serde_json::Deserializer::from_str(source);
        let plan: Self =
            serde_path_to_error::deserialize(&mut deserializer).map_err(|e| e.to_string())?;
        deserializer.end().map_err(|e| e.to_string())?;
        plan.validate()?;
        Ok(plan)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.kind != "plan" {
            return Err("kind: expected plan".into());
        }
        if self.title.trim().is_empty() {
            return Err("title: empty".into());
        }
        macro_rules! unique {
            ($items:expr, $path:literal) => {{
                let mut seen = HashSet::new();
                for (i, item) in $items.iter().enumerate() {
                    if item.id.trim().is_empty() || !seen.insert(&item.id) {
                        return Err(format!(concat!($path, "[{}].id: empty or duplicate"), i));
                    }
                }
            }};
        }
        unique!(self.findings.as_deref().unwrap_or(&[]), "findings");
        unique!(self.decisions.as_deref().unwrap_or(&[]), "decisions");
        unique!(self.contracts.as_deref().unwrap_or(&[]), "contracts");
        unique!(self.questions.as_deref().unwrap_or(&[]), "questions");
        let tickets = self.tickets.as_deref().unwrap_or(&[]);
        let questions = self.questions.as_deref().unwrap_or(&[]);
        let contracts = self.contracts.as_deref().unwrap_or(&[]);
        let mut ids = HashSet::new();
        let mut previous = 0;
        for (i, t) in tickets.iter().enumerate() {
            let path = format!("tickets[{i}]");
            let n =
                t.id.strip_prefix('T')
                    .and_then(|s| s.parse::<u32>().ok())
                    .filter(|n| *n > 0)
                    .ok_or_else(|| format!("{path}.id: invalid ticket id"))?;
            if t.id != format!("T{n}") || n <= previous {
                return Err(format!("{path}.id: tickets must be in build order"));
            }
            previous = n;
            if !ids.insert(&t.id) {
                return Err(format!("{path}.id: duplicate"));
            }
            if t.goal.trim().is_empty()
                || t.behavior.is_empty()
                || t.acceptance.is_empty()
                || t.behavior.iter().any(|b| b.text.trim().is_empty())
                || t.acceptance.iter().any(|a| a.trim().is_empty())
            {
                return Err(format!(
                    "{path}: goal, behavior and acceptance must be nonempty"
                ));
            }
            for (j, b) in t.behavior.iter().enumerate() {
                let count = usize::from(b.row.is_some())
                    + usize::from(b.rows.is_some())
                    + usize::from(b.questions.is_some());
                if count != 1
                    || b.row.as_ref().is_some_and(|r| r.trim().is_empty())
                    || b.rows
                        .as_ref()
                        .is_some_and(|r| r.is_empty() || r.iter().any(|x| x.trim().is_empty()))
                    || b.questions.as_ref().is_some_and(|q| {
                        q.is_empty() || q.iter().any(|id| !questions.iter().any(|x| x.id == *id))
                    })
                {
                    return Err(format!("{path}.behavior[{j}]: invalid references"));
                }
            }
            let mut build = false;
            for (j, d) in t.depends_on.iter().enumerate() {
                let dn =
                    d.id.strip_prefix('T')
                        .and_then(|s| s.parse::<u32>().ok())
                        .unwrap_or(u32::MAX);
                if dn >= n
                    || !tickets.iter().any(|other| other.id == d.id)
                    || !["build", "merge"].contains(&d.mode.as_str())
                {
                    return Err(format!("{path}.dependsOn[{j}]: invalid dependency"));
                }
                build |= d.mode == "build";
            }
            for (j, q) in t.status.questions.iter().enumerate() {
                if !questions.iter().any(|x| x.id == *q && x.blocking) {
                    return Err(format!(
                        "{path}.status.questions[{j}]: unknown or nonblocking question"
                    ));
                }
            }
            let has_q = !t.status.questions.is_empty();
            if !match t.status.state.as_str() {
                "buildable" => !build && !has_q,
                "blocked-by-tickets" => build && !has_q,
                "blocked-by-questions" => has_q,
                _ => false,
            } {
                return Err(format!("{path}.status.state: inconsistent"));
            }
            if let Some(c) = &t.contract {
                if !contracts.iter().any(|x| {
                    x.id == c.r#ref
                        && match c.role.as_str() {
                            "produces" => x.produced_by.contains(&t.id),
                            "consumes" => x.consumed_by.contains(&t.id),
                            _ => false,
                        }
                }) {
                    return Err(format!("{path}.contract: invalid reference"));
                }
            }
            if t.size.lines[0] > t.size.lines[1]
                || !["S", "M", "L"].contains(&t.size.label.as_str())
            {
                return Err(format!("{path}.size: invalid size"));
            }
            for (j, w) in t.r#where.iter().enumerate() {
                if w.path.trim().is_empty()
                    || w.status.as_deref().is_some_and(|status| {
                        !["measured", "claimed", "inferred", "unverified"].contains(&status)
                    })
                {
                    return Err(format!("{path}.where[{j}].path: empty"));
                }
            }
        }
        for (i, q) in questions.iter().enumerate() {
            for (j, id) in q.blocks.iter().enumerate() {
                if !tickets
                    .iter()
                    .any(|t| t.id == *id && t.status.questions.contains(&q.id))
                {
                    return Err(format!(
                        "questions[{i}].blocks[{j}]: ticket does not list question"
                    ));
                }
            }
        }
        for (i, f) in self.findings.as_deref().unwrap_or(&[]).iter().enumerate() {
            if !["measured", "claimed", "inferred", "unverified"].contains(&f.status.as_str()) {
                return Err(format!("findings[{i}].status: invalid"));
            }
            if ["measured", "claimed"].contains(&f.status.as_str())
                && !f.cite.as_ref().is_some_and(|c| {
                    c.path.as_ref().is_some_and(|s| !s.trim().is_empty())
                        || c.url.as_ref().is_some_and(|s| !s.trim().is_empty())
                })
            {
                return Err(format!("findings[{i}].cite: required"));
            }
        }
        Ok(())
    }
    pub fn validate_sheet(
        &self,
        sheet: &crate::decision_sheet::DecisionSheet,
    ) -> Result<(), String> {
        let mut rows: Vec<(&str, &crate::decision_sheet::Status)> = sheet
            .decision_rows()
            .into_iter()
            .map(|r| (r.id, r.status))
            .collect();
        for model in &sheet.models {
            for field in &model.fields {
                rows.push((&field.id, field.status.as_ref().unwrap_or(&model.status)));
            }
        }
        for endpoint in &sheet.endpoints {
            for line in endpoint.request.iter().chain(&endpoint.response) {
                rows.push((&line.id, line.status.as_ref().unwrap_or(&endpoint.status)));
            }
            for caller in &endpoint.callers {
                rows.push((
                    &caller.id,
                    caller.status.as_ref().unwrap_or(&endpoint.status),
                ));
            }
        }
        for flow in &sheet.flows {
            for step in &flow.steps {
                let status = step.status.as_ref().unwrap_or(&flow.status);
                rows.push((&step.id, status));
                for child in &step.parallel {
                    rows.push((&child.id, child.status.as_ref().unwrap_or(status)));
                }
            }
        }
        let check = |id: &str, path: String| {
            if rows
                .iter()
                .any(|(row, status)| *row == id && status.state == "decided")
            {
                Ok(())
            } else {
                Err(format!("{path}: row {id} missing or not decided"))
            }
        };
        for (i, d) in self.decisions.as_deref().unwrap_or(&[]).iter().enumerate() {
            for (j, r) in d.rows.iter().enumerate() {
                check(r, format!("decisions[{i}].rows[{j}]"))?;
            }
        }
        for (i, t) in self.tickets.as_deref().unwrap_or(&[]).iter().enumerate() {
            for (j, b) in t.behavior.iter().enumerate() {
                if let Some(r) = &b.row {
                    check(r, format!("tickets[{i}].behavior[{j}].row"))?;
                }
                for (k, r) in b.rows.as_deref().unwrap_or(&[]).iter().enumerate() {
                    check(r, format!("tickets[{i}].behavior[{j}].rows[{k}]"))?;
                }
            }
        }
        Ok(())
    }
}
pub fn linear_body(t: &Ticket) -> String {
    use std::fmt::Write;
    let mut s = format!("# {}\n\n## Goal\n{}\n\n## Behavior\n", t.title, t.goal);
    for b in &t.behavior {
        let refs = b
            .row
            .iter()
            .chain(b.rows.iter().flatten())
            .chain(b.questions.iter().flatten())
            .map(|r| format!("[{r}]"))
            .collect::<Vec<_>>()
            .join(" ");
        let _ = writeln!(s, "- {} {}", b.text, refs);
    }
    s.push_str("\n## Acceptance\n");
    for a in &t.acceptance {
        let _ = writeln!(s, "- {a}");
    }
    s.push_str("\n## Out of scope\n");
    for o in &t.out_of_scope {
        let _ = writeln!(s, "- {} — {}", o.text, o.lives_in);
    }
    if t.out_of_scope.is_empty() {
        s.push_str("—\n");
    }
    if let Some(c) = &t.contract {
        s.push_str("\n## Contract\n");
        let _ = writeln!(s, "{} ({})", c.r#ref, c.role);
    }
    s.push_str("\n## Where\n");
    for w in &t.r#where {
        let _ = writeln!(
            s,
            "- {}{}",
            w.path,
            w.line.map(|n| format!(":{n}")).unwrap_or_default()
        );
    }
    if t.r#where.is_empty() {
        s.push_str("- —\n");
    }
    s.push_str("\n## Depends on\n");
    for d in &t.depends_on {
        let _ = writeln!(s, "- {} ({})", d.id, d.mode);
    }
    for d in t.external.as_deref().unwrap_or(&[]) {
        let _ = writeln!(s, "- {} ({})", d.label, d.state);
    }
    if t.depends_on.is_empty() && t.external.as_deref().unwrap_or(&[]).is_empty() {
        s.push_str("—\n");
    }
    let _ = write!(
        s,
        "\n## Size\n{}: {}–{} lines",
        t.size.label, t.size.lines[0], t.size.lines[1]
    );
    s
}

/// Render a plan for sharing as Markdown.
pub fn markdown(plan: &Plan) -> String {
    use std::fmt::Write;
    let mut out = format!("# {}\n\n{}\n", plan.title, plan.summary);
    if let Some(scope) = &plan.scope {
        out.push_str("\n## Scope\n");
        for item in &scope.r#in {
            let _ = writeln!(out, "- In: {item}");
        }
        for item in &scope.out {
            let _ = writeln!(out, "- Out: {} — {}", item.item, item.lives_in);
        }
        for item in &scope.optional {
            let _ = writeln!(out, "- Optional: {} — {}", item.item, item.question);
        }
        for item in &scope.constraints {
            let _ = writeln!(out, "- Constraint: {item}");
        }
    }
    out.push_str("\n## Build order\n");
    for ticket in plan.tickets.as_deref().unwrap_or(&[]) {
        let deps = ticket
            .depends_on
            .iter()
            .map(|d| format!("{} ({})", d.id, d.mode))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            out,
            "- {}: {} — depends on: {}; status: {}",
            ticket.id,
            ticket.title,
            if deps.is_empty() { "none" } else { &deps },
            ticket.status.state
        );
    }
    for ticket in plan.tickets.as_deref().unwrap_or(&[]) {
        let _ = write!(out, "\n---\n\n{}\n", linear_body(ticket));
    }
    out.push_str("\n## Contracts\n");
    for contract in plan.contracts.as_deref().unwrap_or(&[]) {
        let _ = writeln!(
            out,
            "\n### {}: {}\n\n{}\n\nProduced by: {}\n\nConsumed by: {}",
            contract.id,
            contract.name,
            contract.summary,
            contract.produced_by.join(", "),
            contract.consumed_by.join(", ")
        );
        if let Some(example) = &contract.example {
            let _ = writeln!(out, "\nExample: {example}");
        }
    }
    out.push_str("\n## Open questions\n");
    for q in plan.questions.as_deref().unwrap_or(&[]) {
        let _ = writeln!(
            out,
            "- {}: {} (row: {}; blocking: {}; blocks: {})",
            q.id,
            q.text,
            q.row,
            q.blocking,
            q.blocks.join(", ")
        );
        if let Some(example) = &q.example {
            let _ = writeln!(out, "  Example: {example}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> serde_json::Value {
        serde_json::from_str(include_str!("../../../docs/artifact-kit/plan-example.json")).unwrap()
    }
    fn parse(v: &serde_json::Value) -> Result<Plan, String> {
        Plan::parse(&v.to_string())
    }
    #[test]
    fn markdown_includes_plan_and_ticket_details() {
        let plan = parse(&fixture()).unwrap();
        let output = markdown(&plan);
        assert_eq!(output, include_str!("snapshots/plan-markdown.md"));
        assert!(output.contains(&format!("# {}", plan.title)));
        assert!(output.contains("## Build order"));
        for ticket in plan.tickets.as_deref().unwrap_or(&[]) {
            assert!(output.contains(&linear_body(ticket)));
        }
    }
    #[test]
    fn synthetic_plan_and_linear_body() {
        let p = parse(&fixture()).unwrap();
        let body = linear_body(&p.tickets.unwrap()[0]);
        assert_eq!(
            body,
            "# Order history improvement 1 with a descriptive title\n\n## Goal\nImprove order history step 1.\n\n## Behavior\n- Show the agreed behavior. [event-order]\n\n## Acceptance\n- The behavior works.\n\n## Out of scope\n—\n\n## Contract\nevents (produces)\n\n## Where\n- app/orders/views.py:120\n\n## Depends on\n—\n\n## Size\nM: 100–200 lines"
        );
    }
    #[test]
    fn linear_body_omits_absent_contract_and_marks_no_dependencies() {
        let p = parse(&fixture()).unwrap();
        let ticket = &p.tickets.unwrap()[4];
        let body = linear_body(ticket);
        assert!(!body.contains("## Contract"));
        assert!(body.contains("## Depends on\n—\n"));
    }
    #[test]
    fn rules_reject_invalid_values() {
        let cases: Vec<(&str, serde_json::Value)> = vec![
            (
                "dependency",
                serde_json::json!([{"id":"T7","mode":"build"}]),
            ),
            ("state", serde_json::json!("blocked-by-tickets")),
            ("question", serde_json::json!(["T1"])),
            ("contract", serde_json::json!("missing")),
            ("lines", serde_json::json!([200, 100])),
            ("cite", serde_json::Value::Null),
            ("goal", serde_json::json!("")),
            ("time", serde_json::json!(3)),
        ];
        for (name, value) in cases {
            let mut v = fixture();
            match name {
                "dependency" => v["tickets"][1]["dependsOn"] = value,
                "state" => v["tickets"][0]["status"]["state"] = value,
                "question" => v["questions"][0]["blocks"] = value,
                "contract" => v["tickets"][0]["contract"]["ref"] = value,
                "lines" => v["tickets"][0]["size"]["lines"] = value,
                "cite" => v["findings"][0]["cite"] = value,
                "goal" => v["tickets"][0]["goal"] = value,
                _ => v["tickets"][0]["days"] = value,
            };
            let error = parse(&v).unwrap_err();
            assert!(
                error.contains(if name == "question" {
                    "questions[0]"
                } else if name == "cite" {
                    "findings[0]"
                } else if name == "dependency" {
                    "tickets[1]"
                } else {
                    "tickets[0]"
                }),
                "{name}: {error}"
            );
        }
    }
    #[test]
    fn missing_sheet_row_is_rejected() {
        let p = parse(&fixture()).unwrap();
        let s = crate::decision_sheet::DecisionSheet::parse(
            r#"{"kind":"decision-sheet","title":"Test"}"#,
        )
        .unwrap();
        assert!(
            p.validate_sheet(&s)
                .unwrap_err()
                .contains("decisions[0].rows[0]")
        );
    }
}
