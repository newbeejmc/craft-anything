//! Agent CLIs (Claude Code, Codex, Gemini CLI, OpenCode, Hermes…) run as owned subprocesses with
//! our MCP server, optionally on another machine over ssh. Their JSON event streams become
//! [`AgentEvent`]s.
//!
//! How each CLI is driven lives in [`adapters`] ([`AgentAdapter`], registered in
//! [`AgentAdapters`]). Flags: Claude Code's were checked against `claude --help`. For the others,
//! the flags we rely on are checked at run time against `<cli> --help`; when they're missing we
//! fall back to the plain adapter (prompt as an argument, stdout streamed as text) and say so,
//! unless the CLI's plain form is interactive (Codex), which keeps its adapter.

pub mod acp;
pub mod adapters;
pub mod claude_cost;
pub mod grants;
pub mod models;
mod shared;
pub mod unfinished;

pub(crate) use shared::SharedConversation;

pub use acp::AcpConversation;
pub use adapters::{AgentAdapter, AgentAdapters, ArgsCx};

use crate::config::BackendOptions;
use crate::model::Stop;
use crate::{AgentEvent, BackendConfig, Conversation, FailureKind, TurnFailure};
use craft_anything_engine::CancelToken;
use craft_anything_evidence::{ErrorCode, ToolError};
use craft_anything_platform::process::Process;
use craft_anything_platform::sync::Mutex;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, PoisonError};
use std::time::Duration;

/// Name our MCP server is registered under.
pub const MCP_NAME: &str = "craft-anything";

/// Known agent CLIs: (agent id, program, display name).
pub const KNOWN: &[(&str, &str, &str)] = &[
    ("claude-code", "claude", "Claude Code"),
    ("codex", "codex", "Codex"),
    ("copilot", "copilot", "GitHub Copilot"),
    ("grok", "grok", "Grok Build"),
    ("gemini-cli", "gemini", "Gemini CLI"),
    ("antigravity", "agy", "Antigravity"),
    ("opencode", "opencode", "OpenCode"),
    ("hermes", "hermes", "Hermes Agent"),
    ("qwen-code", "qwen", "Qwen Code"),
    ("goose", "goose", "Goose"),
    ("maki", "maki", "Maki"),
    ("aider", "aider", "Aider"),
    ("dsh", "dsh", "DeepSeek Harness"),
    ("devin", "devin", "Devin"),
];

pub fn program_for(agent: &str) -> String {
    KNOWN.iter().find(|(id, _, _)| *id == agent).map(|(_, p, _)| p.to_string()).unwrap_or_else(|| agent.to_string())
}

/// The program a backend runs: the one chosen for it, else the agent's usual name. A chosen path
/// that's gone from this computer is looked for again (#721): an agent that installs each version
/// in a folder of its own (Codex's `bin/<build>/codex.exe`) moves on every update, so the same file
/// in the newest folder beside the old one is used, else the usual name on PATH. On another
/// computer (ssh) the chosen command is used as it is. Fails when the usual name would start only
/// the Claude desktop app, which can't build.
pub fn program_of(config: &BackendConfig, options: &BackendOptions) -> Result<String, ToolError> {
    program_in(config, options, craft_anything_platform::which::search_path())
}

/// The saved program path `program` stands in for, when [`program_of`] chose another one (#721).
/// A bare name (`claude.exe`) found as the same agent elsewhere isn't a replacement, nor is a path
/// that's still there (a saved folder, or the Claude desktop app): it isn't gone.
pub fn replaced_program(options: &BackendOptions, program: &str) -> Option<String> {
    let saved = options.command.as_deref().map(str::trim).filter(|c| !c.is_empty() && *c != program)?;
    let path = Path::new(saved);
    (path.parent().is_some_and(|p| !p.as_os_str().is_empty()) && !path.exists()).then(|| saved.to_string())
}

/// Resolve saved commands locally through the shared lookup, but leave SSH commands untouched.
fn program_in(config: &BackendConfig, options: &BackendOptions, search: Option<std::ffi::OsString>) -> Result<String, ToolError> {
    let usual = program_for(&config.model);
    let local = config.ssh_host.as_deref().is_none_or(|h| h.trim().is_empty());
    let Some(chosen) = options.command.as_deref().map(str::trim).filter(|c| !c.is_empty()) else {
        return if local { Ok(usual_program(&usual, search)?.unwrap_or(usual)) } else { Ok(usual) };
    };
    // A folder or app bundle saved before set_command refused it (#467) can't be started: the usual
    // program is used, as when none is chosen.
    if local && Path::new(chosen).is_dir() {
        return Ok(usual_program(&usual, search)?.unwrap_or(usual));
    }
    let path = Path::new(chosen);
    let bare = path.parent().is_none_or(|p| p.as_os_str().is_empty());
    // A saved `claude`, by name or as the desktop app's own file, is looked up as the usual one is,
    // so it never starts the desktop app.
    let named_claude = bare && path.file_stem().is_some_and(|s| s.eq_ignore_ascii_case(&usual));
    if local && usual == "claude" && (named_claude || leads_to_claude_desktop(path)) {
        // A name saved on purpose (`claude.cmd`, a wrapper) is kept when it starts Claude Code itself.
        if named_claude
            && let Some(found) = craft_anything_platform::which::which_in(chosen, search.clone()).into_iter().next()
            && !leads_to_claude_desktop(&found)
        {
            return Ok(chosen.to_string());
        }
        return match usual_program(&usual, search)? {
            Some(program) => Ok(program),
            None if bare => Ok(chosen.to_string()),
            None => Err(desktop_app_error(path)),
        };
    }
    // Saved bare names share detection's desktop-CLI fallback, whichever agent owns it.
    if local && bare {
        return Ok(usual_program(chosen, search)?.unwrap_or_else(|| chosen.to_string()));
    }
    if !local || path.exists() {
        return Ok(chosen.to_string());
    }
    // Gone, and nothing beside it: the usual program if it's really here, else the saved path, so
    // the error names it.
    Ok(moved_program(path).or_else(|| usual_program(&usual, search).ok().flatten()).unwrap_or_else(|| chosen.to_string()))
}

/// How to start the usual program: its name, or the real program's path when the name alone would
/// start something else that comes first on the search path (the Claude desktop app's
/// `claude.exe`). `None` when nothing has that name; an error when only the desktop app does.
fn usual_program(usual: &str, search: Option<std::ffi::OsString>) -> Result<Option<String>, ToolError> {
    let first = craft_anything_platform::which::which_in(usual, search.clone()).into_iter().next();
    match (first, find_program_in(usual, search)) {
        (None, _) => Ok(local_desktop_cli(usual).map(|cli| cli.display().to_string())),
        (Some(first), Some(real)) => Ok(Some(if first == real { usual.to_string() } else { real.display().to_string() })),
        (Some(app), None) => Err(desktop_app_error(&app)),
    }
}

fn desktop_app_error(app: &Path) -> ToolError {
    ToolError::new(
        ErrorCode::EngineMissing,
        format!("Claude Code isn't installed: the `claude` found ({}) is the Claude desktop app, which can't build.", app.display()),
    )
    .with_remediation("Install Claude Code apart from the desktop app, sign in to it once in a terminal, then try again.")
}

/// Where an agent's program is on this computer: the first match on the search path that is
/// really it. Detection, the desktop app hint and starting a run all look through this, so what's
/// offered is what runs.
pub fn find_program(program: &str) -> Option<PathBuf> {
    find_program_in(program, craft_anything_platform::which::search_path()).or_else(|| local_desktop_cli(program))
}

/// Off-PATH desktop CLI locations stay here, not in backend tests or command selection.
fn local_desktop_cli(program: &str) -> Option<PathBuf> {
    let windows = desktop_cli(program, local_app_data().as_deref());
    #[cfg(target_os = "macos")]
    if windows.is_none() && program == "codex" {
        let roots = dirs::home_dir().map(|home| home.join("Applications")).into_iter().chain([PathBuf::from("/Applications")]);
        return macos_codex_in(roots);
    }
    windows
}

/// Dock launches do not inherit a terminal's PATH. Look only in known app-bundle locations,
/// never run an app's GUI executable as a CLI, and leave an explicit/PATH installation first.
#[cfg(any(target_os = "macos", all(test, unix)))]
fn macos_codex_in(roots: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    const INSIDE: &[&str] = &[
        "ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex",
        "ChatGPT.app/Contents/Resources/codex",
        "Codex.app/Contents/Resources/codex",
        "Codex.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex",
    ];
    roots.into_iter().flat_map(|root| INSIDE.iter().map(move |inside| root.join(inside))).find(|path| {
        let Ok(meta) = std::fs::metadata(path) else { return false };
        if !meta.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            meta.permissions().mode() & 0o111 != 0
        }
    })
}

/// Agents whose Windows desktop app keeps the command-line program in its own folder, under
/// another name and off the search path, relative to `%LOCALAPPDATA%` (#1060).
const IN_DESKTOP_APP: &[(&str, &str)] = &[("opencode", r"Programs\ai.opencode.desktop\resources\opencode-cli.exe")];

fn local_app_data() -> Option<PathBuf> {
    cfg!(windows).then(|| std::env::var_os("LOCALAPPDATA")).flatten().map(PathBuf::from)
}

/// `program`'s command-line program inside its desktop app, if that's installed under `local_data`.
fn desktop_cli(program: &str, local_data: Option<&Path>) -> Option<PathBuf> {
    let (_, inside) = IN_DESKTOP_APP.iter().find(|(name, _)| *name == program)?;
    Some(local_data?.join(inside)).filter(|cli| cli.is_file())
}

fn find_program_in(program: &str, path: Option<std::ffi::OsString>) -> Option<PathBuf> {
    craft_anything_platform::which::which_in(program, path).into_iter().find(|p| program != "claude" || !leads_to_claude_desktop(p))
}

/// [`is_claude_desktop`] for the file itself or, when it's a link, the file it leads to.
fn leads_to_claude_desktop(path: &Path) -> bool {
    is_claude_desktop(path) || std::fs::canonicalize(path).is_ok_and(|target| is_claude_desktop(&target))
}

/// A file inside the Claude desktop app, which can't build: its older installer names the app
/// `claude.exe` (`AnthropicClaude\claude.exe`, `AnthropicClaude\app-<version>\claude.exe`), the
/// Store package keeps it under `WindowsApps\Claude_<version>_<arch>__<publisher>`, and on macOS
/// it's in `Claude.app`. Only names are compared; nothing is read.
pub(crate) fn is_claude_desktop(path: &Path) -> bool {
    path.ancestors().skip(1).take(4).any(|dir| {
        let name = dir.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let in_store = dir.parent().and_then(Path::file_name).is_some_and(|p| p.eq_ignore_ascii_case("WindowsApps"));
        name == "anthropicclaude" || name == "claude.app" || (in_store && name.starts_with("claude_"))
    })
}

/// The same file in the most recently changed sibling of its missing folder, if there is one.
fn moved_program(missing: &Path) -> Option<String> {
    let file = missing.file_name()?;
    let versions = missing.parent()?.parent()?;
    std::fs::read_dir(versions)
        .ok()?
        .flatten()
        .take(256)
        .map(|e| e.path().join(file))
        .filter(|p| p.is_file())
        .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
        .map(|p| p.display().to_string())
}

pub fn display_for(agent: &str) -> String {
    KNOWN.iter().find(|(id, _, _)| *id == agent).map(|(_, _, d)| d.to_string()).unwrap_or_else(|| agent.to_string())
}

/// The built-in adapter for an agent id (the plain one for agents without their own).
pub fn adapter_for(agent: &str) -> Arc<dyn AgentAdapter> {
    AgentAdapters::built_in().for_agent(agent)
}

/// The MCP server command handed to the CLI.
#[derive(Debug, Clone, PartialEq)]
pub struct McpCommand {
    pub command: String,
    pub args: Vec<String>,
    /// Where the person's notes wait for a builder on this computer (`rebuild::notes`), so an
    /// agent with tool hooks can hand them over at its next step. None for remote runs.
    pub notes: Option<PathBuf>,
}

impl McpCommand {
    /// Our installed CLI (`craft-anything-cli mcp [--project <dir>] [--inbox <dir>]`). Remote runs
    /// use the name on the remote PATH and have no local project or inbox.
    pub fn local_or_remote(remote: bool, project: Option<PathBuf>, inbox: Option<PathBuf>) -> McpCommand {
        let command = if remote {
            "craft-anything-cli".to_string()
        } else {
            craft_anything_platform::self_cli_path().map(|p| p.display().to_string()).unwrap_or_else(|| "craft-anything-cli".to_string())
        };
        let mut args = vec!["mcp".to_string()];
        if let Some(p) = project.filter(|_| !remote) {
            args.push("--project".into());
            args.push(p.display().to_string());
        }
        let notes = inbox.as_deref().filter(|_| !remote).map(crate::rebuild::notes::notes_dir);
        if let Some(i) = inbox.filter(|_| !remote) {
            args.push("--inbox".into());
            args.push(i.display().to_string());
        }
        McpCommand { command, args, notes }
    }
}

/// Whether the `craft-anything-cli` a builder's tool server runs from (None: there isn't one) can
/// start. Without it Claude Code stops at its first approval with a stack trace (#91). A check
/// that ran out of time says nothing about the program, so the build goes ahead. Stopping the
/// build ends the check, as `ErrorCode::Cancelled`.
pub fn cli_can_start(cli: Option<&Path>, stop: &CancelToken) -> Result<(), ToolError> {
    const REINSTALL: &str = "Install the latest Craft Anything again: it keeps this helper next to the app. If you built the app yourself, run `cargo build -p craft-anything-cli` too.";
    let Some(cli) = cli else {
        return Err(ToolError::new(ErrorCode::EngineMissing, "Craft Anything's helper program (craft-anything-cli) is missing, so the builder can't start.")
            .with_remediation(REINSTALL));
    };
    let failed = |why: String| {
        ToolError::new(ErrorCode::EngineMissing, format!("Craft Anything's helper program at {} wouldn't start: {why}", cli.display()))
            .with_remediation(REINSTALL)
    };
    let stop = stop.clone();
    match Process::new(cli.as_os_str()).arg("--version").timeout(Duration::from_secs(30)).stdout_cap(64 * 1024).cancel_when(move || stop.is_cancelled()).run() {
        Ok(out) if out.success => Ok(()),
        Ok(out) => Err(failed(out.code.map_or_else(|| "it was stopped by the system".to_string(), |c| format!("it exited with code {c}")))),
        Err(e) if e.code == ErrorCode::TimedOut => Ok(()),
        Err(e) if e.code == ErrorCode::Cancelled => Err(e),
        Err(e) => Err(failed(e.message)),
    }
}

/// Whether a local build by `agent` in `cwd` can have our tool server: over ACP every agent is
/// handed it, else its adapter says.
pub fn check_builder_tools(adapter: &dyn AgentAdapter, agent: &str, options: &BackendOptions, cwd: &Path, stop: &CancelToken) -> Result<(), ToolError> {
    if selected_acp_args(adapter, agent, options).is_some() {
        return cli_can_start(craft_anything_platform::self_cli_path().as_deref(), stop);
    }
    adapter.check_native_options(options, Some(cwd))?;
    adapter.check_tool_server(stop)
}

/// The requested transport, shared by builder preflight and execution. ACP cannot carry explicit
/// catalogue model/variant selections yet, so those must pass the native adapter's checks.
fn selected_acp_args(adapter: &dyn AgentAdapter, agent: &str, options: &BackendOptions) -> Option<&'static [&'static str]> {
    if adapter.has_model_catalog() && !adapter.requires_acp() && (options.cli_model.is_some() || options.cli_thinking.is_some()) {
        return None;
    }
    acp::acp_args(agent).filter(|_| options.acp.unwrap_or_else(|| adapter.requires_acp()))
}

fn toml_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// The permission-prompt tool a builder run routes its approvals through.
pub const APPROVAL_TOOL: &str = craft_anything_engine::tools::interaction_tools::APPROVE_TOOL;

/// What an agent run may do with files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Only analyse: no edits.
    Analyse,
    /// The same in Clean room (#564): the agent's own file, shell and web tools are refused outright,
    /// so it works only through our tool server, which keeps the original out of reach. It isn't
    /// left to the agent's defaults (they allow reads in its folder and may change).
    CleanRoomAnalyse,
    /// Build: read freely, edit through our approval tool (which asks the person when needed).
    Build,
    /// Build in Clean room: every read and edit goes through our approval tool, which keeps the
    /// original out of reach.
    CleanRoomBuild,
    /// A question about a build: read the project's files, change nothing, keep no session.
    Question,
    /// The same in Clean room: no file tools at all (the answer comes from what it knows and
    /// what the question says), so the original stays out of reach.
    CleanRoomQuestion,
}

/// argv (after the program) for one prompt.
pub fn build_args(
    adapter: &dyn AgentAdapter,
    opts: &BackendOptions,
    prompt: &str,
    preamble: &str,
    mcp: &McpCommand,
    resume: Option<&str>,
    access: Access,
) -> Vec<String> {
    // A leading dash would be read as a flag.
    let prompt = if prompt.starts_with('-') { format!(" {prompt}") } else { prompt.to_string() };
    let builder = matches!(access, Access::Build | Access::CleanRoomBuild);
    let question = matches!(access, Access::Question | Access::CleanRoomQuestion);
    let reads_checked = matches!(access, Access::CleanRoomBuild | Access::CleanRoomQuestion | Access::CleanRoomAnalyse);
    adapter.args(&ArgsCx { options: opts, prompt: &prompt, preamble, mcp, resume, builder, reads_checked, question })
}

/// What a run may do with files: a question reads only, a builder builds, anything else analyses;
/// Deep mode or not decides whether the original is in reach (#564).
fn access_for(builder: bool, deep: bool, question: bool) -> Access {
    match (builder, deep, question) {
        (_, true, true) => Access::Question,
        (_, false, true) => Access::CleanRoomQuestion,
        (true, true, false) => Access::Build,
        (true, false, false) => Access::CleanRoomBuild,
        (false, true, false) => Access::Analyse,
        (false, false, false) => Access::CleanRoomAnalyse,
    }
}

/// Whether `program`'s help mentions every required flag (10 s limit, stdin closed); `None` when
/// the help couldn't be read (the program is missing, or too slow).
pub fn verify_flags(program: &str, help_args: &[&str], needed: &[&str]) -> Option<bool> {
    verify_flags_with_stop(program, help_args, needed, &Stop::default())
}

fn verify_flags_with_stop(program: &str, help_args: &[&str], needed: &[&str], stop: &Stop) -> Option<bool> {
    if stop.is_stopped() {
        return None;
    }
    if needed.is_empty() {
        return Some(true);
    }
    let stop = stop.clone();
    match Process::new(program).args(help_args).timeout(Duration::from_secs(10)).stdout_cap(256 * 1024).cancel_when(move || stop.is_stopped()).run() {
        Ok(out) => {
            let text = format!("{}{}", out.stdout_text(), out.stderr_text());
            if needed.iter().all(|f| text.contains(f)) {
                return Some(true);
            }
            // A failed probe that printed no usage (a broken launcher) says nothing about the version.
            (out.code == Some(0) || text.to_ascii_lowercase().contains("usage")).then_some(false)
        }
        Err(_) => None,
    }
}

/// The fix for a sign-in failure, in plain words. Sign-in always happens in Claude Code itself;
/// we never see or handle the account.
pub fn sign_in_hint(message: &str) -> Option<String> {
    sign_in_hint_in(message, adapters::in_flatpak(), None)
}

/// [`sign_in_hint`] for a Flatpak or not, and for Claude Code in a folder chosen for it
/// (`folder`), whose sign-in is its own: the command signs in there. In a Flatpak, Claude Code
/// keeps its sign-in and conversations in a folder of the app's own (#506), so signing in from a
/// normal terminal signs in the other copy: it's done through the sandbox, with that folder.
pub(crate) fn sign_in_hint_in(message: &str, flatpak: bool, folder: Option<&Path>) -> Option<String> {
    let m = message.to_ascii_lowercase();
    let wants_sign_in = ["authenticat", "oauth", "log in", "login", "not logged", "api key", "unauthorized"].iter().any(|w| m.contains(w));
    if !wants_sign_in {
        return None;
    }
    let folder = folder.map(|f| f.to_string_lossy().into_owned());
    Some(match (flatpak, folder) {
        (true, None) => format!("Sign in to Claude Code inside this app's sandbox: open a terminal and run `{FLATPAK_SIGN_IN}`, then try again."),
        (true, Some(f)) => format!(
            "Sign in to Claude Code inside this app's sandbox, in the folder chosen for it: open a terminal and run `{}`, then try again.",
            flatpak_sign_in(&crate::ssh::shell_quote(&f))
        ),
        (false, None) => "Sign in to Claude Code: open a terminal and run `claude auth login` (or start `claude` and type /login), then try again.".to_string(),
        (false, Some(f)) if cfg!(windows) => format!(
            "Sign in to Claude Code in the folder chosen for it: open PowerShell and run `$env:CLAUDE_CONFIG_DIR='{}'; claude auth login`, then try again.",
            f.replace('\'', "''")
        ),
        (false, Some(f)) => format!(
            "Sign in to Claude Code in the folder chosen for it: open a terminal and run `CLAUDE_CONFIG_DIR={} claude auth login`, then try again.",
            crate::ssh::shell_quote(&f)
        ),
    })
}

/// Signing in for the Claude Code that a Flatpak build starts: the same folder it will use.
const FLATPAK_SIGN_IN: &str = "flatpak run --command=sh ai.storyteller.craft-anything -c 'CLAUDE_CONFIG_DIR=\"$XDG_DATA_HOME/craft-anything/claude\" \"$HOME/.local/bin/claude\" auth login'";

/// [`FLATPAK_SIGN_IN`] for a chosen folder, already quoted for the sandbox's shell.
fn flatpak_sign_in(quoted: &str) -> String {
    let inner = format!("CLAUDE_CONFIG_DIR={quoted} \"$HOME/.local/bin/claude\" auth login");
    format!("flatpak run --command=sh ai.storyteller.craft-anything -c {}", crate::ssh::shell_quote(&inner))
}

/// Words in an API's answer when the installed Claude Code is older than the model it was asked
/// to use (#165).
pub const TOO_OLD_FOR_MODEL: &str = "does not support this model";

