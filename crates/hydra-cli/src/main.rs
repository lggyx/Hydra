// Swap in mimalloc on Windows 鈥?the default HeapAlloc is the biggest single
// contributor to per-keystroke render latency (hundreds of small Line/Span
// clones per frame). No-op on macOS/Linux where the system allocator is fine.
#[cfg(target_os = "windows")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

mod telemetry_cmd;
mod uninstall;

use hydra_core::agent::{AgentCommand, AgentEvent, AgentLoop, AgentRuntimeFactory};
use hydra_core::config::provider::{default_context_window_for, ProviderConfig};
use hydra_core::config::Config;
use hydra_core::conversation::Conversation;
use hydra_core::lsp::manager::build_lsp_manager;
use hydra_core::mcp::{
    load_mcp_config, login_mcp_oauth, merge_http_oauth_mcp_server_into_json_file,
    merge_stdio_mcp_server_into_json_file, register_mcp_tools, McpHttpAuthConfig,
    McpOAuthLoginOptions, McpRegistry, McpTokenStore, McpTransportConfig,
};
use hydra_core::provider::{create_provider, unavailable_provider};
use hydra_core::session::SessionManager;
use hydra_core::tool::bash::BashTool;
use hydra_core::tool::cd::CdTool;
use hydra_core::tool::diagnostics::DiagnosticsTool;
use hydra_core::tool::edit::EditFileTool;
use hydra_core::tool::glob::GlobTool;
use hydra_core::tool::grep::GrepTool;
use hydra_core::tool::list_dir::ListDirTool;
use hydra_core::tool::open_file::OpenFileTool;
use hydra_core::tool::read::ReadFileTool;
use hydra_core::tool::search_replace::SearchReplaceTool;
use hydra_core::tool::web_fetch::WebFetchTool;
use hydra_core::tool::web_search::WebSearchTool;
use hydra_core::tool::write::WriteFileTool;
use hydra_core::tool::{ToolContext, ToolRegistry};

use hydra_core::auth;
use hydra_telemetry::{
    config::{resolve, ProcessEnv},
    event::SessionMode,
    notice, CliOverride, CurrentContext, Event, Telemetry,
};

/// Set to `true` at the start of `run_headless` so the panic hook and the
/// top-level error handler can skip TUI cleanup. In headless mode raw mode
/// was never enabled, so calling `disable_raw_mode` would be a wasted ioctl
/// and on Windows can panic if the console handle isn't a real TTY.
static HEADLESS_MODE: AtomicBool = AtomicBool::new(false);

/// Restore terminal state if (and only if) we ever entered TUI mode.
/// No-op in headless mode 鈥?see [`HEADLESS_MODE`].
///
/// TUI mode (v4.23.2+) runs entirely in the primary screen via the
/// append-only RetainedRenderer 鈥?we never emit `\x1b[?1049h`, so there
/// is no `LeaveAlternateScreen` counterpart to issue here. Mouse mode,
/// cursor visibility, autowrap and DECSTBM are restored by
/// `RetainedRenderer::Drop` / `shutdown()`; this hook only owns the
/// raw-mode toggle.
fn restore_terminal_if_tui() {
    if HEADLESS_MODE.load(Ordering::Relaxed) {
        return;
    }
    let _ = crossterm::terminal::disable_raw_mode();
}

/// Resolve the working directory at startup. **Always** uses the current
/// working directory unless the user explicitly passed `-C / --dir`.
///
/// We deliberately do **not** read `~/.hydra/recent_dirs.txt` (or any other
/// "remembered" path). The previous implementation silently substituted the
/// first entry of recent_dirs for the user's cwd, which made commands like
/// `hydra -p "describe this project"` operate on whatever directory the
/// TUI happened to visit last 鈥?a violation of least surprise. recent_dirs
/// remains a TUI picker convenience only; it must never override cwd.
fn resolve_working_dir(cli_dir: Option<PathBuf>) -> PathBuf {
    if let Some(d) = cli_dir {
        std::fs::canonicalize(&d).unwrap_or(d)
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }
}

/// Truncate a string to at most `max_chars` *characters* (not bytes), replacing
/// any newlines with spaces and appending "..." when truncated.
///
/// Used for headless-mode log lines on stderr. **Counts characters, not bytes**,
/// so multi-byte UTF-8 (e.g. CJK) is safe 鈥?`&s[..N]` would panic when N falls
/// inside a multi-byte char.
fn truncate_log_line(s: &str, max_chars: usize) -> String {
    let single_line: String = s.chars().map(|c| if c == '\n' { ' ' } else { c }).collect();
    if single_line.chars().count() > max_chars {
        let head: String = single_line.chars().take(max_chars).collect();
        format!("{}...", head)
    } else {
        single_line
    }
}

/// Append a streaming reasoning/thinking `chunk` to `out`, maintaining a
/// single-line `[thinking] ...` representation across many tiny deltas.
///
/// `open` tracks whether a `[thinking]` line is currently open (i.e. has a
/// prefix written but no trailing newline). The first chunk gets a fresh
/// `[thinking] ` prefix; subsequent chunks append directly. Embedded newlines
/// inside a chunk are preserved, with each non-empty new line getting its own
/// `[thinking] ` prefix so multi-line thinking stays readable.
///
/// Pulled out of `run_headless` so it can be unit-tested without spinning up
/// the agent loop. Regression target: the old per-chunk `eprintln!` produced
/// "one word per line" output for streaming reasoning models.
fn format_thinking_chunk(out: &mut String, open: &mut bool, chunk: &str) {
    if chunk.is_empty() {
        return;
    }
    if !*open {
        out.push_str("[thinking] ");
        *open = true;
    }
    let mut parts = chunk.split('\n');
    if let Some(first) = parts.next() {
        out.push_str(first);
    }
    for part in parts {
        out.push('\n');
        *open = false;
        if !part.is_empty() {
            out.push_str("[thinking] ");
            out.push_str(part);
            *open = true;
        }
    }
}

/// Close any in-flight `[thinking]` line by writing a newline if one is open.
/// Mirrors the inline `close_thinking_line` used inside `run_headless`, but
/// writes to a buffer so it can be unit-tested.
fn close_thinking_chunk(out: &mut String, open: &mut bool) {
    if *open {
        out.push('\n');
        *open = false;
    }
}

/// True if `--dev` is present in argv. Used to skip every auto-update
/// path (pre-parse `apply_pending_upgrade`, sync stage+apply, and the
/// post-parse detached stager). Scanned manually because two of those
/// paths run before clap touches argv. The flag is also declared on
/// `Cli` so `clap::Parser` accepts it without erroring after the early
/// scan.
fn is_dev_mode() -> bool {
    std::env::args().skip(1).any(|a| a == "--dev")
}

/// True when the currently-running binary's filename ends in `.bak`.
/// `self_update::replace_binary` renames the previous version to
/// `hydra.bak` (or `hydra.exe.bak`) during an upgrade so the user
/// can roll back. Running that backup must NOT auto-upgrade 鈥?otherwise
/// rolling back is impossible: any launch of `.bak` would just overwrite
/// itself with the latest version again.
///
/// Defensive: if we can't read `current_exe()` for any reason, assume
/// we're the live binary (not backup) so auto-upgrade still works for
/// the common case.
fn is_running_as_backup() -> bool {
    std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|n| n.ends_with(".bak"))
        .unwrap_or(false)
}

/// Decide whether the startup-time synchronous upgrade path should fire.
/// Returns false when any of these hold:
///   * We're running as `hydra.bak` 鈫?user wants the old binary,
///     don't silently swap it back to latest.
///   * `-p` / `--prompt` / `--prompt-file` is in argv 鈫?headless script run,
///     shouldn't stall 5-20 s on a network download for a 2 s task.
///   * A subcommand (login, logout, status, upgrade, rollback, mcp) is in argv
///     鈫?those have their own flows and don't want a surprise re-exec.
///   * Config has `auto_update = false` 鈫?user explicitly opted out.
/// Anything else (including missing config) 鈫?true, because fresh installs
/// that haven't written a config yet are exactly the case we want to help.
///
/// Deliberately scans argv by hand 鈥?clap hasn't parsed yet at this point
/// in main(), and we need to decide before any slower setup happens.
fn should_try_sync_upgrade() -> bool {
    if is_running_as_backup() {
        return false;
    }
    if is_dev_mode() {
        return false;
    }

    let args: Vec<String> = std::env::args().collect();
    let any = |needle: &[&str]| {
        args.iter().skip(1).any(|a| {
            needle
                .iter()
                .any(|n| a == n || a.starts_with(&format!("{}=", n)))
        })
    };

    if any(&["-p", "--prompt", "--prompt-file"]) {
        return false;
    }
    if args.iter().skip(1).any(|a| {
        matches!(
            a.as_str(),
            "login"
                | "logout"
                | "status"
                | "upgrade"
                | "rollback"
                | "uninstall"
                | "mcp"
                | "telemetry"
                | "--version"
                | "-V"
                | "--help"
                | "-h"
        )
    }) {
        return false;
    }

    // Load just enough of the config to honor `auto_update = false`.
    // Failure to load = assume default (true) 鈥?fresh installs benefit.
    let path = hydra_core::config::Config::default_path();
    if path.exists() {
        if let Ok(cfg) = hydra_core::config::Config::load(&path) {
            if !cfg.auto_update {
                return false;
            }
        }
    }
    true
}

/// Startup-time synchronous upgrade. Fetches the manifest, and if a newer
/// release exists, downloads + verifies + stages + applies it in-line,
/// then re-execs into the new binary. Progress is printed to stderr so
/// the user sees something happen during the 5-20 s window (as opposed
/// to a silent hang). Anything that fails 鈫?fall through; the parent's
/// `main` continues with the current binary, and the detached worker
/// spawned later (`spawn_detached_upgrade_prep`) is still there as a
/// second chance for the next session.
///
/// Bounded by an overall 120 s timeout so a slow mirror / hung DNS can't
/// wedge startup forever.
async fn sync_stage_and_apply_if_newer() {
    use hydra_core::self_update::{self, UpgradeEvent};

    let current = format!("v{}", env!("CARGO_PKG_VERSION"));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<UpgradeEvent>();

    // Progress consumer: renders ManifestFetched / Downloading / Verifying
    // as a single-line updating status on stderr. Percent-debounced so a
    // 15 MB download at 64 KiB chunks doesn't flood the terminal.
    let progress = tokio::spawn(async move {
        use std::io::Write;
        let mut last_pct: i32 = -1;
        while let Some(ev) = rx.recv().await {
            match ev {
                UpgradeEvent::ManifestFetched { version } => {
                    eprintln!("鉁?New version available: {}", version);
                }
                UpgradeEvent::Downloading { bytes, total } => {
                    let pct = if total == 0 {
                        0
                    } else {
                        ((bytes * 100) / total) as i32
                    };
                    if pct != last_pct {
                        eprint!(
                            "\r   Downloading {}% ({:.1} / {:.1} MB)      ",
                            pct,
                            bytes as f64 / 1_048_576.0,
                            total as f64 / 1_048_576.0
                        );
                        let _ = std::io::stderr().flush();
                        last_pct = pct;
                    }
                }
                UpgradeEvent::Verifying => {
                    eprintln!("\n鉁?Verifying sha256");
                }
                _ => {}
            }
        }
    });

    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        self_update::prepare_deferred_upgrade(&current, tx),
    )
    .await;

    // Wait briefly for the progress consumer to drain 鈥?it closes when
    // the sender drops at the end of prepare_deferred_upgrade.
    let _ = progress.await;

    match outcome {
        Ok(Ok(Some(_staged))) => {
            // Staged successfully. Apply right now so the user gets the new
            // binary on this same invocation.
            match self_update::apply_pending_upgrade() {
                Ok(Some(applied)) => {
                    eprintln!("鉁?Upgrading to {}...", applied.version);
                    // Save the CURRENT version (before upgrade) so TUI can show "Upgraded old 鈫?new"
                    std::env::set_var(UPGRADED_FROM_ENV, &current);
                    match self_update::re_exec_self(Some(&applied.exe)) {
                        Ok(_infallible) => unreachable!("re_exec_self returned Ok"),
                        Err(e) => {
                            eprintln!(
                                "Upgrade applied but re-exec failed ({}). The new version will be used on the next launch.",
                                e
                            );
                            std::env::remove_var(UPGRADED_FROM_ENV);
                        }
                    }
                }
                _ => {
                    // Stage succeeded but apply didn't 鈥?weird, just continue.
                }
            }
        }
        Ok(Ok(None)) => {
            // Already latest, no-op.
        }
        Ok(Err(_)) | Err(_) => {
            // Network error or 120 s timeout. Don't spam the user 鈥?
            // `/upgrade` will surface the real error if they ask.
            eprintln!("Note: could not check for updates at startup (will retry in background).");
        }
    }
}

