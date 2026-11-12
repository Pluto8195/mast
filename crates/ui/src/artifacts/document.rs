//! Web documents for visual artifacts. Every kind goes through the Codex
//! visualize host (`browser::fragment`): an outer document whose only content
//! is a `sandbox="allow-scripts"` srcdoc frame under the visualize CSP, so
//! fragments, pages, Mermaid diagrams, and render checks share one sandbox.
//!
//! - fragment: `fragment::host_document` exactly as the visualize card does;
//! - page: the same outer policy, the page as the frame's whole document,
//!   full width (pages bring their own layout);
//! - Mermaid: a fragment that loads Mermaid from the allowed jsDelivr CDN
//!   and renders the source with `securityLevel: "strict"`;
//! - image (render checks only): an `<img>` fragment.
//!
//! Render checks add a probe ([`PROBE_SCRIPT`]) inside the frame and a
//! collector in the outer document; the native side reads the collector's
//! `window.__mastProbe` once the probe has posted its report.
use crate::browser::fragment::{self, FragmentTheme, escape_attr, escape_text};

use super::bridge::HOST_SCRIPT as BRIDGE_HOST_SCRIPT;
use super::comments::FRAME_SCRIPT as COMMENT_FRAME_SCRIPT;
use super::find::{FRAME_SCRIPT as FIND_FRAME_SCRIPT, HOST_SCRIPT as FIND_HOST_SCRIPT};
use super::model::WebKind;
use super::question_html::{CSS as QUESTION_CSS, SCRIPT as QUESTION_FRAME_SCRIPT};

/// Mermaid build the diagram wrapper loads (on the visualize allowlist).
pub(crate) const MERMAID_SRC: &str = "https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.min.js";

/// How long the probe waits after `load` before measuring (fonts, icons,
/// first animation frames).
pub(crate) const SETTLE_MS: u32 = 600;

/// Upper bound the probe waits for Mermaid diagrams to finish.
pub(crate) const MERMAID_WAIT_MS: u32 = 4000;

/// The outer document for `kind` + `source`. `probe` adds the render-check
/// probe and collector.
pub(crate) fn web_document(
    kind: WebKind,
    source: &str,
    title: &str,
    mime_type: &str,
    theme: &FragmentTheme,
    reduced_motion: bool,
    probe: bool,
) -> String {
    // Every frame carries the link and find bridges; render checks add the
    // probe.
    let frame_scripts = if probe {
        format!(
            "{LINK_FRAME_SCRIPT}{FIND_FRAME_SCRIPT}{QUESTION_FRAME_SCRIPT}{}",
            probe_script(theme)
        )
    } else {
        // Only the app's own view gets the comment + answer bridge; render
        // checks and compare frames stay passive.
        format!(
            "{LINK_FRAME_SCRIPT}{FIND_FRAME_SCRIPT}{QUESTION_FRAME_SCRIPT}{COMMENT_FRAME_SCRIPT}"
        )
    };
    let probe_script = Some(frame_scripts);
    let document = match kind {
        WebKind::Fragment | WebKind::DecisionSheet | WebKind::Plan => {
            let body = with_probe_prefix(&with_question_style(source), probe_script.as_deref());
            fragment::host_document(
                &body,
                title,
                matches!(kind, WebKind::DecisionSheet | WebKind::Plan),
                theme,
                reduced_motion,
            )
        }
        WebKind::Mermaid => {
            let body = with_probe_prefix(
                &mermaid_fragment(source, theme.dark),
                probe_script.as_deref(),
            );
            fragment::host_document(&body, title, true, theme, reduced_motion)
        }
        WebKind::Image => {
            let body = with_probe_prefix(
                &image_fragment(source, mime_type, title),
                probe_script.as_deref(),
            );
            fragment::host_document(&body, title, true, theme, reduced_motion)
        }
        WebKind::Page => page_host_document(
            &inject_head_markup(source, &question_style()),
            title,
            theme,
            probe_script.as_deref(),
        ),
    };
    let document = with_link_host(&document);
    if probe {
        with_collector(&document)
    } else {
        document
    }
}