/// Claude Code's failure in plain words, and its fix when the person can make one: a version too
/// old for the model (#165), or a sign-in.
pub fn claude_code_failure(why: &str) -> (String, Option<String>) {
    claude_code_failure_in(why, None)
}

/// [`claude_code_failure`] for Claude Code in a folder chosen for it (`folder`), whose sign-in is
/// its own.
pub(crate) fn claude_code_failure_in(why: &str, folder: Option<&Path>) -> (String, Option<String>) {
    if why.to_ascii_lowercase().contains(TOO_OLD_FOR_MODEL) {
        return (
            format!("Claude Code is too old for the model it was asked to use ({}).", why.chars().take(200).collect::<String>()),
            Some(
                "Update the `claude` command-line tool, which is installed and updated apart from the Claude desktop app: open a terminal and run `claude update`, then try again. Or choose another model for Claude Code in Settings → AI."
                    .into(),
            ),
        );
    }
    (format!("Claude Code stopped: {why}"), sign_in_hint_in(why, adapters::in_flatpak(), folder))
}

/// Variables a Claude Code session sets for its own child processes. When Craft Anything itself
/// runs inside such a session, they'd make the person's `claude` act as part of that session
/// instead of as their own install, so they're removed. The person's own settings (provider
/// choice `CLAUDE_CODE_USE_*`, `CLAUDE_CONFIG_DIR`, `ANTHROPIC_*`) are kept.
pub fn host_session_vars(vars: impl Iterator<Item = String>) -> Vec<String> {
    let all: Vec<String> = vars.collect();
    if !all.iter().any(|k| k == "CLAUDECODE") {
        return Vec::new();
    }
    all.into_iter()
        .filter(|k| {
            matches!(k.as_str(), "CLAUDECODE" | "CLAUDE_PID" | "CLAUDE_EFFORT")
                || k.starts_with("CLAUDE_AGENT_SDK_")
                || k.starts_with("CLAUDE_PREVIEW_")
                || (k.starts_with("CLAUDE_CODE_") && !k.starts_with("CLAUDE_CODE_USE_"))
        })
        .collect()
}

/// `program` as the person's own standalone Claude Code (see [`host_session_vars`]).
fn standalone(program: &str) -> Process {
    host_session_vars(std::env::vars().map(|(k, _)| k)).into_iter().fold(Process::new(program), |p, k| p.env_remove(k))
}

/// Parses one CLI's output stream: the adapter reads each line, this keeps what it learned.
pub struct StreamParser {
    adapter: Arc<dyn AgentAdapter>,
    pub session_id: Option<String>,
    /// The agent's own session for this run, kept even when it can't be resumed
    /// ([`AgentAdapter::resumes`]), so a turn that doesn't finish can be cleaned up
    /// ([`AgentAdapter::discard_unfinished`]).
    pub run_session: Option<String>,
    pub tool_names: HashMap<String, String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: Option<f64>,
    /// A missing or invalid per-step cost cannot become a complete turn total later.
    cost_incomplete: bool,
    pub saw_result: bool,
    /// The run was asked to continue a saved session the agent doesn't have (it lives on another
    /// computer, or under another folder), so it did nothing.
    pub session_missing: bool,
    /// A protocol fact about the failed turn, independent of the text shown to the person.
    pub failure: Option<TurnFailure>,
}

fn short_tool(name: &str) -> String {
    name.strip_prefix(&format!("mcp__{MCP_NAME}__")).or_else(|| name.strip_prefix(&format!("{MCP_NAME}__"))).unwrap_or(name).to_string()
}

fn find_evidence_id(text: &str) -> Option<String> {
    let mut rest = text;
    while let Some(pos) = rest.find("ev_") {
        let cand = rest.get(pos..pos + 67).unwrap_or_default();
        if craft_anything_evidence::is_evidence_id(cand) {
            return Some(cand.to_string());
        }
        rest = rest.get(pos + 3..).unwrap_or_default();
    }
    None
}

fn content_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().filter_map(|b| b.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join(""),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn summary_of(s: &str) -> String {
    s.chars().take(200).collect()
}

impl StreamParser {
    /// A parser for one run, reading lines the way `adapter` says.
    pub fn new(adapter: Arc<dyn AgentAdapter>) -> Self {
        StreamParser {
            adapter,
            session_id: None,
            run_session: None,
            tool_names: HashMap::new(),
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: None,
            cost_incomplete: false,
            saw_result: false,
            session_missing: false,
            failure: None,
        }
    }

    /// Events for one output line.
    pub fn line(&mut self, line: &str) -> Vec<AgentEvent> {
        if !self.adapter.streams_json() {
            return vec![AgentEvent::TextDelta { text: format!("{line}\n") }];
        }
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            // Non-JSON lines (warnings, banners) are shown as status, not answer text.
            let t = line.trim();
            return if t.is_empty() { Vec::new() } else { vec![AgentEvent::Status { text: summary_of(t) }] };
        };
        let adapter = self.adapter.clone();
        adapter.parse(self, &v)
    }

    /// A tool call began (adapters call this from `parse`).
    pub fn started(&mut self, id: &str, name: &str, input: Value) -> AgentEvent {
        let tool = short_tool(name);
        self.tool_names.insert(id.to_string(), tool.clone());
        AgentEvent::ToolStarted { call_id: id.to_string(), tool, input }
    }

    /// A tool call ended.
    pub fn finished(&mut self, id: &str, ok: bool, text: &str) -> AgentEvent {
        let tool = self.tool_names.get(id).cloned().unwrap_or_default();
        AgentEvent::ToolFinished { call_id: id.to_string(), tool, ok, summary: summary_of(text), evidence_id: find_evidence_id(text) }
    }
}

/// Everything needed to run one CLI backend.
#[derive(Debug, Clone)]
pub struct CliSpec {
    pub config: BackendConfig,
    pub options: BackendOptions,
    pub beginner: bool,
    /// The working folder (and, without `builder`, the investigation handed to our MCP server).
    pub project: Option<PathBuf>,
    /// The desktop's open targets, active last; the MCP child restores them before serving.
    pub targets: Vec<craft_anything_engine::target::TargetContext>,
    /// Build mode: the agent may edit files in the working folder.
    pub builder: Option<Builder>,
    /// A saved agent session to continue.
    pub resume: Option<String>,
    /// Deep mode: our MCP server runs in it, and a builder sees every tool and may read files
    /// anywhere. In Clean room a builder sees only Clean room's tools, and every file it reads
    /// goes through the approval policy.
    pub deep: bool,
    /// Replaces the default analysis preamble.
    pub preamble: Option<String>,
    /// How each agent CLI is driven.
    pub adapters: AgentAdapters,
    /// A question about a build: read-only, with a fresh session or an unkept fork, so the
    /// builder's own session is never changed. Only adapters that
    /// [answer questions](adapters::AgentAdapter::answers_questions) take it.
    pub question: bool,
    /// Where a failed run writes what's known about it ([`crate::failure_log`]). None writes no log.
    pub logs: Option<PathBuf>,
    /// The backend's stored API key, for an ACP agent that asks the host to authenticate before a
    /// session opens (Devin). None for agents that sign in some other way.
    pub key: Option<String>,
    /// Where Claude Code's session costs are kept, so a resumed turn's cost is what it added
    /// ([`claude_cost`]). None: a resumed turn has no cost.
    pub cost_totals: Option<PathBuf>,
}

/// What a building run needs besides the working folder.
#[derive(Debug, Clone)]
pub struct Builder {
    /// The investigation project our MCP server keeps evidence in.
    pub investigation: Option<PathBuf>,
    /// Where questions and approvals for the person go.
    pub inbox: PathBuf,
}

/// Our MCP server for `spec`, in the run's mode whatever the saved settings say; a builder's lists
/// every tool in Deep mode.
fn mcp_for(spec: &CliSpec, remote: bool) -> McpCommand {
    let mut mcp = match &spec.builder {
        Some(b) => McpCommand::local_or_remote(remote, b.investigation.clone(), Some(b.inbox.clone())),
        None => McpCommand::local_or_remote(remote, spec.project.clone(), None),
    };
    mcp.args.push(if spec.deep { "--deep" } else { "--clean-room" }.into());
    if spec.deep && spec.builder.is_some() {
        mcp.args.push("--all-tools".into());
    }
    if !remote {
        for target in spec.targets.iter().skip(spec.targets.len().saturating_sub(128)) {
            if let Ok(target) = serde_json::to_string(target) {
                mcp.args.extend(["--target".into(), target]);
            }
        }
    }
    mcp
}

impl CliSpec {
    pub fn new(config: BackendConfig, options: BackendOptions, beginner: bool, project: Option<PathBuf>) -> Self {
        CliSpec {
            config,
            options,
            beginner,
            project,
            targets: Vec::new(),
            builder: None,
            resume: None,
            deep: false,
            preamble: None,
            logs: None,
            cost_totals: None,
            adapters: AgentAdapters::built_in(),
            question: false,
            key: None,
        }
    }
}

/// The conversation for a chat with an agent CLI: over ACP when the backend enables or requires
/// it and the agent speaks it (a local, non-building run), else one CLI run per message.
pub fn conversation(spec: CliSpec) -> Box<dyn Conversation> {
    match acp_for(&spec).filter(|_| spec.builder.is_none()) {
        // In Clean room the agent's own file, shell and web requests are refused, not put to the
        // person as a card they could allow with one click (#564).
        Some(c) if !spec.deep => Box::new(c.with_approver(acp::clean_room_chat_approver())),
        Some(c) => Box::new(c),
        None => Box::new(CliConversation::new(spec)),
    }
}

/// The conversation for a builder run: over ACP (permission requests decided by `approver`, the
/// approval policy) when the backend enables or requires it and the agent speaks it, else one
/// CLI run per message (approvals through our approval tool).
pub fn builder_conversation(spec: CliSpec, approver: acp::Approver) -> Box<dyn Conversation> {
    match acp_for(&spec) {
        Some(c) => Box::new(c.with_approver(approver)),
        None => Box::new(CliConversation::new(spec)),
    }
}

/// An ACP conversation for a local backend that enables or requires ACP. The adapter chooses
/// the tool restrictions for this access mode; questions omit MCP startup flags and never load
/// the builder's session. Unsupported routes return to the CLI runner, which refuses ACP-only agents.
fn acp_for(spec: &CliSpec) -> Option<AcpConversation> {
    let agent = spec.config.model.clone();
    let adapter = spec.adapters.for_agent(&agent);
    if spec.question && !adapter.answers_questions() {
        return None;
    }
    let local = spec.config.ssh_host.as_deref().is_none_or(|h| h.trim().is_empty());
    let acp_args = selected_acp_args(adapter.as_ref(), &agent, &spec.options).filter(|_| local)?;
    // No program to start: the plain run reports why.
    let program = program_of(&spec.config, &spec.options).ok()?;
    // Same modes as the plain runner, including Clean room chat (no builder, not deep).
    let access = access_for(spec.builder.is_some(), spec.deep, spec.question);
    let mut args = adapter.acp_options(&spec.options, access);
    args.extend(acp_args.iter().map(|a| a.to_string()));
    args.extend(acp::model_args(&agent, spec.options.cli_model.as_deref()));
    // After the mode words and model flag, still before a trailing subcommand (`stdio` for Grok).
    args.extend(adapter.acp_mode_options(&spec.options, access));
    let cwd = spec.project.clone().or_else(|| std::env::current_dir().ok()).unwrap_or_else(std::env::temp_dir);
    let mcp = mcp_for(spec, false);
    if !spec.question {
        args.extend(adapter.acp_mcp_options(&mcp));
    }
    args.extend(adapter.acp_trailing().iter().map(|word| (*word).to_string()));
    let preamble = spec.preamble.clone().unwrap_or_else(|| crate::prompt::cli_preamble(spec.beginner));
    let session_config = adapter.acp_session_config(&spec.options);
    let auth = acp::key_auth_method(&agent).map(str::to_string).zip(spec.key.clone());
    Some(
        AcpConversation::new(&spec.config.name, &agent, program, args, cwd, mcp, if spec.question { None } else { spec.resume.clone() })
            .with_validation(adapter.validate_options(&spec.options))
            .with_access_limit(adapter.validate_access(access))
            .with_adapter(adapter)
            .with_question(spec.question)
            .with_session_config(session_config)
            .with_preamble(preamble)
            .with_logs(spec.logs.clone())
            .with_auth(auth),
    )
}

/// Copilot's selectable models from a fresh local ACP session, with `auto` offered once at the
/// front. No MCP server or model prompt is sent, and a stale model override can't prevent the
/// catalogue from loading. Blocking, with a 15-second metadata wait: call off the UI thread.
pub fn acp_model_choices(config: &BackendConfig, options: &BackendOptions) -> Result<Vec<String>, ToolError> {
    let mut spec = CliSpec::new(config.clone(), options.clone(), false, None);
    spec.question = true;
    spec.options.cli_model = None;
    spec.options.cli_context = None;
    spec.options.cli_reasoning_effort = None;
    spec.options.effort = None;
    let conversation = acp_for(&spec).ok_or_else(|| ToolError::invalid("This agent needs a local ACP connection to list its models."))?;
    let mut models = conversation.model_choices(Duration::from_secs(15))?;
    models.retain(|model| model != "auto");
    models.insert(0, "auto".into());
    Ok(models)
}

/// Discover settings for the backend's selected model, without a tool server or model prompt.
pub fn acp_model_tuning(config: &BackendConfig, options: &BackendOptions, stop: &Stop) -> Result<crate::model_options::ModelTuning, ToolError> {
    stop.check()?;
    let metadata = acp_model_metadata(config, options, None, stop)?;
    Ok(crate::model_options::ModelTuning::from_metadata(metadata.current_model, &metadata.config_options))
}

pub(crate) fn acp_model_metadata(
    config: &BackendConfig,
    options: &BackendOptions,
    directory: Option<&Path>,
    stop: &Stop,
) -> Result<acp::ModelMetadata, ToolError> {
    let mut spec = CliSpec::new(config.clone(), options.clone(), false, directory.map(Path::to_path_buf));
    spec.question = true;
    spec.options.cli_context = None;
    spec.options.cli_reasoning_effort = None;
    spec.options.cli_thinking = None;
    spec.options.effort = None;
    let conversation = acp_for(&spec).ok_or_else(|| ToolError::invalid("Model settings need a local ACP connection."))?;
    conversation.model_metadata_with_stop(Duration::from_secs(15), stop)
}

/// A conversation backed by an agent CLI. Each message is one CLI run; Claude Code runs resume
/// the previous session.
pub struct CliConversation {
    spec: CliSpec,
    tx: Sender<AgentEvent>,
    rx: Receiver<AgentEvent>,
    busy: Arc<AtomicBool>,
    stop: Stop,
    session: Arc<Mutex<Option<String>>>,
    verified: Arc<Mutex<Option<bool>>>,
    /// The last run's saved session wasn't there (see [`Conversation::session_lost`]).
    lost: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<TurnFailure>>>,
}

impl CliConversation {
    /// A conversation run as `spec` says; with `spec.resume`, its first message continues that
    /// saved session.
    pub fn new(spec: CliSpec) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let session = Arc::new(Mutex::new(spec.resume.clone()));
        CliConversation {
            spec,
            tx,
            rx,
            busy: Arc::default(),
            stop: Stop::default(),
            session,
            verified: Arc::default(),
            lost: Arc::default(),
            failure: Arc::default(),
        }
    }

    /// The agent's session id once a run has reported it (to continue later).
    pub fn session_id(&self) -> Option<String> {
        self.session.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn send_prompt(&mut self, text: &str, retry: bool) {
        if self.busy.swap(true, Ordering::SeqCst) {
            let _ = self.tx.send(AgentEvent::Error {
                message: "Still working on the last message.".into(),
                remediation: Some("Wait for it to finish or press stop.".into()),
                log: None,
            });
            return;
        }
        self.stop.reset();
        self.lost.store(false, Ordering::SeqCst);
        *self.failure.lock().unwrap_or_else(PoisonError::into_inner) = None;
        let (spec, tx, stop, busy) = (self.spec.clone(), self.tx.clone(), self.stop.clone(), self.busy.clone());
        let state = RunState { session: self.session.clone(), verified: self.verified.clone(), lost: self.lost.clone(), failure: self.failure.clone() };
        let text = text.to_string();
        let spawned = std::thread::Builder::new().name("ai-cli".into()).spawn(move || {
            let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_cli(&spec, &text, retry, &tx, &stop, &state))).unwrap_or_else(|_| {
                let _ = tx.send(AgentEvent::Error { message: "The agent runner hit a bug and stopped safely.".into(), remediation: None, log: None });
                nothing_done(&spec.config.name)
            });
            end_turn(&busy, done, |e| {
                let _ = tx.send(e);
            });
        });
        if spawned.is_err() {
            let _ = self.tx.send(AgentEvent::Error { message: "Couldn't start a worker thread.".into(), remediation: None, log: None });
            end_turn(&self.busy, nothing_done(&self.spec.config.name), |e| {
                let _ = self.tx.send(e);
            });
        }
    }
}

/// The environment an agent program runs with: Claude Code's limits raised for a long build, and
/// the effort it thinks with. Other agents ignore these names.
fn agent_env(builder: bool, effort: Option<crate::model::Effort>) -> Vec<(&'static str, String)> {
    let (tool, shell) = if builder { (24 * 3600, 8 * 3600) } else { (40 * 60, 10 * 60) };
    let ms = |secs: u64| (secs * 1000).to_string();
    vec![
        ("MCP_TOOL_TIMEOUT", ms(tool)),
        // Progress updates keep a long tool call alive; this also covers one that sends none.
        ("CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT", "0".into()),
        // A builder waits for every tool result (Ghidra above all) instead of carrying on without it.
        ("CLAUDE_CODE_MCP_AUTO_BACKGROUND_MS", if builder { "0" } else { "120000" }.into()),
        ("BASH_DEFAULT_TIMEOUT_MS", ms(shell / 2)),
        ("BASH_MAX_TIMEOUT_MS", ms(shell)),
        // High effort for Claude Code unless chosen. An environment variable rather than
        // `--effort`: it ranks highest, and an older Claude Code that doesn't know it ignores it.
        ("CLAUDE_CODE_EFFORT_LEVEL", crate::prompt::choose_effort(effort).as_str().into()),
        // Our own CLI, started from inside the agent, calls tools as an agent (not the person).
        (craft_anything_engine::AGENT_ENV, "1".into()),
    ]
}