/// Body of the detached upgrade-prep worker. One call to
/// `prepare_deferred_upgrade` (which fetches the manifest, downloads the
/// next version's binary if newer, verifies sha256, and writes
/// `pending.json`). On success the next parent-hydra start will pick
/// up `pending.json` and apply. Silent: stdout/stderr are already /dev/null
/// (see `spawn_detached_upgrade_prep`), so any output would be discarded.
async fn run_prepare_upgrade_worker() -> i32 {
    let current = format!("v{}", env!("CARGO_PKG_VERSION"));
    // UpgradeEvent stream is per-byte progress; we don't surface it here
    // (parent is gone), so drain to /dev/null.
    let (tx, mut rx) =
        tokio::sync::mpsc::unbounded_channel::<hydra_core::self_update::UpgradeEvent>();
    tokio::spawn(async move { while rx.recv().await.is_some() {} });

    match hydra_core::self_update::prepare_deferred_upgrade(&current, tx).await {
        Ok(_) => 0,
        Err(_) => 1,
    }
}

/// Spawn a detached copy of this binary that runs the upgrade-prep worker
/// and exits. "Detached" means:
///   * New session on Unix (`setsid`) 鈥?parent's Ctrl+C goes to parent's
///     foreground process group only; the child is in its own and ignores it.
///   * `CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW` on Windows, same idea
///   * stdin/stdout/stderr 鈫?/dev/null so the child can't scribble over the
///     parent's terminal and has no reason to stay attached to it.
///
/// Does NOT wait for the child (we intentionally don't 鈥?that would recreate
/// the cancel-on-exit problem we're trying to solve). If spawning fails we
/// just drop the error; auto-upgrade is best-effort.
fn spawn_detached_upgrade_prep() {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };

    let mut cmd = std::process::Command::new(&exe);
    cmd.env(INTERNAL_PREPARE_UPGRADE_ENV, "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                // Detach from parent's controlling terminal / process group.
                // Return value ignored 鈥?setsid only fails when caller is
                // already a process group leader (not our case post-fork).
                libc::setsid();
                Ok(())
            });
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }

    let _ = cmd.spawn();
}

const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("HYDRA_BUILD_ID"),
    env!("HYDRA_BUILD_DIRTY"),
    ")"
);

#[derive(Parser)]
#[command(name = "hydra", version = VERSION, about = "AI coding assistant in your terminal")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Continue the previous session instead of starting a new one
    #[arg(short = 'c', long = "continue")]
    continue_last: bool,

    /// Provider to use (overrides config default)
    #[arg(long)]
    provider: Option<String>,

    /// Model to use (overrides config provider model)
    #[arg(long)]
    model: Option<String>,

    /// Set interface language (e.g. en, zh-CN, zh)
    #[arg(long)]
    lang: Option<String>,

    /// Path to config file
    #[arg(long)]
    config: Option<PathBuf>,

    /// Working directory (defaults to current directory)
    #[arg(long, short = 'C')]
    dir: Option<PathBuf>,

    /// Prompt to run in headless (non-interactive) mode. If omitted, launches the TUI.
    #[arg(short = 'p', long)]
    prompt: Option<String>,

    /// Read the prompt from a file (alternative to -p). Useful for long prompts
    /// that would exceed ARG_MAX or whose trailing newlines matter.
    #[arg(long, value_name = "PATH", conflicts_with = "prompt")]
    prompt_file: Option<std::path::PathBuf>,

    /// Show tool calls, token usage, and turn summary on stderr (headless mode only).
    /// Without this flag, headless output is the assistant reply only 鈥?Claude Code -p style.
    #[arg(short = 'v', long)]
    verbose: bool,

    /// Maximum number of LLM turns before the agent loop is force-stopped.
    /// Bounds context accumulation on long-running tasks (e.g. SWE-bench eval).
    /// Default: unbounded 鈥?the agent stops naturally when the model returns
    /// no tool calls or when the step budget (tool-call cap) is reached.
    #[arg(long)]
    max_turns: Option<usize>,

    /// Disable auto-update for this launch. Skips applying any staged
    /// upgrade, skips the sync stage+apply on startup, and skips the
    /// detached background stager. Use during local development so a
    /// fresh `cargo run` build isn't silently overwritten by the
    /// released binary.
    #[arg(long)]
    dev: bool,

    /// Comma-separated list of tool names to exclude from the registry.
    /// Use this to disable tools that are useless or harmful in a particular
    /// environment 鈥?e.g. `--disable-tools bash,web_fetch` for SWE-bench eval
    /// where the sandbox can't run commands and offline mode is required.
    /// Tools the LLM tries to call after disabling will be invisible to it
    /// (they won't appear in the schemas list at all), so the model will not
    /// retry against a permanently-blocked tool.
    #[arg(long, value_delimiter = ',', value_name = "NAMES")]
    disable_tools: Vec<String>,

    /// Disable telemetry for this invocation.
    #[arg(long = "no-telemetry", default_value_t = false, global = true)]
    pub no_telemetry: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Sign in with AtomGit OAuth and claim CodingPlan models in one
    /// flow: OAuth (if needed) 鈫?claim 鈫?fetch models 鈫?register
    /// providers 鈫?fetch status. Reports each step and exits.
    Login,
    /// Logout from Hydra
    Logout,
    /// Show current login status
    Status,
    /// Upgrade hydra in-place to the latest released version
    Upgrade {
        /// Reinstall even when already on the latest version
        #[arg(long)]
        force: bool,
    },
    /// Roll back to the previous version (swap with .bak on disk)
    Rollback,
    /// Fetch an AtomGit issue assigned to you and let the agent fix it
    /// in the current project (no commit, no push 鈥?edits local files only).
    Fixissue {
        /// Full issue URL, e.g. https://atomgit.com/owner/repo/issues/42
        url: String,
    },
    /// Hidden alias for `hydra login` 鈥?kept so existing scripts /
    /// muscle memory don't break after `/codingplan` and `hydra
    /// codingplan` were folded into the unified `/login` flow.
    #[command(hide = true)]
    Codingplan,
    /// Manage MCP server entries in `.mcp.json` (similar to `claude mcp add`)
    #[command(subcommand)]
    Mcp(McpCli),
    /// Start the HTTP daemon for IDE integration (VS Code extension connects to this)
    Daemon {
        /// Port to listen on (default: 13456)
        #[arg(long, default_value = "13456")]
        port: u16,
        /// Client identifier for telemetry (e.g. "vscode", "hydra-air")
        #[arg(long)]
        client: Option<String>,
    },
    /// Telemetry controls
    Telemetry {
        #[command(subcommand)]
        action: TelemetryAction,
    },
    /// Manage skill/command plugins (mirrors `claude plugin ...`).
    /// Operates on `$HYDRA_HOME/plugins/` shared with the TUI's `/plugin`
    /// slash command 鈥?anything installed via either path is visible to both.
    #[command(subcommand)]
    Plugin(PluginCli),
    /// Uninstall Hydra: remove the binary, PATH edit, and (interactively)
    /// data under ~/.hydra/. With no flags, runs interactively and asks
    /// per-group; pass --yes / --purge / --keep-data for non-interactive use.
    Uninstall {
        /// Skip prompts; use per-group default decisions
        /// (binary=yes, credentials=no, state=yes).
        #[arg(long)]
        yes: bool,
        /// Wipe ~/.hydra/ entirely.
        #[arg(long, conflicts_with = "keep_data")]
        purge: bool,
        /// Keep ~/.hydra/ entirely (only remove binary + PATH edit).
        #[arg(long)]
        keep_data: bool,
        /// Print the plan; do nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Install seed files (skills/commands/hooks/MCP) to `~/.hydra/`.
    Setup {
        /// Take over a stale lock AND force reinstall even if seeds are already present.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum PluginCli {
    /// Marketplace registry operations (add/remove/update/list).
    #[command(subcommand)]
    Marketplace(MarketplaceCli),
    /// Install a plugin from a registered marketplace.
    /// Spec format: `<plugin>@<marketplace>` (matches the slash command).
    Install {
        /// e.g. `ascend-model-agent-plugin@ascend-model-agent-plugin`
        spec: String,
    },
    /// Uninstall a previously-installed plugin (does not touch its marketplace).
    Uninstall {
        /// e.g. `ascend-model-agent-plugin@ascend-model-agent-plugin`
        spec: String,
    },
    /// List installed plugins.
    List,
}

#[derive(Subcommand)]
enum MarketplaceCli {
    /// Clone a marketplace git repo and register it locally.
    Add {
        /// Git URL (https or ssh) of a marketplace repo.
        url: String,
    },
    /// Drop a registered marketplace. Refuses if any plugin still installed.
    Remove {
        /// Marketplace name (the key shown by `marketplace list`).
        name: String,
    },
    /// Re-pull a registered marketplace and refresh its plugin index.
    Update { name: String },
    /// List registered marketplaces.
    List,
}

#[derive(Subcommand)]
enum McpCli {
    /// Add or replace a stdio MCP server (`mcpServers.<name>` with `command` + `args`)
    Add {
        /// Server key (tools appear as `mcp__<name>__鈥)
        name: String,
        /// Executable and arguments, e.g. `npx @playwright/mcp@latest`
        #[arg(required = true, num_args = 1..)]
        command: Vec<String>,
        /// Write `~/.hydra/mcp.json` instead of `<dir>/.mcp.json`
        #[arg(long)]
        global: bool,
        /// Directory for project `.mcp.json` (defaults to current directory)
        #[arg(short = 'C', long)]
        dir: Option<PathBuf>,
    },
    /// Add GitHub's remote MCP server using OAuth.
    AddGithubOauth {
        /// Server key (tools appear as `mcp__<name>__鈥)
        #[arg(default_value = "github")]
        name: String,
        /// Write `~/.hydra/mcp.json` instead of `<dir>/.mcp.json`
        #[arg(long)]
        global: bool,
        /// Directory for project `.mcp.json` (defaults to current directory)
        #[arg(short = 'C', long)]
        dir: Option<PathBuf>,
    },
    /// Complete OAuth login for a remote MCP server.
    Login {
        /// Server key in mcpServers (for GitHub, usually `github`)
        name: String,
        /// OAuth provider to use.
        #[arg(long, default_value = "github")]
        provider: String,
        /// OAuth client id. Defaults to HYDRA_GITHUB_MCP_CLIENT_ID.
        #[arg(long)]
        client_id: Option<String>,
        /// Environment variable containing the OAuth client secret.
        #[arg(long)]
        client_secret_env: Option<String>,
        /// OAuth scopes. Defaults to GitHub MCP's broad repo-oriented set.
        #[arg(long, value_delimiter = ',')]
        scopes: Vec<String>,
    },
    /// Remove saved OAuth credentials for a remote MCP server.
    Logout {
        /// Server key in mcpServers.
        name: String,
    },
}

#[derive(clap::Subcommand)]
pub enum TelemetryAction {
    /// Show current telemetry state and queue stats
    Status,
    /// Enable telemetry (writes to ~/.hydra/config.toml)
    Enable,
    /// Disable telemetry (writes to ~/.hydra/config.toml)
    Disable,
    /// Print pending queued events (never-sent)
    Dump {
        #[arg(long, default_value_t = 50)]
        last: usize,
        #[arg(long)]
        pretty: bool,
    },
    /// Clear queued events (does not change enabled state)
    Clear,
}

/// Environment variable set by this process for its re-exec'd child, so
/// the child knows which version it was just upgraded from and can show
/// a one-time "鉁?Upgraded to vX.Y.Z" banner on the welcome screen.
/// The child clears this env var after reading it so grandchildren
/// (spawned tools, subprocesses) don't inherit a stale hint.
const UPGRADED_FROM_ENV: &str = "HYDRA_UPGRADED_FROM";

/// Env var the parent sets when spawning a detached upgrade-prep worker.
/// The child detects it at the very top of `main` and runs one
/// `prepare_deferred_upgrade` cycle in its own session (setsid'd) so the
/// parent can be Ctrl+C'd without cancelling the download.
const INTERNAL_PREPARE_UPGRADE_ENV: &str = "HYDRA_INTERNAL_PREPARE_UPGRADE";

#[tokio::main]
async fn main() {
    // Set Windows console to UTF-8 so CJK and other multi-byte characters
    // render correctly instead of showing garbled output (mojibake).
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Globalization::CP_UTF8;
        use windows_sys::Win32::System::Console::{SetConsoleCP, SetConsoleOutputCP};
        unsafe {
            SetConsoleOutputCP(CP_UTF8);
            SetConsoleCP(CP_UTF8);
        }
    }

    // Detached upgrade-prep worker mode. The parent hydra spawns a
    // subprocess with this env var set; that subprocess does one full
    // download + verify + `pending.json` write, then exits. Because the
    // subprocess is setsid'd (see `spawn_detached_upgrade_prep`), it
    // survives Ctrl+C / quit in the parent 鈥?which is the whole point,
    // since the previous in-process download was tied to the parent's
    // tokio runtime and got cancelled on any quick exit.
    if std::env::var(INTERNAL_PREPARE_UPGRADE_ENV).is_ok() {
        let code = run_prepare_upgrade_worker().await;
        std::process::exit(code);
    }

    // If this invocation is the `.bak` backup binary (left behind by a
    // previous upgrade), skip all upgrade bootstrapping. `apply_pending_upgrade`
    // would rewrite ourselves with the latest version and destroy the
    // rollback target; the whole point of keeping `.bak` is for the user
    // to be able to run / keep the old version. The only upgrade path
    // still reachable from a `.bak` launch is the explicit `/upgrade`
    // slash command inside the TUI 鈥?that's user-initiated and fine.
    let is_backup = is_running_as_backup();
    let dev_mode = is_dev_mode();
    if dev_mode {
        eprintln!("[dev] auto-update disabled");
    }

    // Bootstrap: if a prior session staged an upgrade, apply it NOW 鈥?before
    // we spin up tokio, the TUI, or any other heavy state. On success we
    // re-exec the new binary (Unix: same PID; Windows: child+exit). The user
    // sees one continuous "hydra" invocation, just 100-300ms longer than
    // normal. On failure we log and carry on with the current binary; the
    // circuit-breaker in `apply_pending_upgrade` ensures a broken release
    // can't wedge this loop indefinitely.
    if !is_backup && !dev_mode {
        // Capture current version BEFORE applying upgrade, so we can pass it to the re-exec'd child
        let current_version = format!("v{}", env!("CARGO_PKG_VERSION"));
        match hydra_core::self_update::apply_pending_upgrade() {
            Ok(Some(applied)) => {
                eprintln!("鉁?Upgrading to {}...", applied.version);
                // Pass the CURRENT version (before upgrade) to the re-exec'd child so the TUI
                // can surface a welcome-screen confirmation exactly once.
                std::env::set_var(UPGRADED_FROM_ENV, &current_version);
                match hydra_core::self_update::re_exec_self(Some(&applied.exe)) {
                    Ok(_infallible) => unreachable!("re_exec_self returned Ok"),
                    Err(e) => {
                        eprintln!(
                        "Upgrade applied but re-exec failed ({}). The new version will be used on the next launch.",
                        e
                    );
                        std::env::remove_var(UPGRADED_FROM_ENV);
                        std::process::exit(1);
                    }
                }
            }
            Ok(None) => {
                // No pre-staged upgrade. If the user isn't passing `-p` /
                // `--prompt-file` (headless one-shots shouldn't pay the network
                // tax) and auto_update isn't disabled, try to fetch + stage +
                // apply v_next right here. This is the "user launched hydra,
                // wants it upgraded NOW" path 鈥?single invocation instead of
                // the stage-on-session-N / apply-on-session-N+1 dance.
                //
                // Anything goes wrong (offline, timeout, sha mismatch, no
                // newer release) 鈫?silently fall through and continue with
                // the current binary. The `/upgrade` slash command is still
                // there as the explicit/loud alternative.
                if should_try_sync_upgrade() {
                    sync_stage_and_apply_if_newer().await;
                }
            }
            Err(e) => {
                eprintln!("Note: pending upgrade could not be applied ({}). Continuing with current version.", e);
            }
        }
    } // end `if !is_backup`

    // Set a minimal pre-telemetry panic hook (replaced after telemetry init in run()).
    std::panic::set_hook(Box::new(|info| {
        restore_terminal_if_tui();
        eprintln!("\nHydra crashed: {}", info);
        if let Some(location) = info.location() {
            eprintln!(
                "  at {}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            );
        }
        eprintln!("\nPlease report this at: https://atomgit.com/atomgit_atomcode/hydra/issues");
    }));

    match run().await {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            restore_terminal_if_tui();
            eprintln!("\nHydra error: {:#}", e);
            std::process::exit(1);
        }
    }
}