/// A standalone document without the host frame or Mast messaging bridges.
/// `source` is already rendered for structured artifact kinds.
pub(crate) fn export_document(kind: WebKind, source: &str, title: &str, pdf: bool) -> String {
    let light = FragmentTheme::from_theme(&crate::theme::Theme::light());
    let dark = FragmentTheme::from_theme(&crate::theme::Theme::dark());
    let mut style = fragment::base_stylesheet(&light, false);
    if !pdf {
        let mut dark_vars = String::from("color-scheme: dark;\n");
        for (name, value) in &dark.vars {
            dark_vars.push_str(&format!("{name}: {value};\n"));
        }
        style.push_str(&format!("\n@media (prefers-color-scheme: dark) {{ :root:not([data-theme]) {{ {dark_vars} }} }}\n:root[data-theme=dark] {{ {dark_vars} }}\n"));
    }
    style.push_str("html, body { background: var(--background); } body { padding: 24px !important; } .mk-sheet-actions { display: none !important; }\n");
    if kind == WebKind::DecisionSheet {
        style.push_str(".mk-grid, .mk-screenshots { grid-template-columns: minmax(0, 1fr); } .mk-table { table-layout: fixed; } .mk-table th, .mk-table td { white-space: normal; min-width: 0; overflow-wrap: anywhere; } .mk-table th:first-child, .mk-table td:first-child { width: 72px; } .mk-row-id, .mk-mono, .mk-step-chip, .mk-step-params { white-space: normal; overflow-wrap: anywhere; word-break: normal; } .mk-screenshot-placeholder { min-height: 0; padding: 10px; }\n");
    }
    if pdf {
        style.push_str("@page { size: A4; margin: 15mm; } @media print { body { padding: 0 !important; } .pl-rail { display: none !important; } }\n");
    }
    let theme_script = if pdf {
        "document.documentElement.dataset.theme='light'; document.addEventListener('DOMContentLoaded',()=>document.querySelectorAll('details').forEach(d => d.open = true));"
    } else {
        "const m=matchMedia('(prefers-color-scheme: dark)'); const theme=()=>document.documentElement.dataset.theme=m.matches?'dark':'light'; theme(); m.addEventListener('change',theme);"
    };
    let extras = format!("<style>{style}{QUESTION_CSS}</style><script>{theme_script}</script>");
    let runtime = format!("<script>{}</script>", fragment::RUNTIME_SCRIPT);
    if kind == WebKind::Page {
        let page = inject_head_markup(source, &extras);
        if let Some(pos) = page.to_ascii_lowercase().rfind("</body>") {
            format!("{}{runtime}{}", &page[..pos], &page[pos..])
        } else {
            format!("{page}{runtime}")
        }
    } else {
        let body = if kind == WebKind::Mermaid {
            mermaid_fragment(source, false)
        } else { source.to_owned() };
        format!("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{}</title>{extras}</head><body>{body}{runtime}</body></html>", escape_text(title))
    }
}

/// The viewport a compare render uses for each pane (and the offscreen
/// renders behind "Highlight changes" and onion skin), so both revisions lay
/// out at the same width and region boxes line up with the panes.
pub(crate) const COMPARE_VIEWPORT_WIDTH: u32 = 520;
/// Tall enough to capture most plans in one snapshot.
pub(crate) const COMPARE_VIEWPORT_HEIGHT: u32 = 1400;

/// The most CSS pixels tall a compare render grows to. Taller documents are
/// compared up to this height and the view says so
/// ([`compare_coverage`]). 16 000 keeps the offscreen view within common
/// GPU texture limits and, at 1x, within [`COMPARE_MAX_PIXELS`].
pub(crate) const COMPARE_MAX_HEIGHT: u32 = 16_000;
/// The most pixels in one compare snapshot (RGBA8: 4 bytes each, and the
/// diff keeps a few masks of the same size), so 20 M is 80 MB per image.
pub(crate) const COMPARE_MAX_PIXELS: u64 = 20_000_000;

/// How a compare render is taken for a document `content_height` CSS pixels
/// tall: the view height (CSS px, between the base viewport and
/// [`COMPARE_MAX_HEIGHT`]) and the snapshot width in pixels, `None` for the
/// native scale (`scale` is 2 on Retina). The snapshot is downscaled, never
/// below 1x, when height x scale would exceed [`COMPARE_MAX_PIXELS`].
pub(crate) fn compare_render_plan(content_height: u32, scale: f32) -> (u32, Option<u32>) {
    let scale = if scale.is_finite() && scale >= 1.0 {
        scale
    } else {
        1.0
    };
    let height = content_height.clamp(COMPARE_VIEWPORT_HEIGHT, COMPARE_MAX_HEIGHT);
    let area = f64::from(COMPARE_VIEWPORT_WIDTH) * f64::from(height);
    let affordable = (COMPARE_MAX_PIXELS as f64 / area).sqrt() as f32;
    let effective = affordable.clamp(1.0, scale);
    let width = (effective < scale).then(|| (COMPARE_VIEWPORT_WIDTH as f32 * effective) as u32);
    (height, width)
}

/// `Some((compared, total))` CSS pixels when a document `content_height`
/// tall is cut off at [`COMPARE_MAX_HEIGHT`], `None` when the render covers
/// all of it.
pub(crate) fn compare_coverage(content_height: u32) -> Option<(u32, u32)> {
    (content_height > COMPARE_MAX_HEIGHT).then_some((COMPARE_MAX_HEIGHT, content_height))
}

