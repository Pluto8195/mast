//! zeron — headed by default; `zeron headless` runs the engine alone. Both start
//! local-only without credentials. `zeron login` and `zeron logout` select the
//! profile used by the next engine start without mutating a live runtime.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod auth_cli;
mod daemon;
mod doctor;
mod paths;
mod preferences;
mod update_cli;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "mast",
    version,
    about = "Multi-device controller for coding agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Open a Mast conversation URL.
    #[arg(value_name = "URL")]
    open_url: Option<String>,
    #[cfg(windows)]
    #[arg(long, hide = true)]
    wait_for_exit: Option<u32>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the engine without a UI (local-only unless a saved session enables sync).
    Headless,
    /// Sign in and enable sync on the next engine start.
    Login,
    /// Remove the saved session and return to local-only on the next start.
    Logout,
    /// Show workspace mode, optional auth, and engine status.
    Status,
    /// Check local prerequisites without changing them.
    Doctor {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Live sync introspection from the running engine: per-room connection
    /// state, last pushed-frame/ack ages, rejoin/probe/resync counters.
    Sync,
    #[cfg(target_os = "linux")]
    /// Trigger an Appshot in the running headed instance (desktop shortcut fallback).
    Appshot,
    /// Serve the Mast MCP (Model Context Protocol) server on stdin/stdout,
    /// proxying to the running engine's IPC. Agents use it to create, read,
    /// and message chats. Logs go to stderr; stdout is the protocol.
    Mcp,
    /// Manage `zeron headless` as a background service (launchd / systemd --user).
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
    /// Check for a newer release and apply it (download → verify → swap →
    /// service restart). `--check` only reports (exits 1 when one is available).
    Update {
        #[arg(long)]
        check: bool,
    },
}

#[derive(Subcommand)]
enum DaemonCommand {
    /// Install, enable, and start the service (captures MAST_* env).
    Install,
    /// Stop and remove the service.
    Uninstall,
    /// Start the installed service.
    Start,
    /// Stop the service.
    Stop,
    /// Restart the service.
    Restart,
    /// Show the service manager's view of the daemon.
    Status,
}

/// Legacy sync endpoint; sync is disabled by default.
/// `MAST_EDGE_URL` overrides (local dev / self-hosting).
const DEFAULT_EDGE_URL: &str = "https://edge.zeron.sh";

/// Production WorkOS AuthKit client id — public knowledge (it appears in every
/// authorize URL), so baking it in is safe. Overridden by `MAST_WORKOS_CLIENT_ID`;
/// set it to the empty string — or set a dev bearer via `MAST_EDGE_TOKEN` — to
/// force dev-mode auth instead.
const DEFAULT_WORKOS_CLIENT_ID: &str = "client_01KWD0EAKZKD50YCQJNYSRE4BY";