async fn run() -> Result<i32> {
    let cli = Cli::parse();

    // 鈹€鈹€ Telemetry init 鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€
    // Load config early (before subcommand dispatch) so we can read the
    // [telemetry] section. Failure to load config is non-fatal; telemetry
    // will operate on defaults (enabled, built-in endpoint).
    let config_path_for_tel = cli.config.clone().unwrap_or_else(Config::default_path);
    let telemetry_cfg = if config_path_for_tel.exists() {
        Config::load(&config_path_for_tel)
            .map(|c| c.telemetry)
            .unwrap_or_default()
    } else {
        Default::default()
    };
    let hydra_dir = Config::config_dir();
    let cli_override = CliOverride {
        disabled: cli.no_telemetry,
    };
    let resolved = resolve(
        &telemetry_cfg,
        &cli_override,
        hydra_dir.clone(),
        &ProcessEnv,
    );

    // First-run notice: only show when telemetry would be active.
    if resolved.state.is_enabled() {
        if let Ok(true) = notice::should_show_and_mark(&resolved.hydra_dir) {
            eprintln!("{}", notice::NOTICE_TEXT);
        }
    }

    let telemetry = Telemetry::init(resolved, env!("CARGO_PKG_VERSION").into());
    install_panic_hook(telemetry.clone());
    // 鈹€鈹€ End telemetry init 鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€

    // Handle subcommands. Most are self-contained (`handle_command` runs
    // and exits); `Login` (and its hidden alias `Codingplan`) run the
    // full OAuth + CodingPlan setup flow and then fall through to the
    // TUI. `Fixissue` is like headless `-p` but with the prompt
    // synthesised from a remote issue payload 鈥?so we resolve it here
    // and hand it to the agent loop via `fixissue_prompt` below.
    let mut fixissue_prompt: Option<String> = None;
    // Saved when fixissue is parsed, so after the agent finishes we can
    // POST the summary back to AtomGit as a comment + add the `fixed`
    // label. `None` = not a fixissue run, skip the post-back step.
    let mut fixissue_ref: Option<hydra_core::atomgit::IssueRef> = None;
    // `fixissue` is an interactive-feeling structured workflow (the user
    // is watching progress, not piping output). Force verbose so they see
    // tool calls / edits instead of long silences while the agent works.
    let mut force_verbose = false;
    if let Some(cmd) = cli.command {
        match cmd {
            Commands::Login | Commands::Codingplan => {
                // Unified login flow: OAuth (if needed) 鈫?claim 鈫?fetch
                // models 鈫?register providers 鈫?fetch status. Falls
                // through to TUI startup regardless of outcome. On
                // success the freshly saved config.toml is picked up by
                // `Config::load` further down. On failure the TUI opens
                // in onboarding mode (no providers) so the user can
                // retry via `/login` without re-launching the binary.
                // Emits open_hydra (mode=headless) then take_codingplan
                // (emitted internally by run_codingplan_core via coding_plan::run).
                HEADLESS_MODE.store(true, Ordering::Relaxed);
                let repo = hydra_core::telemetry_bootstrap::detect_repo_origin(
                    &std::env::current_dir().unwrap_or_default(),
                );
                telemetry.set_account_id(auth::get_stored_auth().map(|a| a.user.id.to_string()));
                let scope_ctx = CurrentContext {
                    repo_origin: Some(repo),
                    mode: Some(SessionMode::Headless),
                    ..CurrentContext::current()
                };
                let outcome = CurrentContext::scope(scope_ctx, || async {
                    telemetry.track(Event::OpenAtomcode);
                    run_codingplan_core(Some(&telemetry))
                })
                .await;
                match outcome {
                    Ok(report) => {
                        print!("{}", report);
                    }
                    Err(e) => {
                        eprintln!("login setup failed: {:#}", e);
                    }
                }
                println!("\n  Starting Hydra...\n");
                HEADLESS_MODE.store(false, Ordering::Relaxed);
                // Fall through to TUI startup below
            }
            Commands::Fixissue { url } => {
                HEADLESS_MODE.store(true, Ordering::Relaxed);
                let cwd = cli.dir.clone().unwrap_or_else(|| {
                    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
                });
                match hydra_core::atomgit::fixissue::prepare(&url, &cwd) {
                    Ok(hydra_core::atomgit::fixissue::Prepared::Run {
                        prompt,
                        issue_title,
                        issue_number,
                        issue_ref,
                    }) => {
                        eprintln!("[fixissue] issue #{}: {}", issue_number, issue_title);
                        fixissue_prompt = Some(prompt);
                        fixissue_ref = Some(issue_ref);
                        force_verbose = true;
                        // Fall through: agent loop will run this as a headless prompt.
                    }
                    Ok(hydra_core::atomgit::fixissue::Prepared::Skip { reason }) => {
                        eprintln!("{}", reason);
                        return Ok(0);
                    }
                    Err(e) => {
                        eprintln!("fixissue failed: {:#}", e);
                        return Ok(1);
                    }
                }
            }
            Commands::Daemon { port, client } => {
                HEADLESS_MODE.store(true, Ordering::Relaxed);
                eprintln!("Starting Hydra daemon on port {}...", port);
                eprintln!("Press Ctrl+C to stop.");
                // Re-exec into the hydra-daemon binary with matching port.
                // This keeps the daemon as a separate compilation unit while
                // providing a user-friendly `hydra daemon` subcommand.
                let daemon_bin = std::env::current_exe().ok().and_then(|p| {
                    let dir = p.parent()?;
                    let daemon = dir.join("hydra-daemon");
                    if daemon.exists() {
                        Some(daemon)
                    } else {
                        None
                    }
                });
                match daemon_bin {
                    Some(bin) => {
                        let mut cmd = std::process::Command::new(bin);
                        cmd.arg("--port").arg(port.to_string());
                        if let Some(ref c) = client {
                            cmd.arg("--client").arg(c);
                        }
                        let status = cmd
                            .status()
                            .context("Failed to start hydra-daemon")?;
                        return Ok(if status.success() { 0 } else { 1 });
                    }
                    None => {
                        eprintln!("Error: hydra-daemon binary not found next to hydra.");
                        eprintln!("Make sure both binaries are installed together.");
                        return Ok(1);
                    }
                }
            }
            Commands::Telemetry { action } => {
                HEADLESS_MODE.store(true, Ordering::Relaxed);
                let config_file_path = Config::default_path();
                match action {
                    TelemetryAction::Status => {
                        telemetry_cmd::status(&hydra_dir, &telemetry_cfg)?
                    }
                    TelemetryAction::Enable => telemetry_cmd::enable(&config_file_path)?,
                    TelemetryAction::Disable => {
                        telemetry_cmd::disable(&config_file_path, &telemetry).await?
                    }
                    TelemetryAction::Dump { last, pretty } => {
                        telemetry_cmd::dump(&hydra_dir, last, pretty)?
                    }
                    TelemetryAction::Clear => telemetry_cmd::clear(&hydra_dir)?,
                }
                return Ok(0);
            }
            Commands::Setup { force } => {
                HEADLESS_MODE.store(true, Ordering::Relaxed);
                let exit_code = run_setup_command(force);
                telemetry
                    .shutdown(std::time::Duration::from_millis(500))
                    .await;
                return Ok(exit_code);
            }
            other => {
                let result = handle_command(other, &telemetry).await.map(|_| 0);
                // Flush any events emitted by the subcommand (e.g. login_success)
                // before the process exits. Bounded by the same 500ms budget as
                // other exit paths.
                telemetry
                    .shutdown(std::time::Duration::from_millis(500))
                    .await;
                return result;
            }
        }
    }

    // Default: start TUI

    let config_path = cli.config.clone().unwrap_or_else(Config::default_path);

    let mut config = if config_path.exists() {
        Config::load(&config_path).unwrap_or_else(|e| {
            eprintln!("Warning: failed to load config ({}), using defaults", e);
            Config {
                default_provider: String::new(),
                default_workdir: None,
                providers: HashMap::new(),
                datalog: Default::default(),
                notifications: Default::default(),
                auto_update: true,
                telemetry: Default::default(),
                lsp: Default::default(),
                auto_commit: false,
                subagent: Default::default(),
                vision_preprocessor_provider: None,
                language: None,
                ui: Default::default(),
            plugin: Default::default(),
            }
        })
    } else {
        // No config yet 鈥?TUI Welcome screen will guide first-run setup
        Config {
            default_provider: String::new(),
            default_workdir: None,
            providers: HashMap::new(),
            datalog: Default::default(),
            notifications: Default::default(),
            auto_update: true,
            telemetry: Default::default(),
            lsp: Default::default(),
            auto_commit: false,
            subagent: Default::default(),
            vision_preprocessor_provider: None,
            language: None,
            ui: Default::default(),
            plugin: Default::default(),
        }
    };

    // 鈹€鈹€ i18n locale 鈹€鈹€
    let locale = hydra_tuix::i18n::resolve_initial_locale(
        cli.lang.as_deref(),
        config.language,
    );
    hydra_tuix::i18n::set_locale(locale);

    // 鈹€鈹€ Plugin marketplace bootstrap + post-upgrade refresh 鈹€鈹€
    //
    // Two best-effort hooks (auto-install default skills marketplace
    // on first startup, `git pull` every installed marketplace after a
    // self-upgrade) used to fire here synchronously, blocking the
    // input box for 1鈥?s on a warm path (and 5鈥?0s on first clone).
    // Both now run as a detached `spawn_blocking` from inside
    // `hydra_tuix::run` after the skill registry is constructed 鈥?
    // see lib.rs near `spawn_plugin_bootstrap`. Newly-installed skills
    // are picked up by a `skill_registry.reload()` + wake pulse the
    // background task fires on completion, so the slash menu refreshes
    // without a restart.

    let unavailable_reason = if config.providers.is_empty() {
        Some(hydra_tuix::i18n::t(hydra_tuix::i18n::Msg::CmdNoActiveProvider).into_owned())
    } else {
        None
    };

    /// Build the placeholder `ProviderConfig` used when no real provider is
    /// available.  The TUI still boots 鈥?the Welcome wizard / status-row
    /// hints nudge the user to `/login`, and a successful
    /// auth flow rebuilds the real provider via `rebuild_provider`.
    fn dummy_provider_config() -> (ProviderConfig, String) {
        (
            ProviderConfig {
                provider_type: "openai".to_string(),
                api_key: Some("unavailable".to_string()),
                model: String::new(),
                base_url: None,
                system_prompt: None,
                user_agent: None,
                context_window: default_context_window_for("openai"),
                max_tokens: None,
                thinking_type: None,
                thinking_keep: None,
                reasoning_history: None,
                thinking_enabled: None,
                thinking_budget: None,
                skip_tls_verify: false,
                ephemeral: false,
            },
            String::new(),
        )
    }

    let (provider_config, model_name) = if unavailable_reason.is_some() {
        dummy_provider_config()
    } else {
        if let Some(ref model) = cli.model {
            let provider_name = cli.provider.as_deref().unwrap_or(&config.default_provider);
            if let Some(p) = config.providers.get_mut(provider_name) {
                p.model = model.clone();
            }
        }
        // Keep api_key as None here so `create_provider()` auto-loads
        // from `~/.hydra/auth.toml`. Setting "not-configured" would
        // bypass that path and force the user to manually provide a key.
        //
        // `active_provider` already falls back to the first available
        // provider when `default_provider` points to a deleted section,
        // but as a defence-in-depth measure we catch any remaining
        // errors and swap in the dummy so the TUI still boots.
        match config.active_provider(cli.provider.as_deref()) {
            Ok(pc) => {
                let name = pc.model.clone();
                (pc.clone(), name)
            }
            Err(e) => {
                eprintln!(
                    "Warning: could not resolve active provider ({}). \
                     Launching TUI in onboarding mode 鈥?use /login to set up.",
                    e
                );
                dummy_provider_config()
            }
        }
    };

    // `create_provider` may need to load an OAuth token from
    // `~/.hydra/auth.toml`. Pre-v4.20 this was a fatal startup
    // error 鈥?if the user had a config.toml with an `AtomGit*` entry
    // (from an older `/login` that auto-registered one) but no
    // auth.toml (fresh machine, or auth.toml was deleted), the CLI
    // bailed before the TUI could load, leaving the user stuck:
    // they wanted to `/login` but couldn't start the app to run it.
    //
    // Graceful fallback: if provider construction fails because the
    // token is unavailable, swap in the same dummy used on first-run
    // so the TUI boots. The Welcome-wizard / status-row hints will
    // nudge the user to `/login`, and a successful
    // auth flow rebuilds the real provider via `rebuild_provider`.
    let (provider, model_name) = if let Some(reason) = unavailable_reason {
        (unavailable_provider(reason), model_name)
    } else {
        match create_provider(&provider_config) {
            Ok(p) => (p, model_name),
            Err(e) => {
                let msg = format!("{:#}", e);
                if is_auth_gap_error(&msg) {
                    eprintln!(
                        "Note: provider credentials not available ({}). \
                         Launching TUI in onboarding mode - use /login to set up.",
                        msg
                    );
                    (
                        unavailable_provider(format!(
                            "Provider credentials unavailable: {}. Please use /login to complete setup and try again.",
                            msg
                        )),
                        String::new(),
                    )
                } else {
                    return Err(e);
                }
            }
        }
    };

    let working_dir = resolve_working_dir(cli.dir.clone());

    // Build the disabled-tool set from --disable-tools (CLI) merged with the
    // HYDRA_DISABLE_TOOLS env var. The env var allows the SWE-bench
    // harness to opt-out of bash without rebuilding hydra or threading a
    // CLI flag through every shell wrapper.
    let mut disabled_tools: std::collections::HashSet<String> = cli
        .disable_tools
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if let Ok(env_list) = std::env::var("HYDRA_DISABLE_TOOLS") {
        for name in env_list
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            disabled_tools.insert(name.to_string());
        }
    }
    if !disabled_tools.is_empty() {
        let mut sorted: Vec<&String> = disabled_tools.iter().collect();
        sorted.sort();
        eprintln!(
            "[hydra] tools disabled: {}",
            sorted
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let enabled = |name: &str| !disabled_tools.contains(name);

    let mut tool_registry = ToolRegistry::new();
    if enabled("read_file") {
        tool_registry.register_sync(Box::new(ReadFileTool));
    }
    if enabled("write_file") {
        tool_registry.register_sync(Box::new(WriteFileTool));
    }
    if enabled("edit_file") {
        tool_registry.register_sync(Box::new(EditFileTool));
    }
    if enabled("bash") {
        tool_registry.register_sync(Box::new(BashTool));
    }
    // change_dir is opt-in: weak models (e.g. deepseek-v4-flash) repeatedly
    // emit empty `arguments: {}` for it, looping the same broken call until
    // the identical-args guard blocks it. Bash with `cd` already covers the
    // legitimate use case (hydra tracks `cd` in bash and mutates
    // `working_dir` accordingly), and `/cd` lets the user switch manually.
    // Set HYDRA_ENABLE_CD=1 to re-expose the tool to the LLM.
    if enabled("change_dir") && std::env::var("HYDRA_ENABLE_CD").is_ok() {
        tool_registry.register_sync(Box::new(CdTool));
    }
    if enabled("grep") {
        tool_registry.register_sync(Box::new(GrepTool));
    }
    if enabled("glob") {
        tool_registry.register_sync(Box::new(GlobTool));
    }
    if enabled("list_directory") {
        tool_registry.register_sync(Box::new(ListDirTool));
    }
    if enabled("web_search") {
        tool_registry.register_sync(Box::new(WebSearchTool));
    }
    if enabled("web_fetch") {
        tool_registry.register_sync(Box::new(WebFetchTool));
    }
    if enabled("search_replace") {
        tool_registry.register_sync(Box::new(SearchReplaceTool));
    }
    if enabled("open_file") {
        tool_registry.register_sync(Box::new(OpenFileTool));
    }

    // Determine if we're running in headless mode BEFORE loading MCP.
    // Headless mode requires MCP tools immediately; TUI can load them in background.
    let is_headless =
        cli.prompt.is_some() || cli.prompt_file.is_some() || fixissue_prompt.is_some();

    // Load MCP tools from .mcp.json (project) and ~/.hydra/mcp.json (user).
    // For TUI mode, start connections in background to avoid blocking startup.
    // For headless mode (-p/--prompt-file/fixissue), wait for connections since tools
    // are needed immediately.
    let (mcp_registry, mcp_connect_rx) = if is_headless {
        // Headless: need tools right now, wait for connections
        let registry = McpRegistry::from_config(&working_dir).await;
        let mcp_tools = registry.list_all_tools().await;
        let mcp_registry = if !mcp_tools.is_empty() {
            let mcp_registry = std::sync::Arc::new(registry);
            register_mcp_tools(&mut tool_registry, mcp_registry.clone(), mcp_tools);
            Some(mcp_registry)
        } else {
            None
        };
        (mcp_registry, None)
    } else {
        // TUI: start in background, tools populate as servers connect
        // Create event channel so TUI can display connection status in scrollback
        use hydra_core::mcp::McpConnectEvent;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<McpConnectEvent>();
        let registry = McpRegistry::from_config_background_with_events(&working_dir, Some(tx));
        let mcp_registry = std::sync::Arc::new(registry);
        // Don't wait for tools - they'll be registered dynamically as servers connect
        (Some(mcp_registry), Some(rx))
    };

    // Build LSP manager from config and inject into ToolContext.
    // TUI mode uses the event-channel constructor so server start /
    // failure surfaces in scrollback (鉁?鉁?lines) instead of being
    // eprintln!'d directly to stderr 鈥?which would land inside the
    // input box while the renderer owns the terminal. Headless keeps
    // the no-channel path: stderr leakage doesn't matter when no TUI
    // is active and CI logs benefit from raw error visibility.
    let (lsp_manager, lsp_connect_rx) = if is_headless {
        (build_lsp_manager(&config.lsp, &working_dir), None)
    } else {
        match hydra_core::lsp::build_lsp_manager_with_events(&config.lsp, &working_dir) {
            Some((mgr, rx)) => (Some(mgr), Some(rx)),
            None => (None, None),
        }
    };
    if lsp_manager.is_some() && enabled("diagnostics") {
        tool_registry.register_sync(Box::new(DiagnosticsTool));
    }

    // Pass the already-initialized telemetry handle into ToolContext.
    let mut tool_context =
        ToolContext::with_telemetry(working_dir.clone(), "default", telemetry.clone());
    tool_context.lsp = lsp_manager;

    // Continue the previous session only when the user explicitly opts
    // in via `-c` / `--continue`. Bare `hydra` starts a fresh
    // session 鈥?no auto-resume, no scrollback replay. Users who want to
    // pick a specific older session can still use `/resume` inside the
    // TUI.
    let session_to_continue = if cli.continue_last {
        let session_manager = SessionManager::new(&working_dir);
        match session_manager.latest() {
            Ok(Some(session)) => Some(session),
            _ => None,
        }
    } else {
        None
    };

    // Start with a fresh conversation each session. When `session_to_continue`
    // is present (via `-c` / `--continue`), the TUI replays the prior session's
    // messages into scrollback AND sends them to the agent via
    // `AgentCommand::SetMessages` so the model context is fully restored.
    // Bare `hydra` (no `-c`) starts completely fresh.
    let conversation = Conversation::new();

    let (mut agent_loop, agent_handle) = AgentLoop::new(
        config.clone(),
        provider,
        tool_registry,
        tool_context.clone(),
        conversation,
    );
    agent_loop.set_max_turns(cli.max_turns);
    let runtime_factory = AgentRuntimeFactory::from_initial_loop(&agent_loop, cli.max_turns);

    // Resolve effective prompt: --prompt-file reads from disk; -p is inline;
    // `fixissue` synthesises one from the AtomGit issue body. fixissue takes
    // precedence 鈥?when it's set we've already committed to headless mode.
    // clap's conflicts_with ensures `-p` and `--prompt-file` can't both be given.
    let effective_prompt: Option<String> = if let Some(p) = fixissue_prompt.take() {
        Some(p)
    } else {
        match (cli.prompt.as_ref(), cli.prompt_file.as_ref()) {
            (Some(p), None) => Some(p.clone()),
            (None, Some(path)) => match std::fs::read_to_string(path) {
                Ok(s) => Some(s),
                Err(e) => {
                    eprintln!(
                        "error: failed to read --prompt-file {}: {}",
                        path.display(),
                        e
                    );
                    std::process::exit(2);
                }
            },
            (None, None) => None,
            (Some(_), Some(_)) => unreachable!("clap conflicts_with prevents this"),
        }
    };

    // Build the session-scope context: repo_origin, mode.
    // session_id and account_id are managed on Telemetry directly via
    // set_session_id() / set_account_id(). Seed account_id from stored auth so
    // events from this session correlate to the user even before any explicit
    // login action this run; login()/logout() update it later as needed.
    // mode: Headless when a prompt is supplied (-p / --prompt-file / fixissue);
    //       Tui when the user launches the interactive terminal UI.
    let repo = hydra_core::telemetry_bootstrap::detect_repo_origin(
        &std::env::current_dir().unwrap_or_else(|_| working_dir.clone()),
    );
    telemetry.set_account_id(auth::get_stored_auth().map(|a| a.user.id.to_string()));
    let session_mode = if effective_prompt.is_some() {
        SessionMode::Headless
    } else {
        SessionMode::Tui
    };
    // Bind telemetry session_id to the Hydra session's UUID (if a session is available).
    // This means events from this process run are correlated to the actual session file.
    // If no session exists yet, session_id stays as launch_id (the default).
    if let Some(ref s) = session_to_continue {
        if let Ok(uuid) = uuid::Uuid::parse_str(s.id.as_str()) {
            telemetry.set_session_id(uuid);
        }
    }
    let scope_ctx = CurrentContext {
        repo_origin: Some(repo),
        mode: Some(session_mode),
        ..CurrentContext::current()
    };

    let result = CurrentContext::scope(scope_ctx, || async {
        // Emit open_hydra once at agent-flow entry. Meta-commands
        // (--version, --help, --update, login, logout, status, upgrade,
        // rollback, telemetry) return via handle_command before reaching
        // this point and must NOT emit open_hydra.
        telemetry.track(Event::OpenAtomcode);

        // Headless mode: -p / --prompt-file triggers non-interactive execution.
        // `fixissue` also sets `force_verbose` so tool activity is visible 鈥?the
        // user is watching a single long-running task, not feeding a pipe.
        let exit_code = if let Some(prompt) = effective_prompt {
            let verbose = cli.verbose || force_verbose;
            // Capture the assistant's streamed text only when we need to post
            // it back to AtomGit (fixissue). Plain `-p` stays zero-alloc.
            let capture = fixissue_ref.is_some();
            let (ec, captured) = run_headless(
                agent_loop,
                agent_handle,
                prompt,
                cli.provider.as_deref(),
                verbose,
                capture,
                working_dir.clone(),
            )
            .await?;

            // Post-run side effects for fixissue: only on clean completion
            // (exit 0 = TurnComplete Natural; 1 = error; 2 = denial; 130 = cancel).
            // On non-zero we leave the issue alone 鈥?the user can retry.
            if let Some(issue_ref) = fixissue_ref {
                if ec == 0 {
                    if let Some(summary) = captured.filter(|s| !s.trim().is_empty()) {
                        match hydra_core::atomgit::fixissue::post_completion(&issue_ref, &summary) {
                            Ok(()) => eprintln!(
                                "[fixissue] posted summary + applied fixed label to issue #{}",
                                issue_ref.number
                            ),
                            Err(e) => eprintln!(
                                "[fixissue] post-back failed (local fix is still saved): {:#}",
                                e
                            ),
                        }
                    } else {
                        eprintln!(
                            "[fixissue] agent produced no text; skipping comment + label on issue #{}",
                            issue_ref.number
                        );
                    }
                } else {
                    eprintln!(
                        "[fixissue] agent exited non-zero ({}); skipping comment + label on issue #{}",
                        ec, issue_ref.number
                    );
                }
            }
            Ok::<i32, anyhow::Error>(ec)
        } else {
            // Fire-and-forget: spawn a setsid'd subprocess to stage the next
            // release if one is out. Detached so a Ctrl+C in this parent doesn't
            // also kill the download 鈥?that was the whole reason "exit and come
            // back" wasn't picking up v_next on short sessions. Only armed when
            // the user hasn't opted out via `auto_update = false` AND we're not
            // running as `hydra.bak` (backup should stay pinned; see the
            // `is_running_as_backup` guard up top).
            if config.auto_update && !is_running_as_backup() && !cli.dev {
                spawn_detached_upgrade_prep();
            }

            // Redirect fd 2 鈫?$HYDRA_HOME/stderr.log before the TUI takes
            // ownership of the terminal. NSPasteboard deprecation warnings
            // (arboard clipboard polling, ~1.5 s interval) and any other
            // rogue C-lib stderr writes would otherwise land at the raw-mode
            // cursor position, painting into the input box.
            //
            // Only fires here 鈥?the TUI branch. Headless (-p/--prompt-file)
            // leaves stderr pointing at the real terminal so the user sees
            // actual errors in their shell/CI output.
            redirect_stderr_to_log_file();

            let ctx = hydra_telemetry::CurrentContext::current();
            tokio::spawn(async move {
                hydra_telemetry::CurrentContext::scope(ctx, || agent_loop.run()).await
            });
            hydra_tuix::run(config, model_name, agent_handle, runtime_factory, working_dir, session_to_continue, mcp_registry, mcp_connect_rx, lsp_connect_rx, telemetry.clone()).await?;
            Ok(0)
        };

        telemetry.shutdown(std::time::Duration::from_millis(500)).await;
        exit_code
    })
    .await;

    result
}

/// On macOS the NSPasteboard runtime prints deprecation warnings to
/// stderr when arboard calls into AppKit (via clipboard polling for
/// the "ctrl+v to paste image" hint). In raw mode, stderr shares the
/// TTY with the TUI paint stream, so those warnings paint into the
/// input box at whatever cursor row happens to be active. Other libs
/// (LSP, MCP shells) can leak the same way.
///
/// Redirect fd 2 to `$HYDRA_HOME/stderr.log` once we know we're
/// entering interactive TUI mode. plain / headless / piped paths
/// don't call this 鈥?they want stderr to reach the terminal so the
/// user sees real errors.
///
/// Best-effort: if the home dir can't be created or the file can't
/// be opened, do nothing and let stderr leak (the original bug); we
/// don't want to take down hydra startup because logging failed.
#[cfg(unix)]
fn redirect_stderr_to_log_file() {
    use std::os::unix::io::AsRawFd;
    let Some(home) = std::env::var_os("HYDRA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".hydra")))
    else {
        return;
    };
    if std::fs::create_dir_all(&home).is_err() {
        return;
    }
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(home.join("stderr.log"))
    else {
        return;
    };
    // Write a session marker so users can see in stderr.log where
    // each hydra session starts 鈥?helps separate one run's noise
    // from another's when grepping for actual problems.
    // Use epoch seconds (std::time only 鈥?no chrono dep needed).
    let epoch_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let marker = format!("\n--- hydra session start (unix={epoch_secs}) ---\n");
    let _ = std::io::Write::write_all(
        &mut std::io::BufWriter::new(&file),
        marker.as_bytes(),
    );
    // SAFETY: dup2 swaps the file descriptor table entry for fd 2
    // to point at `file`'s underlying fd. This is a standard, safe
    // operation; the worst case (dup2 fails) is the redirect doesn't
    // happen and we log nothing 鈥?same as the no-redirect baseline.
    unsafe {
        libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO);
    }
    // Intentionally keep `file` alive via the dup2 鈥?the kernel
    // holds a reference to the underlying inode, so even after
    // `file` is dropped, fd 2 stays pointing at the same file.
    // No need to std::mem::forget.
}

#[cfg(not(unix))]
fn redirect_stderr_to_log_file() {
    // Windows: NSPasteboard is mac-only; arboard on Windows uses
    // OpenClipboard which doesn't NSLog. Not a known leak path.
    // No-op for now; revisit if a similar Windows issue surfaces.
}

/// Run agent in headless mode (pipe-friendly: stdout = LLM text only,
/// logs/diagnostics 鈫?stderr). Non-interactive: `bash` approvals are
/// auto-allowed (stderr logs the reason); other tools that require approval
/// are still denied.
///
/// `verbose=false` (default): Claude Code -p style 鈥?only the assistant reply
/// reaches the user. Tool calls, token usage, and turn summary are silent.
/// Errors, approval denials, and cancellations are still surfaced on stderr.
///
/// `verbose=true`: also emit tool calls, token usage, [done] summary, working
/// dir changes, and sub-agent progress on stderr.
async fn run_headless(
    agent_loop: AgentLoop,
    agent_handle: hydra_core::agent::AgentHandle,
    prompt: String,
    _provider_name: Option<&str>,
    verbose: bool,
    capture: bool,
    working_dir: PathBuf,
) -> Result<(i32, Option<String>)> {
    // Tell the panic hook / error path to skip TUI cleanup 鈥?raw mode was
    // never enabled here, so `disable_raw_mode` would be a wasted ioctl
    // (and on Windows can panic when stdin isn't a real console handle).
    HEADLESS_MODE.store(true, Ordering::Relaxed);

    let notifications = agent_loop.config.notifications.clone();
    let (cmd_tx, mut event_rx) = {
        let handle = agent_handle;
        (handle.client.cmd_tx, handle.event_rx)
    };

    let ctx = hydra_telemetry::CurrentContext::current();
    tokio::spawn(async move {
        hydra_telemetry::CurrentContext::scope(ctx, || agent_loop.run()).await
    });
    cmd_tx.send(AgentCommand::SendMessage {
        text: prompt,
        images: vec![],
        image_markers: vec![],
    })?;

    let mut exit_code: i32 = 0;
    let mut had_denial = false;
    let mut last_text_ended_with_newline = true;
    // Tracks whether we're inside a streaming `[thinking]` line. When true, the
    // next non-reasoning event must close the line with a `\n` so subsequent
    // log lines don't glue onto the tail of the thinking text.
    let mut thinking_line_open = false;
    // When `capture` is set, we also buffer every TextDelta the agent
    // emits so the caller (e.g. the fixissue workflow) can post the
    // full assistant output back to AtomGit as an issue comment.
    let mut captured: Option<String> = if capture { Some(String::new()) } else { None };

    // Helper: close any in-flight `[thinking]` line before emitting a new log
    // line on stderr. Avoids `[thinking] ...[tool鈫?...]` mashups in verbose.
    // Thin wrapper over the unit-tested `close_thinking_chunk` that flushes
    // directly to stderr.
    fn close_thinking_line(open: &mut bool) {
        let mut buf = String::new();
        close_thinking_chunk(&mut buf, open);
        if !buf.is_empty() {
            eprint!("{}", buf);
            let _ = io::stderr().flush();
        }
    }

    while let Some(event) = event_rx.recv().await {
        match event {
            AgentEvent::TextDelta(text) => {
                close_thinking_line(&mut thinking_line_open);
                if !text.is_empty() {
                    last_text_ended_with_newline = text.ends_with('\n');
                }
                if let Some(buf) = captured.as_mut() {
                    buf.push_str(&text);
                }
                print!("{}", text);
                io::stdout().flush()?;
            }
            AgentEvent::ReasoningDelta(text) => {
                // In CLI verbose mode, show reasoning/thinking content.
                // Reasoning arrives as many tiny streaming chunks (often a
                // single token). The old implementation used `eprintln!` per
                // chunk, producing "one word per line" output. We now stream
                // chunks onto a single `[thinking] ...` line via the
                // unit-tested `format_thinking_chunk` helper.
                if verbose && !text.is_empty() {
                    let mut buf = String::new();
                    format_thinking_chunk(&mut buf, &mut thinking_line_open, &text);
                    eprint!("{}", buf);
                    let _ = io::stderr().flush();
                }
            }
            AgentEvent::ToolCallStreaming { name, hint } => {
                if verbose {
                    close_thinking_line(&mut thinking_line_open);
                    let detail = if hint.is_empty() {
                        String::new()
                    } else {
                        format!(" 鈫?{}", hint)
                    };
                    eprintln!("[tool-streaming鈫?{}{}]", name, detail);
                }
            }
            AgentEvent::ToolCallStarted {
                id: _,
                name,
                arguments,
            } => {
                if verbose {
                    close_thinking_line(&mut thinking_line_open);
                    let args = truncate_log_line(&arguments, 200);
                    eprintln!("[tool鈫?{} args={}]", name, args);
                }
            }
            AgentEvent::ToolOutputChunk { call_id: _, chunk } => {
                if verbose {
                    close_thinking_line(&mut thinking_line_open);
                    eprint!("{}", chunk);
                    let _ = io::stderr().flush();
                }
            }
            AgentEvent::ToolCallResult {
                call_id: _,
                name,
                output,
                success,
                duration,
            } => {
                if verbose {
                    close_thinking_line(&mut thinking_line_open);
                    let status = if success { "OK" } else { "FAILED" };
                    let dur_ms = duration.as_millis();
                    let trimmed = output.trim_end();
                    if trimmed.is_empty() {
                        eprintln!("[tool鈫?{} {} {}ms]", name, status, dur_ms);
                    } else {
                        let snippet = truncate_log_line(trimmed, 500);
                        eprintln!("[tool鈫?{} {} {}ms] {}", name, status, dur_ms, snippet);
                    }
                }
            }
            AgentEvent::ApprovalNeeded {
                tool_name, reason, ..
            } => {
                close_thinking_line(&mut thinking_line_open);
                if tool_name == "bash" {
                    // -p / headless cannot prompt; user opts in by using non-interactive mode.
                    eprintln!("[headless] auto-approved bash: {}", reason);
                    cmd_tx.send(AgentCommand::ApproveTool)?;
                } else {
                    // Always shown 鈥?security signal must not be silent.
                    eprintln!("[approval-denied] tool={} reason={}", tool_name, reason);
                    cmd_tx.send(AgentCommand::DenyTool)?;
                    had_denial = true;
                }
            }
            AgentEvent::TokenUsage(usage) => {
                if verbose {
                    close_thinking_line(&mut thinking_line_open);
                    eprintln!(
                        "[tokens] prompt={} completion={}",
                        usage.prompt_tokens, usage.completion_tokens
                    );
                }
            }
            AgentEvent::PhaseChange(_) => {
                // Silent in headless mode (in both default and verbose).
            }
            AgentEvent::TurnComplete {
                duration,
                total_tokens,
                turn_count,
                tool_call_count,
                stop_reason,
                messages: _,
            } => {
                close_thinking_line(&mut thinking_line_open);
                hydra_core::notify::notify_turn_finished(
                    &notifications,
                    hydra_core::notify::TurnNotification {
                        duration,
                        turn_count,
                        tool_call_count,
                        total_tokens: Some(total_tokens),
                        stop_reason,
                        working_dir: Some(&working_dir),
                    },
                );
                // Always ensure stdout ends with a newline so downstream parsers see a clean line.
                if !last_text_ended_with_newline {
                    println!();
                    io::stdout().flush()?;
                }
                if verbose {
                    // Natural completion stays silent on the stop reason to
                    // preserve the familiar Claude Code -p [done] format.
                    // Budget-enforced / error / cancel truncation gets an
                    // explicit `stopped=<tag>` suffix so eval runners and
                    // humans can tell "natural end" from "we hit a limit".
                    let suffix = match stop_reason {
                        hydra_core::agent::TurnStopReason::Natural => String::new(),
                        other => format!(" stopped={}", other.as_tag()),
                    };
                    eprintln!(
                        "[done] {:.1}s tokens={} turns={} tool_calls={}{}",
                        duration.as_secs_f64(),
                        total_tokens,
                        turn_count,
                        tool_call_count,
                        suffix
                    );
                }
                let _ = cmd_tx.send(AgentCommand::Shutdown);
                break;
            }
            AgentEvent::TurnCancelled { .. } => {
                close_thinking_line(&mut thinking_line_open);
                // Always shown 鈥?user needs to know cancellation happened.
                eprintln!("[cancelled]");
                exit_code = 130;
                let _ = cmd_tx.send(AgentCommand::Shutdown);
                break;
            }
            AgentEvent::Error { error, messages: _ } => {
                close_thinking_line(&mut thinking_line_open);
                // Always shown 鈥?errors are not noise.
                eprintln!("[error] {}", error);
                exit_code = 1;
                let _ = cmd_tx.send(AgentCommand::Shutdown);
                break;
            }
            AgentEvent::Warning(w) => {
                // Headless CLI: warnings go to stderr always (they're
                // meant to be loud). No exit-code change, no shutdown 鈥?
                // we expect the turn to keep running.
                eprintln!("[warning] {}", w);
            }
            AgentEvent::WorkingDirChanged(new_dir) => {
                if verbose {
                    eprintln!("[cwd] {}", new_dir.display());
                }
            }
            AgentEvent::ContextStats { .. } => {
                // Silent in headless mode
            }
            AgentEvent::ToolBatchStarted { batch_id: _, calls } => {
                if verbose {
                    eprintln!("[tool-batch] {} calls in parallel", calls.len());
                }
            }
            AgentEvent::ToolBatchCompleted {
                batch_id: _,
                ok,
                total,
                elapsed_ms,
            } => {
                if verbose {
                    eprintln!(
                        "[tool-batch] completed {}/{} ok in {}ms",
                        ok, total, elapsed_ms
                    );
                }
            }
            AgentEvent::SubAgentDispatchStart { tasks } => {
                if verbose {
                    eprintln!("[sub-agent] dispatching {} in parallel", tasks.len());
                    for (i, t) in tasks.iter().enumerate() {
                        eprintln!("[sub-agent {}] {}{}", i, t.path, t.dedup_suffix);
                    }
                }
            }
            AgentEvent::SubAgentDispatchEnd => {
                if verbose {
                    eprintln!("[sub-agent] dispatch complete");
                }
            }
            AgentEvent::SubAgentTaskStarted { index } => {
                if verbose {
                    eprintln!("[sub-agent {}] running", index);
                }
            }
            AgentEvent::SubAgentTaskDone {
                index,
                elapsed_ms,
                turns,
                summary: _,
            } => {
                if verbose {
                    eprintln!(
                        "[sub-agent {}] done {}s 路 {}T",
                        index,
                        elapsed_ms / 1000,
                        turns
                    );
                }
            }
            AgentEvent::SubAgentTaskFailed {
                index,
                elapsed_ms,
                turns: _,
                reason,
            } => {
                if verbose {
                    eprintln!(
                        "[sub-agent {}] failed {}s 路 {}",
                        index,
                        elapsed_ms / 1000,
                        reason.lines().next().unwrap_or("")
                    );
                }
            }
            AgentEvent::BackgroundComplete {
                summary,
                files_edited,
                turns,
                success,
            } => {
                let status = if success { "ok" } else { "fail" };
                eprintln!("[background {} turns={}] {}", status, turns, summary);
                if verbose && !files_edited.is_empty() {
                    eprintln!("[background files={}]", files_edited.join(","));
                }
            }
            // VL preprocessor failure restores pending image bytes for the
            // TUI to re-attach. CLI has no interactive input buffer to put
            // them in, so just ignore 鈥?the failure itself was already
            // surfaced as AgentEvent::Warning above, and the conversation
            // proceeds with the placeholder. No retry path exists in CLI.
            AgentEvent::RestorePendingImages { .. } => {}
            // VL preprocessor success notice. Mirror the TUI behaviour
            // briefly to stderr so non-interactive users (CI, scripts)
            // still see that VL ran. Char count helps spot degenerate
            // outputs.
            AgentEvent::VisionPreprocessSuccess { vl_key, char_count } => {
                eprintln!("[vl-preprocess ok provider={} chars={}]", vl_key, char_count);
            }
            AgentEvent::MessagesSync { .. } => {
                // Only used by TUI for /bg session persistence; ignore in CLI.
            }
        }
    }

    // Priority: Error(1) > Denial(2) > 0; TurnCancelled(130) is absolute.
    if exit_code == 0 && had_denial {
        exit_code = 2;
    }

    Ok((exit_code, captured))
}

/// Drive `hydra_core::setup::run` end-to-end and return the CLI exit code
/// (0 on success, 1 on any setup error). `setup::run` is synchronous; we
/// run it directly since `Commands::Setup` already runs outside the TUI loop.
fn run_setup_command(force: bool) -> i32 {
    use hydra_core::setup;

    let project_root = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("setup error: cannot read current directory: {e}");
            return 1;
        }
    };
    let mut opts = setup::RunOptions::new(project_root);
    opts.force = force;

    match setup::run(opts) {
        Ok(report) => {
            println!("{}", report.render_cli());
            0
        }
        Err(e) => {
            eprintln!("setup error: {e}");
            1
        }
    }
}