/// "Compared the first 16,000 px of 42,000" (digit groups of three).
pub(crate) fn coverage_notice((compared, total): (u32, u32)) -> String {
    fn grouped(n: u32) -> String {
        let digits = n.to_string();
        let mut out = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                out.push(',');
            }
            out.push(c);
        }
        out
    }
    format!(
        "Compared the first {} px of {}",
        grouped(compared),
        grouped(total)
    )
}

/// One pane's sandboxed frame document for the compare view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompareFrame {
    pub(crate) html: String,
    /// The frame's width in the host (what the tab's wrapper gives the
    /// frame at [`COMPARE_VIEWPORT_WIDTH`]).
    pub(crate) width: u32,
    /// Where the frame's top-left sits inside an offscreen render of the same
    /// document at the compare viewport (the host's padding and centering);
    /// region boxes are translated by it.
    pub(crate) origin: (u32, u32),
}

/// The frame document of `kind` + `source` exactly as [`web_document`] puts
/// it in its sandbox frame, plus `script` (scroll sync) and `overlay`
/// (positioned change boxes) inside the frame.
#[allow(clippy::too_many_arguments)]
pub(crate) fn compare_frame(
    kind: WebKind,
    source: &str,
    title: &str,
    mime_type: &str,
    theme: &FragmentTheme,
    reduced_motion: bool,
    script: &str,
    overlay: &str,
) -> CompareFrame {
    let scripts = format!("{LINK_FRAME_SCRIPT}{QUESTION_FRAME_SCRIPT}{script}");
    let body = |fragment: &str| format!("<script>{scripts}</script>\n{fragment}\n{overlay}");
    let (html, wide) = match kind {
        WebKind::Fragment | WebKind::DecisionSheet | WebKind::Plan => (
            fragment::frame_document(
                &body(&with_question_style(source)),
                title,
                theme,
                reduced_motion,
            ),
            matches!(kind, WebKind::DecisionSheet | WebKind::Plan),
        ),
        WebKind::Mermaid => (
            fragment::frame_document(
                &body(&mermaid_fragment(source, theme.dark)),
                title,
                theme,
                reduced_motion,
            ),
            true,
        ),
        WebKind::Image => (
            fragment::frame_document(
                &body(&image_fragment(source, mime_type, title)),
                title,
                theme,
                reduced_motion,
            ),
            true,
        ),
        WebKind::Page => {
            let page = inject_head_script(&inject_head_markup(source, &question_style()), &scripts);
            let lower = page.to_ascii_lowercase();
            let page = match lower.rfind("</body>") {
                Some(at) => format!("{}{overlay}{}", &page[..at], &page[at..]),
                None => format!("{page}{overlay}"),
            };
            (page, false)
        }
    };
    let (width, origin) = if kind == WebKind::Page {
        (COMPARE_VIEWPORT_WIDTH, (0, 0))
    } else {
        let max = if wide {
            fragment::WIDE_WIDTH
        } else {
            fragment::NORMAL_WIDTH
        };
        (
            (COMPARE_VIEWPORT_WIDTH - 32).min(max),
            (
                16 + (COMPARE_VIEWPORT_WIDTH - 32).saturating_sub(max) / 2,
                16,
            ),
        )
    };
    CompareFrame {
        html,
        width,
        origin,
    }
}

/// In the frame: clicks on links (other than same-document `#id`) post the
/// link's raw `href` to the outer document instead of navigating. The frame
/// is `sandbox="allow-scripts"` (no popups, no top navigation) and its
/// `about:srcdoc` base cannot resolve relative links like `ui/foo.tsx:120`,
/// so the page's own `href` text is what the host needs.
pub(crate) const LINK_FRAME_SCRIPT: &str = r##"(() => {
  if (window.__mastLinksInstalled) return;
  window.__mastLinksInstalled = true;
  const reportEditFocus = () => {
    const el = document.activeElement;
    const editable = !!el && (el.isContentEditable || el.tagName === "TEXTAREA" ||
      (el.tagName === "INPUT" && !/^(button|checkbox|radio|submit|reset|range|color|file|image)$/i.test(el.type)));
    try { parent.postMessage({ mastEditFocus: editable }, "*"); } catch (_) {}
  };
  document.addEventListener("focusin", reportEditFocus, true);
  document.addEventListener("focusout", () => setTimeout(reportEditFocus, 0), true);
  const handle = (event) => {
    const node = event.target;
    const a = node && node.closest ? node.closest("a, area") : null;
    if (!a) return;
    const href = (a.getAttribute("href") || a.getAttribute("xlink:href") || "").trim();
    if (href.startsWith("#")) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.type === "click" && href) {
      try { parent.postMessage({ mastLink: href }, "*"); } catch (_) {}
    }
  };
  document.addEventListener("click", handle, true);
  document.addEventListener("auxclick", handle, true);
})();
"##;

