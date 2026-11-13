//! Agents that speak the Agent Client Protocol (ACP: JSON-RPC 2.0, one message per line on the
//! agent's stdin/stdout) as one more [`Conversation`]. One agent process serves the whole chat:
//! `initialize`, then `session/new` (or `session/load` to continue) with our MCP server, then one
//! `session/prompt` per message; `session/update` notifications become [`AgentEvent`]s and
//! `session/request_permission` becomes [`AgentEvent::ConsentNeeded`].
//!
//! [`AcpCore`] is the protocol alone (lines in, lines and events out) so it's tested without a
//! process; [`AcpConversation`] runs it against the agent through `craft_anything_platform`.

use super::{McpCommand, display_for};
use crate::model::Stop;
use crate::{AgentEvent, Conversation, FailureKind, TurnFailure};
use craft_anything_platform::sync::Mutex;
use craft_anything_platform::{RunningProcess, StdinMode};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::PathBuf;
use std::process::ChildStdin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

/// Agents known to speak ACP, and the arguments that start them in that mode.
pub const ACP_AGENTS: &[(&str, &[&str])] = &[
    ("gemini-cli", &["--experimental-acp"]),
    ("qwen-code", &["--experimental-acp"]),
    ("opencode", &["acp"]),
    ("goose", &["acp"]),
    ("hermes", &["acp"]),
    ("maki", &["acp"]),
    ("copilot", &["--acp", "--stdio"]),
    // Flags that belong to this mode (`--model`, `--no-leader`) go between `agent` and `stdio`.
    ("grok", &["agent"]),
    ("devin", &["acp"]),
];

/// Agents whose ACP mode takes the backend's model (`cli_model`), and the flag after the ACP
/// arguments that picks it. Others keep the model they're set to.
pub const ACP_MODEL_FLAGS: &[(&str, &str)] = &[("maki", "-m"), ("grok", "--model"), ("devin", "--model")];

/// Agents that ask the host to `authenticate` before a session opens, and the `authMethods` id
/// the backend's stored key is sent under (`meta.api_key`). Other agents go straight to
/// `session/new` — a stored key is never sent under an id the table doesn't list.
pub const ACP_KEY_AGENTS: &[(&str, &str)] = &[("devin", "devin-browser")];

/// The `authMethods` id the stored key authenticates with for this agent, if it is one the host
/// supplies a key for.
pub fn key_auth_method(agent: &str) -> Option<&'static str> {
    ACP_KEY_AGENTS.iter().find(|(name, _)| *name == agent).map(|(_, method)| *method)
}

/// The backend's stored key for an agent the host authenticates (`ACP_KEY_AGENTS`); `None` for
/// every other agent, so the keychain is never asked for a key nobody would use.
pub fn key_for(store: &crate::KeyStore, agent: &str, backend: &str, kind: crate::BackendKind) -> Option<String> {
    key_auth_method(agent).and_then(|_| store.resolve(backend, kind))
}
/// The arguments after `agent`'s ACP ones that pick `model`; none when its ACP mode isn't known
/// to take one.
pub fn model_args(agent: &str, model: Option<&str>) -> Vec<String> {
    let flag = ACP_MODEL_FLAGS.iter().find(|(id, _)| *id == agent).map(|(_, f)| *f);
    match (flag, model.map(str::trim).filter(|m| !m.is_empty())) {
        (Some(flag), Some(model)) => vec![flag.to_string(), model.to_string()],
        _ => Vec::new(),
    }
}

/// The ACP version we speak.
const PROTOCOL_VERSION: u64 = 1;

/// The arguments that start `agent` in ACP mode, if it has one.
pub fn acp_args(agent: &str) -> Option<&'static [&'static str]> {
    ACP_AGENTS.iter().find(|(id, _)| *id == agent).map(|(_, a)| *a)
}

/// What the core asks its driver to do.
#[derive(Debug, PartialEq)]
pub enum Out {
    /// Write this line to the agent.
    Send(String),
    Event(AgentEvent),
    /// The current message is answered (or failed); the conversation is free again.
    TurnEnded,
    /// Ask the approver about a permission request (`tool` and `input` in the approval policy's
    /// shape: `Write {file_path}`, `Bash {command}`…), then [`AcpCore::answer`] it.
    Decide {
        call_id: String,
        tool: String,
        input: Value,
    },
}

/// Decides a builder's permission request (`tool`, `input` as in [`Out::Decide`]); may block
/// while the person is asked. True allows it.
pub type Approver = Arc<dyn Fn(&str, &Value) -> bool + Send + Sync>;

/// The approver for a chat in Clean room (#564): the agent's own reading, writing, commands and
/// fetching are refused (they'd reach the original), and what's left (planning, our tools, which
/// our tool server limits itself) is allowed. There is no project to check paths against in a chat,
/// so nothing of the agent's own is let through.
pub fn clean_room_chat_approver() -> Approver {
    Arc::new(|tool, _input| !matches!(tool, "Read" | "Write" | "Bash" | "WebFetch"))
}

/// Session metadata only: no prompt is sent to discover a model's selectable settings.
pub struct ModelMetadata {
    pub models: Vec<String>,
    pub current_model: String,
    pub config_options: Vec<Value>,
}

