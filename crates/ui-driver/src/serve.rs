//! Warm driver process (runs inside the sandbox) and the file protocol the
//! `scripts/ui-drive` CLI speaks, modelled on `scripts/ui-preview-server.py`.
//!
//! `<root>/ctl/request.json` is renamed to `request.inflight.json`, executed,
//! and answered by an atomic write of `response.json`. `ctl/` is inside the
//! scratch root, the only place the sandbox lets this process write.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::environment::ScratchDirectories;
use crate::session::{Session, Timing};
use crate::template::Template;
use crate::world::SmallWorld;

pub fn control_dir(root: &Path) -> PathBuf {
    root.join("ctl")
}

/// What `start` asked for; written by the launcher before the sandbox starts.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub preset: String,
    pub seed: u64,
    pub render: bool,
    /// Scripted Mock agent: `approval`, `question` or `error`.
    pub mock: Option<String>,
}

/// The scripted-agent names `--mock` accepts and the Mock knob each sets.
fn mock_knob(name: &str) -> Result<&'static str, String> {
    match name {
        "approval" => Ok("MAST_MOCK_APPROVAL"),
        "question" => Ok("MAST_MOCK_QUESTION"),
        "error" => Ok("MAST_MOCK_ERROR"),
        other => Err(format!(
            "unknown mock {other:?}; use approval, question or error"
        )),
    }
}

impl Config {
    pub fn to_json(&self) -> Value {
        json!({"preset": self.preset, "seed": self.seed, "render": self.render, "mock": self.mock})
    }
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let preset = value["preset"].as_str().unwrap_or("small").to_owned();
        if !["small", "medium", "scale"].contains(&preset.as_str()) {
            return Err(format!(
                "unknown preset {preset:?}; use small, medium or scale"
            ));
        }
        Ok(Self {
            preset,
            seed: value["seed"].as_u64().unwrap_or(1),
            render: value["render"].as_bool().unwrap_or(false),
            mock: match value["mock"].as_str() {
                Some(name) => {
                    mock_knob(name)?;
                    Some(name.to_owned())
                }
                None => None,
            },
        })
    }
}

fn timing_json(t: &Timing) -> Value {
    json!({
        "wall_ms": round(t.wall_ms),
        "draw_ms": round(t.draw_ms),
        "animated": t.animated,
        "draws": t.draws,
        "nodes": t.nodes,
        "notifies": t.notifies,
    })
}

fn round(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Execute one request. Returns the response and whether to stop.
fn handle(session: &mut Session, ctl: &Path, request: &Value) -> (Value, bool) {
    let cmd = request["cmd"].as_str().unwrap_or("");
    let locator = request["locator"].as_str().unwrap_or("");
    let text = request["text"].as_str().unwrap_or("");
    let mut stop = false;
    let mut message = Value::Null;
    let result: Result<Option<Timing>, String> = match cmd {
        "tree" => Ok(Some(session.settle())),
        "click" => session.click(locator).map(Some),
        "type" => session.type_text(locator, text).map(Some),
        "key" => session
            .key(request["chord"].as_str().unwrap_or(""))
            .map(Some),
        "png" => {
            let path = ctl.join("step.png");
            session.settle();
            session.png(&path).map(|()| {
                message = json!(path.to_string_lossy());
                None
            })
        }
        "stop" => {
            stop = true;
            Ok(None)
        }
        other => Err(format!("unknown command {other:?}")),
    };
    let mut response = json!({"id": request["id"], "cmd": cmd});
    match result {
        Ok(timing) => {
            response["ok"] = json!(true);
            response["message"] = message;
            if let Some(t) = &timing {
                response["timing"] = timing_json(t);
            }
        }
        Err(error) => {
            response["ok"] = json!(false);
            response["error"] = json!(error);
        }
    }
    if cmd != "stop" {
        response["status"] = json!(session.status());
        let frame = session.frame();
        response["header"] = json!(session.header());
        response["tree"] = json!(frame.render());
        response["selectors"] = json!(frame.render_selectors());
        response["tree_hash"] = json!(frame.hash());
        response["nodes"] = json!(frame.nodes.len());
        // Artifacts a step leaves behind, for agents that want to grep.
        let _ = std::fs::write(ctl.join("tree.txt"), frame.render());
        if let Some(t) = response.get("timing") {
            let _ = std::fs::write(ctl.join("timing.json"), t.to_string());
        }
    }
    (response, stop)
}

/// Build the world, open the app, then answer requests until `stop`.
/// Must run inside `run_isolated`; fails closed otherwise.
pub fn serve() -> Result<(), String> {
    // stderr lands in the launcher's serve.log.
    let _ = tracing_subscriber::fmt()
        .with_env_filter("warn,mast_ui=debug,mast_engine=debug,mast_rpc=debug,loro_internal=off")
        .with_writer(std::io::stderr)
        .try_init();
    let scratch = ScratchDirectories::from_env().map_err(|e| e.to_string())?;
    serve_in(&scratch)
}

/// Tell the Mock harness which script to play. Single-threaded callers only:
/// before the runtime and the app exist. The sandboxed process has a cleared
/// environment, so this is the only way the Mock harness learns the script.
pub fn apply_mock(config: &Config) -> Result<(), String> {
    if let Some(name) = &config.mock {
        unsafe {
            std::env::set_var(mock_knob(name)?, "1");
            // The composer only offers the Mock harness under the dev rig flag.
            std::env::set_var("MAST_HARNESS", "mock");
        }
    }
    Ok(())
}

pub fn new_runtime() -> Result<Arc<tokio::runtime::Runtime>, String> {
    Ok(Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?,
    ))
}