/// In the outer document: turns the frame's link message into a navigation
/// the native delegate recognises (`mast-link:<encoded href>`, see
/// `browser::document_view`), and cancels.
pub(crate) const LINK_HOST_SCRIPT: &str = r##"window.addEventListener("message", (event) => {
  const frame = document.querySelector("iframe");
  if (!frame || event.source !== frame.contentWindow) return;
  if (event.data && typeof event.data.mastEditFocus === "boolean") {
    window.__mastEditFocus = event.data.mastEditFocus;
    return;
  }
  const href = event.data && event.data.mastLink;
  if (typeof href !== "string" || href.length > 4096) return;
  const a = document.createElement("a");
  a.href = "mast-link:" + encodeURIComponent(href);
  a.rel = "noreferrer";
  a.style.display = "none";
  document.body.appendChild(a);
  a.click();
  a.remove();
});"##;

fn with_link_host(document: &str) -> String {
    document.replacen(
        "</head>",
        &format!(
            "<script>{LINK_HOST_SCRIPT}{FIND_HOST_SCRIPT}{BRIDGE_HOST_SCRIPT}</script>\n</head>"
        ),
        1,
    )
}

fn with_probe_prefix(body: &str, probe: Option<&str>) -> String {
    match probe {
        Some(probe) => format!("<script>{probe}</script>\n{body}"),
        None => body.to_owned(),
    }
}

/// A Mermaid source rendered in the fragment wrapper. The source is text
/// content of a `<pre>`, never markup.
pub(crate) fn mermaid_fragment(source: &str, dark: bool) -> String {
    let theme = if dark { "dark" } else { "default" };
    format!(
        "<div class=\"mast-mermaid\" style=\"display:flex;justify-content:center\">\
<pre class=\"mermaid\">{source}</pre></div>\n\
<script src=\"{MERMAID_SRC}\"></script>\n\
<script>(() => {{\n\
  const fail = (message) => {{\n\
    console.error(\"Mermaid: \" + message);\n\
    document.querySelectorAll(\"pre.mermaid\").forEach((el) => el.setAttribute(\"data-mast-error\", message));\n\
  }};\n\
  if (!globalThis.mermaid) {{ fail(\"the Mermaid library could not load\"); return; }}\n\
  mermaid.initialize({{ startOnLoad: false, securityLevel: \"strict\", theme: \"{theme}\" }});\n\
  mermaid.run({{ querySelector: \"pre.mermaid\" }}).catch((error) => fail(error && error.message ? error.message : String(error)));\n\
}})();</script>",
        source = escape_text(source),
    )
}

fn image_fragment(base64: &str, mime_type: &str, title: &str) -> String {
    let mime = if mime_type.starts_with("image/") {
        mime_type
    } else {
        "image/png"
    };
    format!(
        "<img alt=\"{alt}\" src=\"data:{mime};base64,{data}\" style=\"display:block;max-width:100%;height:auto;margin:0 auto\">",
        alt = escape_attr(title),
        data = base64.trim(),
    )
}

/// A standalone page in the visualize sandbox: the outer document carries
/// the visualize host policy (a srcdoc frame inherits it), the page is the
/// frame's whole document.
pub(crate) fn page_host_document(
    page: &str,
    title: &str,
    theme: &FragmentTheme,
    probe: Option<&str>,
) -> String {
    let background = theme
        .vars
        .iter()
        .find(|(name, _)| *name == "--background")
        .map_or("transparent", |(_, value)| value.as_str());
    let page = match probe {
        Some(probe) => inject_head_script(page, probe),
        None => page.to_owned(),
    };
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
<meta name=\"referrer\" content=\"no-referrer\">\n\
<meta http-equiv=\"Content-Security-Policy\" content=\"{csp}\">\n\
<title>{title}</title>\n<style>:root{{color-scheme:{scheme};background:{background}}}\
html,body{{margin:0;height:100%}}\
iframe{{display:block;width:100%;height:100vh;border:0;background:transparent}}\
</style>\n</head>\n<body>\n<iframe sandbox=\"allow-scripts\" referrerpolicy=\"no-referrer\" \
title=\"{title_attr}\" srcdoc=\"{srcdoc}\"></iframe>\n</body>\n</html>\n",
        csp = escape_attr(&fragment::host_csp()),
        title = escape_text(title),
        title_attr = escape_attr(title),
        scheme = if theme.dark { "dark" } else { "light" },
        srcdoc = escape_attr(&page),
    )
}

/// `script` as the first thing a page runs: after `<head …>` when there is
/// one, else after `<html …>`, else at the very start.
pub(crate) fn inject_head_script(page: &str, script: &str) -> String {
    inject_head_markup(page, &format!("<script>{script}</script>"))
}