/// Handle subcommands (login, logout, status)
async fn handle_command(cmd: Commands, telemetry: &std::sync::Arc<Telemetry>) -> Result<()> {
    // Subcommands never enter TUI, so tell the panic hook to skip terminal
    // cleanup 鈥?otherwise `disable_raw_mode` panics on Windows with
    // "initial console mode not set" because raw mode was never enabled.
    HEADLESS_MODE.store(true, Ordering::Relaxed);

    match cmd {
        Commands::Login => {
            // `run()` intercepts Login (and its Codingplan alias) before
            // handle_command is called, running the full OAuth + setup
            // flow and falling through to the TUI. This arm is
            // unreachable in normal execution but kept defensive.
            unreachable!("Login is handled inline in run() before handle_command")
        }
        Commands::Logout => {
            auth::logout()?;
            telemetry.set_account_id(None);
            println!("  You have been logged out.");
            Ok(())
        }
        Commands::Status => {
            if let Some(auth) = auth::get_stored_auth() {
                println!(
                    "\n  Logged in as: {} ({})",
                    auth.user.username, auth.user.id
                );
                if let Some(name) = auth.user.name {
                    println!("  Name: {}", name);
                }
                if let Some(email) = auth.user.email {
                    println!("  Email: {}", email);
                }
                println!("  Auth file: {}\n", auth::auth_file_path().display());
            } else {
                println!("\n  Not logged in.");
                println!("  Run 'hydra login' to authenticate.\n");
            }
            Ok(())
        }
        Commands::Upgrade { force } => run_upgrade_cli(force).await,
        Commands::Rollback => run_rollback_cli(),
        Commands::Uninstall {
            yes,
            purge,
            keep_data,
            dry_run,
        } => uninstall::run(uninstall::Args {
            yes,
            purge,
            keep_data,
            dry_run,
        }),
        Commands::Fixissue { .. } => {
            unreachable!("Fixissue is handled inline in run() before handle_command")
        }
        Commands::Codingplan => {
            // Hidden alias for Login 鈥?`run()` intercepts both before
            // handle_command is called, so this arm is unreachable.
            unreachable!("Codingplan is handled inline in run() before handle_command")
        }
        Commands::Telemetry { .. } => {
            unreachable!("Telemetry is handled inline in run() before handle_command")
        }
        Commands::Daemon { .. } => {
            unreachable!("Daemon is handled inline in run() before handle_command")
        }
        Commands::Setup { .. } => {
            unreachable!("Setup is handled inline in run() before handle_command")
        }
        Commands::Plugin(sub) => handle_plugin_cli(sub),
        Commands::Mcp(McpCli::Add {
            name,
            command,
            global,
            dir,
        }) => {
            let base = resolve_working_dir(dir);
            let path = if global {
                Config::config_dir().join("mcp.json")
            } else {
                base.join(".mcp.json")
            };
            let program = command
                .first()
                .expect("clap ensures at least one command token")
                .clone();
            let args: Vec<String> = command.into_iter().skip(1).collect();
            merge_stdio_mcp_server_into_json_file(&path, &name, &program, &args)?;
            println!(
                "  Added MCP server {:?} 鈫?{} (stdio: {} + {} arg(s))",
                name,
                path.display(),
                program,
                args.len()
            );
            Ok(())
        }
        Commands::Mcp(McpCli::AddGithubOauth { name, global, dir }) => {
            let base = resolve_working_dir(dir);
            let path = if global {
                Config::config_dir().join("mcp.json")
            } else {
                base.join(".mcp.json")
            };
            merge_http_oauth_mcp_server_into_json_file(
                &path,
                &name,
                "https://api.githubcopilot.com/mcp/",
                "github",
            )?;
            println!(
                "  Added GitHub OAuth MCP server {:?} 鈫?{}",
                name,
                path.display()
            );
            Ok(())
        }
        Commands::Mcp(McpCli::Login {
            name,
            provider,
            client_id,
            client_secret_env,
            scopes,
        }) => {
            let configs = load_mcp_config(&std::env::current_dir()?)?;
            let server = configs
                .into_iter()
                .find(|config| config.name == name)
                .ok_or_else(|| anyhow::anyhow!("MCP server {:?} not found in config", name))?;
            let is_github_server = matches!(
                &server.config,
                McpTransportConfig::Http {
                    auth: Some(McpHttpAuthConfig::OAuth(auth)),
                    ..
                } if auth.provider.as_deref() == Some("github")
            );
            let client_id = client_id.or_else(|| {
                if is_github_server && provider == "github" {
                    std::env::var("HYDRA_GITHUB_MCP_CLIENT_ID").ok()
                } else {
                    None
                }
            });
            let token = login_mcp_oauth(
                &server,
                McpOAuthLoginOptions {
                    client_id,
                    client_secret_env,
                    scopes,
                },
            )?;
            println!(
                "  Saved {} OAuth token for MCP server {:?} with {} scope(s)",
                token.provider,
                name,
                token.scopes.len()
            );
            Ok(())
        }
        Commands::Mcp(McpCli::Logout { name }) => {
            let removed = McpTokenStore::default().delete_token(&name)?;
            if removed {
                println!("  Removed saved OAuth token for MCP server {:?}", name);
            } else {
                println!("  No saved OAuth token found for MCP server {:?}", name);
            }
            Ok(())
        }
    }
}

