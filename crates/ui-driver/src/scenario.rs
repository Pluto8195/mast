//! YAML scenario runner (batch mode) for the headless UI driver.
//!
//! A scenario is a world plus a list of steps (`open`, `click`, `hover`,
//! `type`, `key`, `scroll`, `settle`, `reset`, `expect`). `Scenario::parse`
//! reads it; `launch` runs it from outside the sandbox by re-executing a test
//! binary inside `run_isolated`, where `serve_scenario` builds the world,
//! opens the app and plays every step in one process. Nothing here is reachable
//! from the installed binary: the crate is test-only.
//!
//! `expect` is polled: until it holds or `timeout_ms` (default 8000) passes the
//! runner settles and retries, so a step that waits on the Mock agent needs no
//! explicit wait. An ambiguous locator is an error at once, never retried.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use crate::environment::ScratchDirectories;
use crate::serve;
use crate::session::{Session, Timing};
use crate::template::Template;
use crate::tree::{Frame, LocateError};
use crate::world::SmallWorld;

pub const DEFAULT_EXPECT_TIMEOUT_MS: u64 = 8000;
const IDLE_TIMEOUT: Duration = Duration::from_secs(20);

/// Name of the ignored test (in the test binary passed to `launch`) that
/// calls `serve_scenario` inside the sandbox.
pub const SERVE_TEST: &str = "scenario_serve";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct World {
    #[serde(default = "default_preset")]
    pub preset: String,
    #[serde(default = "default_seed")]
    pub seed: u64,
    #[serde(default)]
    pub mock: Option<String>,
    #[serde(default)]
    pub shape: Option<PathBuf>,
}

fn default_preset() -> String {
    SmallWorld::PRESET.to_owned()
}
fn default_seed() -> u64 {
    1
}

fn validate_preset(preset: &str, shape_available: bool) -> Result<(), String> {
    if !["small", "medium", "scale"].contains(&preset) {
        return Err(format!(
            "unknown preset {preset:?}; use small, medium or scale"
        ));
    }
    if preset == "scale" && !shape_available {
        return Err("scale preset requires world.shape or MAST_UI_DRIVER_SHAPE".into());
    }
    Ok(())
}