/// The stylesheet for `data-mast-question` cards, as a `<style>` element.
fn question_style() -> String {
    format!("<style>{QUESTION_CSS}</style>")
}

/// A fragment with the question card styles in front of it.
fn with_question_style(fragment: &str) -> String {
    format!("{}\n{fragment}", question_style())
}

/// `tag` (markup) as the first thing in a page: after `<head …>` when there
/// is one, else after `<html …>`, else at the very start.
fn inject_head_markup(page: &str, tag: &str) -> String {
    let lower = page.to_ascii_lowercase();
    for open in ["<head", "<html"] {
        if let Some(start) = lower.find(open)
            && let Some(end) = lower[start..].find('>')
        {
            let at = start + end + 1;
            return format!("{}{tag}{}", &page[..at], &page[at..]);
        }
    }
    format!("{tag}{page}")
}

/// The outer document's collector: keeps the frame's report where the
/// native side can read it (`window.__mastProbe`).
pub(crate) const COLLECTOR_SCRIPT: &str = "window.__mastProbe = null;\
window.addEventListener(\"message\", (event) => {\
 if (event.data && event.data.mastRenderProbe) window.__mastProbe = event.data.mastRenderProbe;\
});";

fn with_collector(document: &str) -> String {
    document.replacen(
        "</head>",
        &format!("<script>{COLLECTOR_SCRIPT}</script>\n</head>"),
        1,
    )
}

/// What the native side evaluates to read the report (`null` until ready).
pub(crate) const READ_PROBE_SCRIPT: &str =
    "window.__mastProbe ? JSON.stringify(window.__mastProbe) : null";

/// The in-frame probe: console errors, uncaught errors and rejections, CSP
/// violations, failed resource loads, Mermaid errors, empty render, and
/// layout warnings (horizontal overflow, elements outside the viewport, low
/// contrast text), plus per-element metrics the native side classifies into
/// the layout heuristics ([`super::layout_heuristics`]). Posts
/// `{mastRenderProbe: {errors, warnings, metrics}}` to the parent once the
/// page has loaded and settled.
pub(crate) fn probe_script(theme: &FragmentTheme) -> String {
    let background = theme
        .vars
        .iter()
        .find(|(name, _)| *name == "--background")
        .map(|(_, value)| value.as_str())
        .unwrap_or(if theme.dark {
            "rgb(0 0 0)"
        } else {
            "rgb(255 255 255)"
        });
    PROBE_TEMPLATE
        .replace(
            "__THEME_BACKGROUND__",
            &serde_json::to_string(background).expect("theme background"),
        )
        .replace("__SETTLE_MS__", &SETTLE_MS.to_string())
        .replace("__MERMAID_WAIT_MS__", &MERMAID_WAIT_MS.to_string())
}