/// Dispatch `hydra plugin ...` subcommands. Each branch calls the same
/// `hydra_core::plugin::*` API the TUI's `/plugin` slash command uses, so
    /// CLI installs and TUI installs share state under `$HYDRA_HOME/plugins/`.
fn handle_plugin_cli(sub: PluginCli) -> Result<()> {
    use hydra_core::plugin::{installer, marketplace};
    match sub {
        PluginCli::Marketplace(MarketplaceCli::Add { url }) => {
            let info = marketplace::add_marketplace(&url)
                .map_err(|e| anyhow::anyhow!("add marketplace: {:#}", e))?;
            println!(
                "  marketplace `{}` added at {} ({} plugins)",
                info.name,
                &info.git_commit[..7.min(info.git_commit.len())],
                info.plugins.len()
            );
            Ok(())
        }
        PluginCli::Marketplace(MarketplaceCli::Remove { name }) => {
            marketplace::remove_marketplace(&name)
                .map_err(|e| anyhow::anyhow!("remove marketplace: {:#}", e))?;
            println!("  marketplace `{}` removed", name);
            Ok(())
        }
        PluginCli::Marketplace(MarketplaceCli::Update { name }) => {
            let info = marketplace::update_marketplace(&name)
                .map_err(|e| anyhow::anyhow!("update marketplace: {:#}", e))?;
            println!(
                "  marketplace `{}` updated to {}",
                info.name,
                &info.git_commit[..7.min(info.git_commit.len())]
            );
            Ok(())
        }
        PluginCli::Marketplace(MarketplaceCli::List) => {
            let items = marketplace::list_marketplaces()?;
            if items.is_empty() {
                println!("  no marketplaces registered");
            } else {
                for m in items {
                    println!(
                        "  {}  {}  {}  ({} plugins)",
                        m.name,
                        m.source,
                        &m.git_commit[..7.min(m.git_commit.len())],
                        m.plugins.len()
                    );
                }
            }
            Ok(())
        }
        PluginCli::Install { spec } => {
            let (plugin, mp) = parse_plugin_spec(&spec)?;
            let info = installer::install(&plugin, &mp)
                .map_err(|e| anyhow::anyhow!("install: {:#}", e))?;
            println!("  installed `{}@{}`", info.plugin, info.marketplace);
            Ok(())
        }
        PluginCli::Uninstall { spec } => {
            let (plugin, mp) = parse_plugin_spec(&spec)?;
            installer::uninstall(&plugin, &mp)
                .map_err(|e| anyhow::anyhow!("uninstall: {:#}", e))?;
            println!("  uninstalled `{}@{}`", plugin, mp);
            Ok(())
        }
        PluginCli::List => {
            let items = installer::list_installed()?;
            if items.is_empty() {
                println!("  no installed plugins");
            } else {
                for p in items {
                    println!("  {}@{}  {}", p.plugin, p.marketplace, p.plugin_dir);
                }
            }
            Ok(())
        }
    }
}

