//! Render a validated plan into the artifact-kit host.
use crate::plan::{Plan, linear_body};
use std::fmt::Write;
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn inline(source: &str) -> String {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};
    let mut out = String::new();
    let mut links = Vec::new();
    for event in Parser::new(source) {
        match event {
            Event::Text(t) | Event::Html(t) | Event::InlineHtml(t) => out.push_str(&esc(&t)),
            Event::Code(t) => {
                let _ = write!(out, "<code class=\"mk-mono\">{}</code>", esc(&t));
            }
            Event::Start(Tag::Strong) => out.push_str("<strong>"),
            Event::End(TagEnd::Strong) => out.push_str("</strong>"),
            Event::Start(Tag::Emphasis) => out.push_str("<em>"),
            Event::End(TagEnd::Emphasis) => out.push_str("</em>"),
            Event::Start(Tag::Link { dest_url, .. }) => {
                let safe = dest_url.starts_with("https://")
                    || dest_url.starts_with("http://")
                    || dest_url.starts_with("mast-artifact:");
                links.push(safe);
                if safe {
                    let _ = write!(out, "<a href=\"{}\">", esc(&dest_url));
                }
            }
            Event::End(TagEnd::Link) => {
                if links.pop().unwrap_or(false) {
                    out.push_str("</a>");
                }
            }
            Event::SoftBreak | Event::HardBreak => out.push(' '),
            _ => {}
        }
    }
    out
}
fn section(out: &mut String, id: &str, title: &str) {
    let _ = write!(out, "<h2 class=\"mk-section\" id=\"s-{id}\">{title}</h2>");
}
pub fn render(p: &Plan) -> String {
    let mut o = format!(
        "<style>{}</style><div class=\"mk-sheet pl\" id=\"pl-root\"><div class=\"pl-wrap\"><nav class=\"pl-rail\">",
        include_str!("plan.css")
    );
    for (id, label) in [
        ("summary", "Summary"),
        ("order", "Build order"),
        ("tickets", "Tickets"),
        ("contracts", "Contracts"),
        ("decided", "Decided"),
        ("questions", "Questions"),
        ("evidence", "Evidence"),
        ("blast", "Blast radius"),
        ("verify", "Verification"),
    ] {
        let _ = write!(o, "<a data-to=\"s-{id}\">{label}</a>");
    }
    o.push_str("</nav><div class=\"pl-main\">");
    section(&mut o, "summary", &esc(&p.title));
    let ts = p.tickets.as_deref().unwrap_or(&[]);
    let qs = p.questions.as_deref().unwrap_or(&[]);
    let fs = p.findings.as_deref().unwrap_or(&[]);
    o.push_str("<div class=\"mk-summary\">");
    for (state, label) in [
        ("buildable", "buildable"),
        ("blocked-by-tickets", "blocked by tickets"),
        ("blocked-by-questions", "blocked by questions"),
    ] {
        let _ = write!(
            o,
            "<div><strong>{}</strong><span>{label}</span></div>",
            ts.iter().filter(|t| t.status.state == state).count()
        );
    }
    let _ = write!(
        o,
        "<div><strong>{}</strong><span>questions</span></div></div><p class=\"pl-lead\">{}</p>",
        qs.len(),
        inline(&p.summary)
    );
    if let Some(scope) = &p.scope {
        o.push_str("<details class=\"pl-ev\"><summary>Scope</summary><h4>Constraints</h4><ul>");
        for x in &scope.constraints {
            let _ = write!(o, "<li>{}</li>", esc(x));
        }
        o.push_str("</ul><h4>In scope</h4><ul>");
        for x in &scope.r#in {
            let _ = write!(o, "<li>{}</li>", esc(x));
        }
        o.push_str("</ul><h4>Out of scope</h4><ul>");
        for x in &scope.out {
            let _ = write!(o, "<li>{} — {}</li>", esc(&x.item), esc(&x.lives_in));
        }
        o.push_str("</ul><h4>Optional</h4><ul>");
        for x in &scope.optional {
            let _ = write!(o, "<li>{} [{}]</li>", esc(&x.item), esc(&x.question));
        }
        o.push_str("</ul></details>");
    }
    section(&mut o, "order", "Build order");
    o.push_str("<div class=\"pl-strip\">");
    for t in ts {
        let deps = t
            .depends_on
            .iter()
            .map(|d| d.id.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let _ = write!(
            o,
            "<button class=\"pl-tile pl-s-{}\" data-id=\"{}\" data-deps=\"{}\"><span class=\"pl-tn\">{}</span><span class=\"pl-tt\">{}</span><span class=\"pl-tl\"></span></button>",
            esc(&t.status.state),
            esc(&t.id),
            esc(&deps),
            esc(&t.id),
            esc(&t.title)
        );
    }
    o.push_str("</div>");
    section(&mut o, "tickets", "Tickets");
    for t in ts {
        let _ = write!(
            o,
            "<div class=\"pl-ticket\" data-id=\"{}\" hidden><div class=\"pl-two\"><div class=\"pl-card\"><div class=\"pl-th\"><span class=\"pl-tid\">{}</span><h3>{}</h3></div><div class=\"pl-meta\"><span class=\"mk-chip mk-neutral\">{}</span></div><p class=\"pl-goal\">{}</p><h4>Behavior</h4><ul>",
            esc(&t.id),
            esc(&t.id),
            esc(&t.title),
            esc(&t.status.state),
            inline(&t.goal)
        );
        for b in &t.behavior {
            let _ = write!(o, "<li>{}", inline(&b.text));
            for r in b
                .row
                .iter()
                .chain(b.rows.iter().flatten())
                .chain(b.questions.iter().flatten())
            {
                let _ = write!(o, " <span class=\"pl-row pl-static\">{}</span>", esc(r));
            }
            o.push_str("</li>");
        }
        o.push_str("</ul><h4>Acceptance</h4><ul>");
        for a in &t.acceptance {
            let _ = write!(o, "<li>{}</li>", inline(a));
        }
        o.push_str("</ul><h4>Out of scope</h4>");
        if t.out_of_scope.is_empty() {
            o.push_str("<p>—</p>");
        } else {
            o.push_str("<ul>");
            for x in &t.out_of_scope {
                let _ = write!(
                    o,
                    "<li>{} <span class=\"pl-lives\">{}</span></li>",
                    inline(&x.text),
                    esc(&x.lives_in)
                );
            }
            o.push_str("</ul>");
        }
        if let Some(c) = &t.contract {
            let _ = write!(
                o,
                "<div class=\"pl-f\"><b>Contract</b><span>{} · {}</span></div>",
                esc(&c.r#ref),
                esc(&c.role)
            );
        }
        let _ = write!(
            o,
            "<div class=\"pl-f\"><b>Size</b><span>{} · {}–{} lines</span></div>",
            esc(&t.size.label),
            t.size.lines[0],
            t.size.lines[1]
        );
        if let Some(linear) = &t.linear {
            let _ = write!(
                o,
                "<div class=\"pl-f\"><b>Linear</b><span>{}</span></div>",
                esc(&linear.id)
            );
        }
        o.push_str("<div class=\"pl-f\"><b>Where</b><span>");
        for w in &t.r#where {
            let _ = write!(
                o,
                "<span class=\"pl-cite pl-{}\" title=\"{}\"><u></u>{}{}</span>",
                esc(w.status.as_deref().unwrap_or("measured")),
                esc(&w.path),
                esc(w.path.rsplit('/').next().unwrap_or(&w.path)),
                w.line.map(|v| format!(":{v}")).unwrap_or_default()
            );
        }
        o.push_str("</span></div><div class=\"pl-f\"><b>Depends on</b><span>");
        for d in &t.depends_on {
            let _ = write!(
                o,
                "<button class=\"pl-dep\" data-go=\"{}\">{} · {}</button>",
                esc(&d.id),
                esc(&d.id),
                esc(&d.mode)
            );
        }
        for d in t.external.as_deref().unwrap_or(&[]) {
            let _ = write!(
                o,
                "<span class=\"pl-dep\">{} · {}</span>",
                esc(&d.label),
                esc(&d.state)
            );
        }
        if t.depends_on.is_empty() && t.external.as_deref().unwrap_or(&[]).is_empty() {
            o.push_str("—");
        }
        let _ = write!(
            o,
            "</span></div></div><div class=\"pl-linear\"><div class=\"pl-cap\">Linear body</div><pre>{}</pre></div></div></div>",
            esc(&linear_body(t))
        );
    }
    section(&mut o, "contracts", "Contracts");
    o.push_str("<div class=\"mk-grid\">");
    for c in p.contracts.as_deref().unwrap_or(&[]) {
        let _ = write!(
            o,
            "<div class=\"pl-card pl-contract\"><h4>{}</h4><p>{}</p><div>Produced by {}</div><div>Consumed by {}</div><pre class=\"mk-mono pl-ex\">{}</pre></div>",
            esc(&c.name),
            inline(&c.summary),
            esc(&c.produced_by.join(", ")),
            esc(&c.consumed_by.join(", ")),
            esc(c.example.as_deref().unwrap_or(""))
        );
    }
    o.push_str("</div>");
    section(&mut o, "decided", "Decided");
    o.push_str("<details class=\"pl-ev\"><summary>Decisions</summary><div class=\"pl-dlist\">");
    for d in p.decisions.as_deref().unwrap_or(&[]) {
        let _ = write!(
            o,
            "<div class=\"pl-d\"><b>{}</b><p>{}</p>{}</div>",
            esc(&d.title),
            inline(&d.text),
            esc(&d.rows.join(", "))
        );
    }
    o.push_str("</div></details>");
    section(&mut o, "questions", "Open questions");
    for q in qs {
        let _ = write!(
            o,
            "<div class=\"pl-qrow\"><div><span class=\"pl-q\">{}</span> {} <span class=\"pl-row pl-static\">{}</span></div><div class=\"mk-muted\">Blocks {} {}</div></div>",
            esc(&q.id),
            esc(&q.text),
            esc(&q.row),
            esc(&q.blocks.join(", ")),
            inline(q.example.as_deref().unwrap_or(""))
        );
    }
    section(&mut o, "evidence", "Evidence");
    let _ = write!(
        o,
        "<details class=\"pl-ev\"><summary>{} findings</summary><ul class=\"pl-finds\">",
        fs.len()
    );
    for f in fs {
        let cite = f
            .cite
            .as_ref()
            .map(|c| c.path.as_deref().or(c.url.as_deref()).unwrap_or(""))
            .unwrap_or("");
        let _ = write!(
            o,
            "<li>{} <span class=\"pl-cite pl-{}\" title=\"{}\"><u></u>{} <i>{}</i></span></li>",
            inline(&f.claim),
            esc(&f.status),
            esc(cite),
            esc(cite.rsplit('/').next().unwrap_or(cite)),
            esc(&f.status)
        );
    }
    o.push_str("</ul></details>");
    section(&mut o, "blast", "Blast radius");
    o.push_str("<details class=\"pl-ev\"><summary>Surfaces</summary><ul>");
    for b in p.blast_radius.as_deref().unwrap_or(&[]) {
        let _ = write!(
            o,
            "<li><b>{}</b> · {} · {}: {} <span class=\"mk-muted\">{}</span></li>",
            esc(&b.surface),
            esc(&b.kind),
            esc(&b.audience),
            inline(&b.change),
            esc(&b.gate)
        );
    }
    o.push_str("</ul></details>");
    section(&mut o, "verify", "Verification");
    o.push_str("<details class=\"pl-ev\"><summary>Checks</summary><ul>");
    for v in p.verification.as_deref().unwrap_or(&[]) {
        let _ = write!(
            o,
            "<li>{} {}</li>",
            esc(v.ticket.as_deref().unwrap_or("")),
            esc(&v.text)
        );
    }
    o.push_str("</ul></details>");
    if let Some(items) = &p.superseded {
        if !items.is_empty() {
            o.push_str("<details class=\"pl-ev\"><summary>Superseded</summary><ul>");
            for x in items {
                let _ = write!(
                    o,
                    "<li>{} — {} ({})</li>",
                    esc(&x.item),
                    esc(&x.reason),
                    esc(&x.date)
                );
            }
            o.push_str("</ul></details>");
        }
    }
    let _ = write!(
        o,
        "</div></div></div><script>{}</script>",
        include_str!("plan.js")
    );
    o
}
#[cfg(test)]
mod tests {
    #[test]
    fn ticket_lists_render_inline_markdown_and_empty_dependencies() {
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../../../docs/artifact-kit/plan-example.json"))
                .unwrap();
        value["tickets"][0]["acceptance"][0] = serde_json::json!("Use `delivery_days`.");
        value["tickets"][0]["outOfScope"] =
            serde_json::json!([{ "text": "Skip `export`.", "livesIn": "later" }]);
        let plan = super::Plan::parse(&value.to_string()).unwrap();
        let html = super::render(&plan);
        assert!(html.contains("Use <code class=\"mk-mono\">delivery_days</code>."));
        assert!(html.contains("Skip <code class=\"mk-mono\">export</code>."));
        assert!(html.contains("<b>Depends on</b><span>—</span>"));
        assert!(html.contains("<h4>Out of scope</h4><p>—</p>"));
        assert!(!html.contains("<h4>Out of scope</h4><ul></ul>"));
    }
    #[test]
    #[ignore]
    fn render_file() {
        let input = std::env::var("MAST_PLAN_INPUT").unwrap();
        let output = std::env::var("MAST_PLAN_OUTPUT").unwrap();
        let plan = super::Plan::parse(&std::fs::read_to_string(input).unwrap()).unwrap();
        std::fs::write(output, super::render(&plan)).unwrap();
    }
}