const PROBE_TEMPLATE: &str = r##"(() => {
  if (window.__mastProbeInstalled) return;
  window.__mastProbeInstalled = true;
  const errors = [], warnings = [];
  const push = (list, text) => {
    text = String(text).slice(0, 400);
    if (list.length < 40 && !list.includes(text)) list.push(text);
  };
  const show = (value) => {
    if (value instanceof Error) return value.message;
    if (typeof value === "string") return value;
    try { return JSON.stringify(value); } catch (_) { return String(value); }
  };
  const original = console.error.bind(console);
  console.error = (...args) => { push(errors, "Console error: " + args.map(show).join(" ")); original(...args); };
  window.addEventListener("error", (event) => {
    const target = event.target;
    if (target && target !== window && (target.src || target.href)) {
      push(errors, "Failed to load " + String(target.tagName || "resource").toLowerCase() + ": " + (target.src || target.href));
      return;
    }
    push(errors, "JavaScript error: " + (event.message || "unknown error") + (event.lineno ? " (line " + event.lineno + ")" : ""));
  }, true);
  window.addEventListener("unhandledrejection", (event) => {
    const reason = event.reason;
    push(errors, "Unhandled promise rejection: " + (reason && reason.message ? reason.message : show(reason)));
  });
  document.addEventListener("securitypolicyviolation", (event) => {
    push(errors, "Blocked by the content security policy (" + event.violatedDirective + "): " + (event.blockedURI || "inline"));
  });
  const describe = (el) => {
    let text = el.tagName.toLowerCase();
    if (el.id) text += "#" + el.id;
    else if (el.classList && el.classList.length) text += "." + Array.from(el.classList).slice(0, 2).join(".");
    return text;
  };
  const rgb = (value) => {
    const m = String(value).match(/rgba?\(([^)]+)\)/);
    if (!m) return null;
    const parts = m[1].split(/[ ,\/]+/).filter(Boolean).map(Number);
    return { r: parts[0], g: parts[1], b: parts[2], a: parts.length > 3 ? parts[3] : 1 };
  };
  const luminance = (c) => {
    const ch = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4); };
    return 0.2126 * ch(c.r) + 0.7152 * ch(c.g) + 0.0722 * ch(c.b);
  };
  const background = (el) => {
    for (let node = el; node; node = node.parentElement) {
      const c = rgb(getComputedStyle(node).backgroundColor);
      if (c && c.a > 0.5) return c;
    }
    return rgb(__THEME_BACKGROUND__) || { r: 255, g: 255, b: 255, a: 1 };
  };
  // Keep both the serialized report and the native sibling comparisons bounded.
  const maxMetrics = 1000;
  const pixelWidth = (style) => {
    const values = [style.width, style.minWidth].map((value) => {
      const match = String(value || "").trim().match(/^(\d+(?:\.\d+)?)px$/i);
      return match ? Math.round(Number(match[1])) : 0;
    });
    return Math.max(...values);
  };
  const widthRules = () => {
    const found = [];
    const visit = (rules) => {
      for (const rule of rules) {
        if (rule.selectorText && rule.style) {
          const width = pixelWidth(rule.style);
          if (width) found.push([rule.selectorText, width]);
        } else if (rule.cssRules && (!rule.conditionText ||
          (rule.media && matchMedia(rule.conditionText).matches) ||
          (!rule.media && window.CSS && CSS.supports(rule.conditionText)))) {
          visit(rule.cssRules);
        }
      }
    };
    for (const sheet of document.styleSheets) {
      try { if (!sheet.disabled && (!sheet.media || !sheet.media.mediaText || matchMedia(sheet.media.mediaText).matches)) visit(sheet.cssRules); } catch (_) {}
    }
    return found;
  };
  const report = () => {
    const metrics = [];
    let contentHeight = 0;
    try {
    document.querySelectorAll("pre.mermaid, div.mermaid").forEach((el) => {
      const failed = el.getAttribute("data-mast-error");
      if (failed) push(errors, "Mermaid: " + failed);
      else if (/syntax error|parse error/i.test(el.textContent || "")) push(errors, "Mermaid: " + (el.textContent || "").trim().slice(0, 200));
    });
    const body = document.body;
    const text = body ? (body.innerText || "").trim() : "";
    const media = body ? Array.from(body.querySelectorAll("img, svg, canvas, video, iframe, object")).some((el) => {
      const r = el.getBoundingClientRect();
      return r.width > 4 && r.height > 4;
    }) : false;
    if (!text && !media) push(errors, "Empty render: nothing visible was drawn");
    const root = document.scrollingElement || document.documentElement;
    if (root.scrollWidth > root.clientWidth + 1) {
      push(warnings, "Horizontal overflow: content is " + root.scrollWidth + "px wide in a " + root.clientWidth + "px viewport");
    }
    let outside = 0, lowContrast = 0;
    const examples = [], contrastExamples = [];
    const all = body ? body.querySelectorAll("*") : [];
    const indexes = new Map();
    const rules = widthRules();
    for (let i = 0; i < all.length && i < 4000; i++) {
      const el = all[i];
      const r = el.getBoundingClientRect();
      if (r.width === 0 || r.height === 0) continue;
      const style = getComputedStyle(el);
      if (style.visibility === "hidden" || style.visibility === "collapse" || Number(style.opacity) === 0) continue;
      if (metrics.length < maxMetrics) {
        const parentEl = el.parentElement;
        const parentStyle = parentEl ? getComputedStyle(parentEl) : null;
        const ownText = (el.innerText || "").replace(/\s+/g, " ").trim();
        const own = Array.from(el.childNodes).some((n) => n.nodeType === 3 && n.textContent.trim());
        const leaf = own && Array.from(el.children).every((child) => getComputedStyle(child).display.startsWith("inline"));
        const lineHeight = parseFloat(style.lineHeight) || (parseFloat(style.fontSize) || 16) * 1.2;
        let fixed = pixelWidth(el.style);
        for (const [selector, width] of rules) {
          try { if (el.matches(selector)) fixed = Math.max(fixed, width); } catch (_) {}
        }
        const cell = el.closest("td, th");
        const table = cell && cell.closest("table");
        const tableFits = !!(table && table.parentElement && table.scrollWidth <= table.parentElement.clientWidth + 1);
        const index = metrics.length;
        indexes.set(el, index);
        metrics.push({
          sel: (describe(el) + (el.id ? "" : ":nth-child(" + (Array.prototype.indexOf.call(parentEl ? parentEl.children : [], el) + 1) + ")")).slice(0, 200),
          text: ownText.slice(0, 200),
          x: Math.round(r.left + window.scrollX), y: Math.round(r.top + window.scrollY),
          w: Math.round(r.width), h: Math.round(r.height),
          sw: el.scrollWidth, sh: el.scrollHeight, cw: el.clientWidth, ch: el.clientHeight,
          clips: /^(hidden|clip)$/.test(style.overflowX) || /^(hidden|clip)$/.test(style.overflowY) || style.textOverflow === "ellipsis",
          pos: style.position,
          parent: parentEl === body ? -1 : (indexes.has(parentEl) ? indexes.get(parentEl) : -2),
          pcw: parentEl ? parentEl.clientWidth : 0,
          pscroll: !!parentStyle && /^(auto|scroll|overlay)$/.test(parentStyle.overflowX),
          tableFits, leaf, lines: leaf ? Math.max(1, Math.round(r.height / lineHeight)) : 0,
          words: leaf && ownText ? ownText.split(/\s+/).length : 0, fixed
        });
      }
      if (r.right > window.innerWidth + 1 || r.left < -1) {
        outside++;
        if (examples.length < 3) examples.push(describe(el));
      }
      const own = Array.from(el.childNodes).some((n) => n.nodeType === 3 && n.textContent.trim());
      if (own && lowContrast < 50) {
        const fg = rgb(style.color);
        if (fg && fg.a > 0.5 && style.visibility !== "hidden" && Number(style.opacity) > 0.3) {
          const bg = background(el);
          const a = luminance(fg), b = luminance(bg);
          const ratio = (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
          if (ratio < 3) { lowContrast++; if (contrastExamples.length < 3) contrastExamples.push(describe(el) + " " + ratio.toFixed(1) + ":1"); }
        }
      }
    }
    if (outside) push(warnings, outside + " element(s) extend outside the viewport horizontally (" + examples.join(", ") + ")");
    if (lowContrast) push(warnings, lowContrast + " text element(s) have contrast below 3:1 (" + contrastExamples.join(", ") + ")");
    contentHeight = Math.ceil(Math.max(
      document.documentElement ? document.documentElement.scrollHeight : 0,
      document.body ? document.body.scrollHeight : 0));
    } catch (error) {
      // Reserve space for this diagnostic even if the page filled the error list.
      if (errors.length >= 40) errors.pop();
      push(errors, "Render probe failed: " + (error && error.message ? error.message : String(error)));
    }
    try { parent.postMessage({ mastRenderProbe: { errors, warnings, metrics, contentHeight } }, "*"); } catch (_) {}
  };
  const waitMermaid = (deadline) => {
    let pending = false;
    try {
      pending = Array.from(document.querySelectorAll("pre.mermaid, div.mermaid")).some((el) =>
        !el.querySelector("svg") && !el.getAttribute("data-mast-error") && !el.getAttribute("data-processed"));
    } catch (error) {
      push(errors, "Render probe failed: " + (error && error.message ? error.message : String(error)));
    }
    if (pending && Date.now() < deadline) { setTimeout(() => waitMermaid(deadline), 100); return; }
    report();
  };
  const settle = () => setTimeout(() => waitMermaid(Date.now() + __MERMAID_WAIT_MS__), __SETTLE_MS__);
  if (document.readyState === "complete") settle();
  else window.addEventListener("load", settle, { once: true });
})();"##;