/// Split `<plugin>@<marketplace>` into its two parts. Reject empty halves
/// up front so we surface a single clean error instead of letting the
/// installer reject `""` later with a confusing "not found" message.
fn parse_plugin_spec(s: &str) -> Result<(String, String)> {
    let (plugin, mp) = s
        .split_once('@')
        .ok_or_else(|| anyhow::anyhow!("expected <plugin>@<marketplace>, got `{}`", s))?;
    if plugin.trim().is_empty() || mp.trim().is_empty() {
        anyhow::bail!("plugin/marketplace name must not be empty in `{}`", s);
    }
    Ok((plugin.trim().to_string(), mp.trim().to_string()))
}

/// CLI (non-TUI) upgrade driver 鈥?prints progress to stdout and
/// success/error messages the same way `install.sh` does.
async fn run_upgrade_cli(force: bool) -> Result<()> {
    use hydra_core::self_update::{self, UpgradeEvent, ALREADY_LATEST};

    let current = format!("v{}", env!("CARGO_PKG_VERSION"));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<UpgradeEvent>();

    // Spawn the driver; consume events on the main task so stdout
    // writes don't interleave unpredictably with the upgrade work.
    let driver = tokio::spawn(self_update::run_upgrade(current.clone(), force, tx));

    let mut last_pct: i32 = -1;
    while let Some(ev) = rx.recv().await {
        match ev {
            UpgradeEvent::ManifestFetched { version } => {
                println!("==> Latest: {}", version);
            }
            UpgradeEvent::Downloading { bytes, total } => {
                // Debounce to whole percents so we don't spam stdout 鈥?
                // piping the CLI through `tee` with 10k updates is no
                // fun for anyone.
                let pct = if total == 0 {
                    0
                } else {
                    ((bytes * 100) / total) as i32
                };
                if pct != last_pct {
                    print!(
                        "\r    downloading {}% ({} / {} bytes)   ",
                        pct, bytes, total
                    );
                    io::stdout().flush().ok();
                    last_pct = pct;
                }
            }
            UpgradeEvent::Verifying => {
                println!("\n==> Verifying SHA256");
            }
            UpgradeEvent::Replacing => {
                println!("==> Replacing binary");
            }
            UpgradeEvent::Done {
                version,
                backup,
                exe: _,
            } => {
                println!(
                    "\n鉁?Upgraded to {} (previous version kept at {})",
                    version,
                    backup.display()
                );
                println!("  Run `hydra` to start the new version.");
            }
            // CLI path never spawns a rollback via this channel and the
            // driver below translates errors into the returned Result
            // (not a Failed event) 鈥?these arms exist only to keep the
            // match exhaustive if the TUI path ever reuses this code.
            UpgradeEvent::Failed(msg) => {
                eprintln!("\nupgrade failed: {}", msg);
            }
            UpgradeEvent::RolledBack { exe, backup } => {
                println!(
                    "\n鉁?Rolled back. exe={}, backup={}",
                    exe.display(),
                    backup.display()
                );
            }
        }
    }

    match driver.await {
        Ok(Ok(_summary)) => Ok(()),
        Ok(Err(e)) => {
            let msg = format!("{:#}", e);
            if msg.contains(ALREADY_LATEST) {
                // Friendly path 鈥?not an error, just "nothing to do".
                println!("  {}", msg.replace(&format!("{}: ", ALREADY_LATEST), ""));
                Ok(())
            } else {
                Err(e)
            }
        }
        Err(e) => Err(anyhow::anyhow!("upgrade task panicked: {}", e)),
    }
}