/// Restore the template into the scratch data dir and open the app on it.
pub fn boot_session(
    scratch: &ScratchDirectories,
    template: &Template,
    world: &SmallWorld,
    runtime: &Arc<tokio::runtime::Runtime>,
    render: bool,
) -> Result<Session, String> {
    template.restore(scratch).map_err(|e| e.to_string())?;
    // The engine opens the restored stores to read the first frame's rows,
    // then shuts down; the session boots it again behind the window.
    let rows = runtime.block_on(async {
        let core = crate::materialize::open_core(scratch.data())?;
        let rows = crate::materialize::load_rows(world, &core);
        core.shutdown().await;
        rows
    })?;
    Session::open(
        scratch.data(),
        template.header(),
        rows,
        runtime.clone(),
        render,
    )
}

/// The serve loop on an already validated scratch root.
pub fn serve_in(scratch: &ScratchDirectories) -> Result<(), String> {
    let ctl = control_dir(scratch.root());
    let config: Value = serde_json::from_str(
        &std::fs::read_to_string(ctl.join("config.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let config = Config::from_json(&config)?;
    apply_mock(&config)?;
    let runtime = new_runtime()?;
    let world = match config.preset.as_str() {
        "small" => SmallWorld::small(config.seed),
        "medium" => SmallWorld::shaped(config.seed, "medium", &crate::shape::WorldShape::medium())?,
        "scale" => {
            let bytes = std::fs::read(ctl.join("shape.json"))
                .map_err(|_| "scale preset requires MAST_UI_DRIVER_SHAPE")?;
            let shape =
                serde_json::from_slice(&bytes).map_err(|e| format!("invalid world shape: {e}"))?;
            SmallWorld::shaped(config.seed, "scale", &shape)?
        }
        _ => return Err("unknown world preset".into()),
    };
    let template = runtime.block_on(Template::build(scratch, &world))?;
    let mut session = boot_session(scratch, &template, &world, &runtime, config.render)?;
    let first = json!({
        "header": session.header(),
        "tree_hash": session.frame().hash(),
    });
    write_atomic(&ctl.join("ready"), &first.to_string()).map_err(|e| e.to_string())?;
    let request_path = ctl.join("request.json");
    let inflight = ctl.join("request.inflight.json");
    let response_path = ctl.join("response.json");
    let started = Instant::now();
    loop {
        if !request_path.exists() {
            // A launcher that died takes the stop file with it.
            if ctl.join("shutdown").exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        std::fs::rename(&request_path, &inflight).map_err(|e| e.to_string())?;
        let request: Value = std::fs::read_to_string(&inflight)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()))
            .unwrap_or_else(|e| json!({"cmd": "invalid", "error": e}));
        let (mut response, stop) = handle(&mut session, &ctl, &request);
        response["uptime_s"] = json!(started.elapsed().as_secs());
        write_atomic(&response_path, &response.to_string()).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&inflight);
        if stop {
            break;
        }
    }
    drop(session);
    // Engine shutdown needs the runtime; dropping the Arc last ends it.
    drop(runtime);
    Ok(())
}