/// The probe's report as the native side reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub(crate) struct ProbeReport {
    #[serde(default)]
    pub(crate) errors: Vec<String>,
    #[serde(default)]
    pub(crate) warnings: Vec<String>,
    /// Per-element measurements for [`super::layout_heuristics::classify`].
    #[serde(default)]
    pub(crate) metrics: Vec<super::layout_heuristics::ElementMetrics>,
    /// The frame document's full scroll height in CSS pixels.
    #[serde(default, rename = "contentHeight")]
    pub(crate) content_height: Option<u32>,
}

#[cfg(test)]
mod probe_tests {
    use super::PROBE_TEMPLATE;

    #[test]
    fn probe_payload_names_are_declared() {
        let payload = PROBE_TEMPLATE
            .split("mastRenderProbe: {")
            .nth(1)
            .and_then(|s| s.split('}').next())
            .expect("probe payload");
        let declarations = PROBE_TEMPLATE.split("parent.postMessage").next().unwrap();
        for name in payload.split(',').map(str::trim) {
            assert!(
                declarations.contains(&format!("const {name} "))
                    || declarations.contains(&format!("let {name} "))
                    || (name == "warnings"
                        && declarations.contains("const errors = [], warnings = []")),
                "payload name {name} must be declared before posting"
            );
        }
        assert_eq!(payload.split(',').count(), 4);
    }