fn run_rollback_cli() -> Result<()> {
    let summary = hydra_core::self_update::run_rollback()?;
    println!(
        "鉁?Rolled back. Previous binary is now at {}, other version saved at {}",
        summary.exe.display(),
        summary.backup.display()
    );
    println!("  Run `hydra` to start the rolled-back version.");
    Ok(())
}

/// Core CodingPlan flow shared by CLI-exit and CLI鈫扵UI paths. Loads
/// the config (or starts from defaults if missing), runs the shared
/// `coding_plan::setup` orchestrator, persists the config on success,
/// and returns the rendered human-readable report 鈥?the caller decides
/// whether to print it to stdout or stash it for the TUI to surface.
fn run_codingplan_core(
    telemetry: Option<&std::sync::Arc<hydra_telemetry::Telemetry>>,
) -> Result<String> {
    let path = Config::default_path();
    // Missing config is legitimate on first install 鈥?start from defaults
    // so the flow can still add AtomGit providers to a fresh config.toml.
    let mut config = match Config::load(&path) {
        Ok(c) => c,
        Err(_) => Config {
            default_provider: String::new(),
            default_workdir: None,
            providers: std::collections::HashMap::new(),
            datalog: Default::default(),
            auto_update: true,
            notifications: Default::default(),
            telemetry: Default::default(),
            lsp: Default::default(),
            auto_commit: false,
            subagent: Default::default(),
            vision_preprocessor_provider: None,
            language: None,
            ui: Default::default(),
            plugin: Default::default(),
        },
    };

    // If the stored token is locally valid (file present, expires_in
    // not yet past) but the server rejects it (revoked, refresh-token
    // dead, etc.), the orchestrator sets `report.auth_expired = true`.
    // Run OAuth *once* on that path 鈥?same flow `hydra login` would
    // use 鈥?then re-run setup against the fresh token. Without this
    // the user sees the report ending in "claim failed 鈥?run `hydra
    // login` again" and has to do manually what `codingplan` could
    // do itself.
    let mut report = hydra_core::coding_plan::run(&mut config, telemetry)?;
    if report.auth_expired {
        use hydra_core::i18n::{t, Msg};
        print!("{}", t(Msg::CpReauthAfter401));
        match hydra_core::auth::login(telemetry)
            .and_then(|auth| hydra_core::auth::save_auth(&auth).map(|_| auth))
        {
            Ok(_) => {
                report = hydra_core::coding_plan::run(&mut config, telemetry)?;
            }
            Err(e) => {
                // Re-OAuth itself failed (user pressed Ctrl+C, network
                // dead, etc.). Print the *original* report so users
                // still see what triggered the retry, then bail.
                println!("{}", report.render());
                anyhow::bail!("re-authentication failed: {:#}", e);
            }
        }
    }

    if report.should_persist_config() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = config.save(&path) {
            eprintln!("  鈿?Failed to save config to {}: {:#}", path.display(), e);
        }
        // Stamp the sync marker alongside the config write. The drift
        // monitor on the TUI side reads this to decide whether to warn
        // about stale provider lists (> 24h + server drift). A failed
        // marker write is non-fatal 鈥?the config already landed; only
        // the 24h hint would be miscounted, which self-corrects on the
        // next successful run.
        if let Err(e) = hydra_core::coding_plan::write_last_sync_now() {
            eprintln!("  鈿?Failed to write codingplan sync marker: {:#}", e);
        }
    }

    Ok(report.render())
}

/// Install the telemetry-aware panic hook. Replaces the minimal pre-init hook
/// set in `main()` so panics are both reported cleanly to the terminal AND
/// sent as a `Panic` telemetry event before the process exits.
fn install_panic_hook(telemetry: std::sync::Arc<hydra_telemetry::Telemetry>) {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal_if_tui();
        let home = hydra_core::tool::real_home_dir();
        let cwd = std::env::current_dir().ok();
        let loc = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown".into());
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        let bt = std::backtrace::Backtrace::force_capture().to_string();
        let scrubbed_loc =
            hydra_telemetry::scrub::scrub_path(&loc, home.as_deref(), cwd.as_deref());
        let scrubbed_msg = hydra_telemetry::scrub::truncate_head(
            &hydra_telemetry::scrub::scrub_path(&msg, home.as_deref(), cwd.as_deref()),
            hydra_telemetry::scrub::HEAD_MAX,
        );
        let frames =
            hydra_telemetry::scrub::backtrace_top_k(&bt, 5, home.as_deref(), cwd.as_deref());
        telemetry.track(hydra_telemetry::Event::Panic {
            location: scrubbed_loc,
            message_head: scrubbed_msg,
            thread: std::thread::current().name().unwrap_or("unknown").into(),
            backtrace_top_5: frames,
            error_kind: Some("panic".to_string()),
            error_data: Some(serde_json::json!({
                "session_duration_secs": telemetry.uptime().as_secs() as u32,
                "turns_completed": null,
                "last_tool_name": null,
                "last_event": null,
            }).to_string()),
        });
        default_hook(info);
    }));
}

