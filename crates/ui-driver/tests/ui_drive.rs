//! B4: the warm driver (serve mode) and its file protocol, end to end.
//!
//! `drive_launcher` runs outside the sandbox: it allocates the scratch root
//! and runs `drive_serve` (this same binary) inside `run_isolated`.
//! `scripts/ui-drive` starts the launcher; the tests below start it the same
//! way and talk through `Client`, which is the protocol the CLI speaks.
#![cfg(target_os = "macos")]
#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use mast_ui_driver::{environment::ScratchDirectories, serve};
use serde_json::{Value, json};

fn account_home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME"))
}

/// Inside the sandbox: build the world, open the app, answer requests.
#[test]
#[ignore = "runs inside the driver sandbox; started by drive_launcher"]
fn drive_serve() {
    if let Err(error) = serve::serve() {
        panic!("driver failed: {error}");
    }
}

/// Outside the sandbox: `MAST_DRIVE_SESSION` is a directory the caller owns.
/// Writes `root` there, runs the sandboxed server, then `serve.log` and
/// `exited`, and removes the scratch root it created.
#[test]
#[ignore = "requires permission to apply a macOS sandbox"]
fn drive_launcher() {
    let session =
        PathBuf::from(std::env::var_os("MAST_DRIVE_SESSION").expect("MAST_DRIVE_SESSION"));
    let seed: u64 = std::env::var("MAST_DRIVE_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let preset = std::env::var("MAST_DRIVE_PRESET").unwrap_or_else(|_| "small".into());
    let render = std::env::var_os("MAST_DRIVE_RENDER").is_some();
    let shape = mast_ui_driver::scenario::World { preset: preset.clone(), seed, mock: None, shape: None }
        .shape_bytes().expect("invalid world shape");
    let account = account_home();
    let scratch = ScratchDirectories::create(&account).expect("scratch");
    let ctl = serve::control_dir(scratch.root());
    std::fs::create_dir(&ctl).unwrap();
    if let Some(bytes) = shape {
        std::fs::write(ctl.join("shape.json"), bytes).unwrap();
    }
    let config = serve::Config {
        preset,
        seed,
        render,
        mock: std::env::var("MAST_DRIVE_MOCK").ok(),
    };
    std::fs::write(ctl.join("config.json"), config.to_json().to_string()).unwrap();
    std::fs::write(
        session.join("root"),
        scratch.root().to_string_lossy().as_bytes(),
    )
    .unwrap();
    let exe = std::env::current_exe().unwrap();
    let args = [
        "--ignored",
        "--exact",
        "drive_serve",
        "--test-threads=1",
        "--nocapture",
    ];
    let args: Vec<&std::ffi::OsStr> = args.iter().map(std::ffi::OsStr::new).collect();
    let out = scratch.run_isolated(&account, &exe, &args);
    let log = match &out {
        Ok(out) => format!(
            "status: {}\n{}\n{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        Err(error) => format!("could not start: {error}"),
    };
    std::fs::write(session.join("serve.log"), log).unwrap();
    std::fs::write(session.join("exited"), b"").unwrap();
    // The scratch root is ours: created above by mktemp.
    std::fs::remove_dir_all(scratch.root()).unwrap();
}

struct Driver {
    launcher: Child,
    session: PathBuf,
    ctl: PathBuf,
    next_id: u64,
}

impl Driver {
    fn start(seed: u64, render: bool) -> Self {
        Self::start_with(seed, render, None)
    }

    fn start_with(seed: u64, render: bool, mock: Option<&str>) -> Self {
        let session = tempfile::tempdir().unwrap().keep();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--ignored", "--exact", "drive_launcher", "--test-threads=1"])
            .env("MAST_DRIVE_SESSION", &session)
            .env("MAST_DRIVE_SEED", seed.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if render {
            command.env("MAST_DRIVE_RENDER", "1");
        }
        if let Some(mock) = mock {
            command.env("MAST_DRIVE_MOCK", mock);
        }
        let launcher = command.spawn().unwrap();
        let mut driver = Self {
            launcher,
            ctl: PathBuf::new(),
            session,
            next_id: 0,
        };
        let started = Instant::now();
        loop {
            if let Ok(root) = std::fs::read_to_string(driver.session.join("root"))
                && !root.is_empty()
            {
                let ctl = serve::control_dir(Path::new(&root));
                if ctl.join("ready").exists() {
                    driver.ctl = ctl;
                    return driver;
                }
            }
            if driver.session.join("exited").exists() {
                let log =
                    std::fs::read_to_string(driver.session.join("serve.log")).unwrap_or_default();
                panic!("driver exited during start:\n{log}");
            }
            assert!(
                started.elapsed() < Duration::from_secs(300),
                "driver start timed out"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn call(&mut self, mut request: Value) -> Value {
        self.next_id += 1;
        request["id"] = json!(self.next_id);
        let response = self.ctl.join("response.json");
        let _ = std::fs::remove_file(&response);
        let tmp = self.ctl.join("request.tmp");
        std::fs::write(&tmp, request.to_string()).unwrap();
        std::fs::rename(&tmp, self.ctl.join("request.json")).unwrap();
        let started = Instant::now();
        loop {
            if let Ok(text) = std::fs::read_to_string(&response)
                && let Ok(value) = serde_json::from_str::<Value>(&text)
                && value["id"] == json!(self.next_id)
            {
                return value;
            }
            assert!(
                started.elapsed() < Duration::from_secs(120),
                "request timed out: {request}"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn tree(&mut self) -> Value {
        self.call(json!({"cmd": "tree"}))
    }
    fn click(&mut self, locator: &str) -> Value {
        self.call(json!({"cmd": "click", "locator": locator}))
    }

    fn stop(mut self) {
        self.call(json!({"cmd": "stop"}));
        let _ = self.launcher.wait();
        let log = std::fs::read_to_string(self.session.join("serve.log")).unwrap_or_default();
        assert!(
            log.contains("status: exit status: 0"),
            "serve exited badly:\n{log}"
        );
        std::fs::remove_dir_all(&self.session).unwrap();
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        // A failed test must not leave a sandboxed server polling forever.
        if !self.ctl.as_os_str().is_empty() {
            let _ = std::fs::write(self.ctl.join("shutdown"), b"");
        }
        let _ = self.launcher.wait();
    }
}

fn tree_of(response: &Value) -> &str {
    response["tree"].as_str().expect("tree text")
}

fn selected_chat(response: &Value) -> String {
    let status = response["status"].as_str().unwrap();
    status
        .split_whitespace()
        .find_map(|part| part.strip_prefix("selected="))
        .unwrap()
        .to_owned()
}

#[test]
#[ignore = "requires permission to apply a macOS sandbox"]
fn select_chat_type_key_and_ambiguity() {
    let mut d = Driver::start(1, false);
    let first = d.tree();
    assert_eq!(first["ok"], json!(true));
    assert!(
        first["header"]
            .as_str()
            .unwrap()
            .starts_with("preset=small seed=1 template=")
    );
    assert!(tree_of(&first).contains("Review the workspace layout."));
    let before = selected_chat(&first);

    // Select another chat by role + name: the engine-backed selection changes
    // and that chat's transcript replaces the first one.
    let world = mast_ui_driver::world::SmallWorld::small(1);
    let target = world
        .chats
        .iter()
        .find(|c| c.title == "Verify the changes")
        .unwrap();
    let wanted = target
        .entries
        .iter()
        .flat_map(|e| e["parts"].as_array().cloned().unwrap_or_default())
        .find_map(|p| p["text"].as_str().map(str::to_owned))
        .expect("a text part");
    let clicked = d.click("button[name=\"Verify the changes\"]");
    assert_eq!(clicked["ok"], json!(true), "{clicked}");
    assert_ne!(selected_chat(&clicked), before);
    assert_eq!(selected_chat(&clicked), target.id);
    let snippet: String = wanted.chars().take(40).collect();
    assert!(
        tree_of(&clicked).contains(&snippet),
        "transcript missing {snippet:?}:\n{}",
        tree_of(&clicked)
    );
    assert!(!tree_of(&clicked).contains("Review the workspace layout."));
    assert!(clicked["timing"]["draws"].as_u64().unwrap() >= 1);
    assert!(clicked["timing"]["nodes"].as_u64().unwrap() > 5);

    // Type into the composer; the value shows in the tree. Then one key.
    let typed = d.call(json!({"cmd": "type", "locator": "#composer-input", "text": "hello world"}));
    assert_eq!(typed["ok"], json!(true), "{typed}");
    assert!(
        tree_of(&typed).contains("value=\"hello world\""),
        "{}",
        tree_of(&typed)
    );
    let keyed = d.call(json!({"cmd": "key", "chord": "backspace"}));
    assert!(
        tree_of(&keyed).contains("value=\"hello worl\""),
        "{}",
        tree_of(&keyed)
    );

    // Ambiguous: two nodes contain "Verify"; an error lists both, nothing is clicked.
    let ambiguous = d.click("text=\"Verify\"");
    assert_eq!(ambiguous["ok"], json!(false));
    let error = ambiguous["error"].as_str().unwrap();
    assert!(
        error.contains("ambiguous") && error.matches("\n  e").count() >= 2,
        "{error}"
    );
    // :nth picks one explicitly; unknown locators and bad syntax are errors too.
    assert_eq!(d.click("#nope")["ok"], json!(false));
    assert_eq!(d.click("???")["ok"], json!(false));
    d.stop();
}

#[test]
#[ignore = "requires permission to apply a macOS sandbox and Metal"]
fn png_is_written_and_same_seed_same_tree() {
    let mut a = Driver::start(7, true);
    let tree_a = a.tree();
    let png = a.call(json!({"cmd": "png"}));
    assert_eq!(png["ok"], json!(true), "{png}");
    let bytes = std::fs::read(png["message"].as_str().unwrap()).unwrap();
    assert!(
        bytes.starts_with(b"\x89PNG") && bytes.len() > 10_000,
        "png is {} bytes",
        bytes.len()
    );
    a.stop();
    // A run without the renderer sees the same tree: the gate never needs Metal.
    let mut b = Driver::start(7, false);
    let tree_b = b.tree();
    assert_eq!(tree_a["tree_hash"], tree_b["tree_hash"]);
    assert_eq!(tree_of(&tree_a), tree_of(&tree_b));
    let refused = b.call(json!({"cmd": "png"}));
    assert_eq!(
        refused["ok"],
        json!(false),
        "png without --render must fail closed"
    );
    b.stop();
}

/// `[x,y WxH]` of the first node or loose selector named `name`, from a step
/// response's tree and selector listing.
fn bounds_of(response: &Value, name: &str) -> Option<[f32; 4]> {
    let needle = format!("sel={name} [");
    let text = format!(
        "{}\n{}",
        response["tree"].as_str().unwrap_or(""),
        response["selectors"].as_str().unwrap_or("")
    );
    let rest = &text[text.find(&needle)? + needle.len()..];
    let inner = &rest[..rest.find(']')?];
    let (xy, wh) = inner.split_once(' ')?;
    let (x, y) = xy.split_once(',')?;
    let (w, h) = wh.split_once('x')?;
    Some([
        x.parse().ok()?,
        y.parse().ok()?,
        w.parse().ok()?,
        h.parse().ok()?,
    ])
}

/// Excerpt of the tree lines that matter to a step, for the flow transcript.
fn excerpt(response: &Value, needles: &[&str]) -> String {
    tree_of(response)
        .lines()
        .filter(|l| needles.iter().any(|n| l.contains(n)))
        .map(|l| l.trim_start_matches(['│', '├', '└', ' ']))
        .collect::<Vec<_>>()
        .join("\n    ")
}

/// Loose per-step budget: a step settles in a bounded number of draws and the
/// summed draw time stays far under what a human would notice as a stall. Not
/// a benchmark (the driver forces full uncached draws); it catches a runaway
/// redraw loop or a per-frame cost that scales with the transcript.
const STEP_MAX_DRAWS: u64 = 24;
const STEP_MAX_DRAW_MS: f64 = 4000.0;

fn step(name: &str, response: &Value, log: &mut Vec<String>, needles: &[&str]) {
    assert_eq!(response["ok"], json!(true), "{name}: {response}");
    let t = &response["timing"];
    let (draws, draw_ms) = (
        t["draws"].as_u64().unwrap_or(0),
        t["draw_ms"].as_f64().unwrap_or(0.0),
    );
    log.push(format!(
        "{name}: draws={draws} draw_ms={draw_ms} wall_ms={} nodes={}\n    {}",
        t["wall_ms"],
        t["nodes"],
        excerpt(response, needles)
    ));
    println!("{}", log.last().unwrap());
    // Frame-callback draws (glide, spinner) are bounded by the driver; the
    // guard is on the rest.
    let animated = t["animated"].as_u64().unwrap_or(0);
    assert!(
        draws - animated <= STEP_MAX_DRAWS,
        "{name}: {draws} draws ({animated} animated)"
    );
    assert!(
        draw_ms <= STEP_MAX_DRAW_MS,
        "{name}: {draw_ms}ms of draw time"
    );
}

/// B5, the first flow: open a chat, send a message, approve the agent's
/// command, watch it finish, and keep the composer pinned to the bottom.
/// Set `MAST_FLOW_LOG=<file>` to write the step transcript there.
#[test]
#[ignore = "requires permission to apply a macOS sandbox"]
fn first_flow_send_approve_finish() {
    let mut d = Driver::start_with(1, false, Some("approval"));
    let mut log = Vec::new();
    let start = d.tree();
    step("start", &start, &mut log, &["Fresh chat", "composer-input"]);
    let resting = bounds_of(&start, "composer-input").expect("composer-input bounds");

    // 1. Open a chat from the sidebar.
    let opened = d.click("button[name=\"Fresh chat\"]");
    step(
        "open chat",
        &opened,
        &mut log,
        &["Fresh chat", "composer-input", "transcript-region"],
    );
    assert!(
        bounds_of(&opened, "transcript-region").is_some(),
        "transcript region missing"
    );

    // 2. Type into the composer and press Enter.
    let typed =
        d.call(json!({"cmd": "type", "locator": "#composer-input", "text": "run the tests"}));
    step("type", &typed, &mut log, &["composer-input"]);
    assert!(
        tree_of(&typed).contains("value=\"run the tests\""),
        "{}",
        tree_of(&typed)
    );
    assert!(
        tree_of(&typed)
            .lines()
            .any(|l| l.contains("composer-input") && l.contains("focused"))
    );
    let sent = d.call(json!({"cmd": "key", "chord": "enter"}));
    step("enter", &sent, &mut log, &["run the tests"]);

    // 3. The user message appears; 4. the approval card follows (the mock
    // agent answers after a short pause, so poll).
    let mut card = sent;
    for _ in 0..40 {
        if tree_of(&card).contains("run the tests") && tree_of(&card).contains("question-panel") {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
        card = d.tree();
    }
    step(
        "approval card",
        &card,
        &mut log,
        &[
            "run the tests",
            "question-panel",
            "Approve",
            "Deny",
            "Before I wire",
        ],
    );
    // The agent's reply text is in the tree: the own-send glide ran, so the
    // reply is not parked below the fold.
    assert!(
        tree_of(&card).contains("Before I wire the reconciliation path"),
        "reply text missing:\n{}",
        tree_of(&card)
    );
    assert!(
        tree_of(&card)
            .lines()
            .any(|l| l.contains("paragraph \"run the tests\"")),
        "user message not in the transcript:\n{}",
        tree_of(&card)
    );
    assert!(
        tree_of(&card).contains("sel=question-panel"),
        "{}",
        tree_of(&card)
    );
    assert!(tree_of(&card).contains("\"Approve command\""));

    // 5. Approve. 6. The agent finishes.
    let approved = d.click("button[name=\"Approve\"]");
    step(
        "approve",
        &approved,
        &mut log,
        &["Approve", "composer-send", "composer-stop"],
    );
    let mut done = approved;
    for _ in 0..60 {
        if tree_of(&done).contains("sel=composer-send")
            && !tree_of(&done).contains("question-panel")
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
        done = d.tree();
    }
    step(
        "finished",
        &done,
        &mut log,
        &[
            "run the tests",
            "composer-send",
            "composer-input",
            "Locked in",
        ],
    );
    assert!(
        tree_of(&done).contains("Locked in"),
        "final reply missing:\n{}",
        tree_of(&done)
    );
    assert!(
        tree_of(&done).contains("Approve"),
        "chosen answer not in the final reply:\n{}",
        tree_of(&done)
    );
    // Idle again: Send is back and Stop is gone.
    assert!(
        tree_of(&done).contains("sel=composer-send")
            && !tree_of(&done).contains("sel=composer-stop"),
        "{}",
        tree_of(&done)
    );
    assert!(
        !tree_of(&done).contains("sel=question-panel"),
        "approval card still showing"
    );

    // 7. The composer is pinned to the bottom of the window.
    let surface = bounds_of(&done, "composer-surface").expect("composer-surface bounds");
    let viewport_h = 800.0;
    let bottom_gap = viewport_h - (surface[1] + surface[3]);
    assert!(
        (0.0..=60.0).contains(&bottom_gap),
        "composer is {bottom_gap}px from the bottom: {surface:?}"
    );
    let input = bounds_of(&done, "composer-input").expect("composer-input bounds");
    assert!(
        (input[1] + input[3] - resting[1] - resting[3]).abs() <= 2.0,
        "composer moved: before {resting:?}, after {input:?}"
    );
    log.push(format!(
        "composer-surface {surface:?} bottom gap {bottom_gap}px"
    ));

    let transcript = log.join("\n");
    println!("{transcript}");
    if let Some(path) = std::env::var_os("MAST_FLOW_LOG") {
        std::fs::write(path, transcript).unwrap();
    }
    d.stop();
}