    #[test]
    fn probe_reports_metrics_and_report_errors_in_node_when_available() {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let mut node = match Command::new("node").stdin(Stdio::piped()).spawn() {
            Ok(node) => node,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => panic!("start node: {error}"),
        };
        let harness = r#"
const vm = require('vm');
const script = PROBE;
function run(fail) {
  let result;
  const style = { visibility:'visible', opacity:'1', display:'block', lineHeight:'20px', fontSize:'16px',
    color:'rgb(0, 0, 0)', backgroundColor:'rgba(0, 0, 0, 0)', overflowX:'visible', overflowY:'visible', position:'static' };
  const body = { innerText:'hi', scrollHeight:20, clientWidth:300, children:[], querySelectorAll: s => s === '*' ? [el] : [] };
  const el = { tagName:'P', id:'', classList:[], style:{}, parentElement:body, children:[], childNodes:[{nodeType:3,textContent:'hi'}],
    innerText:'hi', scrollWidth:100, scrollHeight:20, clientWidth:100, clientHeight:20,
    getBoundingClientRect:() => { if (fail) throw Error('fixture failure'); return {left:0,top:0,right:100,width:100,height:20}; }, matches:() => false, closest:() => null };
  body.children.push(el);
  const root = {scrollWidth:300, clientWidth:300, scrollHeight:20};
  const document = {readyState:'complete', body, documentElement:root, scrollingElement:root, styleSheets:[], addEventListener(){}, querySelectorAll:() => []};
  const window = {innerWidth:300, scrollX:0, scrollY:0, addEventListener(){}};
  const context = {window, document, parent:{postMessage: x => result = x.mastRenderProbe}, console,
    getComputedStyle:() => style, matchMedia:() => ({matches:true}), setTimeout: f => f(), Date};
  vm.runInNewContext(script, context);
  return result;
}
const ok = run(false);
if (!ok || ok.errors.length || ok.warnings.length || ok.metrics.length !== 1 || ok.metrics[0].text !== 'hi' || ok.metrics[0].parent !== -1) throw Error('normal report failed');
const broken = run(true);
if (!broken || !broken.errors.some(e => e.includes('Render probe failed: fixture failure'))) throw Error('failure report missing');
"#;
        let script = super::probe_script(&crate::browser::fragment::FragmentTheme {
            dark: false,
            vars: vec![("--background", "rgb(255 255 255)".into())],
            font_sans: String::new(),
            font_mono: String::new(),
        });
        let input = harness.replace("PROBE", &serde_json::to_string(&script).unwrap());
        node.stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        assert!(node.wait().unwrap().success());
    }
}

#[cfg(test)]
mod export_tests {
    use super::*;

    #[test]
    fn mermaid_uses_cdn_and_pdf_expands_details() {
        let html = export_document(WebKind::Mermaid, "graph TD; A-->B", "Diagram", false);
        assert!(html.contains(&format!("<script src=\"{MERMAID_SRC}\"></script>")));
        let pdf_mermaid = export_document(WebKind::Mermaid, "graph TD; A-->B", "Diagram", true);
        assert!(pdf_mermaid.contains(&format!("<script src=\"{MERMAID_SRC}\"></script>")));
        assert!(html.contains("graph TD; A--&gt;B"));
        let pdf = export_document(WebKind::Fragment, "<details><summary>More</summary>Text</details>", "PDF", true);
        assert!(pdf.contains("d.open = true"));
        assert!(pdf.contains("@page"));
    }

    #[test]
    fn standalone_documents_have_kit_without_mast_bridges() {
        let plan = mast_proto::plan::Plan::parse(include_str!("../../../../docs/artifact-kit/plan-example.json")).unwrap();
        let sheet = mast_proto::decision_sheet::DecisionSheet::parse(include_str!("../../../../docs/artifact-kit/decision-sheet-example.json")).unwrap();
        if std::env::var_os("MAST_WRITE_EXPORT_EXAMPLES").is_some() {
            let dir = std::path::Path::new("/private/tmp/artifact-export");
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join("plan.html"), export_document(WebKind::Plan, &mast_proto::plan_render::render(&plan), &plan.title, false)).unwrap();
            std::fs::write(dir.join("decision-sheet.html"), export_document(WebKind::DecisionSheet, &mast_proto::decision_sheet_render::render(&sheet), &sheet.title, false)).unwrap();
        }
        for (kind, body) in [
            (WebKind::Plan, mast_proto::plan_render::render(&plan)),
            (WebKind::DecisionSheet, mast_proto::decision_sheet_render::render(&sheet)),
            (WebKind::Fragment, "<p>Fragment</p>".to_owned()),
        ] {
            let html = export_document(kind, &body, "Example", false);
            assert!(html.contains("--mk-blue"));
            assert!(html.contains("prefers-color-scheme"));
            assert!(!html.contains("mastMsg"));
            assert!(!html.contains("mastRenderProbe"));
            assert!(!html.contains("srcdoc="));
        }
    }
}