/// An ACP tool call in the approval policy's shape.
fn policy_action(call: &Value) -> (String, Value) {
    let s = |p: &str| call.pointer(p).and_then(Value::as_str).map(str::to_string);
    let path =
        s("/locations/0/path").or_else(|| s("/rawInput/path")).or_else(|| s("/rawInput/file_path")).or_else(|| s("/rawInput/abs_path")).unwrap_or_default();
    let command = match call.pointer("/rawInput/command") {
        Some(Value::String(c)) => c.clone(),
        Some(Value::Array(parts)) => parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
        _ => s("/title").unwrap_or_default(),
    };
    match call.get("kind").and_then(Value::as_str).unwrap_or("other") {
        "edit" | "delete" | "move" => ("Write".into(), json!({"file_path": path})),
        "execute" => ("Bash".into(), json!({"command": command})),
        "read" | "search" => ("Read".into(), json!({"file_path": path})),
        "fetch" => ("WebFetch".into(), call.get("rawInput").cloned().unwrap_or(Value::Null)),
        "think" | "switch_mode" => ("TodoWrite".into(), Value::Null),
        _ => (s("/title").unwrap_or_else(|| "an unnamed tool".into()), call.get("rawInput").cloned().unwrap_or(Value::Null)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Pending {
    Initialize,
    Authenticate,
    NewSession,
    LoadSession,
    /// `session/set_config_option`. The session is not ready until these finish: model discovery
    /// waits on `ready`, and some agents replace the CLI's model flag when the session opens.
    SetConfig,
    Prompt,
}

struct QueuedPrompt {
    text: String,
    continuation: Option<String>,
}

/// The ACP protocol for one conversation.
pub struct AcpCore {
    backend: String,
    /// The agent id (`devin`), for [`key_auth_method`] checks.
    agent: String,
    agent_name: String,
    cwd: PathBuf,
    mcp: McpCommand,
    resume: Option<String>,
    next_id: u64,
    pending: HashMap<u64, Pending>,
    session: Option<String>,
    ready: bool,
    queued: VecDeque<QueuedPrompt>,
    /// Only a loaded session or one already sent a prompt can continue an interrupted turn.
    session_has_history: bool,
    /// Permission requests waiting for an answer: our call id → (JSON-RPC id, options).
    asks: HashMap<String, (Value, Vec<Value>)>,
    /// Tool call titles by id, for the finished event.
    tools: HashMap<String, String>,
    /// Permission requests go to an approver ([`Out::Decide`]) instead of the person's chat.
    decides: bool,
    failure: Option<TurnFailure>,
    /// A read-only question: no MCP server and no permission escalation into a build.
    question: bool,
    /// The adapter's startup remediation. Sign-in itself stays outside the ACP connection.
    sign_in_fix: Option<String>,
    /// Model ids advertised by the opened or loaded session, not a local guess at its catalogue.
    models: Vec<String>,
    current_model: String,
    config_options: Vec<Value>,
    /// `(configId, value)` applied in order after the session opens, before it is ready.
    config_queue: VecDeque<(String, String)>,
    /// A loaded session already has history. Set when the session response arrives, and copied
    /// onto [`session_has_history`](Self::session_has_history) once the config queue is done.
    opened_with_history: bool,
    /// The adapter asked for `"type": "stdio"` on our MCP server. Off for every other agent.
    typed_mcp: bool,
    /// The backend's stored key and the `authMethods` id it authenticates with
    /// (`ACP_KEY_AGENTS`), for an agent that asks the host to `authenticate` before a session
    /// opens (Devin needs it; its own CLI sign-in doesn't count). `None` for every other agent,
    /// so a stored key is never sent under a method that doesn't take one.
    auth: Option<(String, String)>,
    /// `agentCapabilities.loadSession` from `initialize`, kept for after `authenticate`.
    can_load: bool,
    /// The saved session a builder's turn was to continue couldn't be had, so the run starts a
    /// new one from the brief instead ([`crate::Conversation::session_lost`]).
    lost: bool,
}

impl AcpCore {
    pub fn new(backend: &str, agent: &str, agent_name: &str, cwd: PathBuf, mcp: McpCommand, resume: Option<String>) -> Self {
        AcpCore {
            backend: backend.into(),
            agent: agent.into(),
            agent_name: agent_name.into(),
            cwd,
            mcp,
            resume,
            next_id: 0,
            pending: HashMap::new(),
            session: None,
            ready: false,
            queued: VecDeque::new(),
            session_has_history: false,
            asks: HashMap::new(),
            tools: HashMap::new(),
            decides: false,
            failure: None,
            question: false,
            sign_in_fix: None,
            models: Vec::new(),
            current_model: String::new(),
            config_options: Vec::new(),
            config_queue: VecDeque::new(),
            opened_with_history: false,
            typed_mcp: false,
            auth: None,
            can_load: false,
            lost: false,
        }
    }

    /// The stored key and the `authMethods` id it authenticates with (`key_auth_method`), when
    /// this agent takes one.
    pub fn with_auth(mut self, auth: Option<(String, String)>) -> Self {
        self.auth = auth;
        self
    }

    /// Send permission requests to an approver (a builder run) instead of the chat.
    pub fn deciding(mut self) -> Self {
        self.decides = true;
        self
    }

    /// The agent's session, once it has one (to continue later with `session/load`).
    pub fn session_id(&self) -> Option<String> {
        self.session.clone()
    }

    /// The session a builder's turn was to continue can't be had ([`crate::Conversation::session_lost`]).
    pub fn lost(&self) -> bool {
        self.lost
    }

    fn request(&mut self, method: &str, params: Value, kind: Pending) -> Out {
        self.next_id += 1;
        self.pending.insert(self.next_id, kind);
        Out::Send(json!({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params}).to_string())
    }

    /// The first lines to send once the agent is running.
    pub fn start(&mut self) -> Vec<Out> {
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            // The agent reads and writes files and runs commands itself, asking us first.
            "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false},
            "clientInfo": {"name": "craft-anything", "version": env!("CARGO_PKG_VERSION")},
        });
        vec![self.request("initialize", params, Pending::Initialize)]
    }

    /// A message from the person: sent now, or once the session is ready.
    pub fn prompt(&mut self, text: &str) -> Vec<Out> {
        self.queue_prompt(text, None)
    }

    fn queue_prompt(&mut self, text: &str, continuation: Option<String>) -> Vec<Out> {
        self.failure = None;
        self.queued.push_back(QueuedPrompt { text: text.to_string(), continuation });
        // A start that failed earlier (an `initialize` error) left nothing under way: start again,
        // or the message would wait for a session that never comes (#271).
        if !self.ready && self.pending.is_empty() {
            return self.start();
        }
        self.flush()
    }

    fn flush(&mut self) -> Vec<Out> {
        let busy = self.pending.values().any(|p| *p == Pending::Prompt);
        let Some(sid) = self.session.clone().filter(|_| self.ready && !busy) else { return Vec::new() };
        let Some(prompt) = self.queued.pop_front() else { return Vec::new() };
        let text = if self.session_has_history { prompt.continuation.unwrap_or(prompt.text) } else { prompt.text };
        self.session_has_history = true;
        let params = json!({"sessionId": sid, "prompt": [{"type": "text", "text": text}]});
        vec![self.request("session/prompt", params, Pending::Prompt)]
    }

    fn mcp_servers(&self) -> Value {
        // Questions attach no server. `"type": "stdio"` is only added for an adapter that
        // requires it (Grok). Other agents keep the payload their sessions already accept.
        if self.question {
            return json!([]);
        }
        let mut server = json!({"name": super::MCP_NAME, "command": self.mcp.command, "args": self.mcp.args, "env": []});
        if self.typed_mcp {
            server["type"] = json!("stdio");
        }
        json!([server])
    }

    /// The current string value of a session config option, when the agent reports one.
    fn config_current(options: &[Value], id: &str) -> Option<String> {
        let option = options.iter().find(|option| option.get("id").and_then(Value::as_str) == Some(id))?;
        match option.get("currentValue") {
            Some(Value::String(value)) => Some(value.clone()),
            Some(value) => value.get("value").and_then(Value::as_str).map(str::to_string),
            None => None,
        }
    }

    /// Send the next config override that the session does not already have, then mark it ready.
    /// A value is a JSON string: Grok rejects a nested `{"value": ...}` wrapper.
    fn advance_config(&mut self) -> Vec<Out> {
        while let Some((id, value)) = self.config_queue.front().cloned() {
            if Self::config_current(&self.config_options, &id).as_deref() == Some(value.as_str()) {
                self.config_queue.pop_front();
                continue;
            }
            let Some(sid) = self.session.clone() else {
                return self.fail(format!("{} started no session", self.agent_name), None);
            };
            let params = json!({"sessionId": sid, "configId": id, "value": value});
            return vec![self.request("session/set_config_option", params, Pending::SetConfig)];
        }
        self.ready = true;
        self.session_has_history = self.opened_with_history;
        self.flush()
    }

    /// `session/load` for a saved session the agent can take back, else `session/new`.
    fn open_session(&mut self) -> Vec<Out> {
        match self.resume.clone().filter(|_| self.can_load) {
            Some(sid) => {
                self.session = Some(sid.clone());
                let params = json!({"sessionId": sid, "cwd": self.cwd, "mcpServers": self.mcp_servers()});
                vec![self.request("session/load", params, Pending::LoadSession)]
            }
            None => {
                // A builder's saved session the agent can't take back would get a follow-up
                // with none of the build in it: fail the turn marked lost, and the run starts
                // its new session from the brief. A chat still continues with a fresh session.
                if self.resume.is_some() && self.decides {
                    self.resume = None;
                    self.lost = true;
                    return self.fail(format!("{} can't continue its saved conversation.", self.agent_name), None);
                }
                let params = json!({"cwd": self.cwd, "mcpServers": self.mcp_servers()});
                vec![self.request("session/new", params, Pending::NewSession)]
            }
        }
    }

    /// One line from the agent.
    pub fn line(&mut self, line: &str) -> Vec<Out> {
        let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else { return Vec::new() };
        let method = msg.get("method").and_then(Value::as_str);
        match (method, msg.get("id")) {
            (Some(m), Some(id)) => self.agent_request(m, id.clone(), msg.get("params").unwrap_or(&Value::Null)),
            // session/load replays history before its response, not activity in the new turn.
            (Some("session/update"), None) if self.pending.values().any(|p| *p == Pending::LoadSession) => Vec::new(),
            (Some("session/update"), None) => self.update(msg.get("params").and_then(|p| p.get("update")).unwrap_or(&Value::Null)),
            (Some(_), None) => Vec::new(),
            (None, Some(id)) => self.response(id.as_u64().unwrap_or(0), &msg),
            (None, None) => Vec::new(),
        }
    }

    fn fail(&mut self, message: String, remediation: Option<String>) -> Vec<Out> {
        self.queued.clear();
        vec![Out::Event(AgentEvent::Error { message, remediation, log: None }), self.turn_done(), Out::TurnEnded]
    }

    fn turn_done(&self) -> Out {
        // This client has no attributable ACP usage totals. Leave the cost unknown instead of
        // treating another provider's prices, or a session-wide total, as this turn's bill.
        Out::Event(AgentEvent::TurnDone { backend: self.backend.clone(), input_tokens: 0, output_tokens: 0, cost_usd: None, cost_is_estimate: false })
    }

    fn response(&mut self, id: u64, msg: &Value) -> Vec<Out> {
        let Some(kind) = self.pending.remove(&id) else { return Vec::new() };
        if let Some(err) = msg.get("error") {
            let text = err.get("message").and_then(Value::as_str).unwrap_or("an error");
            // A session that can't be continued starts afresh rather than failing the message —
            // unless a builder's resume was refused: then the turn fails marked lost and the run
            // sends the brief to a new session, not a follow-up to an empty one.
            if kind == Pending::LoadSession {
                self.resume = None;
                self.session = None;
                if self.decides {
                    self.lost = true;
                    return self.fail(format!("{} can't continue its saved conversation ({text}).", self.agent_name), None);
                }
                let params = json!({"cwd": self.cwd, "mcpServers": self.mcp_servers()});
                return vec![self.request("session/new", params, Pending::NewSession)];
            }
            self.failure = Some(TurnFailure::new(match err.get("code").and_then(Value::as_i64) {
                Some(-32000) => FailureKind::Authentication,
                Some(-32602..=-32600) => FailureKind::Configuration,
                _ => FailureKind::Unknown,
            }));
            let fix = match kind {
                Pending::Authenticate => Some(format!("{} refused the key: add a fresh one in Settings → AI for this backend.", self.agent_name)),
                Pending::Initialize | Pending::NewSession => {
                    // "Needs a stored key" guidance only for an agent that takes one, and only
                    // when the refusal is auth: the typed failure kind first, the agent's own
                    // words as a loose match when it doesn't use the protocol's code.
                    let text_lower = text.to_lowercase();
                    let auth_refusal = self.failure.is_some_and(|f| f.kind == FailureKind::Authentication)
                        || text_lower.contains("authenticat")
                        || text_lower.contains("api_key");
                    if key_auth_method(&self.agent).is_some() && auth_refusal {
                        Some(format!("{} needs an API key for ACP sessions: add one in Settings → AI for this backend.", self.agent_name))
                    } else {
                        Some(
                            self.sign_in_fix
                                .clone()
                                .unwrap_or_else(|| format!("Check that {} is installed and signed in: run it once in a terminal.", self.agent_name)),
                        )
                    }
                }
                Pending::SetConfig => Some("Choose a model and thinking depth this agent lists, then try again.".into()),
                Pending::LoadSession | Pending::Prompt => None,
            };
            return self.fail(format!("{}: {text}", self.agent_name), fix);
        }
        let result = msg.get("result").unwrap_or(&Value::Null);
        match kind {
            Pending::Initialize => {
                self.can_load = result.pointer("/agentCapabilities/loadSession").and_then(Value::as_bool).unwrap_or(false);
                // `authenticate` goes only to an agent that advertises the method its stored key
                // authenticates with (`ACP_KEY_AGENTS`): a key never goes under another method,
                // and agents that need no key auth go straight to a session, whatever they
                // advertise.
                if let Some((method, key)) = self.auth.clone() {
                    let advertised = |m: &Value| m.get("id").and_then(Value::as_str) == Some(method.as_str());
                    if result.get("authMethods").and_then(Value::as_array).is_some_and(|m| m.iter().any(advertised)) {
                        let params = json!({"methodId": method, "meta": {"api_key": key}});
                        return vec![self.request("authenticate", params, Pending::Authenticate)];
                    }
                }
                self.open_session()
            }
            Pending::Authenticate => self.open_session(),
            Pending::NewSession | Pending::LoadSession => {
                self.current_model = result.pointer("/models/currentModelId").and_then(Value::as_str).unwrap_or_default().to_string();
                self.config_options = result.get("configOptions").and_then(Value::as_array).cloned().unwrap_or_default();
                // The running agent decides what's available to this account. Entries without
                // an id can't be selected, so skip them rather than inventing one from a label.
                self.models = result
                    .pointer("/models/availableModels")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|model| model.get("modelId").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect();
                if let Some(sid) = result.get("sessionId").and_then(Value::as_str) {
                    self.session = Some(sid.to_string());
                }
                if self.session.is_none() {
                    return self.fail(format!("{} started no session", self.agent_name), None);
                }
                // Ready waits until the queued overrides are applied. Grok's `--model` flag does
                // not survive `session/new`; the saved default comes back and has to be set here.
                self.opened_with_history = kind == Pending::LoadSession;
                self.advance_config()
            }
            Pending::SetConfig => {
                if let Some(options) = result.get("configOptions").and_then(Value::as_array) {
                    self.config_options = options.clone();
                }
                if let Some(model) = Self::config_current(&self.config_options, "model").filter(|model| !model.is_empty()) {
                    self.current_model = model;
                }
                // One attempt. A response that does not echo the new value is not sent again.
                self.config_queue.pop_front();
                self.advance_config()
            }
            Pending::Prompt => {
                let mut out = Vec::new();
                match result.get("stopReason").and_then(Value::as_str) {
                    Some("cancelled") => out.push(Out::Event(AgentEvent::Status { text: "Stopped.".into() })),
                    Some("max_tokens" | "max_turn_requests") => {
                        out.push(Out::Event(AgentEvent::Status { text: format!("{} stopped at its length limit.", self.agent_name) }))
                    }
                    Some("refusal") => out.push(Out::Event(AgentEvent::Status { text: format!("{} declined to continue.", self.agent_name) })),
                    _ => {}
                }
                out.extend([self.turn_done(), Out::TurnEnded]);
                out.extend(self.flush());
                out
            }
        }
    }

    fn agent_request(&mut self, method: &str, id: Value, params: &Value) -> Vec<Out> {
        if method != "session/request_permission" {
            // We offer no file system or terminal to the agent; it does its own.
            let err = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("{method} isn't offered by this client")}});
            return vec![Out::Send(err.to_string())];
        }
        if self.question {
            // Startup already limits the tools; a question cannot ask for broader access or
            // turn an unexpected permission request into an approval card for the person.
            return vec![Out::Send(json!({"jsonrpc": "2.0", "id": id, "result": {"outcome": {"outcome": "cancelled"}}}).to_string())];
        }
        let call = params.get("toolCall").unwrap_or(&Value::Null);
        let s = |k: &str| call.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let call_id = format!("acp-perm-{}", id.as_u64().map(|n| n.to_string()).unwrap_or_else(|| id.to_string()));
        let options = params.get("options").and_then(Value::as_array).cloned().unwrap_or_default();
        let title = if s("title").is_empty() { "Use a tool".to_string() } else { s("title") };
        self.asks.insert(call_id.clone(), (id, options));
        if self.decides {
            let (tool, input) = policy_action(call);
            return vec![Out::Decide { call_id, tool, input }];
        }
        vec![Out::Event(AgentEvent::ConsentNeeded {
            call_id,
            tool: if s("kind").is_empty() { "other".into() } else { s("kind") },
            title: title.clone(),
            explanation: format!("{} wants to: {title}", self.agent_name),
        })]
    }

    /// Answer a permission request: the first option of the matching kind, once.
    pub fn answer(&mut self, call_id: &str, allow: bool) -> Vec<Out> {
        let Some((id, options)) = self.asks.remove(call_id) else { return Vec::new() };
        let want = if allow { "allow_once" } else { "reject_once" };
        let pick = options
            .iter()
            .find(|o| o.get("kind").and_then(Value::as_str) == Some(want))
            .or_else(|| options.iter().find(|o| o.get("kind").and_then(Value::as_str).is_some_and(|k| k.starts_with(if allow { "allow" } else { "reject" }))))
            .and_then(|o| o.get("optionId").cloned());
        let outcome = match pick {
            Some(option) => json!({"outcome": "selected", "optionId": option}),
            None => json!({"outcome": "cancelled"}),
        };
        vec![Out::Send(json!({"jsonrpc": "2.0", "id": id, "result": {"outcome": outcome}}).to_string())]
    }

    /// Stop the current message: cancel the prompt and any open permission requests.
    pub fn cancel(&mut self) -> Vec<Out> {
        // A message still waiting for the session has no prompt to cancel: its turn ends here.
        let waiting = !self.queued.is_empty() && !self.pending.values().any(|p| *p == Pending::Prompt);
        self.queued.clear();
        let mut out: Vec<Out> = self
            .asks
            .drain()
            .map(|(_, (id, _))| Out::Send(json!({"jsonrpc": "2.0", "id": id, "result": {"outcome": {"outcome": "cancelled"}}}).to_string()))
            .collect();
        if let Some(sid) = self.session.clone().filter(|_| self.pending.values().any(|p| *p == Pending::Prompt)) {
            out.push(Out::Send(json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": sid}}).to_string()));
        }
        if waiting {
            out.extend([self.turn_done(), Out::TurnEnded]);
        }
        out
    }

    fn update(&mut self, u: &Value) -> Vec<Out> {
        let s = |k: &str| u.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let text = || u.pointer("/content/text").and_then(Value::as_str).unwrap_or_default().to_string();
        let ev = match s("sessionUpdate").as_str() {
            "agent_message_chunk" => AgentEvent::TextDelta { text: text() },
            "agent_thought_chunk" => AgentEvent::Status { text: text() },
            "plan" => {
                let steps: Vec<String> = u
                    .get("entries")
                    .and_then(Value::as_array)
                    .map(|e| e.iter().filter_map(|x| x.get("content").and_then(Value::as_str)).map(str::to_string).collect())
                    .unwrap_or_default();
                AgentEvent::Status { text: format!("Plan: {}", steps.join("; ")) }
            }
            "tool_call" => {
                let id = s("toolCallId");
                let title = if s("title").is_empty() { s("kind") } else { s("title") };
                self.tools.insert(id.clone(), title.clone());
                let started = AgentEvent::ToolStarted { call_id: id.clone(), tool: title, input: u.get("rawInput").cloned().unwrap_or(Value::Null) };
                return match finished(u, &id, self.tools.get(&id).cloned().unwrap_or_default()) {
                    Some(f) => vec![Out::Event(started), Out::Event(f)],
                    None => vec![Out::Event(started)],
                };
            }
            "tool_call_update" => {
                let id = s("toolCallId");
                let title = self.tools.get(&id).cloned().unwrap_or_else(|| s("title"));
                return finished(u, &id, title).map(|f| vec![Out::Event(f)]).unwrap_or_default();
            }
            _ => return Vec::new(),
        };
        vec![Out::Event(ev)]
    }
}

/// The finished event for a tool call whose status is final.
fn finished(u: &Value, id: &str, tool: String) -> Option<AgentEvent> {
    let ok = match u.get("status").and_then(Value::as_str)? {
        "completed" => true,
        "failed" => false,
        _ => return None,
    };
    let summary: String = u
        .get("content")
        .and_then(Value::as_array)
        .map(|c| c.iter().filter_map(|x| x.pointer("/content/text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"))
        .unwrap_or_default();
    let summary = summary.chars().take(2000).collect();
    Some(AgentEvent::ToolFinished { call_id: id.to_string(), tool, ok, summary, evidence_id: None })
}

/// The running agent and its protocol state, shared with the output reader.
struct Shared {
    core: AcpCore,
    stdin: Option<ChildStdin>,
    stderr: VecDeque<String>,
    approver: Option<Approver>,
}

type SharedRef = Arc<Mutex<Option<Shared>>>;

impl Shared {
    /// Carry out what the core asked for. A decision runs on its own thread (it may wait for the
    /// person) so the agent's output keeps being read.
    fn apply(&mut self, outs: Vec<Out>, shared: &SharedRef, tx: &Sender<AgentEvent>, busy: &Arc<AtomicBool>) {
        for o in outs {
            match o {
                Out::Send(line) => {
                    let ok = self.stdin.as_mut().is_some_and(|s| s.write_all(format!("{line}\n").as_bytes()).and_then(|_| s.flush()).is_ok());
                    if !ok {
                        self.stdin = None;
                    }
                }
                Out::Event(e) => {
                    let _ = tx.send(e);
                }
                Out::TurnEnded => busy.store(false, Ordering::SeqCst),
                Out::Decide { call_id, tool, input } => {
                    let (sh, tx2, busy2, approver, id) = (shared.clone(), tx.clone(), busy.clone(), self.approver.clone(), call_id.clone());
                    let spawned = std::thread::Builder::new().name("acp-approval".into()).spawn(move || {
                        let allow = approver.is_some_and(|a| a(&tool, &input));
                        let mut g = sh.lock().unwrap_or_else(PoisonError::into_inner);
                        if let Some(s) = g.as_mut() {
                            let outs = s.core.answer(&id, allow);
                            s.apply(outs, &sh, &tx2, &busy2);
                        }
                    });
                    // No thread to ask on: refuse rather than leave the agent waiting.
                    if spawned.is_err() {
                        let outs = self.core.answer(&call_id, false);
                        self.apply(outs, shared, tx, busy);
                    }
                }
            }
        }
    }
}

/// A conversation with an ACP agent running on this computer.
pub struct AcpConversation {
    program: String,
    args: Vec<String>,
    /// Process setup and startup guidance from the same registered adapter that chose the flags.
    adapter: Arc<dyn super::AgentAdapter>,
    agent: String,
    backend: String,
    cwd: PathBuf,
    mcp: McpCommand,
    resume: Option<String>,
    shared: Arc<Mutex<Option<Shared>>>,
    proc: Option<RunningProcess>,
    /// Stop ends the owned process; it never waits for a protocol acknowledgement.
    stopping: bool,
    tx: Sender<AgentEvent>,
    rx: Receiver<AgentEvent>,
    busy: Arc<AtomicBool>,
    approver: Option<Approver>,
    preamble: String,
    logs: Option<PathBuf>,
    failure: Option<TurnFailure>,
    /// Suppress MCP attachment and refuse permission requests; startup flags restrict native tools.
    question: bool,
    /// Report invalid backend settings through the conversation before starting a child process.
    validation_error: Option<craft_anything_evidence::ToolError>,
    /// An access mode this agent cannot hold. Cleared for model-list discovery, which sends no prompt.
    access_error: Option<craft_anything_evidence::ToolError>,
    /// Copied onto the core when the process starts, and applied before the session is ready.
    session_config: Vec<(String, String)>,
    /// The backend's stored key and its `authMethods` id, for an agent that asks the host to
    /// `authenticate` (`ACP_KEY_AGENTS`: Devin).
    auth: Option<(String, String)>,
}

impl AcpConversation {
    /// Open a session and read its model catalogue without sending a prompt. Waits for metadata
    /// until `timeout`, carrying startup errors back to the caller; dropping this temporary
    /// conversation stops its process. Blocking: call off the UI thread.
    pub fn model_choices(self, timeout: Duration) -> Result<Vec<String>, craft_anything_evidence::ToolError> {
        self.model_metadata(timeout).map(|metadata| metadata.models)
    }

    /// Read a fresh session's selected model and configuration choices without a model turn.
    /// Blocking and bounded, with the same process cleanup as model catalogue discovery.
    pub fn model_metadata(self, timeout: Duration) -> Result<ModelMetadata, craft_anything_evidence::ToolError> {
        self.model_metadata_with_stop(timeout, &Stop::default())
    }

    pub(crate) fn model_metadata_with_stop(mut self, timeout: Duration, stop: &Stop) -> Result<ModelMetadata, craft_anything_evidence::ToolError> {
        stop.check()?;
        // No prompt is sent. The question-shaped session only keeps the tool server off, so a
        // refusal of questions and Clean room does not apply to reading the model list.
        self.access_error = None;
        self.spawn()?;
        self.busy.store(true, Ordering::SeqCst);
        let started = Instant::now();
        loop {
            stop.check()?;
            let ready = self.lock().as_ref().filter(|shared| shared.core.ready).map(|shared| ModelMetadata {
                models: shared.core.models.clone(),
                current_model: shared.core.current_model.clone(),
                config_options: shared.core.config_options.clone(),
            });
            if let Some(metadata) = ready {
                return Ok(metadata);
            }
            for event in self.poll() {
                if let AgentEvent::Error { message, remediation, .. } = event {
                    let error = craft_anything_evidence::ToolError::unavailable(message);
                    return Err(match remediation {
                        Some(fix) => error.with_remediation(fix),
                        None => error,
                    });
                }
            }
            if started.elapsed() >= timeout {
                return Err(self.model_metadata_timeout());
            }
            std::thread::park_timeout(Duration::from_millis(10));
        }
    }

    fn model_metadata_timeout(&self) -> craft_anything_evidence::ToolError {
        craft_anything_evidence::ToolError::unavailable(format!("{} did not list its models before the deadline.", display_for(&self.agent)))
            .with_remediation(self.adapter.sign_in_fix().unwrap_or_else(|| "Check the agent in a terminal, then try again.".into()))
    }

    /// Keep an adapter's settings check until the conversation starts. A failure is reported
    /// through the usual events without launching a process or changing the person's settings.
    pub fn with_validation(mut self, result: Result<(), craft_anything_evidence::ToolError>) -> Self {
        self.validation_error = result.err();
        self
    }

    /// Keep an access-mode refusal until the conversation starts. Model-list discovery clears it.
    pub fn with_access_limit(mut self, result: Result<(), craft_anything_evidence::ToolError>) -> Self {
        self.access_error = result.err();
        self
    }

    /// A read-only question: attach no MCP server and refuse permission requests. The caller
    /// must also choose a fresh session and restrictive agent flags; this switch alone doesn't
    /// prevent native tools from running without a permission request.
    pub fn with_question(mut self, question: bool) -> Self {
        self.question = question;
        self
    }

    /// Use the selected registry adapter for process setup and startup remediation. This keeps
    /// custom adapters and test stand-ins on the same path as the built-in agents.
    pub fn with_adapter(mut self, adapter: Arc<dyn super::AgentAdapter>) -> Self {
        self.adapter = adapter;
        self
    }

    /// Where to write a log if the agent stops unexpectedly ([`crate::failure_log`]).
    pub fn with_logs(mut self, logs: Option<PathBuf>) -> Self {
        self.logs = logs;
        self
    }

    /// The stored key and the `authMethods` id it authenticates with (`key_auth_method`), for an
    /// agent that asks the host to `authenticate` before a session opens.
    pub fn with_auth(mut self, auth: Option<(String, String)>) -> Self {
        self.auth = auth;
        self
    }

    /// Text put before each message, as the CLI runs do (who it works for, how to use our tools).
    pub fn with_preamble(mut self, preamble: String) -> Self {
        self.preamble = preamble;
        self
    }

    /// Config options applied with `session/set_config_option` after `session/new` or `session/load`.
    pub fn with_session_config(mut self, config: Vec<(String, String)>) -> Self {
        self.session_config = config;
        self
    }

    /// A builder run: the agent's permission requests are decided by `approver` (the approval
    /// policy), not asked in the chat.
    pub fn with_approver(mut self, approver: Approver) -> Self {
        self.approver = Some(approver);
        self
    }

    /// `program args…` starts the agent in ACP mode; `cwd` is its working folder.
    pub fn new(backend: &str, agent: &str, program: String, args: Vec<String>, cwd: PathBuf, mcp: McpCommand, resume: Option<String>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        AcpConversation {
            program,
            args,
            adapter: super::adapter_for(agent),
            agent: agent.into(),
            backend: backend.into(),
            cwd,
            mcp,
            resume,
            shared: Arc::default(),
            proc: None,
            stopping: false,
            tx,
            rx,
            busy: Arc::default(),
            approver: None,
            preamble: String::new(),
            logs: None,
            failure: None,
            question: false,
            validation_error: None,
            access_error: None,
            session_config: Vec::new(),
            auth: None,
        }
    }

    fn lock(&self) -> craft_anything_platform::sync::MutexGuard<'_, Option<Shared>> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Start the agent and send `initialize`.
    fn spawn(&mut self) -> Result<(), craft_anything_evidence::ToolError> {
        if let Some(error) = self.validation_error.clone().or_else(|| self.access_error.clone()) {
            return Err(error);
        }
        let name = display_for(&self.agent);
        let resume = self.lock().as_ref().and_then(|s| s.core.session_id()).or_else(|| self.resume.clone());
        let mut core = AcpCore::new(&self.backend, &self.agent, &name, self.cwd.clone(), self.mcp.clone(), resume).with_auth(self.auth.clone());
        core.question = self.question;
        core.sign_in_fix = self.adapter.sign_in_fix();
        core.typed_mcp = self.adapter.acp_mcp_stdio_type();
        core.config_queue = VecDeque::from(self.session_config.clone());
        if self.approver.is_some() {
            core = core.deciding();
        }
        // A pending approval callback belongs to this process, never the next one started
        // after Stop. Give each process its own protocol state.
        self.shared = Arc::new(Mutex::new(Some(Shared { core, stdin: None, stderr: VecDeque::new(), approver: self.approver.clone() })));
        let (shared, tx, busy) = (self.shared.clone(), self.tx.clone(), self.busy.clone());
        let shared_err = self.shared.clone();
        let tx_err = tx.clone();
        let mut proc = self
            .adapter
            .process(&self.program)
            .args(&self.args)
            .cwd(&self.cwd)
            // Our own CLI, started from inside the agent, calls tools as an agent (not the person).
            .env(craft_anything_engine::AGENT_ENV, "1")
            .stdin(StdinMode::Piped)
            // One process serves the whole chat; a builder turn may take hours.
            .timeout(Duration::from_secs(24 * 3600))
            .stdout_cap(4 << 20)
            .stderr_cap(1 << 20)
            .on_stdout_line(move |l| {
                let mut g = shared.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some(s) = g.as_mut() {
                    let outs = s.core.line(l);
                    s.apply(outs, &shared, &tx, &busy);
                }
            })
            .on_stderr_line(move |l| {
                if !l.trim().is_empty() {
                    let _ = tx_err.send(AgentEvent::Output { text: l.to_string() });
                }
                let mut g = shared_err.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some(s) = g.as_mut() {
                    if s.stderr.len() >= 20 {
                        s.stderr.pop_front();
                    }
                    s.stderr.push_back(l.to_string());
                }
            })
            .label(name.clone())
            .remediation(format!("Install {name}, or choose its program in Settings → AI."))
            .spawn()?;
        let stdin = proc.take_stdin();
        self.proc = Some(proc);
        let mut g = self.lock();
        if let Some(s) = g.as_mut() {
            s.stdin = stdin;
            let outs = s.core.start();
            s.apply(outs, &self.shared, &self.tx, &self.busy);
        }
        let _ = self.tx.send(AgentEvent::Status { text: "Usage totals are unavailable over ACP. Check usage and billing in the agent CLI.".into() });
        Ok(())
    }

    fn with_core(&mut self, f: impl FnOnce(&mut AcpCore) -> Vec<Out>) {
        let mut g = self.lock();
        if let Some(s) = g.as_mut() {
            let outs = f(&mut s.core);
            s.apply(outs, &self.shared, &self.tx, &self.busy);
        }
    }
    fn send_prompt(&mut self, text: &str, retry: bool) {
        if self.stopping || self.busy.swap(true, Ordering::SeqCst) {
            let _ = self.tx.send(AgentEvent::Error {
                message: "Still working on the last message.".into(),
                remediation: Some("Wait for it to finish or press stop.".into()),
                log: None,
            });
            return;
        }
        self.failure = None;
        let running = self.proc.as_mut().is_some_and(|p| !p.is_finished());
        if !running {
            let _ = self.tx.send(AgentEvent::Status { text: format!("Starting {}…", display_for(&self.agent)) });
            if let Err(e) = self.spawn() {
                self.busy.store(false, Ordering::SeqCst);
                self.failure = Some(TurnFailure::from_process(&e));
                let _ = self.tx.send(AgentEvent::Error { message: e.message, remediation: e.remediation, log: None });
                let _ = self.tx.send(AgentEvent::TurnDone {
                    backend: self.backend.clone(),
                    input_tokens: 0,
                    output_tokens: 0,
                    cost_usd: None,
                    cost_is_estimate: false,
                });
                return;
            }
        }
        let text = format!("{}{text}", self.preamble);
        let continuation = retry.then(|| format!("{}{}", self.preamble, crate::failure::CONTINUE_TURN));
        self.with_core(|c| c.queue_prompt(&text, continuation));
    }
}

impl Conversation for AcpConversation {
    fn last_failure(&self) -> Option<TurnFailure> {
        self.failure.or_else(|| self.lock().as_ref().and_then(|s| s.core.failure))
    }

    fn send(&mut self, text: &str) {
        self.send_prompt(text, false);
    }

    fn retry(&mut self, text: &str) {
        self.send_prompt(text, true);
    }

    fn session_id(&self) -> Option<String> {
        self.lock().as_ref().and_then(|s| s.core.session_id()).or_else(|| self.resume.clone())
    }

    /// A builder's saved session the agent couldn't continue (no `loadSession`, or a refused
    /// `session/load`): the run sends the brief to the new session instead of a follow-up.
    fn session_lost(&self) -> bool {
        self.lock().as_ref().is_some_and(|s| s.core.lost())
    }

    fn poll(&mut self) -> Vec<AgentEvent> {
        let mut events: Vec<AgentEvent> = self.rx.try_iter().collect();
        if self.stopping {
            // A late protocol response cannot free the turn while its process is still alive.
            events.retain(|event| !matches!(event, AgentEvent::TurnDone { .. }));
            if self.proc.as_mut().is_none_or(RunningProcess::is_finished) {
                events.extend(self.rx.try_iter());
                events.retain(|event| !matches!(event, AgentEvent::TurnDone { .. }));
                self.stopping = false;
                self.busy.store(false, Ordering::SeqCst);
                self.failure = None;
                events.push(AgentEvent::Status { text: "Stopped.".into() });
                events.push(super::nothing_done(&self.backend));
            }
            return events;
        }
        // An agent that exits mid-message would otherwise leave the conversation busy forever.
        if self.busy.load(Ordering::SeqCst) && self.proc.as_mut().is_some_and(RunningProcess::is_finished) {
            self.busy.store(false, Ordering::SeqCst);
            let ready = self.lock().as_ref().is_some_and(|s| s.core.ready);
            self.failure = Some(TurnFailure::new(if ready { FailureKind::Transient } else { FailureKind::Unknown }));
            let tail = self.lock().as_ref().map(|s| s.stderr.iter().cloned().collect::<Vec<_>>().join("\n")).unwrap_or_default();
            events.extend(self.rx.try_iter());
            let command = craft_anything_platform::process::printable_command(std::ffi::OsStr::new(&self.program), &self.args);
            let check = format!("{} --version", crate::ssh::shell_quote(&self.program));
            let log = self.logs.as_ref().and_then(|dir| {
                let report = crate::failure_log::Report {
                    what: format!("{} ({})", display_for(&self.agent), self.backend),
                    command: command.clone(),
                    check: check.clone(),
                    error: Some("It stopped in the middle of a message.".into()),
                    cwd: Some(self.cwd.clone()),
                    stderr: tail.clone().into_bytes(),
                    ..Default::default()
                };
                crate::failure_log::write(dir, &report).map_err(|e| log::warn!("couldn't write a failure log: {e}")).ok()
            });
            events.push(AgentEvent::Error {
                message: format!("{} stopped unexpectedly.{}", display_for(&self.agent), if tail.is_empty() { String::new() } else { format!("\n{tail}") }),
                remediation: Some(format!(
                    "Check that {} is installed and signed in. To test it, run:\n{check}\n\nWhat ran:\n{command}",
                    display_for(&self.agent)
                )),
                log,
            });
            events.push(AgentEvent::TurnDone { backend: self.backend.clone(), input_tokens: 0, output_tokens: 0, cost_usd: None, cost_is_estimate: false });
        }
        events
    }

    fn answer_consent(&mut self, call_id: &str, allow: bool) {
        self.with_core(|c| c.answer(call_id, allow));
    }

    fn stop(&mut self) {
        if !self.is_busy() || self.stopping {
            return;
        }
        self.stopping = true;
        // This is non-blocking and kills the agent's whole owned process group, including
        // tools it started. A session/cancel notification alone can be ignored indefinitely.
        if let Some(process) = self.proc.as_ref() {
            process.kill();
        }
    }

    fn is_busy(&self) -> bool {
        self.stopping || self.busy.load(Ordering::SeqCst)
    }
}

impl Drop for AcpConversation {
    fn drop(&mut self) {
        if let Some(mut p) = self.proc.take() {
            p.kill_and_wait(Duration::from_secs(3));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "owned subprocess fixture for cancellation tests"]
    fn unresponsive_agent_fixture() {
        use std::io::BufRead;
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            let Ok(request) = serde_json::from_str::<Value>(&line) else { continue };
            let result = match request.get("method").and_then(Value::as_str) {
                Some("initialize") => json!({"protocolVersion": 1, "agentCapabilities": {"loadSession": true}}),
                Some("session/new" | "session/load") => json!({"sessionId": "stop-test-session"}),
                Some("session/prompt") => {
                    println!(
                        "{}",
                        json!({"jsonrpc": "2.0", "method": "session/update", "params": {
                        "sessionId": "stop-test-session", "update": {"sessionUpdate": "agent_message_chunk",
                        "content": {"type": "text", "text": "fixture ready"}}}})
                    );
                    let _ = std::io::stdout().flush();
                    if !request.to_string().contains("next message") {
                        continue;
                    }
                    json!({"stopReason": "end_turn"})
                }
                // Deliberately ignore session/cancel, as a hung or uncooperative agent can.
                _ => continue,
            };
            println!("{}", json!({"jsonrpc": "2.0", "id": request["id"], "result": result}));
            let _ = std::io::stdout().flush();
        }
    }

    fn pump(conversation: &mut AcpConversation, mut done: impl FnMut(&[AgentEvent]) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if done(&conversation.poll()) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn stop_ends_an_agent_that_ignores_cancellation_and_can_resume() {
        let directory = tempfile::tempdir().unwrap();
        let executable = std::env::current_exe().unwrap();
        let mcp = McpCommand { command: "not-started".into(), args: Vec::new(), notes: None };
        let args = ["--ignored", "--exact", "cli_agents::acp::tests::unresponsive_agent_fixture", "--nocapture", "--format=terse"].map(str::to_string).to_vec();
        let mut conversation = AcpConversation::new("Test", "gemini-cli", executable.display().to_string(), args, directory.path().to_owned(), mcp, None);
        conversation.send("a long task");
        assert!(pump(&mut conversation, |events| events.iter().any(|e| matches!(e, AgentEvent::TextDelta { text } if text == "fixture ready"))));
        conversation.stop();
        conversation.stop();
        let mut terminal = 0;
        let mut errors = Vec::new();
        let finished = pump(&mut conversation, |events| {
            terminal += events.iter().filter(|event| matches!(event, AgentEvent::TurnDone { .. })).count();
            errors.extend(events.iter().filter(|event| matches!(event, AgentEvent::Error { .. })).cloned());
            terminal > 0
        });
        assert!(finished && !conversation.is_busy(), "Stop must not depend on the agent acknowledging cancellation");
        assert!(conversation.proc.as_mut().is_some_and(RunningProcess::is_finished), "the owned process has ended");
        assert_eq!(terminal, 1);
        assert!(errors.is_empty(), "a requested stop is not an agent crash: {errors:?}");
        assert_eq!(conversation.session_id().as_deref(), Some("stop-test-session"));
        assert!(!conversation.poll().iter().any(|event| matches!(event, AgentEvent::TurnDone { .. })), "Stop is idempotent");
        conversation.send("next message");
        assert!(pump(&mut conversation, |events| events.iter().any(|event| matches!(event, AgentEvent::TurnDone { .. }))));
        assert!(!conversation.is_busy(), "the stopped session accepts a new turn");
    }

    #[test]
    fn copilot_uses_stdio_acp() {
        assert_eq!(acp_args("copilot"), Some(["--acp", "--stdio"].as_slice()));
        assert!(acp_args("unknown-agent").is_none());
    }

    #[test]
    fn copilot_authentication_stays_in_its_cli() {
        let mut core = core(None);
        core.sign_in_fix = super::super::adapter_for("copilot").sign_in_fix();
        core.start();
        let output = events(core.line(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"Authentication required"}}"#));
        assert!(
            matches!(&output[0], AgentEvent::Error { remediation: Some(fix), .. } if fix.contains("copilot login") && fix.contains("restart Craft Anything"))
        );
    }

    #[test]
    fn copilot_model_choices_come_from_the_session_not_a_fixed_catalogue() {
        let mut core = core(None);
        core.start();
        core.line(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#);
        core.line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"copilot-test","models":{"availableModels":[{"modelId":"model-one","name":"One"},{"modelId":"model-two","name":"Two"},{"name":"missing-id"}]}}}"#);
        assert_eq!(core.models, ["model-one", "model-two"]);
        assert!(core.ready);
    }

    #[test]
    fn model_metadata_deadlines_name_the_current_agent() {
        for (agent, name) in [("gemini-cli", "Gemini CLI"), ("copilot", "GitHub Copilot"), ("grok", "Grok Build")] {
            let mcp = McpCommand { command: "not-started".into(), args: Vec::new(), notes: None };
            let conversation = AcpConversation::new("Test", agent, "not-started".into(), Vec::new(), PathBuf::new(), mcp, None);
            let error = conversation.model_metadata_timeout();
            assert_eq!(error.message, format!("{name} did not list its models before the deadline."));
            let fix = error.remediation.unwrap();
            assert_eq!(fix.contains("copilot login"), agent == "copilot");
            assert_eq!(fix.contains("grok login"), agent == "grok");
        }
    }

    #[test]
    fn a_read_only_question_opens_a_fresh_session_without_mcp_and_refuses_permissions() {
        let mut core = core(None);
        core.question = true;
        core.start();
        let new = sent(&core.line(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#));
        assert_eq!(new[0]["method"], "session/new");
        assert_eq!(new[0]["params"]["mcpServers"], json!([]));
        let reply = sent(&core.line(r#"{"jsonrpc":"2.0","id":"permission","method":"session/request_permission","params":{"toolCall":{"kind":"edit"},"options":[{"optionId":"allow","kind":"allow_once"}]}}"#));
        assert_eq!(reply[0]["result"]["outcome"]["outcome"], "cancelled");
    }

    fn core(resume: Option<&str>) -> AcpCore {
        let mcp = McpCommand { command: "/opt/ca/craft-anything-cli".into(), args: vec!["mcp".into()], notes: None };
        AcpCore::new("Gemini", "gemini-cli", "Gemini CLI", PathBuf::from("/w/project"), mcp, resume.map(str::to_string))
    }

    fn sent(outs: &[Out]) -> Vec<Value> {
        outs.iter().filter_map(|o| if let Out::Send(l) = o { serde_json::from_str(l).ok() } else { None }).collect()
    }

    fn events(outs: Vec<Out>) -> Vec<AgentEvent> {
        outs.into_iter().filter_map(|o| if let Out::Event(e) = o { Some(e) } else { None }).collect()
    }

    #[test]
    fn a_message_goes_through_initialize_and_a_new_session() {
        let mut c = core(None);
        let init = sent(&c.start());
        assert_eq!(init[0]["method"], "initialize");
        assert_eq!(init[0]["params"]["protocolVersion"], 1);
        assert_eq!(init[0]["params"]["clientCapabilities"]["terminal"], false);
        assert!(c.prompt("What does it do?").is_empty(), "held until the session is ready");
        let new = sent(&c.line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true}}}"#));
        assert_eq!(new[0]["method"], "session/new");
        assert_eq!(new[0]["params"]["cwd"], "/w/project");
        assert_eq!(new[0]["params"]["mcpServers"][0]["name"], "craft-anything");
        assert!(new[0]["params"]["mcpServers"][0].get("type").is_none(), "only an adapter that asks sends type");
        assert_eq!(new[0]["params"]["mcpServers"][0]["env"], json!([]));
        let prompt = sent(&c.line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s-1"}}"#));
        assert_eq!(prompt[0]["method"], "session/prompt");
        assert_eq!(prompt[0]["params"], json!({"sessionId": "s-1", "prompt": [{"type": "text", "text": "What does it do?"}]}));
        assert_eq!(c.session_id().as_deref(), Some("s-1"));

        let ev = events(c.line(r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s-1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"It "}}}}"#));
        assert_eq!(ev, [AgentEvent::TextDelta { text: "It ".into() }]);
        let ev = events(c.line(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s-1","update":{"sessionUpdate":"tool_call","toolCallId":"t1","title":"Read main.rs","kind":"read","status":"pending","rawInput":{"path":"main.rs"}}}}"#,
        ));
        assert!(matches!(&ev[..], [AgentEvent::ToolStarted { call_id, tool, .. }] if call_id == "t1" && tool == "Read main.rs"));
        let ev = events(c.line(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s-1","update":{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"completed","content":[{"type":"content","content":{"type":"text","text":"fn main"}}]}}}"#,
        ));
        assert!(matches!(&ev[..], [AgentEvent::ToolFinished { ok: true, summary, tool, .. }] if summary == "fn main" && tool == "Read main.rs"));
        let end = c.line(r#"{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}"#);
        assert!(end.contains(&Out::TurnEnded));
        assert!(matches!(events(end)[..], [AgentEvent::TurnDone { .. }]));
        // The next message goes straight out on the same session.
        assert_eq!(sent(&c.prompt("And then?"))[0]["params"]["sessionId"], "s-1");
    }

    /// An agent that advertises auth methods it doesn't need a stored key for (Gemini's OAuth)
    /// goes straight to `session/new`: a key is never sent under another method.
    /// The key lookup asks the store only for a `ACP_KEY_AGENTS` agent: another agent with a key
    /// set gets `None` (no keychain read), and so does a key agent without one.
    #[test]
    fn key_for_resolves_only_for_key_auth_agents() {
        let store = crate::KeyStore::file_only(std::env::temp_dir().join(format!("ca-key-for-{}.toml", std::process::id())));
        store.set("Devin", "sk-dev").unwrap();
        assert_eq!(key_for(&store, "devin", "Devin", crate::BackendKind::AgentCli).as_deref(), Some("sk-dev"));
        assert!(key_for(&store, "gemini-cli", "Devin", crate::BackendKind::AgentCli).is_none(), "not a key agent: store is never asked");
        assert!(key_for(&store, "devin", "Other", crate::BackendKind::AgentCli).is_none(), "no key stored for the backend");
    }

    #[test]
    fn advertised_methods_without_key_auth_open_a_session() {
        let mut c = core(None);
        c.start();
        let new = sent(&c.line(
            r#"{"jsonrpc":"2.0","id":1,"result":{"agentCapabilities":{"loadSession":true},"authMethods":[{"id":"oauth-personal","name":"Log in with Google"},{"id":"api-key","name":"API key"}]}}"#,
        ));
        assert_eq!(new[0]["method"], "session/new", "no authenticate: this agent takes no stored key");
    }

    /// Devin advertises `devin-browser`; the backend's stored key authenticates with it before a
    /// session opens, and a refused key ends the turn with the fix.
    #[test]
    fn devin_authenticates_with_the_stored_key_before_opening_a_session() {
        let mut c = core(None).with_auth(Some(("devin-browser".into(), "sk-test".into())));
        c.start();
        let auth = sent(&c.line(
            r#"{"jsonrpc":"2.0","id":1,"result":{"agentCapabilities":{"loadSession":true},"authMethods":[{"id":"devin-browser","name":"Log in with browser"}]}}"#,
        ));
        assert_eq!(auth[0]["method"], "authenticate");
        assert_eq!(auth[0]["params"], json!({"methodId": "devin-browser", "meta": {"api_key": "sk-test"}}));
        let new = sent(&c.line(r#"{"jsonrpc":"2.0","id":2,"result":null}"#));
        assert_eq!(new[0]["method"], "session/new");

        // Its method isn't advertised: no authenticate, the session is asked for and its own
        // "not authenticated" refusal is what the run reports.
        let mut c = core(None).with_auth(Some(("devin-browser".into(), "sk-test".into())));
        c.start();
        let new = sent(&c.line(r#"{"jsonrpc":"2.0","id":1,"result":{"authMethods":[{"id":"something-else"}]}}"#));
        assert_eq!(new[0]["method"], "session/new");

        // A refused key ends the turn and says to add a fresh one.
        let mut c = core(None).with_auth(Some(("devin-browser".into(), "sk-test".into())));
        c.start();
        c.line(r#"{"jsonrpc":"2.0","id":1,"result":{"authMethods":[{"id":"devin-browser"}]}}"#);
        let ev = events(c.line(r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"bad key"}}"#));
        assert!(matches!(ev.first(), Some(AgentEvent::Error { remediation: Some(r), .. }) if r.contains("refused the key")), "{ev:?}");

        // A `session/new` refusal with ACP's auth-required code gets the stored-key fix for an
        // agent that takes one, whatever its message says ("Login required" names no key).
        let mcp = McpCommand { command: "/opt/ca/craft-anything-cli".into(), args: vec!["mcp".into()], notes: None };
        let mut c = AcpCore::new("Devin", "devin", "Devin", PathBuf::from("/w/project"), mcp, None).with_auth(Some(("devin-browser".into(), "sk-test".into())));
        c.start();
        c.line(r#"{"jsonrpc":"2.0","id":1,"result":{"authMethods":[{"id":"devin-browser"}]}}"#);
        c.line(r#"{"jsonrpc":"2.0","id":2,"result":null}"#);
        let ev = events(c.line(r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32000,"message":"Login required"}}"#));
        assert!(matches!(ev.first(), Some(AgentEvent::Error { remediation: Some(r), .. }) if r.contains("API key")), "{ev:?}");

        // The same refusal for an agent that takes no stored key keeps its sign-in fix.
        let mut c = core(None);
        c.start();
        c.line(r#"{"jsonrpc":"2.0","id":1,"result":{"authMethods":[{"id":"oauth-personal"}]}}"#);
        let ev = events(c.line(r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"Authentication required"}}"#));
        assert!(matches!(ev.first(), Some(AgentEvent::Error { remediation: Some(r), .. }) if r.contains("signed in")), "{ev:?}");
    }

    /// A builder's saved session the agent can't continue (no `loadSession`, or a refused
    /// `session/load`) fails the turn marked lost, so the run sends the brief to a new session —
    /// a follow-up to an empty session would lack the build's context. Chats still continue with
    /// a fresh session.
    #[test]
    fn a_saved_session_a_builder_cant_continue_fails_lost() {
        // The agent doesn't take sessions back: builder fails lost, then starts fresh.
        let mut c = core(Some("old")).deciding();
        c.start();
        let out = c.line(r#"{"jsonrpc":"2.0","id":1,"result":{"agentCapabilities":{"loadSession":false}}}"#);
        assert!(out.contains(&Out::TurnEnded) && c.lost(), "no context-less session: {out:?}");
        let again = sent(&c.prompt("Go"));
        assert_eq!(again[0]["method"], "initialize");
        let new = sent(&c.line(r#"{"jsonrpc":"2.0","id":2,"result":{"agentCapabilities":{"loadSession":false}}}"#));
        assert_eq!(new[0]["method"], "session/new", "resume forgotten: a new session gets the brief");

        // A chat with the same answer continues silently with a fresh session.
        let mut c = core(Some("old"));
        c.start();
        let new = sent(&c.line(r#"{"jsonrpc":"2.0","id":1,"result":{"agentCapabilities":{"loadSession":false}}}"#));
        assert_eq!(new[0]["method"], "session/new");

        // A refused `session/load`: builder fails lost, chat falls back to a new session.
        let mut c = core(Some("old")).deciding();
        c.start();
        c.line(r#"{"jsonrpc":"2.0","id":1,"result":{"agentCapabilities":{"loadSession":true}}}"#);
        let out = c.line(r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32603,"message":"no such session"}}"#);
        assert!(out.contains(&Out::TurnEnded) && c.lost(), "refused load fails lost for a builder: {out:?}");
        let mut c = core(Some("old"));
        c.start();
        c.line(r#"{"jsonrpc":"2.0","id":1,"result":{"agentCapabilities":{"loadSession":true}}}"#);
        let new = sent(&c.line(r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32603,"message":"no such session"}}"#));
        assert_eq!(new[0]["method"], "session/new");
    }

    #[test]
    fn a_saved_session_is_loaded_and_a_lost_one_starts_afresh() {
        let mut c = core(Some("old"));
        c.start();
        let load = sent(&c.line(r#"{"jsonrpc":"2.0","id":1,"result":{"agentCapabilities":{"loadSession":true}}}"#));
        assert_eq!(load[0]["method"], "session/load");
        assert_eq!(load[0]["params"]["sessionId"], "old");
        let new = sent(&c.line(r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32603,"message":"no such session"}}"#));
        assert_eq!(new[0]["method"], "session/new");
        // An agent that can't load sessions gets a new one straight away.
        let mut c = core(Some("old"));
        c.start();
        assert_eq!(sent(&c.line(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#))[0]["method"], "session/new");
    }

    #[cfg(unix)]
    fn prompt_after_interrupted_acp_turn(recovery: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("acp-agent");
        let script = r#"#!/bin/sh
d=$(dirname "$0")
if [ ! -f "$d/started" ]; then
    : > "$d/started"
    IFS= read -r init
    echo '{"jsonrpc":"2.0","id":1,"result":{"agentCapabilities":{"loadSession":true}}}'
    IFS= read -r new
    echo '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"saved"}}'
    IFS= read -r prompt
    printf '%s\n' "$prompt" > "$d/first-prompt"
    exit 7
fi
IFS= read -r init
if [ "$1" = unsupported ]; then
    echo '{"jsonrpc":"2.0","id":1,"result":{}}'
    IFS= read -r new
    echo '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"fresh"}}'
    prompt_id=3
else
    echo '{"jsonrpc":"2.0","id":1,"result":{"agentCapabilities":{"loadSession":true}}}'
    IFS= read -r load
    printf '%s\n' "$load" > "$d/load"
    if [ "$1" = loaded ]; then
        echo '{"jsonrpc":"2.0","id":2,"result":{}}'
        prompt_id=3
    else
        echo '{"jsonrpc":"2.0","id":2,"error":{"code":-32603,"message":"session unavailable"}}'
        IFS= read -r new
        echo '{"jsonrpc":"2.0","id":3,"result":{"sessionId":"fresh"}}'
        prompt_id=4
    fi
fi
IFS= read -r prompt
printf '%s\n' "$prompt" > "$d/retry-prompt"
printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn"}}\n' "$prompt_id"
"#;
        std::fs::write(&program, script).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mcp = McpCommand { command: "craft-anything-cli".into(), args: vec!["mcp".into()], notes: None };
        let mut c = AcpConversation::new("Scripted ACP", "gemini-cli", program.display().to_string(), vec![recovery.into()], dir.path().into(), mcp, None)
            .with_preamble("P:".into());
        let finish = |c: &mut AcpConversation| {
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
        c.send(original);
        assert!(finish(&mut c).iter().any(|e| matches!(e, AgentEvent::Error { .. })));
        assert_eq!(c.last_failure(), Some(TurnFailure::new(FailureKind::Transient)));
        assert_eq!(c.session_id().as_deref(), Some("saved"));
        c.retry(original);
        let events = finish(&mut c);
        assert!(!events.iter().any(|e| matches!(e, AgentEvent::Error { .. })), "{events:?}");
        let read = |name: &str| -> Value { serde_json::from_str(&std::fs::read_to_string(dir.path().join(name)).unwrap()).unwrap() };
        assert_eq!(read("first-prompt")["params"]["prompt"][0]["text"], format!("P:{original}"));
        if recovery != "unsupported" {
            assert_eq!(read("load")["params"]["sessionId"], "saved");
        }
        let retry = read("retry-prompt");
        assert_eq!(retry["method"], "session/prompt");
        assert_eq!(retry["params"]["sessionId"], if recovery == "loaded" { "saved" } else { "fresh" });
        retry["params"]["prompt"][0]["text"].as_str().unwrap().to_string()
    }

    #[cfg(unix)]
    #[test]
    fn acp_retry_preserves_the_request_when_recovery_starts_a_new_session() {
        for recovery in ["missing", "unsupported"] {
            assert_eq!(prompt_after_interrupted_acp_turn(recovery), "P:Build the requested blue header.", "{recovery}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn acp_retry_continues_a_successfully_loaded_session() {
        assert_eq!(prompt_after_interrupted_acp_turn("loaded"), format!("P:{}", crate::failure::CONTINUE_TURN));
    }

    fn update(c: &mut AcpCore, value: Value) -> Vec<Out> {
        c.line(&json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "old", "update": value}}).to_string())
    }

    #[test]
    fn loading_a_session_keeps_replayed_messages_and_tools_out_of_the_new_turn() {
        let mut c = core(Some("old"));
        c.start();
        c.prompt("Continue here.");
        c.line(r#"{"jsonrpc":"2.0","id":1,"result":{"agentCapabilities":{"loadSession":true}}}"#);
        let history = [
            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Earlier answer"}}),
            json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "Earlier thought"}}),
            json!({"sessionUpdate": "plan", "entries": [{"content": "Earlier plan"}]}),
            json!({"sessionUpdate": "tool_call", "toolCallId": "old-tool", "title": "Earlier tool", "status": "completed"}),
            json!({"sessionUpdate": "tool_call_update", "toolCallId": "old-tool", "status": "completed"}),
        ];
        for notification in &history {
            assert!(update(&mut c, notification.clone()).is_empty(), "history must not appear as current activity: {notification}");
        }
        assert!(c.tools.is_empty(), "replay must not populate the live tool-title map");
        assert!(c.line(r#"{"jsonrpc":"2.0","id":99,"result":{}}"#).is_empty());
        assert!(update(&mut c, history[0].clone()).is_empty(), "an unrelated response does not finish loading");

        let prompt = sent(&c.line(r#"{"jsonrpc":"2.0","id":2,"result":{}}"#));
        assert_eq!(prompt[0]["method"], "session/prompt");
        assert_eq!(prompt[0]["params"]["sessionId"], "old");
        assert_eq!(prompt[0]["params"]["prompt"][0]["text"], "Continue here.");
        assert_eq!(
            events(update(&mut c, history[0].clone())),
            [AgentEvent::TextDelta { text: "Earlier answer".into() }],
            "the same text after loading is live"
        );
        let live = events(update(&mut c, json!({"sessionUpdate": "tool_call", "toolCallId": "live-tool", "title": "Current tool", "status": "pending"})));
        assert!(matches!(&live[..], [AgentEvent::ToolStarted { tool, .. }] if tool == "Current tool"));
        let done = events(update(&mut c, json!({"sessionUpdate": "tool_call_update", "toolCallId": "live-tool", "status": "completed"})));
        assert!(matches!(&done[..], [AgentEvent::ToolFinished { tool, ok: true, .. }] if tool == "Current tool"));
        assert_eq!(c.tools.len(), 1);
    }

    #[test]
    fn a_failed_load_does_not_suppress_the_replacement_sessions_updates() {
        let mut c = core(Some("old"));
        c.start();
        c.prompt("Try again.");
        c.line(r#"{"jsonrpc":"2.0","id":1,"result":{"agentCapabilities":{"loadSession":true}}}"#);
        let message = json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Answer"}});
        assert!(update(&mut c, message.clone()).is_empty());
        let new = sent(&c.line(r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32603,"message":"load failed"}}"#));
        assert_eq!(new[0]["method"], "session/new");
        let prompt = sent(&c.line(r#"{"jsonrpc":"2.0","id":3,"result":{"sessionId":"replacement"}}"#));
        assert_eq!(prompt[0]["params"]["sessionId"], "replacement");
        let live = c.line(&json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "replacement", "update": message}}).to_string());
        assert_eq!(events(live), [AgentEvent::TextDelta { text: "Answer".into() }]);
    }

    #[test]
    fn cancelling_a_loading_session_does_not_publish_replay_or_send_the_cancelled_prompt() {
        let mut c = core(Some("old"));
        c.start();
        c.prompt("Stop this.");
        c.line(r#"{"jsonrpc":"2.0","id":1,"result":{"agentCapabilities":{"loadSession":true}}}"#);
        assert!(c.cancel().contains(&Out::TurnEnded));
        assert!(update(&mut c, json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "History"}})).is_empty());
        assert!(c.line(r#"{"jsonrpc":"2.0","id":2,"result":{}}"#).is_empty(), "the cancelled prompt stays cancelled when loading finishes");
        assert_eq!(sent(&c.prompt("A new request."))[0]["params"]["prompt"][0]["text"], "A new request.");
    }

    #[test]
    fn permission_requests_become_consent_and_are_answered_once() {
        let mut c = core(None);
        let ev = events(c.line(
            r#"{"jsonrpc":"2.0","id":7,"method":"session/request_permission","params":{"sessionId":"s","toolCall":{"toolCallId":"t2","title":"Run npm test","kind":"execute"},"options":[{"optionId":"yes-always","name":"Always","kind":"allow_always"},{"optionId":"yes","name":"Allow","kind":"allow_once"},{"optionId":"no","name":"Reject","kind":"reject_once"}]}}"#,
        ));
        let Some(AgentEvent::ConsentNeeded { call_id, tool, title, .. }) = ev.first() else { panic!("{ev:?}") };
        assert_eq!((tool.as_str(), title.as_str()), ("execute", "Run npm test"));
        let reply = sent(&c.answer(call_id, true));
        assert_eq!(reply[0], json!({"jsonrpc": "2.0", "id": 7, "result": {"outcome": {"outcome": "selected", "optionId": "yes"}}}));
        assert!(c.answer(call_id, true).is_empty(), "answered once");
    }

    /// #271: after `initialize` failed, the next message waited for a session forever, and Stop
    /// didn't end it.
    #[test]
    fn a_failed_start_is_tried_again_and_stop_ends_a_waiting_message() {
        let mut c = core(None);
        c.start();
        let failed = c.line(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"not signed in"}}"#);
        assert!(failed.contains(&Out::TurnEnded));
        let again = sent(&c.prompt("Hello"));
        assert_eq!(again[0]["method"], "initialize", "the next message starts the agent's session again");
        let failed = c.line(r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32603,"message":"not signed in"}}"#);
        assert!(failed.contains(&Out::TurnEnded), "a second failure ends that message too");
        c.prompt("Hello");
        let stopped = c.cancel();
        assert!(stopped.contains(&Out::TurnEnded), "Stop ends a message still waiting for the session");
        assert!(sent(&c.line(r#"{"jsonrpc":"2.0","id":3,"result":{}}"#)).iter().all(|m| m["method"] != "session/prompt"));
    }

    #[test]
    fn errors_end_the_turn_and_unoffered_requests_are_refused() {
        let mut c = core(None);
        c.start();
        c.prompt("hi");
        let outs = c.line(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"Authentication required"}}"#);
        assert_eq!(c.failure, Some(TurnFailure::new(FailureKind::Authentication)));
        assert!(outs.contains(&Out::TurnEnded));
        assert!(matches!(&events(outs)[0], AgentEvent::Error { message, remediation: Some(_), .. } if message.contains("Authentication required")));
        let refused = sent(&c.line(r#"{"jsonrpc":"2.0","id":9,"method":"fs/read_text_file","params":{"path":"/etc/passwd"}}"#));
        assert_eq!(refused[0]["error"]["code"], -32601);
        assert!(c.line("not json").is_empty());
    }

    #[test]
    fn a_builder_sends_permission_requests_to_the_approval_policy() {
        let mut c = core(None).deciding();
        let outs = c.line(
            r#"{"jsonrpc":"2.0","id":4,"method":"session/request_permission","params":{"toolCall":{"toolCallId":"t","title":"Edit App.tsx","kind":"edit","locations":[{"path":"/w/project/app/src/App.tsx"}]},"options":[{"optionId":"ok","name":"Allow","kind":"allow_once"},{"optionId":"no","name":"Reject","kind":"reject_once"}]}}"#,
        );
        let [Out::Decide { call_id, tool, input }] = &outs[..] else { panic!("{outs:?}") };
        assert_eq!((tool.as_str(), input), ("Write", &json!({"file_path": "/w/project/app/src/App.tsx"})));
        assert_eq!(sent(&c.answer(call_id, false))[0]["result"]["outcome"]["optionId"], "no");

        let run = |kind: &str, raw: Value| policy_action(&json!({"kind": kind, "title": "t", "rawInput": raw}));
        assert_eq!(run("execute", json!({"command": "npm run build"})), ("Bash".to_string(), json!({"command": "npm run build"})));
        assert_eq!(run("execute", json!({"command": ["cargo", "build"]})).1, json!({"command": "cargo build"}));
        assert_eq!(run("read", json!({"path": "/a"})).0, "Read");
        assert_eq!(run("fetch", json!({"url": "https://x"})).0, "WebFetch");
        assert_eq!(run("other", json!({})).0, "t", "unknown kinds are asked about by title");
    }

    #[test]
    fn maki_starts_in_acp_mode_with_the_backends_model() {
        assert_eq!(acp_args("maki"), Some(&["acp"][..]));
        assert_eq!(model_args("maki", Some(" anthropic/claude-sonnet-4-6 ")), ["-m", "anthropic/claude-sonnet-4-6"]);
        assert!(model_args("maki", None).is_empty() && model_args("maki", Some(" ")).is_empty());
        // An agent whose ACP mode isn't known to take a model keeps its own, as before.
        assert!(model_args("opencode", Some("x/y")).is_empty());
    }

    /// `maki -m <model> acp` starts with Maki's default model: the flag only counts after `acp`.
    #[test]
    fn a_maki_backend_runs_acp_then_its_model_after_its_own_args() {
        let config = crate::BackendConfig {
            name: "Maki".into(),
            kind: crate::BackendKind::AgentCli,
            model: "maki".into(),
            base_url: None,
            ssh_host: None,
            enabled: true,
        };
        let options = crate::BackendOptions {
            acp: Some(true),
            cli_model: Some("anthropic/claude-sonnet-4-6".into()),
            args: vec!["--no-plugins".into()],
            ..Default::default()
        };
        let Some(c) = super::super::acp_for(&super::super::CliSpec::new(config, options, false, None)) else { panic!("Maki speaks ACP") };
        assert_eq!((c.program.as_str(), c.args.as_slice()), ("maki", &["--no-plugins", "acp", "-m", "anthropic/claude-sonnet-4-6"].map(String::from)[..]));
        assert!(!c.adapter.acp_mcp_stdio_type());
    }

    /// Grok's server enum needs `"type": "stdio"`. A session for any other agent leaves it off,
    /// and a question still attaches no server.
    #[test]
    fn only_grok_sends_a_typed_stdio_server() {
        let mut plain = core(None);
        plain.start();
        let untyped = sent(&plain.line(r#"{"jsonrpc":"2.0","id":1,"result":{"agentCapabilities":{"loadSession":true}}}"#));
        assert_eq!(untyped[0]["method"], "session/new");
        assert!(untyped[0]["params"]["mcpServers"][0].get("type").is_none());

        let mut grok = core(None);
        grok.typed_mcp = true;
        grok.start();
        let typed = sent(&grok.line(r#"{"jsonrpc":"2.0","id":1,"result":{"agentCapabilities":{"loadSession":true}}}"#));
        assert_eq!(typed[0]["params"]["mcpServers"][0]["type"], "stdio");
        assert_eq!(typed[0]["params"]["mcpServers"][0]["name"], "craft-anything");

        let mut question = core(None);
        question.question = true;
        question.typed_mcp = true;
        question.start();
        let asked = sent(&question.line(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#));
        assert_eq!(asked[0]["params"]["mcpServers"], json!([]));
    }

    /// `grok agent [flags] stdio`: the model flag and `--no-leader` sit between `agent` and `stdio`.
    #[test]
    fn a_grok_backend_runs_agent_then_its_flags_then_stdio() {
        let options = crate::BackendOptions { cli_model: Some("grok-4.6".into()), cli_thinking: Some(crate::Effort::Low), ..Default::default() };
        let Some(c) = super::super::acp_for(&super::super::CliSpec::new(grok_config(), options, false, None)) else { panic!("Grok speaks ACP") };
        assert_eq!(
            (c.program.as_str(), c.args.as_slice()),
            ("grok", &["agent", "--model", "grok-4.6", "--no-leader", "--reasoning-effort", "low", "stdio"].map(String::from)[..])
        );
        assert_eq!(c.session_config, [("model".into(), "grok-4.6".into()), ("reasoning_effort".into(), "low".into())]);
        assert!(!c.args.iter().any(|arg| arg.contains("always-approve") || arg.contains("yolo")));

        let quiet = crate::BackendOptions::default();
        let Some(plain) = super::super::acp_for(&super::super::CliSpec::new(grok_config(), quiet.clone(), false, None)) else { panic!("Grok speaks ACP") };
        assert_eq!(plain.args, ["agent", "--no-leader", "stdio"]);
        assert!(plain.session_config.is_empty());
        assert!(plain.adapter.acp_mcp_stdio_type());
        // A chat that is not deep is Clean room. Grok cannot hold that, so the run is refused
        // before the process starts. The argv is still the one a deep build uses.
        assert!(plain.access_error.as_ref().is_some_and(|error| error.message.contains("Clean room")));

        let mut question = super::super::CliSpec::new(
            grok_config(),
            crate::BackendOptions { cli_model: Some("grok-4.5".into()), effort: Some(crate::Effort::Medium), ..Default::default() },
            false,
            None,
        );
        question.question = true;
        let Some(asked) = super::super::acp_for(&question) else { panic!("Grok answers questions") };
        assert_eq!(asked.args, ["agent", "--model", "grok-4.5", "--no-leader", "--reasoning-effort", "medium", "stdio"]);
        assert!(asked.question);
        assert!(asked.access_error.as_ref().is_some_and(|error| error.message.contains("question")));
        assert_eq!(asked.session_config, [("model".into(), "grok-4.5".into()), ("reasoning_effort".into(), "medium".into())]);

        // Deep chat is analysis only. Grok can still edit when it never asks, so that run is refused too.
        let mut chat = super::super::CliSpec::new(grok_config(), crate::BackendOptions::default(), false, None);
        chat.deep = true;
        let Some(analyse) = super::super::acp_for(&chat) else { panic!("Grok speaks ACP") };
        assert!(analyse.access_error.as_ref().is_some_and(|error| error.message.contains("deep chat")));
        assert_eq!(analyse.args, ["agent", "--no-leader", "stdio"]);

        let mut deep = super::super::CliSpec::new(grok_config(), quiet, false, None);
        deep.deep = true;
        deep.builder = Some(super::super::Builder { investigation: None, inbox: std::env::temp_dir() });
        let Some(build) = super::super::acp_for(&deep) else { panic!("a deep build is allowed") };
        assert!(build.access_error.is_none() && build.validation_error.is_none());
        assert_eq!(build.args, ["agent", "--no-leader", "stdio"]);

        let blocked = crate::BackendOptions { args: vec!["--always-approve".into()], ..Default::default() };
        let Some(refused) = super::super::acp_for(&super::super::CliSpec::new(grok_config(), blocked, false, None)) else { panic!("Grok speaks ACP") };
        assert_eq!(refused.args, ["agent", "--no-leader", "stdio"]);
        assert!(refused.validation_error.is_some());

        let mut remote = grok_config();
        remote.ssh_host = Some("other-computer".into());
        assert!(super::super::acp_for(&super::super::CliSpec::new(remote, crate::BackendOptions::default(), false, None)).is_none());
        let off = crate::BackendOptions { acp: Some(false), ..Default::default() };
        assert!(super::super::acp_for(&super::super::CliSpec::new(grok_config(), off, false, None)).is_none());
    }

    fn grok_config() -> crate::BackendConfig {
        crate::BackendConfig { name: "Grok".into(), kind: crate::BackendKind::AgentCli, model: "grok".into(), base_url: None, ssh_host: None, enabled: true }
    }

    fn session_opened(model: &str, effort: &str) -> String {
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "sessionId": "s-1",
                "models": {"currentModelId": model, "availableModels": [{"modelId": "grok-4.7"}, {"modelId": "grok-4.6"}]},
                "configOptions": [
                    {"id": "model", "currentValue": model, "options": [{"value": "grok-4.7", "name": "Grok 4.7"}, {"value": "grok-4.6", "name": "Grok 4.6"}]},
                    {"id": "reasoning_effort", "category": "thought_level", "currentValue": effort, "options": [{"value": "low", "name": "Low"}, {"value": "high", "name": "High"}]}
                ]
            }
        })
        .to_string()
    }

    fn config_reply(id: u64, model: &str, effort: &str) -> String {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {"configOptions": [
                {"id": "model", "currentValue": model},
                {"id": "reasoning_effort", "currentValue": effort, "options": [{"value": "low", "name": "Low"}, {"value": "medium", "name": "Medium"}, {"value": "high", "name": "High"}]}
            ]}
        })
        .to_string()
    }

    /// The CLI model flag does not stick after `session/new`. Overrides go out before the prompt,
    /// a value the session already has is skipped, and the session id is kept.
    #[test]
    fn session_config_is_applied_before_the_prompt_and_a_matching_value_is_skipped() {
        let mut c = core(None);
        c.config_queue = VecDeque::from([("model".into(), "grok-4.6".into()), ("reasoning_effort".into(), "low".into())]);
        c.prompt("Build it.");
        c.line(r#"{"jsonrpc":"2.0","id":1,"result":{"agentCapabilities":{"loadSession":true}}}"#);
        let first = sent(&c.line(&session_opened("grok-4.7", "high")));
        assert_eq!(first[0]["method"], "session/set_config_option");
        assert_eq!(first[0]["params"], json!({"sessionId": "s-1", "configId": "model", "value": "grok-4.6"}));
        assert!(!c.ready, "the picker waits until the override lands");
        assert_eq!(c.models, ["grok-4.7", "grok-4.6"]);
        let second = sent(&c.line(&config_reply(3, "grok-4.6", "high")));
        assert_eq!(second[0]["params"], json!({"sessionId": "s-1", "configId": "reasoning_effort", "value": "low"}));
        assert_eq!(c.current_model, "grok-4.6");
        assert_eq!(c.session_id().as_deref(), Some("s-1"), "a config reply does not carry the session id");
        let prompt = sent(&c.line(&config_reply(4, "grok-4.6", "low")));
        assert_eq!(prompt[0]["method"], "session/prompt");
        assert_eq!(prompt[0]["params"]["prompt"][0]["text"], "Build it.");
        assert!(c.ready);
        assert_eq!(c.models, ["grok-4.7", "grok-4.6"], "the session's model list stays");

        let mut same = core(None);
        same.config_queue = VecDeque::from([("model".into(), "grok-4.7".into()), ("reasoning_effort".into(), "low".into())]);
        same.prompt("Hi");
        same.line(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#);
        let only_effort = sent(&same.line(&session_opened("grok-4.7", "high")));
        assert_eq!(only_effort.len(), 1);
        assert_eq!(only_effort[0]["params"]["configId"], "reasoning_effort");
        assert_eq!(only_effort[0]["params"]["value"], "low");

        let mut failed = core(None);
        failed.config_queue = VecDeque::from([("model".into(), "missing".into())]);
        failed.prompt("Hi");
        failed.line(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#);
        failed.line(&session_opened("grok-4.7", "high"));
        let outs = failed.line(r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32602,"message":"unknown model"}}"#);
        assert_eq!(failed.failure, Some(TurnFailure::new(FailureKind::Configuration)));
        assert!(outs.contains(&Out::TurnEnded));
        assert!(!failed.ready);
        assert!(sent(&outs).is_empty(), "a failed override does not prompt");
    }

    #[test]
    fn stopping_cancels_the_prompt_and_open_questions() {
        let mut c = core(None);
        c.start();
        c.prompt("long task");
        c.line(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#);
        c.line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s"}}"#);
        c.line(r#"{"jsonrpc":"2.0","id":5,"method":"session/request_permission","params":{"toolCall":{"title":"Edit"},"options":[]}}"#);
        let out = sent(&c.cancel());
        assert!(out.iter().any(|m| m["id"] == 5 && m["result"]["outcome"]["outcome"] == "cancelled"));
        assert!(out.iter().any(|m| m["method"] == "session/cancel" && m["params"]["sessionId"] == "s"));
    }
}
