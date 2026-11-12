//! HTML rendering for validated decision sheets.
use crate::decision_sheet::{DecisionSheet, Status};
use std::fmt::Write;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn identifier(s: &str) -> String {
    esc(s).replace('_', "_<wbr>").replace('.', ".<wbr>")
}
fn row_label(id: &str) -> String {
    format!("<span class=\"mk-row-id\">{}</span> ", esc(id))
}
fn status(s: &Status) -> String {
    let (class, label) = match s.state.as_str() {
        "decided" => ("mk-decided", "✓"),
        "rejected" => ("mk-removed", "Rejected"),
        _ => ("mk-open", "Open"),
    };
    format!(
        "<span class=\"mk-chip {class}\">{label}{}</span>",
        s.date
            .as_deref()
            .map(|d| format!(" {}", esc(d)))
            .unwrap_or_default()
    )
}
fn compact_status(s: &Status) -> String {
    let (class, label) = match s.state.as_str() {
        "decided" => ("mk-decided", "✓"),
        "rejected" => ("mk-removed", "✗"),
        _ => ("mk-open", "?"),
    };
    format!("<span class=\"mk-chip {class}\">{label}</span>")
}
fn child_status(child: Option<&Status>, parent: &Status) -> String {
    let Some(child) = child else {
        return String::new();
    };
    if child.state == parent.state {
        String::new()
    } else {
        compact_status(child)
    }
}
fn section(out: &mut String, name: &str) {
    write!(out, "<h2 class=\"mk-section\">{}</h2>", esc(name)).unwrap();
}
fn marker(m: &str) -> &'static str {
    match m {
        "+" => "mk-added",
        "~" => "mk-changed",
        "−" => "mk-removed",
        _ => "mk-context",
    }
}