/// Classify a `create_provider` error display string as an "auth gap" 鈥?
/// i.e. the user has no usable OAuth token (file missing, malformed,
/// access_token expired, refresh_token expired/missing, refresh network
/// error, etc.). Auth-gap errors must NOT abort startup; they should
/// fall back to the onboarding TUI so the user can run `/login` from
/// within the app.
///
/// Implementation note 鈥?substring matching, not typed errors:
///
/// `create_provider` returns `anyhow::Error` and the auth-gap producers
/// live in three modules (`provider::mod::load_auth_token`,
/// `provider::mod::refresh_and_save`, `auth::oauth::get_valid_token`).
/// Threading a typed sentinel through all of them is invasive; instead
/// we match on a stable convention: **every auth-gap error message in
/// the codebase ends with the substring `"/login"`** (either
/// `"please /login"` or `"please use /login"`). That hint is part of
/// the user-facing contract 鈥?any future auth-gap producer that wants
/// graceful fallback simply has to follow the same convention.
///
/// The three legacy substrings (`Not logged in` / `Invalid auth.toml`
/// / `Token expired`) are retained as belt-and-braces: they were the
/// original matches before the `"/login"` rule was extracted, and
/// keeping them ensures the test for the historical contract stays
/// green even if some future refactor temporarily strips the `/login`
/// suffix from a specific message.
///
/// Non-auth errors (config parse failure, network issues unrelated to
/// the refresh endpoint, provider validation rejects model name, etc.)
/// fall through to `return Err(e)` and abort startup as designed.
fn is_auth_gap_error(msg: &str) -> bool {
    msg.contains("/login")
        || msg.contains("Not logged in")
        || msg.contains("Invalid auth.toml")
        || msg.contains("Token expired")
}

#[cfg(test)]
mod tests {
    use super::{
        close_thinking_chunk, format_thinking_chunk, is_auth_gap_error, resolve_working_dir,
        truncate_log_line,
    };
    use std::path::PathBuf;

    #[test]
    fn ascii_short_unchanged() {
        assert_eq!(truncate_log_line("hello", 10), "hello");
    }

    #[test]
    fn ascii_long_truncated_with_ellipsis() {
        assert_eq!(truncate_log_line("0123456789abcdef", 10), "0123456789...");
    }

    #[test]
    fn newlines_become_spaces() {
        assert_eq!(truncate_log_line("a\nb\nc", 10), "a b c");
    }

    #[test]
    fn mixed_ascii_cjk_truncates_at_char_boundary() {
        // 8 chars: ['a','b','c','璁?,'绠?,'d','e','f']; max 5 鈫?"abc璁＄畻..."
        assert_eq!(truncate_log_line("abc璁＄畻def", 5), "abc璁＄畻...");
    }

    /// Regression test for panic at `crates/hydra-cli/src/main.rs:272:42`:
    /// "byte index 500 is not a char boundary; it is inside '璁? (bytes 498..501)".
    /// Triggered when ToolCallResult output was a CJK-heavy string > 500 bytes
    /// and the old code did `trimmed[..500]` (byte slice). Pure CJK at 3 bytes
    /// per char means almost any 500-byte cut lands inside a multi-byte char.
    #[test]
    fn cjk_truncation_does_not_panic() {
        let s: String = "璁＄畻".repeat(500); // 1000 chars, 3000 bytes
        let result = truncate_log_line(&s, 500);
        assert_eq!(result.chars().count(), 503); // 500 + "..."
        assert!(result.ends_with("..."));
    }

    /// Regression test for cwd-override bug: when no `-C` is given, working dir
    /// must equal `std::env::current_dir()`. Old code silently substituted the
    /// first line of `~/.hydra/recent_dirs.txt`, breaking `hydra -p` from
    /// any directory that wasn't the TUI's last-visited project.
    #[test]
    fn resolve_working_dir_uses_cwd_when_no_cli_dir() {
        let expected = std::env::current_dir().unwrap();
        assert_eq!(resolve_working_dir(None), expected);
    }

    #[test]
    fn resolve_working_dir_honors_cli_dir() {
        let temp = std::env::temp_dir();
        let canon = std::fs::canonicalize(&temp).unwrap_or(temp.clone());
        assert_eq!(resolve_working_dir(Some(temp)), canon);
    }

    #[test]
    fn resolve_working_dir_falls_back_to_input_when_canonicalize_fails() {
        // Use a non-existent path so canonicalize() returns Err and the
        // function falls back to the raw input rather than panicking.
        let bogus = PathBuf::from("/nonexistent/hydra-test-path-xyzzy");
        assert_eq!(resolve_working_dir(Some(bogus.clone())), bogus);
    }

    /// Verify that std::fs::read_to_string reads a temp file correctly,
    /// which is the core of --prompt-file. This is a unit-level stand-in for
    /// the integration test (full CLI parse requires a running provider).
    #[test]
    fn prompt_file_read_preserves_trailing_newline() {
        use std::io::Write as _;
        let path = std::env::temp_dir().join("hydra_test_prompt_file.txt");
        let content = "fix the bug\n";
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(content.as_bytes()).unwrap();
        }
        let read_back = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(
            read_back, content,
            "--prompt-file must preserve trailing newline (unlike bash $(...))"
        );
    }

    // ---- format_thinking_chunk / close_thinking_chunk ----
    //
    // Regression suite for the "one word per line" bug in headless verbose
    // output. The old `eprintln!("[thinking] {}", text)` printed a fresh
    // prefix + trailing newline for every streaming chunk, so a streaming
    // reasoning model produced output like:
    //
    //   [thinking] are
    //   [thinking] already
    //   [thinking] configured
    //
    // The new formatter must keep a single line open across many tiny
    // chunks until something else (text, tool call, turn complete, etc.)
    // explicitly closes it.

    /// Streaming many single-token chunks must produce ONE line, not N.
    #[test]
    fn thinking_chunks_stream_onto_single_line() {
        let mut buf = String::new();
        let mut open = false;
        for tok in ["are", " already", " configured", " and", " what"] {
            format_thinking_chunk(&mut buf, &mut open, tok);
        }
        assert_eq!(buf, "[thinking] are already configured and what");
        assert!(open, "line should remain open until something closes it");
    }

    /// A non-reasoning event must close the line with a single newline.
    #[test]
    fn close_appends_newline_and_clears_open() {
        let mut buf = String::new();
        let mut open = false;
        format_thinking_chunk(&mut buf, &mut open, "hello");
        close_thinking_chunk(&mut buf, &mut open);
        assert_eq!(buf, "[thinking] hello\n");
        assert!(!open);
        // Closing again is a no-op (idempotent).
        close_thinking_chunk(&mut buf, &mut open);
        assert_eq!(buf, "[thinking] hello\n");
    }

    /// Embedded newlines inside a chunk must produce a re-prefixed next line.
    #[test]
    fn embedded_newline_reprefixes_next_line() {
        let mut buf = String::new();
        let mut open = false;
        format_thinking_chunk(&mut buf, &mut open, "first line\nsecond line");
        assert_eq!(buf, "[thinking] first line\n[thinking] second line");
        assert!(open);
    }

    /// A chunk ending with `\n` closes the line; the next chunk must
    /// re-introduce the `[thinking] ` prefix.
    #[test]
    fn trailing_newline_closes_and_next_chunk_reprefixes() {
        let mut buf = String::new();
        let mut open = false;
        format_thinking_chunk(&mut buf, &mut open, "para1\n");
        assert!(!open, "trailing newline should close the line");
        format_thinking_chunk(&mut buf, &mut open, "para2");
        assert_eq!(buf, "[thinking] para1\n[thinking] para2");
        assert!(open);
    }

    /// Empty chunks must be skipped without emitting a stray prefix.
    #[test]
    fn empty_chunk_is_noop() {
        let mut buf = String::new();
        let mut open = false;
        format_thinking_chunk(&mut buf, &mut open, "");
        assert_eq!(buf, "");
        assert!(!open);
        // Still no prefix after empty input.
        format_thinking_chunk(&mut buf, &mut open, "x");
        assert_eq!(buf, "[thinking] x");
    }

    /// CJK content (common in Chinese reasoning models) must not break the
    /// single-line invariant 鈥?every char-level chunk just appends.
    #[test]
    fn cjk_chunks_stream_correctly() {
        let mut buf = String::new();
        let mut open = false;
        for tok in ["I ", "will ", "check ", "your ", "current ", "environment"] {
            format_thinking_chunk(&mut buf, &mut open, tok);
        }
        assert_eq!(buf, "[thinking] I will check your current environment");
    }

    /// Simulated end-to-end event sequence: thinking deltas, then a tool
    /// call. The tool call must appear on its OWN line, not mashed onto
    /// the tail of the thinking text.
    #[test]
    fn thinking_followed_by_tool_call_is_separated() {
        let mut buf = String::new();
        let mut open = false;
        format_thinking_chunk(&mut buf, &mut open, "I should");
        format_thinking_chunk(&mut buf, &mut open, " check");
        format_thinking_chunk(&mut buf, &mut open, " the file");
        // Now a non-reasoning event arrives 鈫?close, then emit it.
        close_thinking_chunk(&mut buf, &mut open);
        buf.push_str("[tool鈫?read_file]\n");
        assert_eq!(buf, "[thinking] I should check the file\n[tool鈫?read_file]\n");
    }

    // 鈹€鈹€ is_auth_gap_error 鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€
    //
    // Regression set for the "both access_token AND refresh_token
    // expired 鈫?CLI bails before TUI loads" bug. Pre-fix catch list
    // only matched `Not logged in` / `Invalid auth.toml` / `Token
    // expired`, missing the 4 paths below 鈥?user landed in an
    // unrecoverable startup error because they couldn't get into the
    // TUI to run `/login`. Post-fix every auth-gap error's `/login`
    // suffix triggers graceful fallback to onboarding mode.

    #[test]
    fn auth_gap_catches_historical_three() {
        // Pre-existing matches 鈥?must stay green.
        assert!(is_auth_gap_error("Not logged in 鈥?please use /login"));
        assert!(is_auth_gap_error("Invalid auth.toml 鈥?please use /login"));
        assert!(is_auth_gap_error("Token expired 鈥?please use /login"));
    }

    #[test]
    fn auth_gap_catches_refresh_http_failure() {
        // The actual user-reported scenario: access_token expired,
        // `refresh_and_save` POSTs the (also-expired) refresh_token to
        // the broker, broker returns 401, `provider::mod::refresh_and_save`
        // bails with this message. Pre-fix catch list MISSED this 鈥?
        // none of the 3 historical substrings appeared, so CLI fell
        // through to `return Err(e)` and aborted startup.
        assert!(is_auth_gap_error(
            "Token refresh failed (401 Unauthorized) 鈥?please /login"
        ));
        assert!(is_auth_gap_error(
            "Token refresh failed (400 Bad Request) 鈥?please /login"
        ));
    }

    #[test]
    fn auth_gap_catches_refresh_network_failure() {
        // `refresh_and_save` network-layer error path (broker host
        // unreachable, DNS failure, TLS error, etc.). Same root cause
        // as the HTTP path 鈥?token can't be refreshed 鈫?user needs
        // `/login`. Onboarding TUI is the only way they can run it.
        assert!(is_auth_gap_error(
            "Token refresh failed: connection refused 鈥?please /login"
        ));
    }

    #[test]
    fn auth_gap_catches_refresh_parse_failure() {
        // Broker returned 200 but body wasn't valid refresh-token
        // JSON. Treated as auth gap because there's no usable token
        // either way.
        assert!(is_auth_gap_error(
            "Token refresh parse error: missing field `access_token` 鈥?please /login"
        ));
    }

    #[test]
    fn auth_gap_catches_missing_refresh_token() {
        // `auth/oauth.rs::refresh_access_token` when stored auth.toml
        // has access_token but no refresh_token field. Suffix is
        // "please /login again" 鈥?still ends in `/login`, so the new
        // substring rule catches it.
        assert!(is_auth_gap_error(
            "No refresh_token available 鈥?please /login again"
        ));
    }

    #[test]
    fn auth_gap_does_not_swallow_non_auth_errors() {
        // Non-auth errors must NOT trigger the onboarding fallback 鈥?
        // the user should see the real failure, not a generic "please
        // /login" prompt that doesn't apply.
        assert!(!is_auth_gap_error(
            "Config parse error: invalid TOML at line 12"
        ));
        assert!(!is_auth_gap_error(
            "Provider rejected model name 'foo-bar-9000': unknown model"
        ));
        assert!(!is_auth_gap_error(
            "HTTP request to https://api.example.com failed: timeout"
        ));
        assert!(!is_auth_gap_error(""));
    }
}