fn edge_url_from_env() -> String {
    mast_proto::env::var("MAST_EDGE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_EDGE_URL.into())
}

/// WorkOS client id resolution: explicit env wins (empty string = dev mode);
/// otherwise a `MAST_EDGE_TOKEN` dev bearer keeps dev mode (smoke tests,
/// local wrangler); otherwise the baked production client id makes optional
/// sync available while a bare start remains local-only.
fn workos_client_id_from_env(edge_token: &Option<String>) -> Option<String> {
    match mast_proto::env::var("MAST_WORKOS_CLIENT_ID") {
        Ok(v) if v.trim().is_empty() => None,
        Ok(v) => Some(v),
        Err(_) if edge_token.is_some() => None,
        Err(_) => Some(DEFAULT_WORKOS_CLIENT_ID.into()),
    }
}

/// mimalloc, macOS only: libmalloc never returns the streaming churn's
/// high-water pages, so transient allocation became permanent RSS
/// (docs/memory-plan.md §1). Pinned to mimalloc v2 in the workspace manifest —
/// the crate's default v3 has the same pathology (churn retained as permanent
/// RSS, ~6x glibc's growth on identical workloads, no idle recovery). Linux
/// measured flat on glibc, so it keeps the system allocator.
#[cfg(target_os = "macos")]
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// The build id scanned out of an installed binary by a running Mast (see
/// `mast_ui::build_watch`). `#[used]` keeps it in the image.
#[used]
static BUILD_MARKER: &str = concat!(
    "MAST-BUILD-ID:",
    env!("MAST_BUILD_SHA"),
    ":",
    env!("MAST_BUILD_GIT_DIR"),
    "\0"
);

#[cfg(unix)]
fn raise_file_limit() -> std::io::Result<(libc::rlim_t, libc::rlim_t)> {
    let mut limit = std::mem::MaybeUninit::<libc::rlimit>::uninit();
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, limit.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut limit = unsafe { limit.assume_init() };
    let before = limit.rlim_cur;
    limit.rlim_cur = file_limit_target(before, limit.rlim_max);
    if limit.rlim_cur != before && unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok((before, limit.rlim_cur))
}

#[cfg(unix)]
fn file_limit_target(soft: libc::rlim_t, hard: libc::rlim_t) -> libc::rlim_t {
    soft.max(hard.min(8192))
}

#[cfg(all(test, unix))]
#[test]
fn raises_process_file_limit() {
    const CHILD: &str = "MAST_TEST_FILE_LIMIT_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let mut limit = std::mem::MaybeUninit::<libc::rlimit>::uninit();
        assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, limit.as_mut_ptr()) }, 0);
        let mut limit = unsafe { limit.assume_init() };
        limit.rlim_cur = limit.rlim_max.min(64);
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) }, 0);
        let (before, after) = raise_file_limit().unwrap();
        assert_eq!(before, limit.rlim_cur);
        assert_eq!(after, file_limit_target(before, limit.rlim_max));
        return;
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "raises_process_file_limit", "--nocapture"])
        .env(CHILD, "1")
        .status()
        .unwrap();
    assert!(status.success());
}

#[cfg(all(test, unix))]
#[test]
fn file_limit_target_respects_hard_limit_and_never_lowers_soft_limit() {
    assert_eq!(file_limit_target(256, 1024), 1024);
    assert_eq!(file_limit_target(256, 16384), 8192);
    assert_eq!(file_limit_target(12000, 16384), 12000);
}