pub fn render(sheet: &DecisionSheet) -> String {
    let mut out = format!("<div class=\"mk-sheet\"><h1>{}</h1>", esc(&sheet.title));
    let rows = sheet.decision_rows();
    out.push_str("<div class=\"mk-summary\">");
    for (state, label) in [
        ("open", "need you"),
        ("decided", "decided"),
        ("rejected", "rejected"),
    ] {
        write!(
            out,
            "<div><strong>{}</strong><span>{label}</span></div>",
            rows.iter().filter(|r| r.status.state == state).count()
        )
        .unwrap();
    }
    out.push_str("</div>");
    for (state, heading) in [
        ("open", "Needs you"),
        ("decided", "Decided"),
        ("rejected", "Rejected"),
    ] {
        if rows.iter().any(|r| r.status.state == state) {
            section(&mut out, heading);
            for row in rows.iter().filter(|r| r.status.state == state) {
                write!(
                    out,
                    "<div class=\"mk-decision\" id=\"row-{}\" data-sheet-row=\"{}\"><div>{}<strong>{}</strong>",
                    esc(row.id), esc(row.id), row_label(row.id), esc(row.question.unwrap_or(row.title))
                )
                .unwrap();
                if let Some(recommendation) = row.recommendation {
                    write!(
                        out,
                        "<div class=\"mk-recommendation\"><b>Recommendation:</b> {}</div>",
                        esc(recommendation)
                    )
                    .unwrap();
                }
                if state == "open" {
                    out.push_str("<div class=\"mk-sheet-actions\" hidden><button type=\"button\" data-sheet-accept>Accept</button><button type=\"button\" data-sheet-answer>Answer differently</button>");
                    if row.optional { out.push_str("<button type=\"button\" data-sheet-reject>Don't do this</button>"); }
                    out.push_str("<button type=\"button\" data-sheet-undo>Undo</button><label>💬 <input data-sheet-note maxlength=\"600\" placeholder=\"Note\"></label><span data-sheet-status></span></div>");
                }
                write!(out, "</div>{}</div>", status(row.status)).unwrap();
            }
        }
    }
    if !sheet.flows.is_empty() {
        section(&mut out, "Flows");
        for f in &sheet.flows {
            write!(out, "<div class=\"mk-flow\"><div class=\"mk-card-header\"><span>{}{}</span> {}</div><div class=\"mk-lane\">", row_label(&f.id), esc(&f.title), status(&f.status)).unwrap();
            for s in &f.steps {
                for (kind, label, params, child) in
                    std::iter::once((&s.kind, &s.label, &s.params, s.status.as_ref())).chain(
                        s.parallel.iter().map(|p| {
                            (
                                &p.kind,
                                &p.label,
                                &p.params,
                                p.status.as_ref().or(s.status.as_ref()),
                            )
                        }),
                    )
                {
                    if kind == "none" {
                        continue;
                    }
                    write!(out, "<div class=\"mk-step mk-{}\"><div class=\"mk-step-chip\"><div class=\"mk-step-kind\">{}</div><strong>{}</strong></div><div class=\"mk-step-params\">{}</div>{}</div>", kind, esc(&kind.to_uppercase()), identifier(label), params.iter().map(|x| identifier(x)).collect::<Vec<_>>().join(" · "), child_status(child, &f.status)).unwrap();
                }
            }
            out.push_str("</div>");
            for s in &f.steps {
                for (kind, label) in std::iter::once((&s.kind, &s.label))
                    .chain(s.parallel.iter().map(|p| (&p.kind, &p.label)))
                {
                    if kind == "none" {
                        write!(
                            out,
                            "<div class=\"mk-does-not-happen\"><b>Does not happen:</b> {}</div>",
                            esc(label)
                        )
                        .unwrap();
                    }
                }
            }
            out.push_str("</div>");
        }
    }
    if !sheet.models.is_empty() {
        section(&mut out, "Data model");
        out.push_str("<div class=\"mk-grid\">");
        for m in &sheet.models {
            write!(out,"<div class=\"mk-card\"><div class=\"mk-card-header\"><span>{}{}</span> {}</div><table class=\"mk-table\"><tr><th></th><th>field</th><th>type</th><th>null / default</th><th class=\"mk-notes\">constraint</th><th></th></tr>",row_label(&m.id),esc(&m.name),status(&m.status)).unwrap();
            for f in &m.fields {
                write!(out,"<tr id=\"row-{}\"><td class=\"mk-marker {}\">{}</td><td class=\"mk-mono\">{}{}</td><td class=\"mk-mono\">{}</td><td>{}{}</td><td class=\"mk-notes\">{}</td><td>{}</td></tr>",esc(&f.id),marker(&f.marker),esc(&f.marker),row_label(&f.id),identifier(&f.name),identifier(&f.field_type),if f.null { "nullable" } else { "required" }, f.default.as_ref().map(|x| format!(" / {}",esc(x))).unwrap_or_default(),f.constraint.as_deref().map(esc).unwrap_or_else(|| "—".into()),child_status(f.status.as_ref(), &m.status)).unwrap();
            }
            out.push_str("</table></div>");
        }
        out.push_str("</div>");
    }
    for e in &sheet.endpoints {
        section(&mut out, &format!("API contract · {}", e.name));
        write!(out,"<div class=\"mk-grid\"><div class=\"mk-card\"><div class=\"mk-card-header\"><span>{}{}</span> {}</div></div>",row_label(&e.id),esc(&e.name),status(&e.status)).unwrap();
        for (name, lines) in [("request", &e.request), ("response", &e.response)] {
            write!(out,"<div class=\"mk-card\"><div class=\"mk-card-header\">{name}</div><div class=\"mk-diff\">").unwrap();
            for l in lines {
                write!(
                    out,
                    "<div class=\"{}\">{} {} {}</div>",
                    marker(&l.marker),
                    esc(&l.marker),
                    esc(&l.text),
                    child_status(l.status.as_ref(), &e.status)
                )
                .unwrap();
            }
            out.push_str("</div></div>");
        }
        if !e.callers.is_empty() {
            out.push_str("<div class=\"mk-card\"><div class=\"mk-card-header\">existing callers</div><table class=\"mk-table\">");
            for c in &e.callers {
                write!(out, "<tr><td class=\"mk-mono\">{}</td><td class=\"mk-notes\">{}</td><td>{}</td></tr>", esc(&c.name), esc(&c.effect), child_status(c.status.as_ref(), &e.status)).unwrap();
            }
            out.push_str("</table></div>");
        }
        out.push_str("</div>");
    }
    if !sheet.rules.is_empty() {
        section(&mut out, "Rules");
        for r in &sheet.rules {
            out.push_str("<div class=\"mk-card\"><table class=\"mk-table\"><tr><th>state</th>");
            let has_action = r.rows.iter().any(|row| row.action.is_some());
            if has_action {
                out.push_str("<th>action</th>");
            }
            for c in &r.columns {
                write!(out, "<th>{}</th>", esc(c)).unwrap();
            }
            out.push_str("<th></th></tr>");
            for row in &r.rows {
                write!(
                    out,
                    "<tr><td>{}{}</td>",
                    row_label(&row.id),
                    esc(&row.state)
                )
                .unwrap();
                if has_action {
                    write!(
                        out,
                        "<td>{}</td>",
                        row.action.as_deref().map(esc).unwrap_or_else(|| "—".into())
                    )
                    .unwrap();
                }
                for c in &row.cells {
                    let class = match c.as_str() {
                        "✓" => "mk-yes",
                        "—" => "mk-no",
                        _ => "mk-question",
                    };
                    write!(out, "<td class=\"{class}\">{}</td>", esc(c)).unwrap();
                }
                write!(out, "<td>{}</td></tr>", compact_status(&row.status)).unwrap();
            }
            out.push_str("</table></div>");
        }
    }
    if !sheet.validations.is_empty() {
        section(&mut out, "Validations");
        out.push_str("<div class=\"mk-card\"><table class=\"mk-table\"><tr><th>rule</th><th>enforced</th><th>result</th><th></th></tr>");
        for v in &sheet.validations {
            write!(out, "<tr><td>{}{}</td><td>", row_label(&v.id), esc(&v.rule)).unwrap();
            for e in &v.enforced {
                write!(out, "<span class=\"mk-chip mk-neutral\">{}</span>", esc(e)).unwrap();
            }
            write!(
                out,
                "</td><td>{}</td><td>{}</td></tr>",
                esc(&v.result),
                compact_status(&v.status)
            )
            .unwrap();
        }
        out.push_str("</table></div>");
    }
    if !sheet.permissions.is_empty() {
        section(&mut out, "Permissions");
        let columns: std::collections::BTreeSet<&str> = sheet
            .permissions
            .iter()
            .flat_map(|p| p.access.keys().map(String::as_str))
            .collect();
        out.push_str("<div class=\"mk-card\"><table class=\"mk-table\"><tr><th>subject</th>");
        for column in &columns {
            write!(out, "<th>{}</th>", esc(column)).unwrap();
        }
        out.push_str("<th></th></tr>");
        for p in &sheet.permissions {
            write!(
                out,
                "<tr><td class=\"mk-mono\">{}{}</td>",
                row_label(&p.id),
                esc(&p.subject)
            )
            .unwrap();
            for column in &columns {
                write!(
                    out,
                    "<td>{}</td>",
                    p.access
                        .get(*column)
                        .map(|s| esc(s))
                        .unwrap_or_else(|| "—".into())
                )
                .unwrap();
            }
            write!(out, "<td>{}</td></tr>", compact_status(&p.status)).unwrap();
        }
        out.push_str("</table></div>");
    }
    if !sheet.ui_states.is_empty() {
        section(&mut out, "UI states");
        out.push_str("<div class=\"mk-screenshots\">");
        for s in &sheet.ui_states {
            out.push_str("<div class=\"mk-screenshot\"><div class=\"mk-screenshot-placeholder\">");
            if let Some(image) = &s.image {
                write!(out, "<a href=\"{}\">Open screenshot</a>", esc(image)).unwrap();
            } else {
                out.push_str("screenshot");
            }
            out.push_str("</div>");
            write!(
                out,
                "<div class=\"mk-caption\">{}{} {}</div></div>",
                row_label(&s.id),
                esc(&s.label),
                compact_status(&s.status)
            )
            .unwrap();
        }
        out.push_str("</div>");
    }
    if !sheet.out_of_scope.is_empty() {
        section(&mut out, "Out of scope");
        out.push_str("<div class=\"mk-chips\">");
        for s in &sheet.out_of_scope {
            write!(
                out,
                "<span id=\"row-{}\">{}<span class=\"mk-chip mk-removed\">{}</span> {}</span>",
                esc(&s.id),
                row_label(&s.id),
                esc(&s.label),
                compact_status(&s.status)
            )
            .unwrap();
        }
        out.push_str("</div>");
    }
    out.push_str("</div>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_references_are_safe_links() {
        let source = r#"{"kind":"decision-sheet","title":"T","ui_states":[{"id":"screen","label":"Screen","image":"mast-artifact:abc","status":{"state":"open","date":null}}]}"#;
        let sheet = DecisionSheet::parse(source).unwrap();
        assert!(render(&sheet).contains("href=\"mast-artifact:abc\""));
        assert!(
            DecisionSheet::parse(&source.replace("mast-artifact:abc", "javascript:alert(1)"))
                .is_err()
        );
    }
    #[test]
    fn child_statuses_are_compact_and_only_show_changes() {
        let source = r#"{"kind":"decision-sheet","title":"T","flows":[{"id":"f","title":"Flow","status":{"state":"decided","date":"2026-11-09"},"steps":[{"id":"a","kind":"ui","label":"same","params":[],"status":{"state":"decided","date":"2026-11-09"}},{"id":"b","kind":"api","label":"open child","params":[],"status":{"state":"open","date":null}},{"id":"c","kind":"db","label":"rejected child","params":[],"status":{"state":"rejected","date":"2026-11-09"}}]}]}"#;
        let html = render(&DecisionSheet::parse(source).unwrap());
        assert_eq!(html.matches("2026-11-09").count(), 2);
        assert!(html.contains("open child</strong></div><div class=\"mk-step-params\"></div><span class=\"mk-chip mk-open\">?</span>"));
        assert!(html.contains("rejected child</strong></div><div class=\"mk-step-params\"></div><span class=\"mk-chip mk-removed\">✗</span>"));
    }
    #[test]
    fn decided_child_in_open_flow_is_visible() {
        let open = Status {
            state: "open".into(),
            date: None,
        };
        let decided = Status {
            state: "decided".into(),
            date: Some("2026-11-09".into()),
        };
        assert_eq!(
            child_status(Some(&decided), &open),
            "<span class=\"mk-chip mk-decided\">✓</span>"
        );
        assert!(child_status(Some(&decided), &decided).is_empty());
    }
    #[test]
    fn real_sheet_snapshot_when_fixture_available() {
        let Ok(path) = std::env::var("MAST_DECISION_SHEET_SOURCE") else {
            return;
        };
        let source = std::fs::read_to_string(path).unwrap();
        let sheet = DecisionSheet::parse(&source).unwrap();
        let html = render(&sheet);
        for row in sheet.decision_rows() {
            assert!(html.contains(&format!("id=\"row-{}\"", esc(row.id))));
            assert!(html.contains(&row_label(row.id)));
        }
        if let Ok(output) = std::env::var("MAST_DECISION_SHEET_RENDER_PATH") {
            std::fs::write(output, html).unwrap();
        }
    }
    #[test]
    fn sample_render_and_counts() {
        let sheet = DecisionSheet::parse(include_str!(
            "../../../docs/artifact-kit/decision-sheet-example.json"
        ))
        .unwrap();
        let html = render(&sheet);
        if let Ok(path) = std::env::var("MAST_DECISION_SHEET_RENDER_PATH") {
            std::fs::write(path, &html).unwrap();
        }
        assert_eq!(
            html,
            include_str!("../tests/snapshots/decision-sheet.fragment.html")
        );
        for row in sheet.decision_rows() {
            assert_eq!(
                html.matches(&format!("id=\"row-{}\"", esc(row.id))).count(),
                1
            );
            assert!(html.contains(&row_label(row.id)));
        }
        for field in sheet.models.iter().flat_map(|model| &model.fields) {
            assert!(html.contains(&format!("id=\"row-{}\"", esc(&field.id))));
            assert!(html.contains(&row_label(&field.id)));
        }
        for row in &sheet.out_of_scope {
            assert!(html.contains(&format!("id=\"row-{}\"", esc(&row.id))));
            assert!(html.contains(&row_label(&row.id)));
        }
        assert!(html.contains("need you"));
        assert!(html.contains("rejected"));
        for title in [
            "Flows",
            "Data model",
            "API contract",
            "Rules",
            "Validations",
            "Permissions",
            "UI states",
            "Out of scope",
        ] {
            assert!(html.contains(title), "{title}");
        }
        assert!(!html.contains("style=\""));
    }
}