/// The command run on another computer over SSH. ssh passes on none of our environment, so it is
/// set there with `env`. The result is the line for the remote shell, every word quoted, except
/// that a value starting with `~/` gets that computer's home folder (a quoted `~` is just a name).
fn remote_command(program: &str, args: Vec<String>, env: &[(&'static str, String)]) -> String {
    use crate::ssh::shell_quote;
    let set = env.iter().map(|(k, v)| match v.strip_prefix("~/") {
        Some(rest) => format!("{k}=\"$HOME\"/{}", shell_quote(rest)),
        None if v == "~" => format!("{k}=\"$HOME\""),
        None => shell_quote(&format!("{k}={v}")),
    });
    let words = std::iter::once(program.to_string()).chain(args).map(|a| shell_quote(&a));
    std::iter::once("env".to_string()).chain(set).chain(words).collect::<Vec<_>>().join(" ")
}

/// The run's shared state: the session it continues and reports, whether the agent's flags were
/// checked, and whether the session it was to continue was missing.
struct RunState {
    session: Arc<Mutex<Option<String>>>,
    verified: Arc<Mutex<Option<bool>>>,
    lost: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<TurnFailure>>>,
}

/// End a turn: take the next message first, then say the turn is done (`send(done)`), since
/// whoever reads `TurnDone` may send the next message at once. The other order refuses it ("Still
/// working") with no `TurnDone`, and the caller waits forever.
fn end_turn(busy: &AtomicBool, done: AgentEvent, send: impl FnOnce(AgentEvent)) {
    busy.store(false, Ordering::SeqCst);
    send(done);
}

/// The `TurnDone` that ends a turn of `backend` with no usage to report.
fn nothing_done(backend: &str) -> AgentEvent {
    AgentEvent::TurnDone { backend: backend.to_string(), input_tokens: 0, output_tokens: 0, cost_usd: None, cost_is_estimate: false }
}

/// How long a turn's process may keep running after its stream already said the turn was done
/// (Claude Code's final `result` line, or another adapter's equivalent) before we stop waiting on
/// it. The turn is over once the stream says so; a shutdown hook or cleanup step that hangs after
/// that must not leave the build "Pausing" for as long as the idle timeout allows (hours, for a
/// builder turn, since a hung hook alone never touches stdout again to reset it).
const RESULT_GRACE: Duration = Duration::from_secs(5);

/// One message as one run of the agent's program (here or over SSH), its events sent on `tx`
/// as they come. Returns the `TurnDone` that ends the turn, whatever happened, for the caller to
/// send once the conversation can take the next message. The session it reports is kept for the
/// next message; one it was asked to continue but doesn't have is forgotten and flagged in
/// `state.lost` before the error that says so is sent.
fn run_cli(spec: &CliSpec, text: &str, retry: bool, tx: &Sender<AgentEvent>, stop: &Stop, state: &RunState) -> AgentEvent {
    let RunState { session, verified, lost, failure } = state;
    let name = spec.config.name.clone();
    let agent = spec.config.model.clone();
    let cancelled = || {
        stop.is_stopped().then(|| {
            let _ = tx.send(AgentEvent::Status { text: "Stopped.".into() });
            nothing_done(&name)
        })
    };
    if let Some(done) = cancelled() {
        return done;
    }
    let program = match program_of(&spec.config, &spec.options) {
        Ok(program) => program,
        Err(e) => {
            let _ = tx.send(AgentEvent::Error { message: e.message, remediation: e.remediation, log: None });
            return nothing_done(&name);
        }
    };
    let remote = spec.config.ssh_host.clone().filter(|h| !h.trim().is_empty());
    // A folder chosen for it is on the computer it runs on: on another one, it's set below.
    let mut adapter = match remote {
        None => spec.adapters.for_backend(&agent, &spec.options),
        Some(_) => spec.adapters.for_agent(&agent),
    };
    if adapter.requires_acp() {
        let sign_in = adapter.sign_in_fix().map(|fix| format!(" {fix}")).unwrap_or_default();
        let _ = tx.send(AgentEvent::Error {
            message: format!("{name} requires a local ACP connection."),
            remediation: Some(format!("Run {} on this computer, remove the SSH host and do not disable ACP.{sign_in}", display_for(&agent))),
            log: None,
        });
        return nothing_done(&name);
    }
    // An access mode this agent can't hold (Clean room, for one whose reads we can't check) is
    // refused before anything else starts.
    if let Err(e) = adapter.validate_access(access_for(spec.builder.is_some(), spec.deep, spec.question)) {
        let _ = tx.send(AgentEvent::Error { message: e.message, remediation: e.remediation, log: None });
        return nothing_done(&name);
    }
    // The folder the agent's process starts in below.
    let cwd = remote.is_none().then(|| spec.project.clone().or_else(|| std::env::current_dir().ok())).flatten();
    if let Err(e) = adapter.check_native_options(&spec.options, cwd.as_deref()) {
        let _ = tx.send(AgentEvent::Error { message: e.message, remediation: e.remediation, log: None });
        return nothing_done(&name);
    }
    if let Err(e) = adapter.validate_model(&spec.config, &spec.options, spec.project.as_deref(), stop) {
        let event = if e.code == ErrorCode::Cancelled {
            AgentEvent::Status { text: "Stopped.".into() }
        } else {
            AgentEvent::Error { message: e.message, remediation: e.remediation, log: None }
        };
        let _ = tx.send(event);
        return nothing_done(&name);
    }
    let (help, needed) = adapter.required_flags();
    if !needed.is_empty() {
        let checked = match remote {
            Some(_) => Some(true), // can't cheaply check a remote CLI; its errors are streamed back
            None => {
                // Only a help with the flags is kept: anything else is checked again next turn, so
                // a program updated in place is seen.
                let mut v = verified.lock().unwrap_or_else(PoisonError::into_inner);
                if *v != Some(true) {
                    *v = verify_flags_with_stop(&program, help, needed, stop);
                }
                *v
            }
        };
        if let Some(done) = cancelled() {
            return done;
        }
        let ok = checked == Some(true);
        if !ok && adapter.plain_fallback() {
            let _ = tx.send(AgentEvent::Status {
                text: format!("{program}'s help doesn't list {} here, so it runs with the plain adapter (prompt in, text out).", needed.join(" / ")),
            });
            adapter = Arc::new(adapters::Plain);
            // A saved session cannot be continued through the adapter that will actually run.
            // Forget it before retry chooses between the original request and a continuation.
            *session.lock().unwrap_or_else(PoisonError::into_inner) = None;
        } else if !ok {
            // Only a help that was read and lacks the flags says which version this is (#1111).
            if let Some((message, remediation)) = adapter.missing_flags_error().filter(|_| checked == Some(false)) {
                let _ = tx.send(AgentEvent::Error { message: message.into(), remediation: Some(remediation.into()), log: None });
                return nothing_done(&name);
            }
            let _ = tx.send(AgentEvent::Status { text: format!("Couldn't confirm that {program} has {}; trying it anyway.", needed.join(" / ")) });
        }
    }
    let preamble = spec.preamble.clone().unwrap_or_else(|| crate::prompt::cli_preamble(spec.beginner));
    let mut mcp = mcp_for(spec, remote.is_some());
    if let Some(path) = remote.as_deref().and_then(adapters::remote_cli_path) {
        mcp.command = path;
    }
    let resume = session.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let text = if retry && adapter.resumes() && resume.is_some() { crate::failure::CONTINUE_TURN } else { text };
    let builder = spec.builder.is_some();
    // A question goes only to an agent that can be held to reading (a fallback adapter can't).
    if spec.question && !adapter.answers_questions() {
        let _ = tx.send(AgentEvent::Error {
            message: format!("{name} can't be kept to only reading here, so it isn't asked questions about a build."),
            remediation: Some("Ask with Claude Code, or with an AI that has an API key.".into()),
            log: None,
        });
        return nothing_done(&name);
    }
    let access = access_for(spec.builder.is_some(), spec.deep, spec.question);
    let args = build_args(adapter.as_ref(), &spec.options, text, &preamble, &mcp, if adapter.resumes() { resume.as_deref() } else { None }, access);
    let shown = FailureContext::new(&program, &args, text, remote.as_deref(), spec);
    // A builder turn may take hours and sit silent for long stretches: Ghidra's first look at a big
    // game, an engine build, or the person answering a question. Claude Code's own limits are
    // raised to match, so nothing cuts a long clone short (its defaults stop a quiet tool call at
    // 30 minutes and a background build at 10).
    let (turn, quiet) = if builder { (24 * 3600, 8 * 3600) } else { (30 * 60, 15 * 60) };
    let mut env = agent_env(builder, spec.options.effort);
    let tool_env = match &remote {
        None => adapter.mcp_env(&mcp),
        Some(host) => adapter.remote_tool_server(host, &mcp),
    };
    match tool_env {
        Ok(var) => env.extend(var),
        Err(e) => {
            let _ = tx.send(AgentEvent::Error { message: e.message, remediation: e.remediation, log: None });
            return nothing_done(&name);
        }
    }
    // Here the adapter's process runs the agent in its folder (`AgentAdapter::in_folder`); on
    // another computer over SSH it's the folder chosen there, else the agent keeps its own.
    if remote.is_some()
        && let (Some(var), Some(dir)) = (adapter.folder_var(), spec.options.config_dir_text())
    {
        env.push((var, dir.to_string()));
    }
    let (prog, argv) = match &remote {
        Some(host) => ("ssh".to_string(), crate::ssh::ssh_line_args(host, remote_command(&program, args, &env))),
        None => (program.clone(), args),
    };
    let _ =
        tx.send(AgentEvent::Status { text: format!("Asking {}{}…", display_for(&agent), remote.as_ref().map(|h| format!(" on {h}")).unwrap_or_default()) });
    // Sessions earlier turns left behind when Craft Anything was killed mid-turn (#1115). Off this
    // thread: a delete over SSH may wait on a host that's away.
    let records = spec.logs.as_deref().map(unfinished::dir_in);
    if let Some(dir) = records.clone() {
        let (adapters, tx3) = (spec.adapters.clone(), tx.clone());
        let _ = std::thread::Builder::new().name("unfinished-sessions".into()).spawn(move || {
            for text in unfinished::sweep(&dir, &adapters) {
                let _ = tx3.send(AgentEvent::Status { text });
            }
        });
    }
    // This turn's own session, recorded once the agent names it, for a sweep if we're killed.
    let records = records.filter(|_| adapter.discards_sessions());
    // Some once tried, recorded or not.
    let held: Arc<Mutex<Option<Option<unfinished::Held>>>> = Arc::new(Mutex::new(None));
    let (held2, agent2, program2, host2) = (held.clone(), agent.clone(), program.clone(), remote.clone());
    let parser = Arc::new(Mutex::new(StreamParser::new(adapter.clone())));
    let (p2, tx2, tx_err, lost2, failure2) = (parser.clone(), tx.clone(), tx.clone(), lost.clone(), failure.clone());
    let failed_tools = Arc::new(AtomicBool::new(false));
    let (failed_tools2, failed_tools3) = (failed_tools.clone(), failed_tools.clone());
    let stop2 = stop.clone();
    // Set once the stream's own terminal event is parsed, so the process is stopped `RESULT_GRACE`
    // after rather than whenever it happens to exit (its own shutdown hooks aren't ours to wait on).
    let result_seen: Arc<Mutex<Option<std::time::Instant>>> = Arc::new(Mutex::new(None));
    let (result_seen2, result_seen3) = (result_seen.clone(), result_seen.clone());
    let mut proc = (if remote.is_none() { adapter.process(&prog) } else { Process::new(&prog) })
        .args(&argv)
        .timeout(Duration::from_secs(spec.options.timeout_secs.unwrap_or(turn)))
        .idle_timeout(Duration::from_secs(quiet));
    for (k, v) in &env {
        proc = proc.env(k, v.as_str());
    }
    let mut proc = proc
        .stdout_cap(4 * 1024 * 1024)
        .cancel_when(move || {
            let seen: Option<std::time::Instant> = *result_seen3.lock().unwrap_or_else(PoisonError::into_inner);
            stop2.is_stopped() || failed_tools2.load(Ordering::SeqCst) || seen.is_some_and(|at| at.elapsed() >= RESULT_GRACE)
        })
        .on_stdout_line(move |l| {
            if failed_tools3.load(Ordering::SeqCst) {
                return;
            }
            let mut p = p2.lock().unwrap_or_else(PoisonError::into_inner);
            let events = p.line(l);
            if let (Some(dir), Some(id)) = (&records, &p.run_session) {
                let mut h = held2.lock().unwrap_or_else(PoisonError::into_inner);
                if h.is_none() {
                    *h = Some(unfinished::Held::record(dir, &unfinished::Entry::new(&agent2, &program2, host2.as_deref(), id)).ok());
                }
            }
            // Said before the error that follows it, so whoever reads the error can tell.
            if p.session_missing {
                lost2.store(true, Ordering::SeqCst);
            }
            if let Some(failed) = p.failure {
                *failure2.lock().unwrap_or_else(PoisonError::into_inner) = Some(failed);
                if failed.kind == FailureKind::ToolsUnavailable {
                    failed_tools3.store(true, Ordering::SeqCst);
                }
            }
            if p.saw_result {
                let mut seen = result_seen2.lock().unwrap_or_else(PoisonError::into_inner);
                if seen.is_none() {
                    *seen = Some(std::time::Instant::now());
                }
            }
            for e in events {
                let _ = tx2.send(e);
            }
        })
        .on_stderr_line(move |l| {
            if !l.trim().is_empty() {
                let _ = tx_err.send(AgentEvent::Output { text: l.to_string() });
            }
        })
        .remediation(format!("Install {}, or choose its program in Settings → AI.", display_for(&agent)));
    if let Some(p) = spec.project.as_ref().filter(|_| remote.is_none()) {
        proc = proc.cwd(p);
    }
    if adapter.prompt_on_stdin() {
        proc = proc.stdin_bytes(adapter.stdin_text(&preamble, text));
    }
    if let Some(done) = cancelled() {
        return done;
    }
    let result = proc.run();
    let p = parser.lock().unwrap_or_else(PoisonError::into_inner);
    if p.session_missing {
        // The id it reported is the one it couldn't find: the next message starts a new session.
        *session.lock().unwrap_or_else(PoisonError::into_inner) = None;
    } else if let Some(id) = &p.session_id {
        *session.lock().unwrap_or_else(PoisonError::into_inner) = Some(id.clone());
    }
    // The stream already said the turn was done (its terminal `result` line): whatever ended the
    // process after that — our own cancellation once `RESULT_GRACE` passed, or a wall/idle
    // deadline that happened to fire in that same window — isn't a failure of the turn itself. A
    // genuine protocol failure (`p.failure`, handled above) and a deadline hit before any result
    // was parsed still are: this never excuses a turn that didn't finish.
    let ended_after_result = p.saw_result && matches!(result.as_ref().err().map(|e| e.code), Some(ErrorCode::Cancelled) | Some(ErrorCode::TimedOut));
    if p.failure.is_none() {
        let failed = match &result {
            Ok(out) if !out.success => Some(TurnFailure::process_exit(out.code)),
            Err(e) if e.code != ErrorCode::Cancelled && !ended_after_result => Some(TurnFailure::from_process(e)),
            _ => None,
        };
        *failure.lock().unwrap_or_else(PoisonError::into_inner) = failed;
    }
    let unfinished = match &result {
        Ok(out) => !out.success,
        Err(_) => true,
    };
    match result {
        // The protocol already named the required tool server. Stop the owned process without
        // replacing that useful error with the cancellation used to stop it.
        result if failed_tools.load(Ordering::SeqCst) => {
            let (out, error) = match &result {
                Ok(out) => (Some(out), None),
                Err(e) => (None, Some(e.message.as_str())),
            };
            if let Some(path) = shown.save_with(&agent, out, Some("Craft Anything's tools did not start."), error).0 {
                let _ = tx.send(AgentEvent::Status { text: format!("The details are in {}", path.display()) });
            }
        }
        // It said why already, and there's nothing more in its output to keep.
        Ok(_) if p.session_missing => {}
        Ok(out) if !out.success && !p.saw_result => {
            let _ = tx.send(shown.exit_event(&agent, &out));
        }
        Ok(out) if !out.success => {
            // The agent already said what went wrong; keep the rest for anyone looking into it.
            if let Some(path) = shown.save(&agent, Some(&out), None).0 {
                let _ = tx.send(AgentEvent::Status { text: format!("The details are in {}", path.display()) });
            }
        }
        Ok(_) => {}
        // The stream already said the turn was done; it's that result, not this cancellation or
        // deadline, that ends the turn. Whatever kept the process itself running past that (a
        // shutdown hook, logging, cleanup) isn't ours to wait on or report as a stop or a failure.
        Err(_) if ended_after_result => {}
        Err(e) if e.code == craft_anything_evidence::ErrorCode::Cancelled => {
            let _ = tx.send(AgentEvent::Status { text: "Stopped.".into() });
        }
        Err(e) => {
            let _ = tx.send(shown.error_event(&agent, e));
        }
    }
    // #1115: a turn that didn't finish leaves its session marked as running, and the agent's
    // background service may pick it up later and carry on with nobody watching.
    let held = held.lock().unwrap_or_else(PoisonError::into_inner).take().flatten();
    let gone = match &p.run_session {
        Some(id) if unfinished => match adapter.discard_unfinished(&program, id, remote.as_deref()) {
            Ok(()) => true,
            Err(e) => {
                let fix = e.remediation.map(|r| format!(" {r}")).unwrap_or_default();
                let _ = tx.send(AgentEvent::Status { text: format!("Couldn't remove {}'s unfinished session {id}: {}.{fix}", display_for(&agent), e.message) });
                false
            }
        },
        _ => true,
    };
    // Kept (unlocked) when the delete failed, so a later sweep tries again.
    if gone && let Some(h) = held {
        h.done();
    }
    // Claude Code's total is the session's running total (#1187); a question's copy is answered by
    // question.rs, which keeps no cost for it.
    let cost_usd = if adapter.ids().contains(&"claude-code") && !spec.question {
        claude_cost::turn_cost(spec.cost_totals.as_deref(), resume.as_deref(), p.session_id.as_deref(), p.cost_usd)
    } else {
        p.cost_usd
    };
    AgentEvent::TurnDone { backend: name, input_tokens: p.input_tokens, output_tokens: p.output_tokens, cost_usd, cost_is_estimate: true }
}

/// What's needed to explain a failed CLI run: the command with the person's message left out, and
/// where to write the log.
struct FailureContext {
    command: String,
    check: String,
    program: String,
    host: Option<String>,
    backend: String,
    cwd: Option<PathBuf>,
    logs: Option<PathBuf>,
}

impl FailureContext {
    fn new(program: &str, args: &[String], message: &str, host: Option<&str>, spec: &CliSpec) -> Self {
        use craft_anything_platform::process::printable_command;
        let message = message.trim();
        let args: Vec<String> = args
            .iter()
            .map(|a| if !message.is_empty() && a.contains(message) { format!("[your message, {} characters]", message.chars().count()) } else { a.clone() })
            .collect();
        let local = printable_command(OsStr::new(program), &args);
        let (command, check) = match host {
            Some(h) => {
                let options = crate::ssh::SSH_OPTIONS.join(" ");
                (format!("ssh {options} -T {} -- {local}", crate::ssh::shell_quote(h)), crate::ssh::check_command(h, program))
            }
            None => (local, format!("{} --version", crate::ssh::shell_quote(program))),
        };
        FailureContext {
            command,
            check,
            program: program.to_string(),
            host: host.map(str::to_string),
            backend: spec.config.name.clone(),
            cwd: spec.project.clone().filter(|_| host.is_none()),
            logs: spec.logs.clone(),
        }
    }

    /// Why this run failed, when it was ssh (or the remote computer) that says so.
    fn ssh_failure(&self, out: &craft_anything_platform::Output) -> Option<crate::ssh::SshFailure> {
        let host = self.host.as_deref()?;
        crate::ssh::diagnose(host, &self.program, crate::ssh::is_git_server(host), out.code, &out.end_text())
    }

    /// Write the log. Returns its path, or what to tell the person when it couldn't be written.
    fn save(&self, agent: &str, out: Option<&craft_anything_platform::Output>, diagnosis: Option<&str>) -> (Option<PathBuf>, Option<String>) {
        self.save_with(agent, out, diagnosis, None)
    }

    fn save_with(
        &self,
        agent: &str,
        out: Option<&craft_anything_platform::Output>,
        diagnosis: Option<&str>,
        error: Option<&str>,
    ) -> (Option<PathBuf>, Option<String>) {
        let Some(dir) = &self.logs else { return (None, None) };
        let report = crate::failure_log::Report {
            what: format!("{} ({})", display_for(agent), self.backend),
            command: self.command.clone(),
            check: self.check.clone(),
            diagnosis: diagnosis.map(str::to_string),
            error: error.map(str::to_string),
            cwd: self.cwd.clone(),
            exit_code: out.and_then(|o| o.code),
            duration: out.map(|o| o.duration),
            stdout: out.map(|o| o.stdout.clone()).unwrap_or_default(),
            stderr: out.map(|o| o.stderr.clone()).unwrap_or_default(),
        };
        match crate::failure_log::write(dir, &report) {
            Ok(path) => {
                log::warn!("{} failed; details in {}", display_for(agent), path.display());
                (Some(path), None)
            }
            Err(e) => {
                log::warn!("couldn't write a failure log: {e}");
                (None, Some(format!("The details couldn't be saved: {e}")))
            }
        }
    }

    fn exit_event(&self, agent: &str, out: &craft_anything_platform::Output) -> AgentEvent {
        let name = display_for(agent);
        let plain = craft_anything_platform::process::nonzero_error_with(&name, out, Some(&self.command));
        let excerpt = craft_anything_platform::process::output_excerpt(&out.end_text());
        let (message, mut remediation) = match self.ssh_failure(out) {
            Some(f) => {
                let host = self.host.as_deref().unwrap_or_default();
                let said = if excerpt.is_empty() { String::new() } else { format!("\n\nssh printed:\n{excerpt}") };
                (format!("Couldn't run {name} on {host}: {}.", f.what), format!("{}{said}\n\nTo test it yourself, run:\n{}", f.fix, self.check))
            }
            None => (plain.message, plain.remediation.unwrap_or_default()),
        };
        let (log, unsaved) = self.save(agent, Some(out), Some(&message));
        if let Some(u) = unsaved {
            remediation.push_str(&format!("\n\n{u}"));
        }
        AgentEvent::Error { message, remediation: Some(remediation), log }
    }

    fn error_event(&self, agent: &str, e: ToolError) -> AgentEvent {
        let details = e.details.as_ref().map(ToString::to_string);
        let text = match details {
            Some(d) => format!("{}\n{d}", e.message),
            None => e.message.clone(),
        };
        let (log, unsaved) = self.save_with(agent, None, None, Some(&text));
        let remediation = match (e.remediation, unsaved) {
            (Some(r), Some(u)) => Some(format!("{r}\n\n{u}")),
            (r, u) => r.or(u),
        };
        AgentEvent::Error { message: e.message, remediation, log }
    }
}

impl Conversation for CliConversation {
    fn retry(&mut self, text: &str) {
        self.send_prompt(text, true);
    }

    fn last_failure(&self) -> Option<TurnFailure> {
        *self.failure.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn session_id(&self) -> Option<String> {
        CliConversation::session_id(self)
    }

    /// Set by the last run (see `run_cli`) and cleared when the next message is sent.
    fn session_lost(&self) -> bool {
        self.lost.load(Ordering::SeqCst)
    }

    /// Runs the message on a thread of its own; a message while one is running is refused with an
    /// error event. Every turn ends with `TurnDone`, a runner that panics or can't start included,
    /// and the conversation takes the next message before that's sent: whoever reads it may send
    /// the next message at once (a session that wasn't there is followed by a fresh start).
    fn send(&mut self, text: &str) {
        self.send_prompt(text, false);
    }

    fn poll(&mut self) -> Vec<AgentEvent> {
        self.rx.try_iter().collect()
    }

    /// Agent CLIs ask for approval through their own UI; our MCP server refuses consent-gated
    /// tools unless it was started with `--allow-run`.
    fn answer_consent(&mut self, _call_id: &str, _allow: bool) {}

    fn stop(&mut self) {
        self.stop.stop();
    }

    fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }
}

impl Drop for CliConversation {
    fn drop(&mut self) {
        self.stop.stop();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "owned subprocess fixture for startup cancellation tests"]
    fn slow_help_fixture() {
        std::thread::sleep(std::time::Duration::from_secs(30));
    }

    #[test]
    fn stop_interrupts_required_flags_preflight() {
        let executable = std::env::current_exe().unwrap();
        let stop = super::Stop::default();
        let request = stop.clone();
        let cancel = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            request.stop();
        });
        let started = std::time::Instant::now();
        let result = super::verify_flags_with_stop(
            &executable.display().to_string(),
            &["--ignored", "--exact", "cli_agents::tests::slow_help_fixture", "--nocapture"],
            &["--required"],
            &stop,
        );
        cancel.join().unwrap();
        assert_eq!(result, None);
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "Stop must interrupt the help probe before its ten-second timeout");
    }

    use super::*;

    /// #721: Codex's desktop app keeps its CLI in `bin/<build>/codex.exe` and moves it on every
    /// update; a backend set to the old path kept failing on a file that was gone.
    #[test]
    fn a_chosen_program_that_moved_with_an_update_is_found_again() {
        let d = tempfile::tempdir().unwrap();
        let bin = d.path().join("Codex/bin");
        let new = bin.join("build-2").join("codex.exe");
        std::fs::create_dir_all(new.parent().unwrap()).unwrap();
        std::fs::write(&new, b"").unwrap();
        let gone = bin.join("build-1").join("codex.exe");
        let config = |ssh: Option<&str>| BackendConfig {
            name: "Codex".into(),
            kind: crate::BackendKind::AgentCli,
            model: "codex".into(),
            base_url: None,
            ssh_host: ssh.map(str::to_string),
            enabled: true,
        };
        let chosen = |p: &Path| BackendOptions { command: Some(p.display().to_string()), ..Default::default() };
        assert_eq!(program_of(&config(None), &chosen(&gone)).unwrap(), new.display().to_string(), "the same file in the newer build folder");
        assert_eq!(program_of(&config(None), &chosen(&new)).unwrap(), new.display().to_string(), "a path that's there is used as it is");
        let program = program_of(&config(None), &chosen(&gone)).unwrap();
        assert_eq!(replaced_program(&chosen(&gone), &program), Some(gone.display().to_string()), "the gone path is named");
        assert_eq!(replaced_program(&chosen(&new), &new.display().to_string()), None, "nothing replaced");
        assert_eq!(replaced_program(&BackendOptions::default(), "codex"), None, "nothing saved");
        let bare = BackendOptions { command: Some("claude.exe".into()), ..Default::default() };
        assert_eq!(replaced_program(&bare, "claude"), None, "a bare name found as the same agent");
        let folder = new.parent().unwrap();
        assert_eq!(replaced_program(&chosen(folder), &new.display().to_string()), None, "a saved folder that's there isn't gone");
        assert_eq!(program_of(&config(Some("box")), &chosen(&gone)).unwrap(), gone.display().to_string(), "another computer's path isn't checked here");
        assert_eq!(program_of(&config(None), &BackendOptions::default()).unwrap(), "codex", "nothing chosen: the usual name");
        // #467: a saved app bundle is never started; the usual name is looked up instead.
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("Codex.app");
        std::fs::create_dir_all(&bundle).unwrap();
        assert_eq!(program_of(&config(None), &chosen(&bundle)).unwrap(), "codex", "an app bundle is not the program");
        assert_eq!(program_of(&config(Some("box")), &chosen(&bundle)).unwrap(), bundle.display().to_string(), "another computer's path isn't checked here");
        let bare = BackendOptions { command: Some("my-codex".into()), ..Default::default() };
        assert_eq!(program_of(&config(None), &bare).unwrap(), "my-codex", "a bare name is left to PATH");
    }

    /// The Claude desktop app's older installer names its app `claude.exe`; with its folder on
    /// PATH, it was offered as Claude Code (review of #781).
    #[test]
    fn the_claude_desktop_app_is_never_taken_for_claude_code() {
        for app in [
            r"C:\Users\x\AppData\Local\AnthropicClaude\claude.exe",
            r"C:\Users\x\AppData\Local\AnthropicClaude\app-0.10.0\claude.exe",
            r"C:\Program Files\WindowsApps\Claude_4.4212.0.0_x64__pzs8sxrjxfjjc\app\Claude.exe",
            "/Applications/Claude.app/Contents/MacOS/Claude",
        ] {
            let app = PathBuf::from(app.replace('\\', std::path::MAIN_SEPARATOR_STR));
            assert!(is_claude_desktop(&app), "{}", app.display());
        }
        for cli in [r"C:\Users\x\.local\bin\claude.exe", r"C:\Users\x\AppData\Roaming\npm\claude.cmd", "/Users/x/.claude/local/claude"] {
            let cli = PathBuf::from(cli.replace('\\', std::path::MAIN_SEPARATOR_STR));
            assert!(!is_claude_desktop(&cli), "{}", cli.display());
        }

        let d = tempfile::tempdir().unwrap();
        let file = if cfg!(windows) { "claude.exe" } else { "claude" };
        let app = d.path().join("AnthropicClaude").join(file);
        let cli = d.path().join("bin").join(file);
        for p in [&app, &cli] {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"").unwrap();
            #[cfg(unix)]
            std::fs::set_permissions(p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        }
        let path = |dirs: &[&Path]| std::env::join_paths(dirs).ok();
        let (app_dir, cli_dir) = (app.parent().unwrap(), cli.parent().unwrap());
        assert_eq!(find_program_in("claude", path(&[app_dir, cli_dir])), Some(cli.clone()), "the desktop app is passed over");
        assert_eq!(find_program_in("claude", path(&[app_dir])), None, "only the desktop app: no Claude Code");
        // What a run starts (review of #858): never the desktop app.
        let claude = BackendConfig {
            name: "Claude Code".into(),
            kind: crate::BackendKind::AgentCli,
            model: "claude-code".into(),
            base_url: None,
            ssh_host: None,
            enabled: true,
        };
        let none = BackendOptions::default();
        let gone = BackendOptions { command: Some(d.path().join("old").join("claude").join(file).display().to_string()), ..Default::default() };
        let real = cli.display().to_string();
        assert_eq!(program_in(&claude, &none, path(&[app_dir, cli_dir])).unwrap(), real, "the real one, by path, when the app comes first");
        assert_eq!(program_in(&claude, &gone, path(&[app_dir, cli_dir])).unwrap(), real, "a saved path that's gone falls back to the real one");
        assert_eq!(program_in(&claude, &none, path(&[cli_dir, app_dir])).unwrap(), "claude", "the name, when it starts the real one");
        let err = program_in(&claude, &none, path(&[app_dir])).unwrap_err();
        assert_eq!(err.code, ErrorCode::EngineMissing);
        assert!(err.message.contains("desktop app"), "{}", err.message);
        assert_eq!(program_in(&claude, &gone, path(&[app_dir])).unwrap(), gone.command.clone().unwrap(), "only the app: the saved path, not the app");
        // A saved `claude`, by name or as the app's own file (review of #1017), is filtered the same way.
        let saved = |c: &str| BackendOptions { command: Some(c.into()), ..Default::default() };
        let (by_name, the_app) = (saved(file), saved(&app.display().to_string()));
        assert_eq!(program_in(&claude, &by_name, path(&[app_dir, cli_dir])).unwrap(), real, "a saved name, the app first");
        assert_eq!(program_in(&claude, &by_name, path(&[cli_dir, app_dir])).unwrap(), file, "a saved name that starts the real one is kept");
        assert_eq!(program_in(&claude, &by_name, path(&[app_dir])).unwrap_err().code, ErrorCode::EngineMissing);
        assert_eq!(program_in(&claude, &by_name, path(&[])).unwrap(), file, "nothing found: the saved name, so the run says so");
        assert_eq!(program_in(&claude, &the_app, path(&[app_dir, cli_dir])).unwrap(), real, "the app's own file saved: the real one");
        let err = program_in(&claude, &the_app, path(&[])).unwrap_err();
        assert!(err.code == ErrorCode::EngineMissing && err.message.contains("desktop app"), "{}", err.message);
        let saved_cli = saved(&cli.display().to_string());
        assert_eq!(program_in(&claude, &saved_cli, path(&[app_dir])).unwrap(), real, "a saved real path is kept");
        // A wrapper saved by name (`claude.cmd`) stays, though another real `claude` comes first
        // (review of #1036).
        let wrapper = d.path().join("wrap").join(if cfg!(windows) { "claude.cmd" } else { "claude.sh" });
        std::fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
        std::fs::write(&wrapper, b"").unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&wrapper, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let wrapper_name = wrapper.file_name().unwrap().to_string_lossy().to_string();
        let wrap_dir = wrapper.parent().unwrap();
        assert_eq!(program_in(&claude, &saved(&wrapper_name), path(&[cli_dir, wrap_dir])).unwrap(), wrapper_name);
        assert_eq!(program_in(&claude, &saved(&wrapper_name), path(&[app_dir, wrap_dir])).unwrap(), wrapper_name, "the app doesn't hide it");
        // A link to the desktop app is the desktop app (review of #1036). Making a link may need
        // rights Windows doesn't give everyone; then there's nothing to check.
        let link_dir = d.path().join("links");
        std::fs::create_dir_all(&link_dir).unwrap();
        let link = link_dir.join(file);
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&app, &link).is_ok();
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(&app, &link).is_ok();
        if made {
            assert_eq!(find_program_in("claude", path(&[&link_dir, cli_dir])), Some(cli.clone()), "the link is passed over");
            assert_eq!(program_in(&claude, &none, path(&[&link_dir, cli_dir])).unwrap(), real);
            assert_eq!(program_in(&claude, &none, path(&[&link_dir])).unwrap_err().code, ErrorCode::EngineMissing);
            let saved_link = saved(&link.display().to_string());
            assert_eq!(program_in(&claude, &saved_link, path(&[cli_dir])).unwrap(), "claude", "a saved link to the app: the name starts the real one");
            assert_eq!(program_in(&claude, &saved_link, path(&[app_dir, cli_dir])).unwrap(), real, "a saved link to the app, the app first");
        }
        // Another agent's program in such a folder is only filtered for Claude.
        let other = app_dir.join(if cfg!(windows) { "codex.exe" } else { "codex" });
        std::fs::write(&other, b"").unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&other, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        assert_eq!(find_program_in("codex", path(&[app_dir])), Some(other));
    }

    #[test]
    fn failed_craft_tool_server_is_reported_from_its_init_status() {
        let mut p = StreamParser::new(adapter_for("claude-code"));
        let events = p.line(r#"{"type":"system","subtype":"init","session_id":"cannot-build","mcp_servers":[{"name":"craft-anything","status":"failed"}]}"#);
        assert!(
            events.iter().any(|e| matches!(e, AgentEvent::Error { message, .. } if message.contains("tools") && message.contains("start"))),
            "a failed required tool server must end the turn with an actionable error: {events:?}"
        );
        assert_eq!(p.failure, Some(TurnFailure::new(FailureKind::ToolsUnavailable)));
    }

    #[test]
    fn optional_or_pending_mcp_servers_do_not_block_the_builder() {
        for status in ["connected", "pending"] {
            let mut p = StreamParser::new(adapter_for("claude-code"));
            let init = json!({"type":"system","subtype":"init","mcp_servers":[{"name":"craft-anything","status":status},{"name":"optional-service","status":"failed"}]});
            assert!(p.line(&init.to_string()).is_empty());
            assert!(p.failure.is_none());
        }
        let mut p = StreamParser::new(adapter_for("claude-code"));
        assert!(!p.line(r#"{"type":"system","subtype":"init","mcp_servers":[{"name":"craft-anything","status":"needs-auth"}]}"#).is_empty());
        assert_eq!(p.failure, Some(TurnFailure::new(FailureKind::ToolsUnavailable)));
    }

    #[test]
    fn claude_assistant_error_codes_survive_localized_display_messages() {
        for (code, kind) in [
            ("authentication_failed", FailureKind::Authentication),
            ("billing_error", FailureKind::Billing),
            ("rate_limit", FailureKind::RateLimited),
            ("server_error", FailureKind::Transient),
        ] {
            let mut p = StreamParser::new(adapter_for("claude-code"));
            let events = p.line(&json!({"type":"assistant","error":code,"message":{"content":[{"type":"text","text":"服务暂时不可用"}]}}).to_string());
            assert!(matches!(events.as_slice(), [AgentEvent::Error { .. }]));
            assert_eq!(p.failure, Some(TurnFailure::new(kind)));
            p.line(r#"{"type":"result","is_error":true,"result":"the turn ended"}"#);
            assert_eq!(p.failure, Some(TurnFailure::new(kind)), "result must keep the more specific code");
        }
    }

    /// #506: under a Flatpak `~/.claude` is read-only, so Claude Code gets a folder of ours to save
    /// its conversations in (so `--resume` finds them); the person's own `CLAUDE_CONFIG_DIR` and a
    /// normal install are left as they are, and the sign-in hint says how to sign in there.
    #[test]
    fn under_a_flatpak_claude_code_keeps_its_conversations_in_a_folder_it_can_write() {
        let data = || Some(std::path::PathBuf::from("/home/u/.var/app/ai.storyteller.craft-anything/data"));
        let dir = adapters::flatpak_claude_dir_in(true, false, data()).unwrap();
        assert!(dir.ends_with("craft-anything/claude") && dir.starts_with("/home/u/.var/app/ai.storyteller.craft-anything/data"), "{dir:?}");
        assert_eq!(adapters::flatpak_claude_dir_in(true, true, data()), None, "their own choice stays");
        assert_eq!(adapters::flatpak_claude_dir_in(false, false, data()), None, "a normal install keeps ~/.claude");
        assert_eq!(adapters::flatpak_claude_dir_in(true, false, None), None);
        let hint = sign_in_hint_in("Not logged in", true, None).unwrap();
        assert!(hint.contains("flatpak run") && hint.contains("CLAUDE_CONFIG_DIR") && hint.contains("auth login"), "{hint}");
        assert!(sign_in_hint_in("Not logged in", false, None).unwrap().contains("`claude auth login`"));
        assert_eq!(sign_in_hint_in("all fine", true, None), None);
    }

    /// Claude Code in a folder chosen for it signs in there: the fix names that folder, quoted.
    #[test]
    fn signing_in_claude_code_in_its_chosen_folder_names_the_folder() {
        let folder = Path::new("/Users/someone/Claude work");
        let hint = sign_in_hint_in("Not logged in", false, Some(folder)).unwrap();
        if cfg!(windows) {
            assert!(hint.contains("$env:CLAUDE_CONFIG_DIR='/Users/someone/Claude work'; claude auth login"), "{hint}");
        } else {
            assert!(hint.contains("`CLAUDE_CONFIG_DIR='/Users/someone/Claude work' claude auth login`"), "{hint}");
        }
        let hint = sign_in_hint_in("Not logged in", true, Some(folder)).unwrap();
        assert!(hint.starts_with("Sign in to Claude Code inside this app's sandbox, in the folder") && hint.contains("Claude work"), "{hint}");
        assert!(!hint.contains("XDG_DATA_HOME"), "the chosen folder, not the sandbox's own: {hint}");
        let (_, fix) = claude_code_failure_in("Invalid API key · Please run /login", Some(folder));
        assert!(fix.is_some_and(|f| f.contains("Claude work")));
    }

    /// Over SSH our environment doesn't reach the other computer, so the chosen effort (and Claude
    /// Code's raised limits) go in the remote command itself.
    #[test]
    fn an_agent_over_ssh_gets_its_effort_on_the_other_computer() {
        let env = agent_env(true, Some(crate::model::Effort::Max));
        assert!(env.contains(&("CLAUDE_CODE_EFFORT_LEVEL", "max".to_string())));
        assert!(env.contains(&("CLAUDE_CODE_MCP_AUTO_BACKGROUND_MS", "0".to_string())));
        let line = remote_command("claude", vec!["-p".into(), "it's".into()], &env);
        assert!(line.starts_with("env ") && line.contains(" CLAUDE_CODE_EFFORT_LEVEL=max ") && line.ends_with(" claude -p 'it'\\''s'"), "{line}");
        assert_eq!(crate::ssh::ssh_line_args("box", line.clone()).pop(), Some(line));
        assert!(agent_env(false, None).contains(&("CLAUDE_CODE_EFFORT_LEVEL", "high".to_string())), "high unless chosen");
    }

    /// A folder under `~/` is in the other computer's home: its shell expands it, which it won't
    /// do for a quoted `~`. Anything else stays quoted.
    #[test]
    fn a_folder_over_ssh_is_in_the_other_computers_home() {
        let set = |v: &str| remote_command("claude", vec![], &[("CLAUDE_CONFIG_DIR", v.to_string())]);
        assert_eq!(set("~/.claude-work"), "env CLAUDE_CONFIG_DIR=\"$HOME\"/.claude-work claude");
        assert_eq!(set("~/Claude work"), "env CLAUDE_CONFIG_DIR=\"$HOME\"/'Claude work' claude");
        assert_eq!(set("~"), "env CLAUDE_CONFIG_DIR=\"$HOME\" claude");
        assert_eq!(set("/srv/claude/a b"), "env 'CLAUDE_CONFIG_DIR=/srv/claude/a b' claude");
        assert_eq!(set("/x/~/$(rm)"), "env 'CLAUDE_CONFIG_DIR=/x/~/$(rm)' claude");
    }

    /// A sign-in that stopped working is told apart from none by the account `.claude.json` still
    /// names; anything else there names none.
    #[test]
    fn claude_code_was_signed_in_when_its_settings_name_an_account() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".claude.json");
        let names = |text: &[u8]| {
            std::fs::write(&file, text).unwrap();
            adapters::claude_names_account(&file)
        };
        assert!(names(br#"{"numStartups":3,"oauthAccount":{"accountUuid":"0000","emailAddress":"someone@example.com"}}"#));
        for none in [&br#"{"numStartups":3}"#[..], br#"{"oauthAccount":null}"#, br#"{"oauthAccount":"x"}"#, b"not json", b"", b"[1,2]"] {
            assert!(!names(none), "{}", String::from_utf8_lossy(none));
        }
        assert!(!adapters::claude_names_account(&dir.path().join("missing.json")));
        assert!(!adapter_for("codex").signed_in_before());
    }

    /// Claude Code in a chosen folder runs, checks its sign-in and keeps its sessions there; an
    /// agent without such a folder is the same as before.
    #[test]
    fn claude_code_uses_the_folder_chosen_for_it() {
        let config = tempfile::tempdir().unwrap();
        let id = "0e4a6b1c-1111-4222-8333-944455556666";
        let folder = config.path().join("projects").join("-somewhere");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(format!("{id}.jsonl")), "{}\n").unwrap();
        let options = crate::BackendOptions { config_dir: Some(config.path().display().to_string()), ..Default::default() };
        let claude = AgentAdapters::built_in().for_backend("claude-code", &options);
        assert_eq!(claude.session_file(id), Some(folder.join(format!("{id}.jsonl"))));
        assert!(claude.sign_in_fix().is_some_and(|f| f.contains(&config.path().display().to_string())), "signs in there");
        assert_eq!(claude.folder_var(), Some("CLAUDE_CONFIG_DIR"));
        assert!(!claude.signed_in_before(), "nothing there names an account");
        std::fs::write(config.path().join(".claude.json"), r#"{"oauthAccount":{"accountUuid":"0000"},"projects":{}}"#).unwrap();
        assert!(claude.signed_in_before(), "its own folder's .claude.json, not the usual one");
        let codex = AgentAdapters::built_in().for_backend("codex", &options);
        assert_eq!((codex.folder_var(), codex.ids()), (None, adapter_for("codex").ids()));
        assert_eq!(crate::config::config_dir_in("~/a/b", Some("/home/u".into())), Some(std::path::PathBuf::from("/home/u/a/b")));
        assert_eq!(crate::config::config_dir_in("~", Some("/home/u".into())), Some(std::path::PathBuf::from("/home/u")));
        assert_eq!(crate::config::config_dir_in("~/a", None), None, "no home to put it in");
        assert_eq!(crate::config::config_dir_in("/srv/c", None), Some(std::path::PathBuf::from("/srv/c")));
    }
    use crate::BackendKind;

    /// Explicit OpenCode picks run natively, where the tool server comes through inline config.
    #[test]
    fn explicit_opencode_picks_run_natively_with_our_tools() {
        let adapter = adapter_for("opencode");
        let defaults = BackendOptions { acp: Some(true), ..Default::default() };
        assert!(selected_acp_args(adapter.as_ref(), "opencode", &defaults).is_some());
        for (model, thinking) in [(Some("test/model"), None), (None, Some(crate::Effort::High)), (Some("test/model"), Some(crate::Effort::Max))] {
            for acp in [false, true] {
                let options = BackendOptions { cli_model: model.map(str::to_string), cli_thinking: thinking, acp: Some(acp), ..Default::default() };
                assert!(selected_acp_args(adapter.as_ref(), "opencode", &options).is_none());
                assert!(adapter.check_native_options(&options, None).is_ok());
            }
        }
        let mcp = McpCommand { command: "craft-anything-cli".into(), args: vec![], notes: None };
        assert!(adapter.mcp_env(&mcp).unwrap().is_some());
    }

    /// Reject non-executable candidates and prefer the user's application installation.
    #[cfg(unix)]
    #[test]
    fn macos_bundled_codex_needs_an_executable_and_respects_application_root_order() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let user = d.path().join("User Applications");
        let system = d.path().join("Applications");
        let relative = "ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex";
        let user_cli = user.join(relative);
        let system_cli = system.join("Codex.app/Contents/Resources/codex");
        assert!(macos_codex_in([user.clone(), system.clone()]).is_none());
        std::fs::create_dir_all(user_cli.parent().unwrap()).unwrap();
        std::fs::write(&user_cli, b"not executable").unwrap();
        std::fs::set_permissions(&user_cli, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(macos_codex_in([user.clone()]).is_none());
        std::fs::create_dir_all(system_cli.parent().unwrap()).unwrap();
        std::fs::write(&system_cli, b"stand-in").unwrap();
        std::fs::set_permissions(&system_cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(macos_codex_in([user.clone(), system.clone()]), Some(system_cli));
        std::fs::set_permissions(&user_cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(macos_codex_in([user, system]), Some(user_cli));
    }

    /// Every supported bundle accepts its CLI, never its GUI or a directory at the CLI path.
    #[cfg(unix)]
    #[test]
    fn every_supported_macos_codex_bundle_layout_is_found_without_the_gui() {
        use std::os::unix::fs::PermissionsExt;
        for relative in [
            "ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex",
            "ChatGPT.app/Contents/Resources/codex",
            "Codex.app/Contents/Resources/codex",
            "Codex.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex",
        ] {
            let d = tempfile::tempdir().unwrap();
            let cli = d.path().join(relative);
            std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
            // A directory and an app GUI executable are never CLI candidates.
            std::fs::create_dir_all(&cli).unwrap();
            assert!(macos_codex_in([d.path().into()]).is_none());
            std::fs::remove_dir(&cli).unwrap();
            let gui = d.path().join("ChatGPT.app/Contents/MacOS/ChatGPT");
            std::fs::create_dir_all(gui.parent().unwrap()).unwrap();
            std::fs::write(&gui, b"stand-in GUI, never run").unwrap();
            std::fs::set_permissions(&gui, std::fs::Permissions::from_mode(0o700)).unwrap();
            assert!(macos_codex_in([d.path().into()]).is_none());
            std::fs::write(&cli, b"stand-in CLI, never run").unwrap();
            std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
            assert_eq!(macos_codex_in([d.path().into()]), Some(cli));
        }
    }

    /// Saved usual names share desktop lookup for every agent, while SSH names remain remote.
    #[test]
    fn saved_bare_agent_names_share_the_usual_lookup() {
        let empty = std::env::join_paths(std::iter::empty::<&Path>()).ok();
        for (model, program, _) in KNOWN {
            let mut config =
                BackendConfig { name: "Agent test".into(), kind: BackendKind::AgentCli, model: (*model).into(), base_url: None, ssh_host: None, enabled: true };
            let options = BackendOptions { command: Some((*program).into()), ..Default::default() };
            assert_eq!(
                program_in(&config, &options, empty.clone()).unwrap(),
                program_in(&config, &BackendOptions::default(), empty.clone()).unwrap(),
                "{model}"
            );
            config.ssh_host = Some("test-host".into());
            assert_eq!(program_in(&config, &options, empty.clone()).unwrap(), *program, "SSH never resolves a local desktop CLI");
        }
    }

    /// #1060: OpenCode's desktop app keeps its CLI in its own folder, as `opencode-cli.exe`.
    #[test]
    fn a_cli_inside_its_desktop_app_is_found() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(desktop_cli("opencode", Some(d.path())), None, "not installed");
        assert_eq!(desktop_cli("opencode", None), None, "not Windows");
        let cli = d.path().join(IN_DESKTOP_APP[0].1);
        std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
        std::fs::write(&cli, b"").unwrap();
        assert_eq!(desktop_cli("opencode", Some(d.path())), Some(cli));
        assert_eq!(desktop_cli("codex", Some(d.path())), None, "only the agents listed");
    }

    const UNFINISHED_CHILD_ENV: &str = "CRAFT_ANYTHING_TEST_UNFINISHED_CHILD";

    /// The stand-in agent of `a_stopped_turn_discards_its_session`: it names its session in
    /// OpenCode's format, then finishes or hangs as told. Run as an ordinary test, it does nothing.
    #[test]
    fn unfinished_child() {
        let Some(how) = std::env::var_os(UNFINISHED_CHILD_ENV) else { return };
        // On a line of its own: libtest has already written "test … ... " without a line break.
        println!("\n{}", json!({"type":"text","sessionID":"ses_child","part":{"text":"working"}}));
        std::io::Write::flush(&mut std::io::stdout()).unwrap();
        if how == "hang" {
            std::thread::sleep(Duration::from_secs(60));
        }
    }

    /// #1115: a turn stopped part-way removes the session it started, so the agent's background
    /// service can't pick it up again; a turn that finishes leaves its session alone.
    #[test]
    fn a_stopped_turn_discards_its_session() {
        struct StandIn {
            how: &'static str,
            discarded: Arc<Mutex<Vec<String>>>,
        }
        impl AgentAdapter for StandIn {
            /// Register the fixture without replacing any built-in agent.
            fn ids(&self) -> &'static [&'static str] {
                &["test-unfinished"]
            }
            /// Run only `unfinished_child` in this test program.
            fn args(&self, _cx: &ArgsCx) -> Vec<String> {
                ["--exact", "--test-threads=1", "--nocapture", "cli_agents::tests::unfinished_child"].map(String::from).to_vec()
            }
            fn streams_json(&self) -> bool {
                true
            }
            /// Tell the child how to behave (the runner hands the agent this variable).
            fn mcp_env(&self, _mcp: &McpCommand) -> Result<Option<(&'static str, String)>, ToolError> {
                Ok(Some((UNFINISHED_CHILD_ENV, self.how.to_string())))
            }
            /// Read the stream as OpenCode's adapter does.
            fn parse(&self, p: &mut StreamParser, v: &Value) -> Vec<AgentEvent> {
                adapters::OpenCode.parse(p, v)
            }
            /// It stands in for an agent whose sessions are deleted.
            fn discards_sessions(&self) -> bool {
                true
            }
            /// Record what would be deleted.
            fn discard_unfinished(&self, _program: &str, id: &str, _host: Option<&str>) -> Result<(), ToolError> {
                self.discarded.lock().unwrap_or_else(PoisonError::into_inner).push(id.to_string());
                Ok(())
            }
        }
        for (how, stopped) in [("hang", true), ("finish", false)] {
            let discarded = Arc::new(Mutex::new(Vec::new()));
            let config = BackendConfig {
                name: "Test".into(),
                kind: BackendKind::AgentCli,
                model: "test-unfinished".into(),
                base_url: None,
                ssh_host: None,
                enabled: true,
            };
            let options = BackendOptions { command: Some(std::env::current_exe().unwrap().display().to_string()), ..Default::default() };
            let mut spec = CliSpec::new(config, options, false, None);
            let logs = tempfile::tempdir().unwrap();
            spec.logs = Some(logs.path().to_path_buf());
            spec.adapters.register(Arc::new(StandIn { how, discarded: discarded.clone() }));
            let mut c = CliConversation::new(spec);
            c.send("go");
            let deadline = std::time::Instant::now() + Duration::from_secs(60);
            let mut all = Vec::new();
            while !all.iter().any(|e| matches!(e, AgentEvent::TurnDone { .. })) {
                assert!(std::time::Instant::now() < deadline, "{how}: no TurnDone: {all:?}");
                let got = c.poll();
                // Stopped once it's working, as Pause does.
                if stopped && got.iter().any(|e| matches!(e, AgentEvent::TextDelta { .. })) {
                    c.stop();
                }
                all.extend(got);
                std::thread::sleep(Duration::from_millis(10));
            }
            let want: &[&str] = if stopped { &["ses_child"] } else { &[] };
            assert_eq!(*discarded.lock().unwrap_or_else(PoisonError::into_inner), want, "{how}: {all:?}");
            // Its record of the session goes once the session is deleted or finished.
            let left = std::fs::read_dir(unfinished::dir_in(logs.path())).map(|rd| rd.flatten().count());
            assert_eq!(left.ok(), Some(0), "{how}: the session was recorded, then its record removed");
        }
    }

    /// Validation failures finish exactly once, preserving cancellation status and other error details.
    #[test]
    fn cancelled_model_validation_reports_stopped_and_other_failures_stay_errors() {
        struct RefusedModel(ErrorCode);
        impl AgentAdapter for RefusedModel {
            /// Register the fixture without replacing any built-in agent.
            fn ids(&self) -> &'static [&'static str] {
                &["test-validation"]
            }
            /// Fail the test if a rejected model reaches command construction.
            fn args(&self, _cx: &ArgsCx) -> Vec<String> {
                panic!("failed validation must not start the agent")
            }
            /// Return the selected validation failure without performing I/O.
            fn validate_model(&self, _config: &BackendConfig, _options: &BackendOptions, _directory: Option<&Path>, _stop: &Stop) -> Result<(), ToolError> {
                Err(ToolError::new(self.0, "Validation stopped").with_remediation("Reload the models."))
            }
        }
        for code in [ErrorCode::Cancelled, ErrorCode::BadInput, ErrorCode::TimedOut] {
            let config = BackendConfig {
                name: "Test".into(),
                kind: BackendKind::AgentCli,
                model: "test-validation".into(),
                base_url: None,
                ssh_host: None,
                enabled: true,
            };
            let mut spec = CliSpec::new(config, BackendOptions::default(), false, None);
            spec.adapters.register(Arc::new(RefusedModel(code)));
            let mut conversation = CliConversation::new(spec);
            conversation.send("test");
            let events: Vec<_> = (0..2).map(|_| conversation.rx.recv_timeout(Duration::from_secs(5)).unwrap()).collect();
            assert!(!conversation.is_busy(), "the next message is accepted before TurnDone");
            assert!(conversation.poll().is_empty(), "exactly one terminal event");
            if code == ErrorCode::Cancelled {
                assert!(matches!(events.as_slice(), [AgentEvent::Status { text }, AgentEvent::TurnDone { .. }] if text == "Stopped."), "{events:?}");
            } else {
                assert!(
                    matches!(events.as_slice(), [AgentEvent::Error { message, remediation: Some(help), log: None }, AgentEvent::TurnDone { .. }]
                    if message == "Validation stopped" && help == "Reload the models."),
                    "{events:?}"
                );
            }
            assert!(
                matches!(events.last(), Some(AgentEvent::TurnDone {
                backend, input_tokens: 0, output_tokens: 0, cost_usd: None, cost_is_estimate: false,
            }) if backend == "Test"),
                "{events:?}"
            );
        }
    }

    fn mcp() -> McpCommand {
        McpCommand { command: "/opt/ca/craft-anything-cli".into(), args: vec!["mcp".into(), "--project".into(), "/p/x.cany".into()], notes: None }
    }

    #[test]
    fn claude_args() {
        let a = build_args(adapter_for("claude-code").as_ref(), &BackendOptions::default(), "-what?", "PRE", &mcp(), Some("sess-1"), Access::Analyse);
        assert_eq!(a[0], "-p");
        // #402: the message goes in on stdin, so a long one can't pass Windows' command-line limit.
        assert!(adapter_for("claude-code").prompt_on_stdin() && !a.iter().any(|x| x.contains("what?")), "{a:?}");
        let pos = |f: &str| a.iter().position(|x| x == f).unwrap();
        assert_eq!(a[pos("--output-format") + 1], "stream-json");
        assert!(a.contains(&"--verbose".to_string()));
        assert_eq!(a[pos("--tools") + 1], "", "built-in tools are opt-in");
        for access in [Access::Build, Access::CleanRoomBuild] {
            let builder = build_args(adapter_for("claude-code").as_ref(), &BackendOptions::default(), "build it", "PRE", &mcp(), None, access);
            let builder_pos = builder.iter().position(|x| x == "--tools").unwrap();
            let tools = &builder[builder_pos + 1];
            assert!(tools.split(',').any(|tool| tool == "Bash"), "{access:?}: {tools}");
            assert!(tools.split(',').any(|tool| tool == "Write"), "{access:?}: {tools}");
            // #1105: a Deep builder can start subagents; Clean room keeps them out.
            assert_eq!(tools.split(',').any(|tool| tool == "Agent"), access == Access::Build, "{access:?}: {tools}");
        }
        assert_eq!(a[pos("--resume") + 1], "sess-1");
        assert_eq!(a[pos("--allowedTools") + 1], "mcp__craft-anything__*");
        assert_eq!(a[pos("--model") + 1], "claude-opus-5-5", "Opus 5.5 unless configured otherwise");
        let o = BackendOptions { cli_model: Some("opus".into()), ..Default::default() };
        let b = build_args(adapter_for("claude-code").as_ref(), &o, "q", "PRE", &mcp(), None, Access::Analyse);
        assert_eq!(b[b.iter().position(|x| x == "--model").unwrap() + 1], "opus");
        let cfg: Value = serde_json::from_str(&a[pos("--mcp-config") + 1]).unwrap();
        assert_eq!(cfg["mcpServers"]["craft-anything"]["args"][0], "mcp");
        // variadic flags come last: nothing after them is read as one of their values
        assert!(pos("--mcp-config") > pos("--resume"));
    }

    /// #540: only tools that ask the person themselves are approved on Codex's side.
    #[test]
    fn every_tool_approved_for_codex_is_registered_and_asks_for_consent() {
        let e = craft_anything_engine::Engine::shared(craft_anything_engine::testing::session(), crate::rebuild::tools::specs());
        for id in craft_anything_engine::AGENT_APP_PREAPPROVED_TOOLS {
            let spec = e.registry().get(id).unwrap_or_else(|| panic!("{id} isn't registered"));
            assert!(spec.effects.needs_consent(), "{id} must ask for consent itself");
        }
    }

    /// #540: a refused Codex tool call came back as "null"; it says why and how to fix it.
    #[test]
    fn a_codex_tool_refused_for_approval_explains_the_way_out() {
        let mut p = StreamParser::new(adapter_for("codex"));
        let line = r#"{"type":"item.completed","item":{"id":"i1","type":"mcp_tool_call","server":"craft-anything","tool":"rebuild_start","status":"failed","result":null,"error":{"message":"tool rebuild_start requires approval, but approval policy is never"}}}"#;
        let v: Value = serde_json::from_str(line).unwrap();
        let events = adapter_for("codex").parse(&mut p, &v);
        let text = format!("{events:?}");
        assert!(text.contains("setup --client codex --yes"), "{text}");
        // Another server's refusal keeps Codex's words: our setup can't fix it.
        let mut p = StreamParser::new(adapter_for("codex"));
        let theirs = line.replace("craft-anything", "other-server");
        let text = format!("{:?}", adapter_for("codex").parse(&mut p, &serde_json::from_str::<Value>(&theirs).unwrap()));
        assert!(text.contains("approval policy is never") && !text.contains("setup --client codex"), "{text}");
        let mut p = StreamParser::new(adapter_for("codex"));
        let v: Value = serde_json::from_str(line).unwrap();
        let events = adapter_for("codex").parse(&mut p, &v);
        let text = format!("{events:?}");
        assert!(!text.contains("\"null\""), "{text}");
        // Any other failure shows its own message, not the JSON around it.
        let mut p = StreamParser::new(adapter_for("codex"));
        let other =
            r#"{"type":"item.completed","item":{"id":"i2","type":"mcp_tool_call","tool":"x","status":"failed","result":null,"error":{"message":"boom"}}}"#;
        let text = format!("{:?}", adapter_for("codex").parse(&mut p, &serde_json::from_str::<Value>(other).unwrap()));
        assert!(text.contains("boom") && !text.contains("null"), "{text}");
    }

    #[test]
    fn codex_and_gemini_args() {
        let a = build_args(adapter_for("codex").as_ref(), &BackendOptions::default(), "q", "P:", &mcp(), None, Access::Analyse);
        assert_eq!(&a[..2], &["exec".to_string(), "--json".to_string()]);
        // #69: project folders aren't git repositories; the flag comes once, also when the
        // backend's own args already have it.
        let skip = |a: &[String]| a.iter().filter(|x| *x == "--skip-git-repo-check").count();
        assert_eq!(skip(&a), 1);
        let o = BackendOptions { args: vec!["--skip-git-repo-check".into()], ..Default::default() };
        assert_eq!(skip(&build_args(adapter_for("codex").as_ref(), &o, "q", "P:", &mcp(), None, Access::Build)), 1);
        assert!(a.contains(&"mcp_servers.craft-anything.command=\"/opt/ca/craft-anything-cli\"".to_string()));
        // #538: Codex doesn't refresh its callable tools after enable_toolset, so list all permitted
        // tools from startup. Clean room still filters what the server lists and allows.
        assert!(a.contains(&"mcp_servers.craft-anything.args=[\"mcp\", \"--project\", \"/p/x.cany\", \"--all-tools\"]".to_string()));
        let mut already_all = mcp();
        already_all.args.push("--all-tools".into());
        let deep = build_args(adapter_for("codex").as_ref(), &BackendOptions::default(), "q", "P:", &already_all, None, Access::Build);
        let args = deep.iter().find(|a| a.starts_with("mcp_servers.craft-anything.args=")).unwrap();
        assert_eq!(args.matches("--all-tools").count(), 1, "a Deep build already lists every tool");
        // #528: the message goes in on stdin; `-` tells `codex exec` to read it there.
        assert_eq!(a.last().unwrap(), "-");
        // #540: `codex exec` can't ask, so the tools that ask for themselves are approved in the
        // same -c list, and nothing wider.
        assert!(a.contains(&"mcp_servers.craft-anything.tools.rebuild_start.approval_mode=\"approve\"".to_string()));
        assert!(!a.iter().any(|x| x.contains("default_tools_approval_mode")));
        let g = build_args(adapter_for("gemini-cli").as_ref(), &BackendOptions::default(), "q", "", &mcp(), None, Access::Analyse);
        assert!(g.windows(2).any(|w| w[0] == "--output-format" && w[1] == "stream-json"));
        let o = BackendOptions { args: vec!["run".into()], ..Default::default() };
        assert_eq!(build_args(adapter_for("no-such-agent").as_ref(), &o, "q", "", &mcp(), None, Access::Analyse), vec!["run".to_string(), "q".to_string()]);
    }

    #[test]
    fn codex_long_prompt_goes_in_on_stdin() {
        // #528: a long message in argv passes Windows' command-line limit (os error 206).
        let codex = adapter_for("codex");
        let prompt = "x".repeat(40_831);
        let a = build_args(codex.as_ref(), &BackendOptions::default(), &prompt, "P:", &mcp(), None, Access::Build);
        assert!(codex.prompt_on_stdin(), "codex must read its message from stdin");
        assert!(!a.iter().any(|x| x.contains(&prompt) || x.contains("P:")), "{a:?}");
        assert!(a.iter().map(|x| x.len() + 1).sum::<usize>() < 4096, "argv stays short");
        assert_eq!(a.last().unwrap(), "-", "codex exec reads stdin when told to");
        assert_eq!(&a[..2], &["exec".to_string(), "--json".to_string()]);
        assert!(a.contains(&"--skip-git-repo-check".to_string()));
        assert!(a.windows(2).any(|w| w[0] == "--sandbox" && w[1] == "workspace-write"));
        assert!(a.contains(&"mcp_servers.craft-anything.command=\"/opt/ca/craft-anything-cli\"".to_string()));
        assert!(a.contains(&"mcp_servers.craft-anything.args=[\"mcp\", \"--project\", \"/p/x.cany\", \"--all-tools\"]".to_string()));
    }

    #[test]
    fn gemini_long_prompt_goes_in_on_stdin() {
        // npm starts Gemini CLI through `gemini.cmd`, where cmd.exe allows 8,191 characters.
        let gemini = adapter_for("gemini-cli");
        let prompt = "x".repeat(40_831);
        let o = BackendOptions { cli_model: Some("gemini-2.5-pro".into()), ..Default::default() };
        let a = build_args(gemini.as_ref(), &o, &prompt, "P:", &mcp(), None, Access::Analyse);
        assert!(gemini.prompt_on_stdin(), "gemini must read its message from stdin");
        assert_eq!(gemini.stdin_text("P:", &prompt), format!("P:{prompt}"), "the whole message, preamble first");
        assert!(!a.iter().any(|x| x.contains(&prompt) || x.contains("P:")), "{a:?}");
        assert!(a.iter().map(|x| x.len() + 1).sum::<usize>() < 4096, "argv stays short");
        // Gemini CLI appends `-p` to stdin; empty, it adds nothing but keeps the run headless.
        assert_eq!(&a[a.len() - 2..], &["-p".to_string(), String::new()]);
        assert!(a.windows(2).any(|w| w[0] == "-m" && w[1] == "gemini-2.5-pro"));
        assert!(a.windows(2).any(|w| w[0] == "--output-format" && w[1] == "stream-json"));
    }

    #[test]
    fn nested_session_variables_are_dropped_but_user_settings_kept() {
        let v = |names: &[&str]| names.iter().map(|s| s.to_string()).collect::<Vec<_>>().into_iter();
        assert!(host_session_vars(v(&["CLAUDE_CODE_ENTRYPOINT", "PATH"])).is_empty(), "outside a session nothing is touched");
        let mut dropped = host_session_vars(v(&[
            "CLAUDECODE",
            "CLAUDE_CODE_SDK_HAS_HOST_AUTH_REFRESH",
            "CLAUDE_CODE_SESSION_ID",
            "CLAUDE_AGENT_SDK_VERSION",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CONFIG_DIR",
            "ANTHROPIC_API_KEY",
            "PATH",
        ]));
        dropped.sort();
        assert_eq!(dropped, ["CLAUDECODE", "CLAUDE_AGENT_SDK_VERSION", "CLAUDE_CODE_SDK_HAS_HOST_AUTH_REFRESH", "CLAUDE_CODE_SESSION_ID"]);
    }

    fn failed(code: i32, stderr: &str) -> craft_anything_platform::Output {
        craft_anything_platform::Output {
            code: Some(code),
            success: false,
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
            stdout_tail: Vec::new(),
            stderr_tail: Vec::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            duration: Duration::from_millis(400),
        }
    }

    fn failed_with_tail(code: i32, head: &str, tail: &str) -> craft_anything_platform::Output {
        craft_anything_platform::Output {
            code: Some(code),
            success: false,
            stdout: Vec::new(),
            stderr: head.as_bytes().to_vec(),
            stdout_tail: Vec::new(),
            stderr_tail: tail.as_bytes().to_vec(),
            stdout_truncated: false,
            stderr_truncated: true,
            duration: Duration::from_millis(400),
        }
    }

    fn context(host: Option<&str>, logs: Option<PathBuf>) -> FailureContext {
        let config = BackendConfig {
            name: "Mine".into(),
            kind: BackendKind::AgentCli,
            model: "claude-code".into(),
            base_url: None,
            ssh_host: host.map(str::to_string),
            enabled: true,
        };
        let spec = CliSpec { logs, ..CliSpec::new(config, BackendOptions::default(), false, None) };
        let args = ["-p".to_string(), "PRE private plans for my app".to_string(), "--verbose".to_string()];
        FailureContext::new("claude", &args, "private plans for my app", host, &spec)
    }

    #[test]
    fn a_failed_ssh_run_says_what_happened_and_saves_a_log() {
        let dir = tempfile::tempdir().unwrap();
        let ev = context(Some("github.com"), Some(dir.path().to_path_buf())).exit_event("claude-code", &failed(255, "exec request failed on channel 0\n"));
        let AgentEvent::Error { message, remediation, log } = ev else { panic!("not an error") };
        let help = remediation.unwrap();
        assert!(message.contains("Couldn't run Claude Code on github.com: github.com is a git server"), "{message}");
        assert!(help.contains("exec request failed on channel 0") && help.contains("ssh -o BatchMode=yes -T github.com -- claude --version"), "{help}");
        assert!(!help.contains("by hand") && !help.contains("below"), "{help}");
        let text = std::fs::read_to_string(log.unwrap()).unwrap();
        for part in ["Exit code: 255", "exec request failed on channel 0", "git server", "claude --version", "--verbose", "[your message, 24 characters]"] {
            assert!(text.contains(part), "{part} is missing from:\n{text}");
        }
        assert!(!text.contains("private plans"), "the message stays out of the log:\n{text}");
    }

    /// A truncated SSH failure is diagnosed and quoted from its end, not its kept head.
    #[test]
    fn a_truncated_ssh_failure_is_read_from_its_end() {
        let ev = context(Some("github.com"), None)
            .exit_event("claude-code", &failed_with_tail(255, "warning: lots of noise\n", "exec request failed on channel 0\n"));
        let AgentEvent::Error { message, remediation, .. } = ev else { panic!("not an error") };
        assert!(message.contains("github.com is a git server"), "{message}");
        let help = remediation.unwrap();
        assert!(help.contains("exec request failed on channel 0"), "{help}");
    }

    #[test]
    fn a_failed_local_run_shows_its_output_and_command() {
        let dir = tempfile::tempdir().unwrap();
        let ev = context(None, Some(dir.path().to_path_buf())).exit_event("claude-code", &failed(3, "first\nreal reason\n"));
        let AgentEvent::Error { message, remediation, log } = ev else { panic!("not an error") };
        let help = remediation.unwrap();
        assert!(message.contains("exit code 3: real reason"), "{message}");
        assert!(help.contains("first\nreal reason") && help.contains("claude -p '[your message, 24 characters]' --verbose"), "{help}");
        assert!(log.is_some());
    }

    #[test]
    fn without_a_log_folder_nothing_is_written_and_a_failed_write_is_reported() {
        let ev = context(None, None).exit_event("claude-code", &failed(1, "x"));
        assert!(matches!(ev, AgentEvent::Error { log: None, .. }));
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a-file");
        std::fs::write(&file, "").unwrap();
        let AgentEvent::Error { remediation, log, .. } = context(None, Some(file)).exit_event("claude-code", &failed(1, "x")) else { panic!("not an error") };
        assert!(log.is_none());
        assert!(remediation.unwrap().contains("The details couldn't be saved"));
    }

    #[test]
    fn sign_in_failures_get_a_plain_fix() {
        let mut p = StreamParser::new(adapter_for("claude-code"));
        let ev = p.line(&json!({"type":"result","subtype":"success","is_error":true,"result":"Failed to authenticate: OAuth session expired"}).to_string());
        match &ev[0] {
            AgentEvent::Error { message, remediation, .. } => {
                assert_eq!(message, "Claude Code stopped: Failed to authenticate: OAuth session expired");
                assert!(remediation.as_deref().unwrap_or_default().contains("claude auth login"));
            }
            other => panic!("{other:?}"),
        }
        assert!(sign_in_hint("rate limited").is_none());
        // #165: an install too old for the model says so, and how to update it.
        let raw =
            r#"API Error: 400 {"type":"error","error":{"type":"invalid_request_error","message":"Claude Code 2.1.81 does not support this model; version 2"}}"#;
        let (message, fix) = claude_code_failure(raw);
        assert!(message.starts_with("Claude Code is too old for the model it was asked to use") && message.contains("2.1.81"), "{message}");
        let fix = fix.unwrap_or_default();
        assert!(fix.contains("claude update") && fix.contains("desktop app") && fix.contains("Settings → AI"), "{fix}");
        assert_eq!(claude_code_failure("rate limited"), ("Claude Code stopped: rate limited".to_string(), None));
    }

    #[test]
    fn builder_args_route_permissions_and_questions_to_us() {
        let a = build_args(adapter_for("claude-code").as_ref(), &BackendOptions::default(), "build it", "PRE", &mcp(), None, Access::Build);
        let pos = |f: &str| a.iter().position(|x| x == f).unwrap();
        assert_eq!(a[pos("--permission-prompt-tool") + 1], "mcp__craft-anything__core_action_approve");
        assert_eq!(a[pos("--disallowedTools") + 1], "AskUserQuestion");
        assert!(a[pos("--allowedTools")..].contains(&"Read".to_string()));
        assert!(!a.iter().any(|x| x == "Write" || x == "Bash"), "writes and commands always go through the approval tool");
        let analysis = build_args(adapter_for("claude-code").as_ref(), &BackendOptions::default(), "q", "PRE", &mcp(), None, Access::Analyse);
        assert!(!analysis.contains(&"--permission-prompt-tool".to_string()));
        // A local builder gets the hook that hands over notes at its next step; analysis runs don't.
        assert!(!a.contains(&"--settings".to_string()), "no notes folder, no hook");
        let local = McpCommand { notes: Some(PathBuf::from(r"C:\p\inbox\notes")), ..mcp() };
        let b = build_args(adapter_for("claude-code").as_ref(), &BackendOptions::default(), "build it", "PRE", &local, None, Access::Build);
        let settings: serde_json::Value = serde_json::from_str(&b[b.iter().position(|x| x == "--settings").unwrap() + 1]).unwrap();
        let hook = &settings["hooks"]["PostToolUse"][0];
        assert_eq!(hook["matcher"], "*");
        assert_eq!(hook["hooks"][0]["command"], "\"/opt/ca/craft-anything-cli\" builder-notes \"C:/p/inbox/notes\"");
        let no_hook = build_args(adapter_for("claude-code").as_ref(), &BackendOptions::default(), "q", "PRE", &local, None, Access::Analyse);
        assert!(!no_hook.contains(&"--settings".to_string()));
        assert!(McpCommand::local_or_remote(true, None, Some(PathBuf::from("/p/inbox"))).notes.is_none(), "a remote builder can't reach this computer's files");
        let m = McpCommand::local_or_remote(false, Some("/p/inv.cany".into()), Some("/p/inbox".into()));
        assert!(m.args.windows(2).any(|w| w[0] == "--inbox" && w[1] == "/p/inbox"));
        assert!(!McpCommand::local_or_remote(true, None, Some("/p/inbox".into())).args.contains(&"--inbox".to_string()));
    }

    /// #564: in Clean room a chat's reads aren't left to Claude Code's defaults. Its own file, shell
    /// and web tools are refused outright (a deny rule beats any allow), so only our tool server,
    /// which keeps the original out of reach, is left; Deep mode is unchanged.
    #[test]
    fn a_clean_room_chat_refuses_the_agents_own_file_tools_outright() {
        let args = |access| build_args(adapter_for("claude-code").as_ref(), &BackendOptions::default(), "q", "PRE", &mcp(), None, access);
        let clean = args(Access::CleanRoomAnalyse);
        let denied = &clean[clean.iter().position(|x| x == "--disallowedTools").expect("a deny list") + 1..];
        for t in ["Read", "Glob", "Grep", "LS", "NotebookRead", "Write", "Edit", "Bash", "PowerShell", "WebFetch", "WebSearch"] {
            assert!(denied.contains(&t.to_string()), "{t} is refused: {denied:?}");
        }
        let allowed = &clean[clean.iter().position(|x| x == "--allowedTools").unwrap() + 1..clean.iter().position(|x| x == "--disallowedTools").unwrap()];
        assert_eq!(allowed, ["mcp__craft-anything__*"], "only our tools are allowed");
        let deep = args(Access::Analyse);
        assert!(!deep.contains(&"--disallowedTools".to_string()), "Deep mode keeps its defaults: {deep:?}");
        // A Clean room question refuses reading too (it used to leave that to the default).
        let q = args(Access::CleanRoomQuestion);
        let denied = &q[q.iter().position(|x| x == "--disallowedTools").unwrap() + 1..];
        for t in ["Read", "Glob", "Grep", "LS", "NotebookRead", "Write", "Bash"] {
            assert!(denied.contains(&t.to_string()), "{t} is refused in a Clean room question");
        }
        assert!(!q.contains(&"--allowedTools".to_string()));
    }

    /// #564: chats take their access from the mode, as builds and questions do.
    #[test]
    fn a_run_gets_its_access_from_what_it_is_and_the_mode() {
        assert_eq!(access_for(false, true, false), Access::Analyse);
        assert_eq!(access_for(false, false, false), Access::CleanRoomAnalyse);
        assert_eq!(access_for(true, true, false), Access::Build);
        assert_eq!(access_for(true, false, false), Access::CleanRoomBuild);
        assert_eq!(access_for(false, true, true), Access::Question);
        assert_eq!(access_for(false, false, true), Access::CleanRoomQuestion);
        assert_eq!(access_for(true, false, true), Access::CleanRoomQuestion);
    }

    /// #564: an ACP chat in Clean room refuses the agent's own reads, writes, commands and fetches
    /// instead of putting them to the person.
    #[test]
    fn a_clean_room_acp_chat_refuses_the_agents_own_actions() {
        let approve = acp::clean_room_chat_approver();
        for tool in ["Read", "Write", "Bash", "WebFetch"] {
            assert!(!approve(tool, &serde_json::json!({"file_path": "/original/secret.txt"})), "{tool}");
        }
        assert!(approve("TodoWrite", &serde_json::Value::Null));
    }

    #[test]
    fn a_question_reads_a_copy_of_the_builders_session_and_changes_nothing() {
        let q = build_args(adapter_for("claude-code").as_ref(), &BackendOptions::default(), "where?", "PRE", &mcp(), Some("sess-1"), Access::Question);
        let has = |a: &[String], x: &str| a.iter().any(|y| y == x);
        assert!(q.windows(3).any(|w| w[0] == "--resume" && w[1] == "sess-1" && w[2] == "--fork-session"), "a copy, never the session itself: {q:?}");
        assert!(has(&q, "--no-session-persistence") && has(&q, "--strict-mcp-config"));
        assert!(!has(&q, "--mcp-config") && !has(&q, "--permission-prompt-tool"), "no tool server: {q:?}");
        let refused = &q[q.iter().position(|x| x == "--disallowedTools").unwrap()..];
        for t in ["Write", "Edit", "Bash", "PowerShell", "WebFetch"] {
            assert!(has(refused, t), "{t} is refused");
        }
        let allowed = &q[q.iter().position(|x| x == "--allowedTools").unwrap()..];
        assert_eq!(&allowed[1..], ["Read", "Glob", "Grep"]);
        // Clean room: no file tools at all; no session: a fresh one, still not kept.
        let clean = build_args(adapter_for("claude-code").as_ref(), &BackendOptions::default(), "where?", "PRE", &mcp(), None, Access::CleanRoomQuestion);
        assert!(!has(&clean, "--allowedTools") && !has(&clean, "--fork-session") && has(&clean, "--no-session-persistence"));
        // Only agents that can be held to reading are asked.
        assert!(adapter_for("claude-code").answers_questions());
        assert!(!adapter_for("codex").answers_questions() && !adapter_for("gemini-cli").answers_questions() && !adapter_for("aider").answers_questions());
    }

    #[test]
    fn a_clean_room_builder_reads_through_us_and_a_deep_one_sees_every_tool() {
        let clean = build_args(adapter_for("claude-code").as_ref(), &BackendOptions::default(), "build it", "PRE", &mcp(), None, Access::CleanRoomBuild);
        let allowed = &clean[clean.iter().position(|x| x == "--allowedTools").unwrap()..];
        assert!(allowed.contains(&"TodoWrite".to_string()));
        assert!(!["Read", "Glob", "Grep"].iter().any(|t| allowed.contains(&t.to_string())), "every read goes through the approval tool: {allowed:?}");
        let spec = |deep, builder: bool| CliSpec {
            deep,
            builder: builder.then(|| Builder { investigation: None, inbox: PathBuf::from("/p/inbox") }),
            ..CliSpec::new(
                BackendConfig {
                    name: "Claude".into(),
                    kind: BackendKind::AgentCli,
                    model: "claude-code".into(),
                    base_url: None,
                    ssh_host: None,
                    enabled: true,
                },
                BackendOptions::default(),
                false,
                None,
            )
        };
        let args = |deep, builder| mcp_for(&spec(deep, builder), false).args;
        assert!(args(true, true).contains(&"--all-tools".to_string()) && args(true, true).contains(&"--deep".to_string()));
        assert!(!args(false, true).contains(&"--all-tools".to_string()) && args(false, true).contains(&"--clean-room".to_string()));
        // A chat's server runs in the chat's mode too, whatever was saved.
        assert!(args(true, false).contains(&"--deep".to_string()) && !args(true, false).contains(&"--all-tools".to_string()));
        assert!(args(false, false).contains(&"--clean-room".to_string()));
    }

    #[test]
    fn parses_claude_stream() {
        let ev = format!("ev_{}", "a".repeat(64));
        let lines = [
            json!({"type":"system","subtype":"init","session_id":"s1"}).to_string(),
            json!({"type":"assistant","message":{"content":[{"type":"text","text":"Looking"},{"type":"tool_use","id":"t1","name":"mcp__craft-anything__core_session_show","input":{}}]}}).to_string(),
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":format!("{{\"evidence_id\":\"{ev}\"}}")}]}]}}).to_string(),
            json!({"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.12,"usage":{"input_tokens":5,"cache_read_input_tokens":5,"output_tokens":7}}).to_string(),
            "not json warning".to_string(),
        ];
        let mut p = StreamParser::new(adapter_for("claude-code"));
        let events: Vec<AgentEvent> = lines.iter().flat_map(|l| p.line(l)).collect();
        assert_eq!(p.session_id.as_deref(), Some("s1"));
        assert!(matches!(&events[0], AgentEvent::TextDelta { text } if text == "Looking"));
        assert!(matches!(&events[1], AgentEvent::ToolStarted { tool, .. } if tool == "core_session_show"));
        assert!(matches!(&events[2], AgentEvent::ToolFinished { tool, ok: true, evidence_id: Some(id), .. } if tool == "core_session_show" && *id == ev));
        assert!(matches!(&events[3], AgentEvent::Status { .. }));
        assert_eq!((p.input_tokens, p.output_tokens, p.cost_usd), (10, 7, Some(0.12)));
    }

    /// #446: a project copied from another computer names a session this one doesn't have. The
    /// result line is Claude Code 2.1's own.
    #[test]
    fn a_session_that_is_not_on_this_computer_is_told_apart() {
        let missing = json!({"type":"result","subtype":"error_during_execution","is_error":true,"num_turns":0,"session_id":"39b5b312-5678-1234-961c-32112666ffab","total_cost_usd":0,"errors":["No conversation found with session ID: 39b5b312-5678-1234-961c-32112666ffab"]});
        let mut p = StreamParser::new(adapter_for("claude-code"));
        let events = p.line(&missing.to_string());
        assert!(p.session_missing);
        let [AgentEvent::Error { message, remediation: Some(fix), .. }] = events.as_slice() else { panic!("{events:?}") };
        assert!(message.contains("isn't on this computer") && fix.contains("Ask again"), "{message} / {fix}");
        // Any other failure is still that failure.
        let other = json!({"type":"result","subtype":"error_during_execution","is_error":true,"num_turns":3,"errors":["Something else"]});
        let mut p = StreamParser::new(adapter_for("claude-code"));
        let events = p.line(&other.to_string());
        assert!(!p.session_missing);
        assert!(matches!(events.as_slice(), [AgentEvent::Error { message, .. }] if message.contains("error_during_execution")), "{events:?}");
    }

    /// #451: Claude Code's session is found wherever it ran, and only by a real id.
    #[test]
    fn a_claude_code_session_is_found_by_its_id_alone() {
        let config = tempfile::tempdir().unwrap();
        let folder = config.path().join("projects").join("-Users-someone-Documents-Craft-Anything-notes");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("0e4a6b1c-1111-4222-8333-944455556666.jsonl"), "{}\n").unwrap();
        std::fs::write(config.path().join("projects").join("stray.jsonl"), "{}\n").unwrap();
        let found = adapters::claude_session_file(config.path(), "0e4a6b1c-1111-4222-8333-944455556666");
        assert_eq!(found, Some(folder.join("0e4a6b1c-1111-4222-8333-944455556666.jsonl")));
        assert_eq!(adapters::claude_session_file(config.path(), "1b70e5c4-0000-0000-0000-000000000000"), None);
        for bad in ["", "../stray", "a/b", &"a".repeat(65)] {
            assert_eq!(adapters::claude_session_file(config.path(), bad), None, "{bad:?}");
        }
        assert_eq!(adapters::claude_session_file(&config.path().join("nowhere"), "0e4a6b1c-1111-4222-8333-944455556666"), None);
    }

    /// #451: a session kept in the project is put back where Claude Code finds it, once, and
    /// only as the session it was.
    #[test]
    fn a_kept_claude_code_session_is_put_back_on_another_computer() {
        let config = tempfile::tempdir().unwrap();
        let id = "0e4a6b1c-1111-4222-8333-944455556666";
        let saved = format!("{{\"type\":\"user\",\"sessionId\":\"{id}\",\"message\":{{\"content\":\"Make a notes app.\"}}}}\n");
        let cwd = Path::new("/home/someone/Craft Anything/notes.app");
        assert!(adapters::claude_restore(config.path(), id, saved.as_bytes(), cwd).unwrap());
        let put = config.path().join("projects").join("-home-someone-Craft-Anything-notes-app").join(format!("{id}.jsonl"));
        assert_eq!(std::fs::read_to_string(&put).unwrap(), saved);
        assert_eq!(adapters::claude_session_file(config.path(), id), Some(put.clone()), "Claude Code finds it by its id");
        // There already: left as it is (it may have moved on since the copy was kept).
        std::fs::write(&put, "newer").unwrap();
        assert!(!adapters::claude_restore(config.path(), id, saved.as_bytes(), cwd).unwrap());
        assert_eq!(std::fs::read_to_string(&put).unwrap(), "newer");
        // A copy of another session, or an id that isn't one, puts nothing back.
        let other = "1b70e5c4-0000-0000-0000-000000000000";
        assert!(adapters::claude_restore(config.path(), other, saved.as_bytes(), cwd).unwrap_err().message.contains("another session"));
        assert!(adapters::claude_restore(config.path(), "../../x", saved.as_bytes(), cwd).is_err());
        assert_eq!(adapters::claude_session_file(config.path(), other), None);
        assert!(!adapter_for("codex").restore_session(id, saved.as_bytes(), cwd).unwrap());
    }

    /// #451: a kept session read back as a handoff: what was asked and answered, nothing else.
    /// The line shapes are Claude Code 2.1's.
    #[test]
    fn a_kept_claude_code_session_reads_back_as_its_requests_and_replies() {
        let lines = [
            json!({"type":"queue-operation","operation":"enqueue","content":"queued copy"}),
            json!({"type":"user","isSidechain":false,"message":{"role":"user","content":"Make a notes app."}}),
            json!({"type":"assistant","isSidechain":false,"message":{"content":[{"type":"thinking","thinking":"secret plan"}]}}),
            json!({"type":"assistant","isSidechain":false,"message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}),
            json!({"type":"user","isSidechain":false,"message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"app/"}]}}),
            json!({"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"side task"}]}}),
            json!({"type":"assistant","isSidechain":false,"message":{"content":[{"type":"text","text":"Built a notes app with a sidebar."}]}}),
            json!({"type":"assistant","isSidechain":false,"message":{"content":[{"type":"text","text":"No response requested."}]}}),
            json!({"type":"user","isSidechain":false,"message":{"content":"The person asks: \"make it blue\""}}),
            json!({"type":"assistant","isSidechain":false,"message":{"content":[{"type":"text","text":"Made it blue."}]}}),
        ];
        let saved = lines.iter().map(Value::to_string).collect::<Vec<_>>().join("\n") + "\nnot json\n";
        let all = adapters::claude_handoff(&saved, 10_000).unwrap();
        assert_eq!(
            all,
            "Craft Anything asked you: Make a notes app.\n\nYou answered: Built a notes app with a sidebar.\n\nCraft Anything asked you: The person asks: \"make it blue\"\n\nYou answered: Made it blue."
        );
        // Short of room, the newest is kept.
        let short = adapters::claude_handoff(&saved, 80).unwrap();
        assert!(short.ends_with("You answered: Made it blue.") && !short.contains("notes app"), "{short}");
        // A first brief runs long: it's cut short.
        let long = json!({"type":"user","message":{"content":"x".repeat(10_000)}}).to_string();
        assert!(adapters::claude_handoff(&long, 10_000).unwrap().chars().count() < 1600);
        assert_eq!(adapters::claude_handoff("", 10_000), None);
        assert_eq!(adapters::claude_handoff("\u{0}garbage", 10_000), None);
        assert!(adapter_for("claude-code").handoff(&saved, 10_000).is_some() && adapter_for("codex").handoff(&saved, 10_000).is_none());
    }

    /// Native stream events preserve text, tool results and usage for both adapters.
    #[test]
    fn parses_codex_and_gemini_streams() {
        let mut p = StreamParser::new(adapter_for("codex"));
        let mut ev = Vec::new();
        for l in [
            json!({"type":"thread.started","thread_id":"th"}),
            json!({"type":"item.started","item":{"id":"i1","type":"mcp_tool_call","server":"craft-anything","tool":"core_target_open","arguments":{"path":"/bin/ls"}}}),
            json!({"type":"item.completed","item":{"id":"i1","type":"mcp_tool_call","tool":"core_target_open","status":"completed","result":{"ok":true}}}),
            json!({"type":"item.completed","item":{"id":"i2","type":"agent_message","text":"It lists files."}}),
            json!({"type":"turn.completed","usage":{"input_tokens":3,"output_tokens":4}}),
        ] {
            ev.extend(p.line(&l.to_string()));
        }
        assert!(matches!(&ev[0], AgentEvent::ToolStarted { tool, .. } if tool == "core_target_open"));
        assert!(matches!(&ev[1], AgentEvent::ToolFinished { ok: true, .. }));
        assert!(matches!(&ev[2], AgentEvent::TextDelta { text } if text.starts_with("It lists")));
        assert!(p.saw_result && p.output_tokens == 4);
        let auth = p.line(&json!({"type":"turn.failed","error":{"message":"authentication expired"}}).to_string());
        assert!(auth.iter().any(|event| matches!(event, AgentEvent::Error { remediation: Some(fix), .. } if fix.contains("Sign in with ChatGPT"))));
        let mut g = StreamParser::new(adapter_for("gemini-cli"));
        let ev: Vec<_> = [
            json!({"type":"message","role":"assistant","content":"Hi","delta":true}),
            json!({"type":"tool_use","tool_name":"craft-anything__core_targets_list","tool_id":"x","parameters":{}}),
            json!({"type":"tool_result","tool_id":"x","status":"error","output":"boom"}),
        ]
        .iter()
        .flat_map(|l| g.line(&l.to_string()))
        .collect();
        assert!(matches!(&ev[1], AgentEvent::ToolStarted { tool, .. } if tool == "core_targets_list"));
        assert!(matches!(&ev[2], AgentEvent::ToolFinished { ok: false, .. }));
    }

    /// CodeRabbit review on #1426: `saw_result` alone now decides whether a lingering process
    /// counts as a finished turn (`run_cli`'s `RESULT_GRACE`), so a `result` event that itself
    /// reports failure must still fail the turn, not quietly succeed. Documented stream-json only
    /// ever ends a run with `result` on success (a failure is its own `error` event instead), but
    /// this is defensive: if a non-"success" status is ever sent on `result`, it must not be
    /// silently swallowed the way an unrecognised field would be.
    #[test]
    fn a_gemini_result_that_reports_failure_still_fails_the_turn() {
        let mut p = StreamParser::new(adapter_for("gemini-cli"));
        let ev = p.line(&json!({"type":"result","status":"error","error":"the model refused the request"}).to_string());
        assert!(p.saw_result, "a result line always marks the stream's turn as done");
        assert!(ev.iter().any(|e| matches!(e, AgentEvent::Error { message, .. } if message.contains("the model refused the request"))), "{ev:?}");
        // An ordinary successful result still reports nothing extra.
        let mut ok = StreamParser::new(adapter_for("gemini-cli"));
        let ev = ok.line(&json!({"type":"result","status":"success","stats":{"input_tokens":1,"output_tokens":1}}).to_string());
        assert!(ok.saw_result && ev.is_empty(), "{ev:?}");
    }

    /// Login-shell, tool and network failures are not evidence that the account needs sign-in.
    #[test]
    fn codex_sign_in_guidance_only_matches_authentication_failures() {
        for kind in ["turn.failed", "error"] {
            for (message, needs_login) in [
                ("login required", true),
                ("Authentication expired", true),
                ("authentication failed", true),
                ("authentication required", true),
                ("Unauthorized", true),
                ("not logged in", true),
                ("token expired", true),
                ("login shell failed", false),
                ("authentication service unavailable", false),
                ("tool execution failed", false),
                ("network timeout", false),
            ] {
                let mut parser = StreamParser::new(adapter_for("codex"));
                let events = parser.line(&json!({"type":kind,"error":{"message":message}}).to_string());
                assert!(
                    matches!(events.as_slice(), [AgentEvent::Error { remediation, message: shown, .. }]
                    if remediation.is_some() == needs_login && shown == &format!("Codex: {message}")),
                    "{kind}: {message}: {events:?}"
                );
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn generic_cli_runs_through_owned_process_and_streams_text() {
        // `echo` stands in for an unknown agent CLI: prompt via argv, stdout as text.
        let spec = CliSpec::new(
            BackendConfig { name: "Echo".into(), kind: BackendKind::AgentCli, model: "echo".into(), base_url: None, ssh_host: None, enabled: true },
            BackendOptions::default(),
            true,
            None,
        );
        let mut c = CliConversation::new(spec);
        c.send("hello there");
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut all = Vec::new();
        while std::time::Instant::now() < deadline && !all.iter().any(|e| matches!(e, AgentEvent::TurnDone { .. })) {
            all.extend(c.poll());
            std::thread::sleep(Duration::from_millis(10));
        }
        let text: String = all.iter().filter_map(|e| if let AgentEvent::TextDelta { text } = e { Some(text.clone()) } else { None }).collect();
        assert!(text.contains("hello there"), "{all:?}");
        assert!(text.contains("Request"), "preamble is passed too");
    }

    #[cfg(unix)]
    #[test]
    fn a_retry_uses_the_adapter_selected_after_checking_required_flags() {
        retry_after_checking_flags(true);
    }

    #[cfg(unix)]
    #[test]
    fn a_first_retry_waits_for_the_actual_adapter_selection() {
        retry_after_checking_flags(false);
    }

    #[cfg(unix)]
    fn retry_after_checking_flags(send_first: bool) {
        use std::os::unix::fs::PermissionsExt;
        for supported in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let program = dir.path().join("agy");
            let help = if supported { "--output-format stream-json" } else { "old command options" };
            let script = format!(
                "#!/bin/sh\nd=$(dirname \"$0\")\nif [ \"$1\" = --help ]; then echo '{help}'; exit 0; fi\nprintf '%s\\n' \"$@\" > \"$d/argv\"\necho run >> \"$d/runs\"\nexit 7\n"
            );
            std::fs::write(&program, script).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            let config = BackendConfig {
                name: "Antigravity".into(),
                kind: BackendKind::AgentCli,
                model: "antigravity".into(),
                base_url: None,
                ssh_host: None,
                enabled: true,
            };
            let options = BackendOptions { command: Some(program.display().to_string()), ..Default::default() };
            let mut spec = CliSpec::new(config, options, false, None);
            spec.preamble = Some("P:".into());
            spec.resume = Some("saved".into());
            let mut c = CliConversation::new(spec);
            let finish = |c: &mut CliConversation| {
                let deadline = std::time::Instant::now() + Duration::from_secs(10);
                let mut all = Vec::new();
                while !all.iter().any(|e| matches!(e, AgentEvent::TurnDone { .. })) {
                    assert!(std::time::Instant::now() < deadline, "no TurnDone: {all:?}");
                    all.extend(c.poll());
                    std::thread::sleep(Duration::from_millis(10));
                }
                all
            };
            let original = "Build the requested blue header.";
            if send_first {
                c.send(original);
                let first = finish(&mut c);
                assert!(first.iter().any(|e| matches!(e, AgentEvent::Error { .. })), "{first:?}");
            }
            c.retry(original);
            let retried = finish(&mut c);
            assert_eq!(retried.iter().any(|e| matches!(e, AgentEvent::Status { text } if text.contains("plain adapter"))), !supported);
            let argv = std::fs::read_to_string(dir.path().join("argv")).unwrap();
            let expected = format!("P:{}", if supported { crate::failure::CONTINUE_TURN } else { original });
            assert_eq!(argv.lines().last(), Some(expected.as_str()), "supported={supported}: {argv}");
            assert_eq!(argv.contains("--conversation\nsaved\n"), supported, "{argv}");
            assert_eq!(std::fs::read_to_string(dir.path().join("runs")).unwrap(), if send_first { "run\nrun\n" } else { "run\n" });
        }
    }

    /// #402: a 34,641-character message in argv passed Windows' command-line limit (os error 206);
    /// Claude Code reads it from stdin instead.
    #[cfg(unix)]
    #[test]
    fn claude_code_reads_a_long_message_on_stdin() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("claude");
        let reply = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"read %s bytes"}]}}\n"#;
        std::fs::write(&program, format!("#!/bin/sh\nn=$(wc -c)\nprintf '{reply}' $n\n")).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config =
            BackendConfig { name: "Claude".into(), kind: BackendKind::AgentCli, model: "claude-code".into(), base_url: None, ssh_host: None, enabled: true };
        let options = BackendOptions { command: Some(program.display().to_string()), ..Default::default() };
        let mut c = CliConversation::new(CliSpec::new(config, options, false, None));
        c.send(&"a".repeat(40_000));
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut all = Vec::new();
        while std::time::Instant::now() < deadline && !all.iter().any(|e| matches!(e, AgentEvent::TurnDone { .. })) {
            all.extend(c.poll());
            std::thread::sleep(Duration::from_millis(10));
        }
        let text: String = all.iter().filter_map(|e| if let AgentEvent::TextDelta { text } = e { Some(text.clone()) } else { None }).collect();
        assert_eq!(text, "read 40000 bytes", "{all:?}");
    }

    /// #528: a stand-in `codex` gets the whole message (preamble first) on stdin and only short
    /// options in argv, for a normal message and for one longer than Windows' command line.
    #[cfg(unix)]
    #[test]
    fn codex_reads_its_message_on_stdin() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("codex");
        let script = r#"#!/bin/sh
d=$(dirname "$0")
if [ "$2" = "--help" ]; then echo "--json"; exit 0; fi
printf '%s\n' "$@" > "$d/argv"
cat > "$d/stdin"
echo '{"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}}'
"#;
        std::fs::write(&program, script).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        for prompt in ["q".to_string(), "x".repeat(40_831)] {
            let config =
                BackendConfig { name: "Codex".into(), kind: BackendKind::AgentCli, model: "codex".into(), base_url: None, ssh_host: None, enabled: true };
            let options = BackendOptions { command: Some(program.display().to_string()), ..Default::default() };
            let mut spec = CliSpec::new(config, options, false, None);
            spec.preamble = Some("P:".into());
            let mut c = CliConversation::new(spec);
            c.send(&prompt);
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let mut all = Vec::new();
            while std::time::Instant::now() < deadline && !all.iter().any(|e| matches!(e, AgentEvent::TurnDone { .. })) {
                all.extend(c.poll());
                std::thread::sleep(Duration::from_millis(10));
            }
            let stdin = std::fs::read_to_string(dir.path().join("stdin")).unwrap_or_default();
            let argv = std::fs::read_to_string(dir.path().join("argv")).unwrap_or_default();
            assert_eq!(stdin.len(), prompt.len() + 2, "{all:?}");
            assert!(stdin == format!("P:{prompt}"), "the whole message, preamble first");
            assert!(!argv.contains("P:"), "{argv}");
            assert_eq!(argv.lines().last(), Some("-"), "{argv}");
            assert_eq!(argv.lines().take(2).collect::<Vec<_>>(), ["exec", "--json"]);
        }
    }

    /// #528, on every platform: the Codex command line stays short, and a child process started
    /// with a message longer than Windows' 32,767-character limit gets all of it on stdin. The
    /// child is this test binary, running `stdin_echo_child`.
    #[test]
    fn codex_long_message_reaches_a_child_process_whole() {
        let prompt = "x".repeat(40_831);
        let codex = adapter_for("codex");
        let a = build_args(codex.as_ref(), &BackendOptions::default(), &prompt, "P:", &mcp(), None, Access::Build);
        assert!(a.iter().map(|x| x.len() + 1).sum::<usize>() < 32_767, "argv must fit Windows' command line");
        assert!(codex.prompt_on_stdin());
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("received");
        let exe = std::env::current_exe().unwrap();
        let message = format!("P:{prompt}");
        let run = Process::new(exe.as_os_str())
            // The Codex argv rides along after `--` (extra name filters that match no test).
            .args(["--exact", "--test-threads=1", "--", "cli_agents::tests::stdin_echo_child"])
            .args(&a)
            .env(STDIN_ECHO_ENV, out.as_os_str())
            .stdin_bytes(message.clone())
            .timeout(Duration::from_secs(60))
            .run();
        assert!(run.is_ok(), "{run:?}");
        assert_eq!(std::fs::read_to_string(&out).unwrap_or_default(), message);
    }

    /// A stand-in `gemini.cmd`, started the way npm installs Gemini CLI on Windows, gets the whole
    /// message (preamble first) on stdin and only short options in argv, for a normal message and
    /// for one longer than cmd.exe's 8,191 characters. Its stdin goes to `stdin_echo_child`.
    #[cfg(windows)]
    #[test]
    fn gemini_cmd_reads_its_message_on_stdin() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("gemini.cmd");
        let exe = std::env::current_exe().unwrap();
        let script = format!(
            "@echo off\r\nif \"%~1\"==\"--help\" (echo --output-format stream-json& exit /b 0)\r\necho %*> \"%~dp0argv\"\r\nset {STDIN_ECHO_ENV}=%~dp0stdin\r\n\"{}\" --exact --test-threads=1 cli_agents::tests::stdin_echo_child >nul\r\necho {{\"type\":\"result\",\"stats\":{{\"input_tokens\":1,\"output_tokens\":1}}}}\r\n",
            exe.display()
        );
        std::fs::write(&program, script).unwrap();
        for prompt in ["q".to_string(), "x".repeat(40_831)] {
            let _ = std::fs::remove_file(dir.path().join("stdin"));
            let config =
                BackendConfig { name: "Gemini".into(), kind: BackendKind::AgentCli, model: "gemini-cli".into(), base_url: None, ssh_host: None, enabled: true };
            let options = BackendOptions { command: Some(program.display().to_string()), ..Default::default() };
            let mut spec = CliSpec::new(config, options, false, None);
            spec.preamble = Some("P:".into());
            let mut c = CliConversation::new(spec);
            c.send(&prompt);
            let deadline = std::time::Instant::now() + Duration::from_secs(60);
            let mut all = Vec::new();
            while std::time::Instant::now() < deadline && !all.iter().any(|e| matches!(e, AgentEvent::TurnDone { .. })) {
                all.extend(c.poll());
                std::thread::sleep(Duration::from_millis(10));
            }
            let stdin = std::fs::read_to_string(dir.path().join("stdin")).unwrap_or_default();
            let argv = std::fs::read_to_string(dir.path().join("argv")).unwrap_or_default();
            assert_eq!(stdin.len(), prompt.len() + 2, "{all:?}");
            assert!(stdin == format!("P:{prompt}"), "the whole message, preamble first");
            assert!(!argv.contains("P:"), "{argv}");
            assert!(argv.trim_end().ends_with("-p \"\""), "{argv}");
        }
    }

    /// The desktop Harness uses a custom batch launcher rather than an npm shim. Exercise the
    /// real conversation runner through such a launcher, with no installed agent or network.
    #[cfg(windows)]
    #[test]
    fn dsh_cmd_receives_multiline_and_long_tasks_on_stdin() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("dsh.cmd");
        let exe = std::env::current_exe().unwrap().display().to_string().replace('%', "%%");
        let script = format!(
            "@echo off\r\nsetlocal DisableDelayedExpansion\r\n\
             if \"%~1\"==\"--help\" (echo --profile& exit /b 0)\r\n\
             if not \"%~5\"==\"\" exit /b 2\r\n\
             echo %*> \"%~dp0argv\"\r\n\
             set \"{STDIN_ECHO_ENV}=%~dp0stdin\"\r\n\
             \"{exe}\" --exact --test-threads=1 cli_agents::tests::stdin_echo_child >nul\r\n\
             if errorlevel 1 exit /b 1\r\n\
             echo DSH_STDIN_FIXTURE_OK\r\n"
        );
        std::fs::write(&program, script).unwrap();
        let cases = [
            ("Instructions:\r\n", "-question?\r\n中文 %PATH% & ^ \"quoted\"\n\n".to_string()),
            ("", "--not-a-flag\n".to_string()),
            ("Instructions:\n", format!("{}\nlast line\n", "x".repeat(40_831))),
        ];
        for (preamble, prompt) in cases {
            let _ = std::fs::remove_file(dir.path().join("stdin"));
            let config =
                BackendConfig { name: "DSH fixture".into(), kind: BackendKind::AgentCli, model: "dsh".into(), base_url: None, ssh_host: None, enabled: true };
            let options = BackendOptions { command: Some(program.display().to_string()), timeout_secs: Some(20), ..Default::default() };
            let mut spec = CliSpec::new(config, options, false, Some(dir.path().to_path_buf()));
            spec.preamble = Some(preamble.into());
            spec.builder = Some(Builder { investigation: None, inbox: dir.path().join("inbox") });
            let mut c = CliConversation::new(spec);
            c.send(&prompt);
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            let mut events = Vec::new();
            while std::time::Instant::now() < deadline && !events.iter().any(|e| matches!(e, AgentEvent::TurnDone { .. })) {
                events.extend(c.poll());
                std::thread::sleep(Duration::from_millis(10));
            }
            c.stop();
            assert!(events.iter().any(|e| matches!(e, AgentEvent::TurnDone { .. })), "{events:?}");
            assert!(!events.iter().any(|e| matches!(e, AgentEvent::Error { .. })), "{events:?}");
            assert!(events.iter().any(|e| matches!(e, AgentEvent::TextDelta { text } if text.contains("DSH_STDIN_FIXTURE_OK"))), "{events:?}");
            assert_eq!(std::fs::read(dir.path().join("stdin")).unwrap(), format!("{preamble}{prompt}").as_bytes());
            // cmd.exe echoes filenames in the active Windows code page; flags are ASCII.
            // The task's exact UTF-8 bytes are checked separately above.
            let argv_bytes = std::fs::read(dir.path().join("argv")).unwrap();
            let argv = String::from_utf8_lossy(&argv_bytes);
            assert!(argv.starts_with("--profile headless --patch "), "{argv}");
            assert!(!argv.trim_end().ends_with(" -"), "old Harness versions would run a literal dash: {argv}");
            assert!(!argv.contains(&prompt), "task text must not appear in argv");
        }
    }

    const STDIN_ECHO_ENV: &str = "CRAFT_ANYTHING_TEST_STDIN_ECHO";

    /// Not a test on its own: when run as the child of the test above, writes its stdin to the
    /// file it's told to.
    #[test]
    fn stdin_echo_child() {
        if let Some(path) = std::env::var_os(STDIN_ECHO_ENV) {
            let mut got = Vec::new();
            std::io::Read::read_to_end(&mut std::io::stdin(), &mut got).unwrap();
            std::fs::write(path, got).unwrap();
        }
    }

    /// Whoever reads `TurnDone` may send the next message at once (a session that wasn't there is
    /// followed by a fresh start), so the conversation takes it before the turn is said to be
    /// done. The race this prevents lasts nanoseconds, too short to catch from outside, so the
    /// order is checked where it's made.
    #[test]
    fn a_turn_is_said_to_be_done_only_once_the_next_message_can_be_taken() {
        let busy = AtomicBool::new(true);
        let mut sent = None;
        end_turn(&busy, nothing_done("Claude"), |e| {
            assert!(!busy.load(Ordering::SeqCst), "TurnDone sent while still busy");
            sent = Some(e);
        });
        assert!(matches!(sent, Some(AgentEvent::TurnDone { .. })));
    }

    /// Message after message, each turn ends with `TurnDone`, with nothing refused as "still
    /// working" when the next is sent the moment the last is done.
    #[cfg(unix)]
    #[test]
    fn the_next_message_is_taken_as_soon_as_a_turn_is_done() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("claude");
        let reply = r#"{"type":"result","subtype":"success","is_error":false,"num_turns":1,"session_id":"s1"}"#;
        std::fs::write(&program, format!("#!/bin/sh\ncat >/dev/null\necho '{reply}'\n")).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config =
            BackendConfig { name: "Claude".into(), kind: BackendKind::AgentCli, model: "claude-code".into(), base_url: None, ssh_host: None, enabled: true };
        let options = BackendOptions { command: Some(program.display().to_string()), ..Default::default() };
        let mut c = CliConversation::new(CliSpec::new(config, options, false, None));
        for round in 0..25 {
            c.send("go");
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let mut events = Vec::new();
            loop {
                let got = c.poll();
                let done = got.iter().any(|e| matches!(e, AgentEvent::TurnDone { .. }));
                events.extend(got);
                if done {
                    assert!(!c.is_busy(), "round {round}: the turn was said to be done while still busy");
                    break;
                }
                assert!(std::time::Instant::now() < deadline, "round {round}: no TurnDone: {events:?}");
                std::hint::spin_loop();
            }
            assert!(!events.iter().any(|e| matches!(e, AgentEvent::Error { .. })), "round {round}: {events:?}");
        }
    }

    /// A fake `claude` that prints the turn's `result` line, then hangs well past it (standing in
    /// for a shutdown hook or cleanup step that got stuck after Claude Code's own turn was done).
    #[cfg(unix)]
    fn hangs_after_its_result(dir: &std::path::Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let program = dir.join("claude");
        let reply = r#"{"type":"result","subtype":"success","is_error":false,"num_turns":1,"session_id":"s1"}"#;
        // `exec`: sleep replaces the shell rather than running as its child, so the group kill
        // that ends this stand-in doesn't depend on also reaching a grandchild process.
        std::fs::write(&program, format!("#!/bin/sh\necho '{reply}'\nexec sleep 20\n")).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        program
    }

    #[cfg(windows)]
    fn hangs_after_its_result(dir: &std::path::Path) -> PathBuf {
        let program = dir.join("claude.cmd");
        let reply = r#"{"type":"result","subtype":"success","is_error":false,"num_turns":1,"session_id":"s1"}"#;
        std::fs::write(&program, format!("@echo off\r\necho {reply}\r\nping -n 21 127.0.0.1 >nul\r\n")).unwrap();
        program
    }

    /// A turn whose stream already said it was done (the `result` line) must not stay "Building"
    /// for as long as the process takes to actually exit: a shutdown hook or cleanup step that
    /// hangs after that line isn't ours to wait on. Without `RESULT_GRACE`, this test would see no
    /// `TurnDone` until the fake CLI's 20 s sleep ends; with it, `TurnDone` arrives within seconds.
    #[test]
    fn a_turn_already_said_done_ends_without_waiting_for_the_process_to_exit() {
        let dir = tempfile::tempdir().unwrap();
        let program = hangs_after_its_result(dir.path());
        let config =
            BackendConfig { name: "Claude".into(), kind: BackendKind::AgentCli, model: "claude-code".into(), base_url: None, ssh_host: None, enabled: true };
        let options = BackendOptions { command: Some(program.display().to_string()), ..Default::default() };
        let mut spec = CliSpec::new(config, options, false, None);
        // A short, single-line preamble: on Windows, a `.cmd` stand-in can't take the real
        // multi-line default (a batch file's arguments can't carry a newline), which is beside
        // the point of this test either way.
        spec.preamble = Some("P:".into());
        let mut c = CliConversation::new(spec);
        c.send("go");
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut events = Vec::new();
        loop {
            let got = c.poll();
            let done = got.iter().any(|e| matches!(e, AgentEvent::TurnDone { .. }));
            events.extend(got);
            if done {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "no TurnDone within {RESULT_GRACE:?} of the result line: {events:?}");
            std::thread::sleep(Duration::from_millis(10));
        }
        let bad = |e: &AgentEvent| matches!(e, AgentEvent::Error { .. }) || matches!(e, AgentEvent::Status { text } if text == "Stopped.");
        assert!(!events.iter().any(bad), "{events:?}");
    }

    /// PR #1426 review: a wall deadline shorter than `RESULT_GRACE` can fire before our own grace
    /// period does, even though the stream already parsed the terminal result. That race must
    /// still count as the turn completing, not as a retryable failure (a builder could otherwise
    /// retry a turn that had already finished).
    #[test]
    fn a_wall_deadline_during_result_grace_still_counts_as_done() {
        let dir = tempfile::tempdir().unwrap();
        let program = hangs_after_its_result(dir.path());
        let config =
            BackendConfig { name: "Claude".into(), kind: BackendKind::AgentCli, model: "claude-code".into(), base_url: None, ssh_host: None, enabled: true };
        // Shorter than RESULT_GRACE: the result is parsed almost at once, but the process (asleep
        // for 20 s) is still alive when this deadline, not our own grace period, ends the run.
        let options = BackendOptions { command: Some(program.display().to_string()), timeout_secs: Some(2), ..Default::default() };
        let mut spec = CliSpec::new(config, options, false, None);
        spec.preamble = Some("P:".into());
        let mut c = CliConversation::new(spec);
        c.send("go");
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut events = Vec::new();
        loop {
            let got = c.poll();
            let done = got.iter().any(|e| matches!(e, AgentEvent::TurnDone { .. }));
            events.extend(got);
            if done {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "no TurnDone: {events:?}");
            std::thread::sleep(Duration::from_millis(10));
        }
        let bad = |e: &AgentEvent| matches!(e, AgentEvent::Error { .. }) || matches!(e, AgentEvent::Status { text } if text == "Stopped.");
        assert!(!events.iter().any(bad), "a result parsed before the wall deadline must not be reported as a failure: {events:?}");
    }

    /// #91: a Mac app with no CLI beside it reached Claude Code, which stopped with a stack trace.
    #[test]
    fn a_missing_or_broken_helper_is_said_plainly() {
        let stop = CancelToken::default();
        let none = cli_can_start(None, &stop).unwrap_err();
        assert!(none.message.contains("craft-anything-cli") && none.message.contains("missing"), "{none:?}");
        assert!(none.remediation.as_deref().is_some_and(|r| r.contains("Install the latest")), "{none:?}");

        let dir = tempfile::tempdir().unwrap();
        let gone = cli_can_start(Some(&dir.path().join("craft-anything-cli")), &stop).unwrap_err();
        assert!(gone.message.contains("wouldn't start"), "{gone:?}");
        assert!(gone.remediation.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn a_helper_is_judged_by_whether_it_runs() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let helper = |name: &str, script: &str| {
            let p = dir.path().join(name);
            std::fs::write(&p, script).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };
        let stop = CancelToken::default();
        assert!(cli_can_start(Some(&helper("good", "#!/bin/sh\necho 1.0\n")), &stop).is_ok());
        let bad = cli_can_start(Some(&helper("bad", "#!/bin/sh\nexit 3\n")), &stop).unwrap_err();
        assert!(bad.message.contains("exited with code 3"), "{bad:?}");
    }

    /// Stopping a build doesn't wait for a helper that hangs, and reads as stopped, not as broken.
    #[cfg(unix)]
    #[test]
    fn stopping_ends_the_check_of_a_hung_helper() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let hung = dir.path().join("hung");
        std::fs::write(&hung, "#!/bin/sh\nexec sleep 25\n").unwrap();
        std::fs::set_permissions(&hung, std::fs::Permissions::from_mode(0o755)).unwrap();
        let stop = CancelToken::default();
        let stopper = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            stopper.cancel();
        });
        let began = std::time::Instant::now();
        let e = cli_can_start(Some(&hung), &stop).unwrap_err();
        assert_eq!(e.code, ErrorCode::Cancelled, "{e:?}");
        assert!(began.elapsed() < Duration::from_secs(10), "it waited for the helper: {:?}", began.elapsed());
    }

    /// Agents that are never handed the tool server build without the helper.
    #[test]
    fn only_agents_handed_the_tool_server_need_the_helper() {
        // Aider is handed no tool server, so it needs no helper; it still needs a model or key
        // before it runs (#72), which its own tests cover.
        let aider = BackendOptions { cli_model: Some("anthropic/claude-sonnet-4".into()), ..Default::default() };
        for (agent, options) in [("gemini-cli", BackendOptions::default()), ("aider", aider), ("no-such-agent", BackendOptions::default())] {
            assert!(check_builder_tools(adapter_for(agent).as_ref(), agent, &options, Path::new("."), &CancelToken::default()).is_ok(), "{agent}");
        }
    }

    #[test]
    fn missing_cli_is_a_clear_error() {
        let spec = CliSpec::new(
            BackendConfig {
                name: "Nope".into(),
                kind: BackendKind::AgentCli,
                model: "no-such-agent-cli-xyz".into(),
                base_url: None,
                ssh_host: None,
                enabled: true,
            },
            BackendOptions::default(),
            false,
            None,
        );
        let mut c = CliConversation::new(spec);
        c.send("x");
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut all = Vec::new();
        while std::time::Instant::now() < deadline && !all.iter().any(|e| matches!(e, AgentEvent::TurnDone { .. })) {
            all.extend(c.poll());
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(all.iter().any(|e| matches!(e, AgentEvent::Error { message, .. } if message.contains("isn't installed"))), "{all:?}");
    }

    /// #199: plain `codex <prompt>` is its interactive TUI ("stdin is not a terminal"), so a
    /// failed check of `codex exec --help` keeps Codex's own adapter.
    #[test]
    fn codex_is_never_run_plain() {
        assert!(!adapter_for("codex").plain_fallback() && adapters::Plain.plain_fallback());
        let options = BackendOptions { command: Some("no-such-codex-xyz".into()), ..Default::default() };
        let config = BackendConfig { name: "Codex".into(), kind: BackendKind::AgentCli, model: "codex".into(), base_url: None, ssh_host: None, enabled: true };
        let mut c = CliConversation::new(CliSpec::new(config, options, false, None));
        c.send("What does this program do?");
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut all = Vec::new();
        while std::time::Instant::now() < deadline && !all.iter().any(|e| matches!(e, AgentEvent::TurnDone { .. })) {
            all.extend(c.poll());
            std::thread::sleep(Duration::from_millis(10));
        }
        let said = |w: &str| all.iter().any(|e| matches!(e, AgentEvent::Status { text } if text.contains(w)));
        assert!(said("trying it anyway") && !said("plain adapter"), "{all:?}");
    }

    /// #1111: OpenCode 1.x's `run --help` lists no `--standalone` and rejects it, so the turn is
    /// stopped with the fix before the program runs (it would print its usage and fail).
    #[cfg(unix)]
    #[test]
    fn opencode_1_is_refused_before_it_runs() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("opencode");
        let ran = dir.path().join("ran");
        // OpenCode 1.x's help, and a run of it that leaves a mark so the test sees it happened.
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = run ] && [ \"$2\" = --help ]; then printf '%s\\n' '--format --title --auto'; exit 0; fi\ntouch '{}'\nexit 1\n",
            ran.display()
        );
        std::fs::write(&program, &script).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config =
            BackendConfig { name: "OpenCode".into(), kind: BackendKind::AgentCli, model: "opencode".into(), base_url: None, ssh_host: None, enabled: true };
        let options = BackendOptions { command: Some(program.display().to_string()), ..Default::default() };
        let mut c = CliConversation::new(CliSpec::new(config, options, false, None));
        c.send("What does this program do?");
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut all = Vec::new();
        while std::time::Instant::now() < deadline && !all.iter().any(|e| matches!(e, AgentEvent::TurnDone { .. })) {
            all.extend(c.poll());
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            all.iter().any(|e| matches!(
                e,
                AgentEvent::Error { message, remediation: Some(fix), .. }
                    if message.contains("OpenCode 1.x") && fix.contains("@opencode/cli")
            )),
            "{all:?}"
        );
        let said = |w: &str| all.iter().any(|e| matches!(e, AgentEvent::Status { text } if text.contains(w)));
        assert!(!said("trying it anyway"), "{all:?}");
        assert!(!ran.exists(), "OpenCode 1.x ran anyway: {all:?}");

        // Updated in place to OpenCode 2: the next turn checks again and runs it.
        let v2 = dir.path().join("opencode-v2");
        std::fs::write(&v2, script.replace("--format --title --auto", "--format --standalone")).unwrap();
        std::fs::set_permissions(&v2, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::rename(&v2, &program).unwrap();
        let all = one_turn(&mut c, "And now?");
        assert!(!all.iter().any(|e| matches!(e, AgentEvent::Error { message, .. } if message.contains("1.x"))), "{all:?}");
        assert!(ran.exists(), "OpenCode 2 didn't run: {all:?}");
    }

    /// A help probe that fails without printing usage (a broken launcher) isn't taken for
    /// OpenCode 1.x: the run goes on and shows its own failure.
    #[cfg(unix)]
    #[test]
    fn a_failed_help_probe_is_not_called_version_1() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("opencode");
        std::fs::write(&program, "#!/bin/sh\necho 'env: node: No such file or directory' >&2\nexit 1\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config =
            BackendConfig { name: "OpenCode".into(), kind: BackendKind::AgentCli, model: "opencode".into(), base_url: None, ssh_host: None, enabled: true };
        let options = BackendOptions { command: Some(program.display().to_string()), ..Default::default() };
        let mut c = CliConversation::new(CliSpec::new(config, options, false, None));
        let all = one_turn(&mut c, "What does this program do?");
        assert!(!all.iter().any(|e| matches!(e, AgentEvent::Error { message, .. } if message.contains("1.x"))), "{all:?}");
        assert!(all.iter().any(|e| matches!(e, AgentEvent::Status { text } if text.contains("trying it anyway"))), "{all:?}");
    }

    /// Sends `text` and collects the turn's events up to its end. Only the Unix tests use it.
    #[cfg(unix)]
    fn one_turn(c: &mut CliConversation, text: &str) -> Vec<AgentEvent> {
        c.send(text);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut all = Vec::new();
        while std::time::Instant::now() < deadline && !all.iter().any(|e| matches!(e, AgentEvent::TurnDone { .. })) {
            all.extend(c.poll());
            std::thread::sleep(Duration::from_millis(10));
        }
        all
    }

    /// A help that couldn't be read (here, no program) isn't taken for OpenCode 1.x: the run goes
    /// on to say the program is missing.
    #[test]
    fn a_missing_opencode_is_not_called_version_1() {
        let config =
            BackendConfig { name: "OpenCode".into(), kind: BackendKind::AgentCli, model: "opencode".into(), base_url: None, ssh_host: None, enabled: true };
        let options = BackendOptions { command: Some("no-such-opencode-xyz".into()), ..Default::default() };
        let mut c = CliConversation::new(CliSpec::new(config, options, false, None));
        c.send("What does this program do?");
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut all = Vec::new();
        while std::time::Instant::now() < deadline && !all.iter().any(|e| matches!(e, AgentEvent::TurnDone { .. })) {
            all.extend(c.poll());
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(all.iter().any(|e| matches!(e, AgentEvent::Error { message, .. } if message.contains("isn't installed"))), "{all:?}");
        assert!(!all.iter().any(|e| matches!(e, AgentEvent::Error { message, .. } if message.contains("1.x"))), "{all:?}");
    }

    /// A fork's own agent: JSON lines of `{"say": …}`, registered under its id.
    struct EchoAgent;
    impl AgentAdapter for EchoAgent {
        fn ids(&self) -> &'static [&'static str] {
            &["echo-agent"]
        }
        fn args(&self, cx: &ArgsCx) -> Vec<String> {
            vec!["--json".into(), cx.prompt.to_string()]
        }
        fn parse(&self, _p: &mut StreamParser, v: &Value) -> Vec<AgentEvent> {
            v.get("say").and_then(Value::as_str).map(|t| AgentEvent::TextDelta { text: t.into() }).into_iter().collect()
        }
    }

    #[test]
    fn a_registered_adapter_drives_its_agent_and_unknown_ones_get_plain() {
        let mut reg = AgentAdapters::built_in();
        reg.register(Arc::new(EchoAgent));
        let echo = reg.for_agent("echo-agent");
        assert_eq!(build_args(echo.as_ref(), &BackendOptions::default(), "hi", "", &mcp(), None, Access::Analyse), ["--json", "hi"]);
        let mut p = StreamParser::new(echo);
        assert!(matches!(p.line(r#"{"say":"hello"}"#).as_slice(), [AgentEvent::TextDelta { text }] if text == "hello"));
        // Unknown agents stream their output as text; the built-ins keep their own adapters.
        assert!(!reg.for_agent("no-such-agent").streams_json());
        assert!(reg.for_agent("claude").resumes() && !reg.for_agent("codex").resumes() && reg.for_agent("antigravity").resumes());
        // Registering an adapter for an existing id replaces that one.
        reg.register(Arc::new(EchoAgent));
        assert_eq!(
            format!("{reg:?}"),
            r#"["claude-code", "codex", "gemini-cli", "opencode", "antigravity", "hermes", "maki", "aider", "copilot", "grok", "dsh", "devin", "echo-agent"]"#
        );
    }

    #[test]
    fn copilot_defaults_to_local_acp_without_changing_other_agents() {
        let config =
            BackendConfig { name: "Copilot".into(), kind: BackendKind::AgentCli, model: "copilot".into(), base_url: None, ssh_host: None, enabled: true };
        let mut spec = CliSpec::new(config, BackendOptions::default(), false, None);
        assert!(acp_for(&spec).is_some());
        assert_eq!(program_for("copilot"), "copilot");
        assert_eq!(display_for("copilot"), "GitHub Copilot");
        spec.options.acp = Some(false);
        assert!(acp_for(&spec).is_none());
        spec.options.acp = Some(true);
        spec.config.ssh_host = Some("build-machine".into());
        assert!(acp_for(&spec).is_none());
        spec.config.ssh_host = None;
        spec.options.acp = None;
        spec.config.model = "gemini-cli".into();
        assert!(acp_for(&spec).is_none());
        spec.options.acp = Some(true);
        assert!(acp_for(&spec).is_some());
        spec.question = true;
        assert!(acp_for(&spec).is_none());
        spec.config.model = "copilot".into();
        assert!(acp_for(&spec).is_some());
    }

    /// A remote host runs whichever craft-anything-cli it has, of any release. Every release, from
    /// 0.1.0 on, knows the global `--deep` and `--clean-room` and applies them in `mcp`, and has
    /// `mcp --all-tools`, so a remote tool server gets only those (adapters add nothing else, Codex
    /// only `--all-tools`). A flag outside them needs the remote CLI checked first, or an older one
    /// stops the build at its first tool. Of our own variables the remote side gets only
    /// `CRAFT_ANYTHING_AGENT`, which no release up to 0.5.0 reads (R2.8 keeps that open).
    #[test]
    fn a_remote_tool_server_gets_only_flags_every_release_knows() {
        use craft_anything_engine::{TargetKind, target::TargetContext};
        for deep in [false, true] {
            for builder in [false, true] {
                let config = BackendConfig {
                    name: "Remote".into(),
                    kind: BackendKind::AgentCli,
                    model: "claude-code".into(),
                    base_url: None,
                    ssh_host: Some("build-box".into()),
                    enabled: true,
                };
                let mut spec = CliSpec::new(config, BackendOptions::default(), false, Some(PathBuf::from("/p/investigation")));
                spec.deep = deep;
                spec.builder = builder.then(|| Builder { investigation: Some(PathBuf::from("/p/investigation")), inbox: PathBuf::from("/p/inbox") });
                spec.targets = vec![TargetContext { path: PathBuf::from("/p/original.bin"), sha256: "0".repeat(64), kind: TargetKind::Elf, raw: None }];
                let mcp = mcp_for(&spec, true);
                assert_eq!(mcp.command, "craft-anything-cli", "the remote PATH's");
                assert!(mcp.args.iter().all(|a| ["mcp", "--deep", "--clean-room", "--all-tools"].contains(&a.as_str())), "{deep} {builder}: {:?}", mcp.args);
                assert!(mcp.args.contains(&if deep { "--deep" } else { "--clean-room" }.to_string()), "the run's mode, whatever the remote settings say");
            }
        }
        for builder in [false, true] {
            let ours: Vec<&str> = agent_env(builder, None).iter().map(|(name, _)| *name).filter(|name| name.starts_with("CRAFT_")).collect();
            assert_eq!(ours, [craft_anything_engine::AGENT_ENV]);
        }
    }

    /// A host without the helper is told apart from a failed connection: the fix is to install it
    /// or put it on the PATH ssh sessions use.
    #[cfg(unix)]
    #[test]
    fn a_host_without_the_helper_is_told_to_install_it() {
        let empty = tempfile::tempdir().unwrap();
        let out = craft_anything_platform::Process::new("/bin/sh")
            .args(["-c", adapters::REMOTE_CHECK_LINE])
            .env_clear()
            .env("PATH", empty.path())
            .timeout(std::time::Duration::from_secs(5))
            .run()
            .unwrap();
        assert_eq!(out.code, Some(127));
        let err = adapters::remote_failure("build-box", &out);
        assert!(err.message.contains("craft-anything-cli wasn't found on build-box"), "{}", err.message);
        assert!(err.remediation.unwrap_or_default().contains("PATH"));
    }

    /// Claude Code and Codex hand a remote host's own craft-anything-cli the tool server, so a build
    /// over SSH asks the host for it first, as one here checks this computer's.
    #[cfg(unix)]
    #[test]
    fn a_remote_builder_checks_the_hosts_tool_server_first() {
        for agent in ["claude-code", "codex"] {
            let err = adapter_for(agent).check_remote_tool_server("no-such-host.invalid", &CancelToken::default()).expect_err(agent);
            assert_eq!(err.code, ErrorCode::EngineMissing, "{agent}: {}", err.message);
        }
    }

    #[test]
    fn mcp_target_handoff_is_bounded_and_keeps_the_active_target_last() {
        use craft_anything_engine::{TargetKind, target::TargetContext};
        let directory = tempfile::tempdir().unwrap();
        for count in [0, 128, 129, 130] {
            let config =
                BackendConfig { name: "Test".into(), kind: BackendKind::AgentCli, model: "copilot".into(), base_url: None, ssh_host: None, enabled: true };
            let mut spec = CliSpec::new(config, BackendOptions::default(), false, None);
            spec.targets = (0..count)
                .map(|index| TargetContext {
                    path: directory.path().join(format!("target-{index}")),
                    sha256: format!("{index:064x}"),
                    kind: TargetKind::Elf,
                    raw: None,
                })
                .collect();
            let command = mcp_for(&spec, false);
            let shared: Vec<TargetContext> =
                command.args.windows(2).filter(|pair| pair[0] == "--target").map(|pair| serde_json::from_str(&pair[1]).unwrap()).collect();
            assert_eq!(shared.len(), count.min(128));
            assert_eq!(
                shared.iter().map(|target| &target.path).collect::<Vec<_>>(),
                spec.targets.iter().skip(count.saturating_sub(128)).map(|target| &target.path).collect::<Vec<_>>()
            );
            assert_eq!(shared.last().map(|target| &target.path), spec.targets.last().map(|target| &target.path));
            assert!(!mcp_for(&spec, true).args.iter().any(|argument| argument == "--target"));
        }
    }

    #[test]
    fn copilot_never_falls_back_to_native_or_ssh_execution() {
        for host in [None, Some("build-machine".to_string())] {
            let config =
                BackendConfig { name: "Copilot".into(), kind: BackendKind::AgentCli, model: "copilot".into(), base_url: None, ssh_host: host, enabled: true };
            let options = BackendOptions { acp: Some(false), command: Some("no-such-copilot-xyz".into()), ..Default::default() };
            let spec = CliSpec::new(config, options, false, None);
            let (sender, receiver) = std::sync::mpsc::channel();
            let state = RunState { session: Arc::default(), verified: Arc::default(), lost: Arc::default(), failure: Arc::default() };
            let done = run_cli(&spec, "hello", false, &sender, &Stop::default(), &state);
            let mut output: Vec<_> = receiver.try_iter().collect();
            output.push(done);
            assert!(matches!(&output[..], [AgentEvent::Error { message, .. }, AgentEvent::TurnDone { .. }] if message.contains("local ACP")));
        }
    }

    #[test]
    fn acp_only_refusals_use_the_registered_adapters_sign_in_guidance() {
        struct AcpOnlyAgent(Option<&'static str>);
        impl AgentAdapter for AcpOnlyAgent {
            fn ids(&self) -> &'static [&'static str] {
                &["test-acp-agent"]
            }
            fn args(&self, _context: &ArgsCx) -> Vec<String> {
                Vec::new()
            }
            fn requires_acp(&self) -> bool {
                true
            }
            fn sign_in_fix(&self) -> Option<String> {
                self.0.map(str::to_string)
            }
        }
        for guidance in [None, Some("Use test-agent sign-in.")] {
            let config = BackendConfig {
                name: "Test agent".into(),
                kind: BackendKind::AgentCli,
                model: "test-acp-agent".into(),
                base_url: None,
                ssh_host: None,
                enabled: true,
            };
            let mut spec = CliSpec::new(config, BackendOptions::default(), false, None);
            spec.adapters.register(Arc::new(AcpOnlyAgent(guidance)));
            let (sender, receiver) = std::sync::mpsc::channel();
            let state = RunState { session: Arc::default(), verified: Arc::default(), lost: Arc::default(), failure: Arc::default() };
            let done = run_cli(&spec, "hello", false, &sender, &Stop::default(), &state);
            let mut output: Vec<_> = receiver.try_iter().collect();
            output.push(done);
            let [AgentEvent::Error { remediation: Some(fix), .. }, AgentEvent::TurnDone { .. }] = output.as_slice() else { panic!("{output:?}") };
            assert!(fix.contains("test-acp-agent") && fix.contains("remove the SSH host") && fix.contains("do not disable ACP"), "{fix}");
            assert!(!fix.to_lowercase().contains("copilot"), "{fix}");
            assert_eq!(fix.contains("Use test-agent sign-in."), guidance.is_some());
        }
    }

    /// A Clean room run of an agent whose reads can't be checked is refused on the path every
    /// native run takes, before its program starts.
    #[cfg(unix)]
    #[test]
    fn a_clean_room_run_of_aider_is_refused_before_it_starts() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("aider");
        std::fs::write(&program, "#!/bin/sh\necho run >> \"$(dirname \"$0\")/runs\"\nexit 0\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = BackendConfig { name: "Aider".into(), kind: BackendKind::AgentCli, model: "aider".into(), base_url: None, ssh_host: None, enabled: true };
        let options = BackendOptions { command: Some(program.display().to_string()), ..Default::default() };
        let mut spec = CliSpec::new(config, options, false, None);
        spec.deep = false;
        let mut c = CliConversation::new(spec);
        c.send("x");
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut all = Vec::new();
        while std::time::Instant::now() < deadline && !all.iter().any(|e| matches!(e, AgentEvent::TurnDone { .. })) {
            all.extend(c.poll());
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(all.iter().any(|e| matches!(e, AgentEvent::Error { message, .. } if message.contains("Clean room"))), "{all:?}");
        assert!(!dir.path().join("runs").exists(), "Aider must not start in a Clean room run");
    }
}