fn main() -> anyhow::Result<()> {
    #[cfg(unix)]
    let file_limit = raise_file_limit();
    mast_ui::build_watch::set_running_build(env!("MAST_BUILD_SHA"), env!("MAST_BUILD_GIT_DIR"));
    #[cfg(windows)]
    attach_parent_console();
    let cli = Cli::parse();
    #[cfg(all(target_os = "macos", not(test)))]
    if cli.command.is_none() {
        preferences::migrate();
    }
    // Classify before logging or either engine mode creates startup files.
    if cli.command.is_none() || matches!(cli.command, Some(Command::Headless)) {
        mast_ui::settings::UiSettings::load_for_startup(&paths::data_dir());
    }
    #[cfg(windows)]
    if let Some(pid) = cli.wait_for_exit {
        mast_update::windows::wait_for_exit(pid)?;
    }
    // Long-running modes log at info, one-shot CLI commands at warn (RUST_LOG
    // overrides either).
    // loro's internal block-encode diagnostics log at info and flood
    // journald on every snapshot export — enough to fill a disk on a
    // long-running headless host. Quiet them by default (RUST_LOG still
    // overrides the whole filter).
    let long_running = matches!(&cli.command, None | Some(Command::Headless));
    let default_filter = if long_running {
        "info,loro_internal=warn,loro=warn"
    } else {
        "warn"
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| default_filter.into());
    // Long-running modes mirror stdout logging to {data_dir}/logs — a headed
    // app launched from Finder has no visible stdout, which left every sync
    // wedge report ("stale until restart") with zero diagnostics even though
    // the engine logs the exact failure line. One file per launch, previous
    // launch kept as `.old`.
    let log_file = if long_running {
        let mode = if cli.command.is_some() {
            "headless"
        } else {
            "headed"
        };
        open_log_file(mode)
    } else {
        None
    };
    {
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;
        // MCP and doctor --json reserve stdout for machine-readable output.
        if matches!(&cli.command, Some(Command::Mcp | Command::Doctor { .. })) {
            tracing_subscriber::registry()
                .with(filter)
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_ansi(false)
                        .with_writer(std::io::stderr),
                )
                .init();
        } else {
            let registry = tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer());
            match log_file {
                Some(file) => registry
                    .with(
                        tracing_subscriber::fmt::layer()
                            .with_ansi(false)
                            .with_writer(file),
                    )
                    .init(),
                None => registry.init(),
            }
        }
    }

    #[cfg(unix)]
    if long_running {
        match file_limit {
            Ok((before, after)) => tracing::info!(before, after, "open-file limit"),
            Err(error) => tracing::warn!(%error, "could not raise open-file limit"),
        }
    }

    if long_running {
        // Finder launches have no visible stderr. Mirror the panic location
        // and backtrace into the same rotating log as engine diagnostics.
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            tracing::error!(panic = %info,
                backtrace = %std::backtrace::Backtrace::force_capture(),
                "application panic");
            default_hook(info);
        }));
    }

    match cli.command {
        Some(Command::Headless) => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(async {
                let engine = mast_engine::Engine::new(engine_config_from_env());
                engine.run().await
            })
        }
        Some(Command::Login) => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(auth_cli::login(engine_config_from_env()))
        }
        Some(Command::Logout) => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(auth_cli::logout(engine_config_from_env()))
        }
        Some(Command::Status) => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(auth_cli::status(engine_config_from_env()))
        }
        Some(Command::Doctor { json }) => {
            let runtime = tokio::runtime::Runtime::new()?;
            let port = mast_proto::env::var("MAST_IPC_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(27654);
            let (data_dir, reason) = paths::data_dir_read_only();
            let report = runtime.block_on(doctor::inspect(data_dir, reason, port));
            doctor::print_report(&report, json)?;
            if report.exit_code() != 0 {
                std::process::exit(1);
            }
            Ok(())
        }
        Some(Command::Sync) => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(sync_cli(engine_config_from_env().ipc_port))
        }
        Some(Command::Mcp) => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(mast_mcp::run(mast_mcp::McpConfig::from_env()))
        }
        #[cfg(target_os = "linux")]
        Some(Command::Appshot) => {
            mast_ui::appshots::request_running_appshot(&engine_config_from_env().data_dir)
                .map_err(anyhow::Error::msg)
        }
        Some(Command::Update { check }) => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(update_cli::update(&edge_url_from_env(), check))
        }
        Some(Command::Daemon { command }) => match command {
            DaemonCommand::Install => daemon::install(&engine_config_from_env().data_dir),
            DaemonCommand::Uninstall => daemon::uninstall(),
            DaemonCommand::Start => daemon::start(),
            DaemonCommand::Stop => daemon::stop(),
            DaemonCommand::Restart => daemon::restart(),
            DaemonCommand::Status => daemon::status(),
        },
        None => {
            let edge_token = mast_proto::env::var("MAST_EDGE_TOKEN").ok();
            // Headed: the UI probes MAST_IPC_PORT and connects to a running
            // daemon, or embeds the engine in-process (ARCHITECTURE §1).
            mast_ui::run_app(mast_ui::UiConfig {
                data_dir: paths::data_dir(),
                ipc_port: mast_proto::env::var("MAST_IPC_PORT")
                    .ok()
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(27654),
                edge_url: edge_url_from_env(),
                workos_client_id: workos_client_id_from_env(&edge_token),
                edge_token,
                org_id: mast_proto::env::var("MAST_ORG_ID").ok(),
                default_harness: mast_ui::HarnessId::ClaudeCode,
                initial_url: cli.open_url,
            });
            Ok(())
        }
    }
}