impl Default for World {
    fn default() -> Self {
        Self {
            preset: default_preset(),
            seed: 1,
            mock: None,
            shape: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub enum Screenshot {
    #[default]
    #[serde(rename = "on-fail")]
    OnFail,
    #[serde(rename = "every-step")]
    EveryStep,
    #[serde(rename = "off")]
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settle {
    /// Draw until the tree is stable.
    Quiet,
    /// Also wait until the agent is idle (no Stop button).
    Idle,
}

/// `[min, max]` limits, in pixels, on one edge or size of a region.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundsCheck {
    pub top_edge_within: Option<[f32; 2]>,
    /// Distance from the region's bottom edge to the window's bottom.
    pub bottom_edge_within: Option<[f32; 2]>,
    pub left_edge_within: Option<[f32; 2]>,
    /// Distance from the region's right edge to the window's right.
    pub right_edge_within: Option<[f32; 2]>,
    pub width_within: Option<[f32; 2]>,
    pub height_within: Option<[f32; 2]>,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Perf {
    /// Draws of the preceding step that were not frame-callback (animated) draws.
    pub max_draws: Option<u64>,
    pub max_draw_ms: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    pub visible: Option<String>,
    pub not_visible: Option<String>,
    /// With `visible`: whether the element holds keyboard focus.
    pub focused: Option<bool>,
    /// With `visible`: the element's value (a text input's contents).
    pub value: Option<String>,
    pub text: Option<String>,
    pub not_text: Option<String>,
    /// Restricts `text` / `not_text` to the subtree of this locator.
    pub within: Option<String>,
    #[serde(default)]
    pub bounds: BTreeMap<String, BoundsCheck>,
    pub perf: Option<Perf>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Open { chat: String },
    Click(String),
    Hover(String),
    Type { into: String, text: String },
    Key(String),
    Scroll { at: String, dx: f32, dy: f32 },
    Settle(Settle),
    Reset,
    Expect(Expect),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub action: Action,
    /// The step as written, one line of JSON, for reports.
    pub source: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scenario {
    pub name: String,
    pub world: World,
    pub screenshot: Screenshot,
    pub steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    world: World,
    #[serde(default)]
    screenshot: Screenshot,
    steps: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TypeArgs {
    into: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenArgs {
    chat: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScrollArgs {
    at: String,
    #[serde(default)]
    dx: f32,
    #[serde(default)]
    dy: f32,
}

const STEP_NAMES: &str = "open, click, hover, type, key, scroll, settle, reset, expect";

fn string_arg(value: &Value, step: &str) -> Result<String, String> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("`{step}` takes a string"))
}

fn parse_step(value: &Value) -> Result<Action, String> {
    // `reset` and `settle` may be written bare: `- reset`.
    if let Some(name) = value.as_str() {
        return match name {
            "reset" => Ok(Action::Reset),
            "settle" => Ok(Action::Settle(Settle::Quiet)),
            other => Err(format!("unknown step {other:?}; known steps: {STEP_NAMES}")),
        };
    }
    let map = value
        .as_mapping()
        .ok_or("a step is a mapping like `click: \"#id\"`")?;
    if map.len() != 1 {
        return Err(format!("a step has exactly one key, found {}", map.len()));
    }
    let (key, arg) = map.iter().next().expect("one entry");
    let key = key.as_str().ok_or("a step name must be a string")?;
    let args = |e: serde_yaml::Error| format!("`{key}`: {e}");
    match key {
        "open" => {
            let a: OpenArgs = serde_yaml::from_value(arg.clone()).map_err(args)?;
            if a.chat.contains('"') {
                return Err("`open`: chat title must not contain a double quote".into());
            }
            Ok(Action::Open { chat: a.chat })
        }
        "click" => Ok(Action::Click(string_arg(arg, key)?)),
        "hover" => Ok(Action::Hover(string_arg(arg, key)?)),
        "key" => Ok(Action::Key(string_arg(arg, key)?)),
        "type" => {
            let a: TypeArgs = serde_yaml::from_value(arg.clone()).map_err(args)?;
            Ok(Action::Type {
                into: a.into,
                text: a.text,
            })
        }
        "scroll" => {
            let a: ScrollArgs = serde_yaml::from_value(arg.clone()).map_err(args)?;
            if a.dx == 0.0 && a.dy == 0.0 {
                return Err("`scroll`: dx and dy are both zero".into());
            }
            Ok(Action::Scroll {
                at: a.at,
                dx: a.dx,
                dy: a.dy,
            })
        }
        "settle" => match arg.as_str() {
            Some("idle") => Ok(Action::Settle(Settle::Idle)),
            Some("quiet") => Ok(Action::Settle(Settle::Quiet)),
            None if arg.is_null() => Ok(Action::Settle(Settle::Quiet)),
            _ => Err("`settle` takes `idle` or `quiet`".into()),
        },
        "reset" if arg.is_null() => Ok(Action::Reset),
        "reset" => Err("`reset` takes no arguments".into()),
        "expect" => {
            let e: Expect = serde_yaml::from_value(arg.clone()).map_err(args)?;
            validate_expect(&e)?;
            Ok(Action::Expect(e))
        }
        other => Err(format!("unknown step {other:?}; known steps: {STEP_NAMES}")),
    }
}

fn validate_expect(e: &Expect) -> Result<(), String> {
    let checks = [
        e.visible.is_some(),
        e.not_visible.is_some(),
        e.text.is_some(),
        e.not_text.is_some(),
        !e.bounds.is_empty(),
        e.perf.is_some(),
    ];
    if !checks.contains(&true) {
        return Err(
            "`expect` has no checks (visible, not_visible, text, not_text, bounds, perf)".into(),
        );
    }
    if (e.focused.is_some() || e.value.is_some()) && e.visible.is_none() {
        return Err("`expect`: `focused` and `value` need `visible`".into());
    }
    if e.within.is_some() && e.text.is_none() && e.not_text.is_none() {
        return Err("`expect`: `within` needs `text` or `not_text`".into());
    }
    if let Some(perf) = &e.perf
        && perf.max_draws.is_none()
        && perf.max_draw_ms.is_none()
    {
        return Err("`expect`: `perf` needs max_draws or max_draw_ms".into());
    }
    Ok(())
}

impl Scenario {
    /// Parse a scenario. `fallback_name` names it when the YAML has no `name`.
    pub fn parse(yaml: &str, fallback_name: &str) -> Result<Self, String> {
        let raw: Raw = serde_yaml::from_str(yaml).map_err(|e| format!("invalid scenario: {e}"))?;
        validate_preset(
            &raw.world.preset,
            raw.world.shape.is_some() || std::env::var_os("MAST_UI_DRIVER_SHAPE").is_some(),
        )?;
        if let Some(name) = &raw.world.mock {
            serve::Config::from_json(&serde_json::json!({"mock": name}))?;
        }
        if raw.steps.is_empty() {
            return Err("invalid scenario: `steps` is empty".into());
        }
        let mut steps = Vec::new();
        for (index, value) in raw.steps.iter().enumerate() {
            let action = parse_step(value).map_err(|e| format!("step {}: {e}", index + 1))?;
            let source = serde_json::to_string(value).unwrap_or_default();
            steps.push(Step { action, source });
        }
        Ok(Self {
            name: raw.name.unwrap_or_else(|| fallback_name.to_owned()),
            world: raw.world,
            screenshot: raw.screenshot,
            steps,
        })
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self::parse(&text, &stem).map_err(|e| format!("{}: {e}", path.display()))
    }
}

// ---- reports ---------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StepReport {
    pub index: usize,
    pub source: String,
    pub ok: bool,
    pub draws: u32,
    pub animated: u32,
    pub draw_ms: f64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Report {
    pub name: String,
    pub ok: bool,
    pub steps: Vec<StepReport>,
    /// Files left in the output directory (tree, PNGs).
    pub artifacts: Vec<String>,
    pub notes: Vec<String>,
}

impl Report {
    pub fn failure(&self) -> Option<&StepReport> {
        self.steps.iter().find(|s| !s.ok)
    }

    pub fn summary(&self) -> String {
        let mut out = String::new();
        for s in &self.steps {
            out.push_str(&format!(
                "  {} step {}: {}  [draws={} animated={} draw_ms={:.1}]\n",
                if s.ok { "ok  " } else { "FAIL" },
                s.index,
                s.source,
                s.draws,
                s.animated,
                s.draw_ms
            ));
            if let Some(error) = &s.error {
                for line in error.lines() {
                    out.push_str(&format!("        {line}\n"));
                }
            }
        }
        for note in &self.notes {
            out.push_str(&format!("  note: {note}\n"));
        }
        for artifact in &self.artifacts {
            out.push_str(&format!("  artifact: {artifact}\n"));
        }
        out.push_str(&format!(
            "{} {}\n",
            if self.ok { "PASS" } else { "FAIL" },
            self.name
        ));
        out
    }
}

// ---- running ---------------------------------------------------------------

enum CheckError {
    /// Not true yet; retried until the timeout.
    Unmet(String),
    /// Wrong however long we wait (ambiguous or malformed locator).
    Fatal(String),
}

fn locate(frame: &Frame, locator: &str) -> Result<Option<usize>, CheckError> {
    match frame.resolve(locator) {
        Ok(m) => Ok(Some(m.node.unwrap_or(usize::MAX))),
        Err(LocateError::NotFound(_)) => Ok(None),
        Err(e) => Err(CheckError::Fatal(e.to_string())),
    }
}

fn subtree_text(frame: &Frame, root: usize, out: &mut String) {
    let node = &frame.nodes[root];
    for part in [&node.label, &node.value].into_iter().flatten() {
        out.push_str(part);
        out.push('\n');
    }
    for &child in &node.children {
        subtree_text(frame, child, out);
    }
}

fn in_range(label: &str, actual: f32, [lo, hi]: [f32; 2]) -> Result<(), CheckError> {
    if (lo..=hi).contains(&actual) {
        Ok(())
    } else {
        Err(CheckError::Unmet(format!(
            "{label} is {actual}, wanted {lo}..={hi}"
        )))
    }
}

fn check_frame(frame: &Frame, e: &Expect) -> Result<(), CheckError> {
    if let Some(locator) = &e.visible {
        let Some(index) = locate(frame, locator)? else {
            return Err(CheckError::Unmet(format!("{locator} is not visible")));
        };
        if let Some(want) = e.focused {
            let focused = frame.nodes.get(index).is_some_and(|n| n.focused);
            if focused != want {
                return Err(CheckError::Unmet(format!(
                    "{locator} focused={focused}, wanted {want}"
                )));
            }
        }
        if let Some(want) = &e.value {
            let value = frame
                .nodes
                .get(index)
                .and_then(|n| n.value.clone())
                .unwrap_or_default();
            if &value != want {
                return Err(CheckError::Unmet(format!(
                    "{locator} value is {value:?}, wanted {want:?}"
                )));
            }
        }
    }
    if let Some(locator) = &e.not_visible
        && locate(frame, locator)?.is_some()
    {
        return Err(CheckError::Unmet(format!("{locator} is visible")));
    }
    if e.text.is_some() || e.not_text.is_some() {
        let mut haystack = String::new();
        match &e.within {
            Some(locator) => match locate(frame, locator)? {
                Some(index) if index != usize::MAX => subtree_text(frame, index, &mut haystack),
                Some(_) => {
                    return Err(CheckError::Fatal(format!(
                        "{locator} has no a11y node to read text from"
                    )));
                }
                None => {
                    return Err(CheckError::Unmet(format!(
                        "{locator} (within) is not visible"
                    )));
                }
            },
            None => haystack = frame.all_text(),
        }
        let scope = e
            .within
            .as_deref()
            .map(|w| format!(" within {w}"))
            .unwrap_or_default();
        if let Some(text) = &e.text
            && !haystack.contains(text.as_str())
        {
            return Err(CheckError::Unmet(format!("text {text:?} not found{scope}")));
        }
        if let Some(text) = &e.not_text
            && haystack.contains(text.as_str())
        {
            return Err(CheckError::Unmet(format!(
                "text {text:?} is present{scope}"
            )));
        }
    }
    for (selector, check) in &e.bounds {
        let locator = if selector.starts_with('#') {
            selector.clone()
        } else {
            format!("#{selector}")
        };
        let rect = match frame.resolve_with_bounds(&locator) {
            Ok((_, rect)) => rect,
            Err(LocateError::NotFound(_)) => {
                return Err(CheckError::Unmet(format!("{locator} is not visible")));
            }
            Err(e) => return Err(CheckError::Fatal(e.to_string())),
        };
        let [x, y, w, h] = rect;
        let (vw, vh) = frame.viewport;
        let at = |what: &str| format!("{locator} {what}");
        if let Some(r) = check.top_edge_within {
            in_range(&at("top edge"), y, r)?;
        }
        if let Some(r) = check.bottom_edge_within {
            in_range(&at("bottom edge gap"), vh - (y + h), r)?;
        }
        if let Some(r) = check.left_edge_within {
            in_range(&at("left edge"), x, r)?;
        }
        if let Some(r) = check.right_edge_within {
            in_range(&at("right edge gap"), vw - (x + w), r)?;
        }
        if let Some(r) = check.width_within {
            in_range(&at("width"), w, r)?;
        }
        if let Some(r) = check.height_within {
            in_range(&at("height"), h, r)?;
        }
    }
    Ok(())
}

fn check_perf(perf: &Perf, timing: &Timing) -> Result<(), String> {
    let drawn = u64::from(timing.draws.saturating_sub(timing.animated));
    if let Some(max) = perf.max_draws
        && drawn > max
    {
        return Err(format!(
            "perf: {drawn} draws ({} animated excluded), max {max}",
            timing.animated
        ));
    }
    if let Some(max) = perf.max_draw_ms
        && timing.draw_ms > max
    {
        return Err(format!(
            "perf: {:.1}ms of draw time, max {max}",
            timing.draw_ms
        ));
    }
    Ok(())
}

fn add(total: &mut Timing, t: &Timing) {
    total.draws += t.draws;
    total.animated += t.animated;
    total.draw_ms += t.draw_ms;
    total.wall_ms += t.wall_ms;
    total.nodes = t.nodes;
}

fn agent_idle(frame: &Frame) -> bool {
    let has = |name: &str| {
        frame
            .nodes
            .iter()
            .any(|n| n.selector.as_deref() == Some(name))
            || frame.loose_bounds.iter().any(|(k, _)| k == name)
    };
    !has("composer-stop")
}

struct Runner<'a> {
    scratch: &'a ScratchDirectories,
    template: Template,
    world: SmallWorld,
    runtime: std::sync::Arc<tokio::runtime::Runtime>,
    render: bool,
    session: Option<Session>,
    out: PathBuf,
    report: Report,
}

impl Runner<'_> {
    fn session(&mut self) -> &mut Session {
        self.session.as_mut().expect("session is open")
    }

    fn save_tree(&mut self, name: &str) {
        let frame = self.session().frame();
        let text = format!(
            "{}\n# regions without an a11y node\n{}",
            frame.render(),
            frame.render_selectors()
        );
        let path = self.out.join(name);
        if std::fs::write(&path, text).is_ok() {
            self.report
                .artifacts
                .push(path.to_string_lossy().into_owned());
        }
    }

    fn save_png(&mut self, name: &str) {
        if !self.render {
            let note = "png skipped: the renderer is off (pass --render)".to_owned();
            if !self.report.notes.contains(&note) {
                self.report.notes.push(note);
            }
            return;
        }
        let path = self.out.join(name);
        match self.session().png(&path) {
            Ok(()) => self
                .report
                .artifacts
                .push(path.to_string_lossy().into_owned()),
            Err(e) => self.report.notes.push(format!("png failed: {e}")),
        }
    }

    fn expect(&mut self, e: &Expect, last: &Timing) -> Result<(), String> {
        let timeout = Duration::from_millis(e.timeout_ms.unwrap_or(DEFAULT_EXPECT_TIMEOUT_MS));
        let started = Instant::now();
        loop {
            match check_frame(self.session().frame(), e) {
                Ok(()) => break,
                Err(CheckError::Fatal(m)) => return Err(m),
                Err(CheckError::Unmet(m)) => {
                    if started.elapsed() >= timeout {
                        return Err(format!("{m} (after {}ms)", timeout.as_millis()));
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(100));
            self.session().settle();
        }
        match &e.perf {
            Some(perf) => check_perf(perf, last),
            None => Ok(()),
        }
    }

    fn act(&mut self, action: &Action, last: &mut Timing) -> Result<(), String> {
        let timing = match action {
            Action::Open { chat } => self.session().click(&format!("button[name=\"{chat}\"]"))?,
            Action::Click(locator) => self.session().click(locator)?,
            Action::Hover(locator) => self.session().hover(locator)?,
            Action::Type { into, text } => self.session().type_text(into, text)?,
            Action::Key(chord) => self.session().key(chord)?,
            Action::Scroll { at, dx, dy } => self.session().scroll(at, *dx, *dy)?,
            Action::Settle(Settle::Quiet) => self.session().settle(),
            Action::Settle(Settle::Idle) => {
                let started = Instant::now();
                let mut total = Timing::default();
                loop {
                    let t = self.session().settle();
                    add(&mut total, &t);
                    if agent_idle(self.session().frame()) {
                        break;
                    }
                    if started.elapsed() > IDLE_TIMEOUT {
                        return Err(format!(
                            "agent still busy after {}s (composer-stop is showing)",
                            IDLE_TIMEOUT.as_secs()
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                total
            }
            Action::Reset => {
                self.session = None; // drop the app before its stores are replaced
                let session = serve::boot_session(
                    self.scratch,
                    &self.template,
                    &self.world,
                    &self.runtime,
                    self.render,
                )?;
                self.session = Some(session);
                self.session().settle()
            }
            Action::Expect(_) => unreachable!("handled by the caller"),
        };
        *last = timing;
        Ok(())
    }
}

/// Play `scenario` on the scratch root. Must run inside `run_isolated`.
/// Artifacts go to `out`. Setup failures come back as a failed report.
pub fn serve_scenario(
    scenario: &Scenario,
    scratch: &ScratchDirectories,
    out: &Path,
    render: bool,
) -> Report {
    let mut report = Report {
        name: scenario.name.clone(),
        ..Report::default()
    };
    let setup = (|| -> Result<Runner<'_>, String> {
        let config = serve::Config {
            preset: scenario.world.preset.clone(),
            seed: scenario.world.seed,
            render,
            mock: scenario.world.mock.clone(),
        };
        serve::apply_mock(&config)?;
        let runtime = serve::new_runtime()?;
        let world = match config.preset.as_str() {
            "small" => SmallWorld::small(config.seed),
            "medium" => {
                SmallWorld::shaped(config.seed, "medium", &crate::shape::WorldShape::medium())?
            }
            "scale" => {
                let path = scenario
                    .world
                    .shape
                    .clone()
                    .or_else(|| std::env::var_os("MAST_UI_DRIVER_SHAPE").map(PathBuf::from))
                    .ok_or("scale preset requires a shape file")?;
                let bytes =
                    std::fs::read(path).map_err(|e| format!("cannot read world shape: {e}"))?;
                let shape = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("invalid world shape: {e}"))?;
                SmallWorld::shaped(config.seed, "scale", &shape)?
            }
            _ => return Err("unknown world preset".into()),
        };
        let template = runtime.block_on(Template::build(scratch, &world))?;
        let session = serve::boot_session(scratch, &template, &world, &runtime, render)?;
        Ok(Runner {
            scratch,
            template,
            world,
            runtime,
            render,
            session: Some(session),
            out: out.to_path_buf(),
            report: Report {
                name: scenario.name.clone(),
                ..Report::default()
            },
        })
    })();
    let mut runner = match setup {
        Ok(r) => r,
        Err(e) => {
            report.steps.push(StepReport {
                index: 0,
                source: "setup".into(),
                error: Some(e),
                ..StepReport::default()
            });
            return report;
        }
    };
    let _ = std::fs::create_dir_all(out);
    let mut last = Timing::default();
    runner.session().settle();
    let mut failed = false;
    for (i, step) in scenario.steps.iter().enumerate() {
        let index = i + 1;
        let before = last.clone();
        let result = match &step.action {
            Action::Expect(e) => runner.expect(e, &last),
            action => runner.act(action, &mut last),
        };
        let (draws, animated, draw_ms) = if matches!(step.action, Action::Expect(_)) {
            (0, 0, 0.0)
        } else if last == before {
            (0, 0, 0.0)
        } else {
            (last.draws, last.animated, last.draw_ms)
        };
        runner.report.steps.push(StepReport {
            index,
            source: step.source.clone(),
            ok: result.is_ok(),
            draws,
            animated,
            draw_ms,
            error: result.as_ref().err().cloned(),
        });
        if scenario.screenshot == Screenshot::EveryStep {
            runner.save_png(&format!("step-{index:02}.png"));
        }
        if result.is_err() {
            failed = true;
            runner.save_tree("fail-tree.txt");
            if scenario.screenshot != Screenshot::Off {
                runner.save_png("fail.png");
            }
            break;
        }
    }
    runner.report.ok = !failed;
    let report = std::mem::take(&mut runner.report);
    drop(runner.session.take());
    report
}

// ---- outside the sandbox ---------------------------------------------------

/// Run one scenario file from outside the sandbox: allocate a scratch root,
/// re-execute `exe` (a test binary exposing `SERVE_TEST`) inside
/// `run_isolated`, then collect the report and artifacts into `out`.
pub fn launch(
    exe: &Path,
    account_home: &Path,
    scenario_path: &Path,
    out: &Path,
    render: bool,
) -> Result<Report, String> {
    let scenario = Scenario::load(scenario_path)?; // fail on bad YAML before any sandbox
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let scratch = ScratchDirectories::create(account_home).map_err(|e| e.to_string())?;
    let ctl = serve::control_dir(scratch.root());
    std::fs::create_dir_all(ctl.join("artifacts")).map_err(|e| e.to_string())?;
    let original = std::fs::read_to_string(scenario_path).map_err(|e| e.to_string())?;
    let mut yaml: serde_yaml::Value = serde_yaml::from_str(&original).map_err(|e| e.to_string())?;
    if scenario.world.preset == "scale" {
        let path = scenario
            .world
            .shape
            .clone()
            .or_else(|| std::env::var_os("MAST_UI_DRIVER_SHAPE").map(PathBuf::from))
            .ok_or("scale preset requires a shape file")?;
        let bytes = std::fs::read(path).map_err(|e| format!("cannot read world shape: {e}"))?;
        let _: crate::shape::WorldShape =
            serde_json::from_slice(&bytes).map_err(|e| format!("invalid world shape: {e}"))?;
        let shape_path = ctl.join("shape.json");
        std::fs::write(&shape_path, bytes).map_err(|e| e.to_string())?;
        yaml["world"]["shape"] =
            serde_yaml::Value::String(shape_path.to_string_lossy().into_owned());
    }
    std::fs::write(
        ctl.join("scenario.yaml"),
        serde_yaml::to_string(&yaml).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(ctl.join("name"), &scenario.name).map_err(|e| e.to_string())?;
    std::fs::write(ctl.join("render"), if render { "1" } else { "0" })
        .map_err(|e| e.to_string())?;
    let args = [
        SERVE_TEST,
        "--ignored",
        "--exact",
        "--test-threads=1",
        "--nocapture",
    ];
    let args: Vec<&std::ffi::OsStr> = args.iter().map(std::ffi::OsStr::new).collect();
    let output = scratch.run_isolated(account_home, exe, &args);
    let result = (|| -> Result<Report, String> {
        let output = output.map_err(|e| format!("could not start the sandbox: {e}"))?;
        let text = std::fs::read_to_string(ctl.join("report.json")).map_err(|_| {
            format!(
                "no report written ({}):\n{}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })?;
        let mut report: Report = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        // Move artifacts out of the scratch root, which is about to go.
        let mut moved = Vec::new();
        for artifact in &report.artifacts {
            let name = Path::new(artifact).file_name().ok_or("bad artifact path")?;
            let from = ctl.join("artifacts").join(name);
            let to = out.join(name);
            if std::fs::copy(&from, &to).is_ok() {
                moved.push(to.to_string_lossy().into_owned());
            }
        }
        report.artifacts = moved;
        Ok(report)
    })();
    // The scratch root is ours: created above by ScratchDirectories::create.
    let _ = std::fs::remove_dir_all(scratch.root());
    result
}

/// The inside half, for the `SERVE_TEST` test: read what `launch` wrote,
/// play the scenario and write `report.json` into the control directory.
pub fn serve_from_control() -> Result<(), String> {
    let scratch = ScratchDirectories::from_env().map_err(|e| e.to_string())?;
    let ctl = serve::control_dir(scratch.root());
    let read =
        |name: &str| std::fs::read_to_string(ctl.join(name)).map_err(|e| format!("{name}: {e}"));
    let name = read("name")?;
    let scenario = Scenario::parse(&read("scenario.yaml")?, &name)?;
    let render = read("render")?.trim() == "1";
    let artifacts = ctl.join("artifacts");
    let report = serve_scenario(&scenario, &scratch, &artifacts, render);
    std::fs::write(
        ctl.join("report.json"),
        serde_json::to_string(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

/// Scenario files in `dir`, sorted.
pub fn scenario_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "yaml" || x == "yml"))
        .collect();
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_requires_a_shape() {
        assert!(
            validate_preset("scale", false)
                .unwrap_err()
                .contains("requires")
        );
        assert!(validate_preset("scale", true).is_ok());
    }

    const GOOD: &str = r##"
world: { preset: small, seed: 7, mock: approval }
screenshot: on-fail
steps:
  - open: { chat: "Review the workspace" }
  - expect: { visible: "#composer-input", focused: true }
  - type: { into: "#composer-input", text: "run the tests" }
  - key: "enter"
  - key: "cmd-k"
  - hover: "#composer-input"
  - scroll: { at: "#transcript-region", dy: -300 }
  - settle: idle
  - reset
  - expect:
      text: "Locked in"
      within: "#transcript-region"
      bounds: { "#composer-surface": { bottom_edge_within: [0, 60] } }
      perf: { max_draws: 24, max_draw_ms: 400 }
"##;

    fn err(yaml: &str) -> String {
        Scenario::parse(yaml, "t").unwrap_err()
    }

    #[test]
    fn parses_every_step_kind() {
        let s = Scenario::parse(GOOD, "t").unwrap();
        assert_eq!(
            s.world,
            World {
                preset: "small".into(),
                seed: 7,
                mock: Some("approval".into()),
                shape: None
            }
        );
        assert_eq!(s.screenshot, Screenshot::OnFail);
        assert_eq!(s.steps.len(), 10);
        assert_eq!(
            s.steps[0].action,
            Action::Open {
                chat: "Review the workspace".into()
            }
        );
        assert_eq!(s.steps[4].action, Action::Key("cmd-k".into()));
        assert_eq!(
            s.steps[6].action,
            Action::Scroll {
                at: "#transcript-region".into(),
                dx: 0.0,
                dy: -300.0
            }
        );
        assert_eq!(s.steps[7].action, Action::Settle(Settle::Idle));
        assert_eq!(s.steps[8].action, Action::Reset);
        let Action::Expect(e) = &s.steps[9].action else {
            panic!("expect")
        };
        assert_eq!(
            e.bounds["#composer-surface"].bottom_edge_within,
            Some([0.0, 60.0])
        );
        assert_eq!(e.perf.as_ref().unwrap().max_draws, Some(24));
        assert_eq!(s.name, "t");
    }

    #[test]
    fn defaults_apply() {
        let s = Scenario::parse("steps:\n  - settle\n", "x").unwrap();
        assert_eq!(s.world, World::default());
        assert_eq!(s.screenshot, Screenshot::OnFail);
    }

    #[test]
    fn bad_yaml_is_an_error() {
        assert!(err("steps: [unclosed").starts_with("invalid scenario:"));
        assert!(err("just a string").starts_with("invalid scenario:"));
        assert!(err("steps: {a: 1}").starts_with("invalid scenario:"));
        assert!(err("steps: []").contains("`steps` is empty"));
        assert!(err("world: {seed: 1}").contains("steps"));
    }

    #[test]
    fn unknown_keys_and_values_are_errors() {
        assert!(err("stepz: []\nsteps: [reset]").contains("stepz"));
        assert!(err("world: {preset: huge}\nsteps: [reset]").contains("unknown preset"));
        assert!(err("world: {mock: nope}\nsteps: [reset]").contains("unknown mock"));
        assert!(err("screenshot: sometimes\nsteps: [reset]").contains("sometimes"));
        assert!(err("world: {colour: red}\nsteps: [reset]").contains("colour"));
    }

    #[test]
    fn unknown_steps_name_the_step_and_the_known_ones() {
        let e = err("steps:\n  - reset\n  - tap: \"#x\"\n");
        assert!(e.starts_with("step 2: unknown step \"tap\""), "{e}");
        assert!(e.contains("known steps: open, click"), "{e}");
        assert!(err("steps: [dance]").contains("unknown step \"dance\""));
    }

    #[test]
    fn malformed_steps_are_errors() {
        assert!(err("steps:\n  - {click: \"#a\", hover: \"#b\"}").contains("exactly one key"));
        assert!(err("steps:\n  - click: 3").contains("takes a string"));
        assert!(err("steps:\n  - type: {into: \"#a\"}").contains("text"));
        assert!(err("steps:\n  - type: {into: \"#a\", text: x, extra: 1}").contains("extra"));
        assert!(err("steps:\n  - scroll: {at: \"#a\"}").contains("both zero"));
        assert!(err("steps:\n  - settle: later").contains("`settle` takes"));
        assert!(err("steps:\n  - reset: 1").contains("no arguments"));
        assert!(err("steps:\n  - open: {chat: 'a\"b'}").contains("double quote"));
        assert!(err("steps:\n  - [1, 2]").contains("a step is a mapping"));
    }

    #[test]
    fn expect_must_make_sense() {
        assert!(err("steps:\n  - expect: {}").contains("no checks"));
        assert!(err("steps:\n  - expect: {focused: true}").contains("no checks"));
        assert!(err("steps:\n  - expect: {text: a, focused: true}").contains("need `visible`"));
        assert!(
            err("steps:\n  - expect: {visible: \"#a\", within: \"#b\"}").contains("`within` needs")
        );
        assert!(err("steps:\n  - expect: {perf: {}}").contains("max_draws or max_draw_ms"));
        assert!(err("steps:\n  - expect: {visible: \"#a\", bogus: 1}").contains("bogus"));
        assert!(err("steps:\n  - expect: {bounds: {\"#a\": {middle: [0, 1]}}}").contains("middle"));
    }

    #[test]
    fn perf_check_excludes_animated_draws() {
        let t = Timing {
            draws: 30,
            animated: 10,
            draw_ms: 50.0,
            ..Timing::default()
        };
        assert!(
            check_perf(
                &Perf {
                    max_draws: Some(20),
                    max_draw_ms: Some(100.0)
                },
                &t
            )
            .is_ok()
        );
        assert!(
            check_perf(
                &Perf {
                    max_draws: Some(19),
                    max_draw_ms: None
                },
                &t
            )
            .unwrap_err()
            .contains("20 draws")
        );
        assert!(
            check_perf(
                &Perf {
                    max_draws: None,
                    max_draw_ms: Some(10.0)
                },
                &t
            )
            .unwrap_err()
            .contains("draw time")
        );
    }

    #[test]
    fn scenario_files_lists_only_yaml() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["b.yaml", "a.yml", "c.txt"] {
            std::fs::write(dir.path().join(name), "").unwrap();
        }
        let names: Vec<String> = scenario_files(dir.path())
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.yml", "b.yaml"]);
    }
}