#[cfg(windows)]
fn attach_parent_console() {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE,
        STD_OUTPUT_HANDLE, SetStdHandle,
    };

    // The GUI subsystem prevents Explorer from creating a console at startup.
    // Reuse an existing parent's console for CLI output and cargo run, without
    // allocating one. Attach before Clap so help and argument errors work too.
    // Preserve redirected pipes/files: attaching may replace standard handles.
    unsafe {
        let saved = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE]
            .map(|id| (id, GetStdHandle(id)));
        if AttachConsole(ATTACH_PARENT_PROCESS) != 0 {
            for (id, handle) in saved {
                if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
                    SetStdHandle(id, handle);
                }
            }
        }
    }
}

/// The env-resolved engine configuration shared by `headless`, `login`,
/// `logout`, and `status` — one resolution so the CLI auth commands always
/// operate on the exact session the daemon will load.
fn engine_config_from_env() -> mast_engine::EngineConfig {
    // Dev-mode bearer (no WorkOS): an explicit token enables sync.
    let edge_token = mast_proto::env::var("MAST_EDGE_TOKEN").ok();
    mast_engine::EngineConfig {
        data_dir: paths::data_dir(),
        edge_url: edge_url_from_env(),
        ipc_port: mast_proto::env::var("MAST_IPC_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(27654),
        default_harness: harness_from_env(),
        // WorkOS mode: the signed-in session's org wins; MAST_ORG_ID (dev
        // default "dev-org") scopes the workspace room otherwise.
        org_id: mast_proto::env::var("MAST_ORG_ID").ok(),
        // Real auth against production by default; see
        // `workos_client_id_from_env` for the dev-mode escape hatches.
        workos_client_id: workos_client_id_from_env(&edge_token),
        edge_token,
    }
}

/// `MAST_HARNESS` (kebab-case id) picks the default harness for chats without a
/// config row — `mock` powers the e2e smoke; default `claude-code`.
fn harness_from_env() -> mast_engine::HarnessId {
    match mast_proto::env::var("MAST_HARNESS")
        .as_deref()
        .map(str::trim)
    {
        Ok("mock") => mast_engine::HarnessId::Mock,
        Ok("codex") => mast_engine::HarnessId::Codex,
        Ok("cursor") => mast_engine::HarnessId::Cursor,
        Ok("devin") => mast_engine::HarnessId::Devin,
        Ok("grok") => mast_engine::HarnessId::Grok,
        Ok("hermes") => mast_engine::HarnessId::Hermes,
        Ok("pi") => mast_engine::HarnessId::Pi,
        Ok("antigravity") => mast_engine::HarnessId::Antigravity,
        _ => mast_engine::HarnessId::ClaudeCode,
    }
}

/// `zeron sync`: dial the running engine's IPC and print per-room sync state.
/// The introspection surface every 2026-08 incident was missing — "is this
/// device's workspace room actually receiving?" as a one-liner.
async fn sync_cli(ipc_port: u16) -> anyhow::Result<()> {
    let client = mast_rpc::connect_ws(&format!("ws://127.0.0.1:{ipc_port}"))
        .await
        .map_err(|e| {
            anyhow::anyhow!("no engine listening on 127.0.0.1:{ipc_port} ({e}) — is zeron running?")
        })?;
    let status = client
        .call(mast_rpc::methods::SYNC_STATUS, serde_json::json!({}))
        .await
        .map_err(|e| anyhow::anyhow!("SyncStatus failed: {e}"))?;
    let now = status.get("nowMs").and_then(|v| v.as_i64()).unwrap_or(0);
    let age = |ms: i64| -> String {
        if ms <= 0 {
            return "never".into();
        }
        let s = (now - ms).max(0) / 1000;
        if s >= 3600 {
            format!("{}h{}m ago", s / 3600, (s % 3600) / 60)
        } else if s >= 60 {
            format!("{}m{}s ago", s / 60, s % 60)
        } else {
            format!("{s}s ago")
        }
    };
    let room_line = |room: Option<&serde_json::Value>| -> String {
        let Some(room) = room else {
            return "no room (dialing or edge-less)".into();
        };
        let get = |k: &str| room.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
        // REJECTED is loud and only shown when nonzero: rejected writes with
        // a fresh-looking room is exactly the latched-session wedge
        // (2026-08-04) this readout previously masked.
        let rejected = get("rejected");
        format!(
            "{} pushed {} · acked {} · rejoins {} probes {} resyncs {} drops {}{}",
            if room.get("connected").and_then(|v| v.as_bool()) == Some(true) {
                "connected ·"
            } else {
                "DISCONNECTED ·"
            },
            age(get("lastPushedMs")),
            age(get("lastAckMs")),
            get("rejoins"),
            get("probes"),
            get("fullResyncs"),
            get("disconnects"),
            if rejected > 0 {
                format!(" REJECTED {rejected}")
            } else {
                String::new()
            },
        )
    };
    println!(
        "Device:    {}",
        status
            .get("deviceId")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
    );
    println!(
        "Workspace: {}",
        room_line(status.get("workspace").filter(|v| !v.is_null()))
    );
    let chats = status
        .get("chats")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if chats.is_empty() {
        println!("Chats:     none open");
    }
    // Chat rooms speak chat2: cursor/head tell "am I caught up?", pending
    // tells "did my writes leave?", resets/rejected are the loud tells.
    let chat_line = |room: Option<&serde_json::Value>| -> String {
        let Some(room) = room else {
            return "no room (dialing or edge-less)".into();
        };
        let get = |k: &str| room.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
        let resets = get("serverResets");
        let rejected = get("rejected");
        format!(
            "{} cursor {}/{} · pending {} · rows {} ({}KB) · rejoins {} drops {}{}{}",
            if room.get("connected").and_then(|v| v.as_bool()) == Some(true) {
                "connected ·"
            } else {
                "DISCONNECTED ·"
            },
            get("cursor"),
            get("headSeq"),
            get("pendingPushes"),
            get("rowCount"),
            get("rowBytes") / 1024,
            get("rejoins"),
            get("disconnects"),
            if resets > 0 {
                format!(" RESETS {resets}")
            } else {
                String::new()
            },
            if rejected > 0 {
                format!(" REJECTED {rejected}")
            } else {
                String::new()
            },
        )
    };
    for chat in &chats {
        println!(
            "Chat {}: {}",
            chat.get("chatId")
                .and_then(|v| v.as_str())
                .map(|s| &s[..s.len().min(8)])
                .unwrap_or("?"),
            chat_line(chat.get("room").filter(|v| !v.is_null()))
        );
    }
    Ok(())
}

/// Append to the current day's log; keep at most seven days and about 5 MiB.
fn open_log_file(mode: &str) -> Option<DailyLog> {
    open_log_file_in(&paths::data_dir().join("logs"), mode)
}

const LOG_DAY_BYTES: u64 = 5 * 1024 * 1024 / 7;
const LOG_DAY_SECONDS: u64 = 24 * 60 * 60;

fn log_day() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_secs() / LOG_DAY_SECONDS
}

struct DailyLog {
    state: std::sync::Mutex<LogState>,
}

struct LogState {
    dir: std::path::PathBuf,
    mode: String,
    file: std::fs::File,
    day: u64,
    canonical: bool,
    swept_day: Option<u64>,
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for DailyLog {
    type Writer = LogGuard<'a>;
    fn make_writer(&'a self) -> Self::Writer {
        LogGuard(self.state.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

struct LogGuard<'a>(std::sync::MutexGuard<'a, LogState>);
impl std::io::Write for LogGuard<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.prepare(log_day())?;
        std::io::Write::write(&mut self.0.file, bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> { std::io::Write::flush(&mut self.0.file) }
}

fn log_path(dir: &std::path::Path, mode: &str) -> std::path::PathBuf {
    dir.join(format!("mast-{mode}.log"))
}

fn open_log_file_in(dir: &std::path::Path, mode: &str) -> Option<DailyLog> {
    std::fs::create_dir_all(dir).ok()?;
    // Only legacy log files in this directory are eligible for removal.
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name.to_str().is_some_and(|n| n.starts_with("zeron-") && n.ends_with(".log"))
                && entry.file_type().is_ok_and(|kind| kind.is_file()) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let path = log_path(dir, mode);
    let open = |path: &std::path::Path| std::fs::OpenOptions::new().read(true).append(true).create(true).open(path);
    let mut file = open(&path).ok()?;
    #[cfg(unix)]
    let canonical = {
        use std::os::fd::AsRawFd;
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
    };
    #[cfg(not(unix))]
    let canonical = true;
    if !canonical {
        file = open(&dir.join(format!("mast-{mode}.{}.log", std::process::id()))).ok()?;
    }
    let day = file.metadata().ok().and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() / LOG_DAY_SECONDS).unwrap_or_else(log_day);
    let mut state = LogState { dir: dir.to_owned(), mode: mode.into(), file, day, canonical, swept_day: None };
    state.prepare(log_day()).ok()?;
    Some(DailyLog { state: std::sync::Mutex::new(state) })
}

impl LogState {
    fn prepare(&mut self, today: u64) -> std::io::Result<()> {
        use std::io::{Read, Seek, SeekFrom, Write};
        if self.canonical && self.day < today {
            let current = log_path(&self.dir, &self.mode);
            let archive = self.dir.join(format!("mast-{}.day-{}.log", self.mode, self.day));
            std::fs::rename(&current, archive)?;
            let replacement = std::fs::OpenOptions::new().read(true).append(true).create(true).open(&current)?;
            #[cfg(unix)] {
                use std::os::fd::AsRawFd;
                if unsafe { libc::flock(replacement.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            self.file = replacement;
            self.day = today;
        }
        if self.canonical && self.swept_day != Some(today) {
            let prefix = format!("mast-{}.day-", self.mode);
            if let Ok(entries) = std::fs::read_dir(&self.dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let day = name.to_str().and_then(|n| n.strip_prefix(&prefix))
                        .and_then(|n| n.strip_suffix(".log")).and_then(|n| n.parse::<u64>().ok());
                    if day.is_some_and(|day| day.saturating_add(6) < today) {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
            self.swept_day = Some(today);
        }
        let len = self.file.metadata()?.len();
        if len > LOG_DAY_BYTES {
            self.file.seek(SeekFrom::Start(len - LOG_DAY_BYTES / 2))?;
            let mut tail = Vec::new();
            self.file.read_to_end(&mut tail)?;
            self.file.set_len(0)?;
            self.file.write_all(&tail)?;
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod log_file_tests {
    use super::*;
    use std::io::Write;
    use tracing_subscriber::fmt::MakeWriter;

    #[test]
    fn logs_rotate_by_day_and_keep_live_writer_separate() {
        let dir = tempfile::tempdir().unwrap();
        let first = open_log_file_in(dir.path(), "headed").unwrap();
        writeln!(first.make_writer(), "first").unwrap();
        let second = open_log_file_in(dir.path(), "headed").unwrap();
        assert!(!second.state.lock().unwrap().canonical);
        drop(second);
        let mut state = first.state.lock().unwrap();
        let day = state.day;
        state.prepare(day + 1).unwrap();
        assert!(dir.path().join(format!("mast-headed.day-{day}.log")).exists());
        state.prepare(day + 7).unwrap();
        assert!(!dir.path().join(format!("mast-headed.day-{day}.log")).exists());
    }

    #[test]
    fn removes_only_legacy_logs_and_bounds_current_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("zeron-headed.log"), b"legacy").unwrap();
        std::fs::write(dir.path().join("zeron-notes.txt"), b"keep").unwrap();
        let log = open_log_file_in(dir.path(), "headed").unwrap();
        assert!(!dir.path().join("zeron-headed.log").exists());
        assert!(dir.path().join("zeron-notes.txt").exists());
        let mut state = log.state.lock().unwrap();
        state.file.write_all(&vec![b'x'; LOG_DAY_BYTES as usize + 1]).unwrap();
        let day = state.day;
        state.prepare(day).unwrap();
        assert!(state.file.metadata().unwrap().len() <= LOG_DAY_BYTES);
    }
}

#[cfg(test)]
#[path = "../../../test_support/isolation.rs"]
mod test_isolation;
