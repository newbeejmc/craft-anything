//! The built-in agent loop: a model, the engine's tools, consent, memory and the journal.
//!
//! Everything here runs on a background worker thread owned by [`ApiConversation`]; the UI only
//! ever calls the non-blocking [`Conversation`] methods.

use crate::anthropic::INVALID_INPUT_KEY;
use crate::context;
use crate::model::{Block, ChatModel, Delta, Effort, Msg, Speaker, Stop, StopReason, ToolDef, TurnRequest, Usage};
use crate::price::{self, Meter};
use crate::prompt;
use crate::{AgentEvent, Conversation, TurnFailure};
use craft_anything_engine::{CallOptions, Caller, CancelToken, Consent, Engine, Registry, ToolOutput, Toolset};
use craft_anything_evidence::{ErrorCode, ToolError};
use craft_anything_platform::sync::Mutex;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, PoisonError};
use std::time::Duration;

/// Byte budget for one tool result sent to the model.
pub const RESULT_BUDGET: usize = 16 * 1024;
/// Default most model turns per user message.
pub const DEFAULT_MAX_TURNS: u32 = 40;
/// Steps one builder message may take (writing, building and fixing a project takes many).
pub const BUILDER_MAX_TURNS: u32 = 400;

/// The build tool whose result says whether the command succeeded (`succeeded`, typed).
const BUILD_COMMAND_TOOL: &str = "build_command_run";

/// Build commands that fail one after another, with none succeeding in between, before a step
/// stops (#399: a model wrote code against APIs that don't exist, never got it to compile, and
/// spent 9.4M tokens in one stage). Each failed build sends the whole conversation again.
pub const MAX_FAILED_BUILDS: u32 = 25;

/// What the loop needs from the engine (a trait so tests can inject behaviour).
pub trait ToolHost: Send + Sync {
    /// Tool definitions for the enabled toolsets (without the meta-tools).
    fn tools(&self, enabled: &BTreeSet<Toolset>) -> Vec<ToolDef>;
    /// The toolset a tool belongs to, if it exists.
    fn toolset_of(&self, id: &str) -> Option<Toolset>;
    fn call(&self, id: &str, input: Value, consent: bool, cancel: CancelToken) -> Result<ToolOutput, ToolError>;
    fn journal(&self, kind: &str, entry: Value);
    /// `core_investigation_show` as compact JSON text.
    fn investigation(&self) -> String;
    fn clean_room(&self) -> bool;
    /// A tool's title (for consent prompts).
    fn title(&self, id: &str) -> String;
    /// Every toolset in use (built-in and added at run time).
    fn toolsets(&self) -> Vec<Toolset> {
        Toolset::ALL.to_vec()
    }
}

/// The real host: the engine.
pub struct EngineHost(pub Arc<Engine>);

/// Remove keys model APIs dislike at the schema root, and write boolean sub-schemas as objects.
fn clean_schema(mut s: Value) -> Value {
    if let Some(o) = s.as_object_mut() {
        o.remove("$schema");
        o.remove("title");
        if !o.contains_key("type") {
            o.insert("type".into(), json!("object"));
        }
        if !o.contains_key("properties") && o.get("type") == Some(&json!("object")) {
            o.insert("properties".into(), json!({}));
        }
    }
    object_subschemas(&mut s, 0);
    s
}

/// Deepest schema nesting rewritten (run-time tools bring their own schemas).
const MAX_SCHEMA_DEPTH: usize = 64;

/// `true` and `false` as sub-schemas (schemars writes `"items": true` for a list of any JSON) become
/// `{}` and `{"not": {}}`: local servers such as LM Studio refuse anything but an object there.
/// `additionalProperties` stays as it is: a boolean is what every API expects in it.
fn object_subschemas(schema: &mut Value, depth: usize) {
    let Some(o) = schema.as_object_mut().filter(|_| depth < MAX_SCHEMA_DEPTH) else { return };
    for (key, sub) in o.iter_mut() {
        let subs: Vec<&mut Value> = match (key.as_str(), sub) {
            ("items" | "additionalItems" | "contains" | "not" | "if" | "then" | "else", s) => vec![s],
            ("properties" | "patternProperties" | "$defs" | "definitions", Value::Object(m)) => m.values_mut().collect(),
            ("anyOf" | "oneOf" | "allOf" | "prefixItems", Value::Array(a)) => a.iter_mut().collect(),
            _ => Vec::new(),
        };
        for s in subs {
            match s {
                Value::Bool(true) => *s = json!({}),
                Value::Bool(false) => *s = json!({"not": {}}),
                _ => object_subschemas(s, depth + 1),
            }
        }
    }
}

pub fn registry_tools(reg: &Registry, enabled: &BTreeSet<Toolset>) -> Vec<ToolDef> {
    reg.all()
        .iter()
        .filter(|s| enabled.contains(&s.toolset))
        .map(|s| ToolDef { name: s.id.to_string(), description: s.summary.to_string(), schema: clean_schema(reg.input_schema(s)) })
        .collect()
}

impl ToolHost for EngineHost {
    fn tools(&self, enabled: &BTreeSet<Toolset>) -> Vec<ToolDef> {
        // Clean room offers only the tools it runs.
        let clean = self.clean_room();
        registry_tools(self.0.registry(), enabled).into_iter().filter(|t| !clean || craft_anything_engine::clean_room::allows(&t.name)).collect()
    }

    fn toolset_of(&self, id: &str) -> Option<Toolset> {
        self.0.registry().get(id).map(|s| s.toolset)
    }

    fn toolsets(&self) -> Vec<Toolset> {
        self.0.registry().toolsets()
    }

    fn call(&self, id: &str, input: Value, consent: bool, cancel: CancelToken) -> Result<ToolOutput, ToolError> {
        let opts = CallOptions::new(Caller::Agent).with_cancel(cancel).with_consent(if consent { Consent::Granted } else { Consent::NotAsked });
        self.0.call(id, input, opts)
    }

    fn journal(&self, kind: &str, entry: Value) {
        self.0.journal(kind, entry);
    }

    fn investigation(&self) -> String {
        match self.0.call("core_investigation_show", json!({}), CallOptions::new(Caller::Agent)) {
            Ok(o) => {
                let v = o.to_json();
                let inner = v.get("result").cloned().unwrap_or(v);
                truncate_json(&inner, 24 * 1024)
            }
            Err(e) => format!("(couldn't load: {})", e.message),
        }
    }

    fn clean_room(&self) -> bool {
        self.0.try_session().map(|s| s.clean_room()).unwrap_or(true)
    }

    fn title(&self, id: &str) -> String {
        self.0.registry().get(id).map(|s| s.title.to_string()).unwrap_or_else(|| id.to_string())
    }
}

/// The engine with only the tools that change nothing: no files written, nothing started, no
/// network, nobody asked (recording evidence in the session is fine). For questions about a build.
pub struct ReadOnlyHost(pub EngineHost);

impl ReadOnlyHost {
    fn changes_nothing(&self, id: &str) -> bool {
        use craft_anything_engine::Effects;
        let harmless = Effects::IDEMPOTENT.union(Effects::MUTATES_SESSION).union(Effects::CARRIES_TARGET_CONTENT);
        self.0.0.registry().get(id).is_some_and(|s| harmless.contains(s.effects))
    }
}

impl ToolHost for ReadOnlyHost {
    fn tools(&self, enabled: &BTreeSet<Toolset>) -> Vec<ToolDef> {
        self.0.tools(enabled).into_iter().filter(|t| self.changes_nothing(&t.name)).collect()
    }
    fn toolset_of(&self, id: &str) -> Option<Toolset> {
        self.0.toolset_of(id)
    }
    fn toolsets(&self) -> Vec<Toolset> {
        self.0.toolsets()
    }
    fn call(&self, id: &str, input: Value, _consent: bool, cancel: CancelToken) -> Result<ToolOutput, ToolError> {
        if !self.changes_nothing(id) {
            return Err(ToolError::new(ErrorCode::NoPermission, format!("{id} changes something, and this is only a question."))
                .with_remediation("Answer from what you can read."));
        }
        // Nothing that needs consent is offered, so none is given.
        self.0.call(id, input, false, cancel)
    }
    fn journal(&self, kind: &str, entry: Value) {
        self.0.journal(kind, entry);
    }
    fn investigation(&self) -> String {
        self.0.investigation()
    }
    fn clean_room(&self) -> bool {
        self.0.clean_room()
    }
    fn title(&self, id: &str) -> String {
        self.0.title(id)
    }
}

/// JSON text no longer than `budget` bytes (cut on a char boundary with a note).
pub fn truncate_json(v: &Value, budget: usize) -> String {
    let s = v.to_string();
    if s.len() <= budget {
        return s;
    }
    let mut cut = budget.saturating_sub(120);
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}… [truncated: {} of {} bytes shown; page with offset/limit or fetch the evidence by id]", s.get(..cut).unwrap_or_default(), cut, s.len())
}

/// What the model sees for a tool's output: evidence summary + id + (bounded) result.
pub fn compact(out: &ToolOutput) -> (String, String, Option<String>) {
    match out {
        ToolOutput::Evidence(ev) => {
            let mut v = ev.summary_view();
            let result_text = ev.result.to_string();
            let summary_len = v.to_string().len();
            if let Some(o) = v.as_object_mut() {
                if result_text.len() + summary_len <= RESULT_BUDGET {
                    o.insert("result".into(), ev.result.clone());
                } else {
                    o.insert("result_preview".into(), json!(truncate_json(&ev.result, RESULT_BUDGET.saturating_sub(summary_len).max(512))));
                    o.insert(
                        "note".into(),
                        json!("Result is large: this is a preview. Page with offset/limit, or get the full record with core_evidence_show {evidence_id}."),
                    );
                }
            }
            let summary = if ev.summary.is_empty() { ev.operation.clone() } else { ev.summary.clone() };
            (truncate_json(&v, RESULT_BUDGET + 1024), summary, Some(ev.id.clone()))
        }
        ToolOutput::Value(v) => {
            let text = truncate_json(&out.to_json(), RESULT_BUDGET);
            let summary: String = v.to_string().chars().take(160).collect();
            (text, summary, None)
        }
    }
}

fn error_text(e: &ToolError) -> String {
    serde_json::to_string(&json!({"error": e})).unwrap_or_else(|_| e.message.clone())
}

/// Consent waiters shared between the worker and the UI-facing handle.
type Waiters = Arc<Mutex<HashMap<String, Sender<bool>>>>;

/// One model with its display name (a link in a fallback chain).
pub struct ModelSlot {
    pub name: String,
    pub model: Box<dyn ChatModel>,
    /// Local models cost nothing.
    pub local: bool,
    pub effort: Option<Effort>,
    pub max_turns: u32,
    /// The backend's conversation budget; zero uses the local/cloud default.
    pub context_tokens: u64,
}

/// Settings for one conversation.
pub struct LoopConfig {
    pub beginner: bool,
    pub prices: BTreeMap<String, crate::config::Price>,
    /// Keep the conversation in this file after every step, and continue it when it's there in
    /// this same mode.
    pub transcript: Option<std::path::PathBuf>,
    /// The mode the conversation runs in (`true`: Deep). A kept transcript from the other mode
    /// isn't loaded, since it may carry what a Deep run read. Must be set whenever `transcript`
    /// is `Some`; `Default` is Deep, the safe direction.
    pub deep: bool,
    /// Override the input tokens the conversation may grow to before it's trimmed. Zero uses
    /// the active backend's budget, or its local/cloud default (see [`crate::context`]).
    pub context_tokens: u64,
    /// A builder on a local model is offered the tools for building, not the whole Core set (see
    /// [`lean_for_local`]).
    pub lean_for_local: bool,
}

impl Default for LoopConfig {
    /// Deep, so a conversation started without an explicit mode keeps the old, safe behavior.
    fn default() -> Self {
        Self { beginner: false, prices: BTreeMap::new(), transcript: None, deep: true, context_tokens: 0, lean_for_local: false }
    }
}

/// The Core tools a builder on a local model keeps. The Core set is 25-odd tools for studying a
/// target (evidence, unknowns, hypotheses, reports…) that make a building model explore instead
/// of writing; their descriptions alone take about 5,000 tokens, more than Ollama's default
/// context holds (#417). Kept are the Build tools, every other enabled toolset, and the Core tools
/// the builder's brief names: asking the person, the check, the generated assets and the unknowns.
const LOCAL_BUILDER_CORE: &[&str] = &[
    "core_person_ask",
    "rebuild_check",
    "rebuild_words",
    "assets_image_generate",
    "assets_model_generate",
    "assets_spent",
    "assets_note",
    "core_unknowns_show",
    "core_unknown_add",
    "core_unknown_edit",
];

/// Drop the Core tools a local builder doesn't need (the models that can take them all don't pass
/// through here).
fn lean_for_local(tools: Vec<ToolDef>, host: &dyn ToolHost) -> Vec<ToolDef> {
    tools.into_iter().filter(|t| host.toolset_of(&t.name) != Some(Toolset::Core) || LOCAL_BUILDER_CORE.contains(&t.name.as_str())).collect()
}

/// The state the worker keeps between user messages.
pub struct AgentLoop {
    host: Arc<dyn ToolHost>,
    chain: Vec<ModelSlot>,
    current: usize,
    history: Vec<Msg>,
    system: Option<String>,
    enabled: BTreeSet<Toolset>,
    cfg: LoopConfig,
    events: Sender<AgentEvent>,
    waiters: Waiters,
    stop: Stop,
    cancel: Arc<Mutex<CancelToken>>,
    pub meter: Meter,
    /// The last turn's real input size (tokens), 0 before the first or after making room.
    last_input: u64,
    /// Estimated history size sent with that turn, before its reply and tool results were added.
    last_history_estimate: u64,
    failure: Arc<Mutex<Option<TurnFailure>>>,
    /// Build commands that failed in a row in this step (see [`MAX_FAILED_BUILDS`]).
    failed_builds: u32,
    /// Context windows servers said they have, by chain position: a request that didn't fit
    /// tells us the real size (#962), which may be smaller than the budget we assumed.
    server_windows: BTreeMap<usize, u64>,
}

impl AgentLoop {
    fn emit(&self, e: AgentEvent) {
        let _ = self.events.send(e);
    }

    fn tool_defs(&self) -> Vec<ToolDef> {
        let mut tools = self.host.tools(&self.enabled);
        if self.cfg.lean_for_local && self.chain.get(self.current).is_some_and(|s| s.local) {
            tools = lean_for_local(tools, self.host.as_ref());
        }
        tools.push(ToolDef {
            name: "list_toolsets".into(),
            description: "List the groups of tools (toolsets), which are enabled, and what each covers.".into(),
            schema: json!({"type": "object", "properties": {}}),
        });
        tools.push(ToolDef {
            name: "enable_toolset".into(),
            description: "Enable a toolset so its tools become available from the next step on. Use list_toolsets to see the ids.".into(),
            schema: json!({"type": "object", "properties": {"toolset": {"type": "string", "enum": self.host.toolsets().iter().map(|t| t.id()).collect::<Vec<_>>()}}, "required": ["toolset"]}),
        });
        crate::anthropic::sorted(tools)
    }

    /// Handle one user message to completion.
    pub fn handle(&mut self, text: &str) {
        self.stop.reset();
        let done = self.handle_turn(Some(text));
        self.emit(done);
    }

    /// A retry has no new user message: keep completed tool calls and their results, then
    /// continue the failed model request using the same history.
    fn handle_turn(&mut self, text: Option<&str>) -> AgentEvent {
        if self.stop.is_stopped() {
            self.emit(AgentEvent::Status { text: "Stopped.".into() });
            return AgentEvent::TurnDone {
                backend: self.chain.get(self.current).map(|slot| slot.name.clone()).unwrap_or_default(),
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: None,
                cost_is_estimate: false,
            };
        }
        let cancel = CancelToken::default();
        *self.cancel.lock().unwrap_or_else(PoisonError::into_inner) = cancel.clone();
        if let Some(text) = text {
            self.host.journal("user_message", json!({"text": crate::journal_text(text)}));
        }
        if self.system.is_none() {
            self.emit(AgentEvent::Status { text: "Reading the investigation so far…".into() });
            let inv = self.host.investigation();
            let memory = !(self.cfg.lean_for_local && self.chain.get(self.current).is_some_and(|s| s.local));
            self.system = Some(prompt::system_prompt(self.cfg.beginner, self.host.clean_room(), &inv, memory));
        }
        if let Some(text) = text {
            context::push_user_text(&mut self.history, text);
        }
        let configured = self.chain.get(self.current).and_then(|s| s.effort);
        let effort = prompt::choose_effort(configured);
        let max_turns = self.chain.get(self.current).map(|s| s.max_turns).unwrap_or(DEFAULT_MAX_TURNS).max(1);
        let mut msg_usage = Usage::default();
        let mut msg_cost: Option<f64> = Some(0.0);
        let mut turns = 0u32;
        self.failed_builds = 0;
        loop {
            if self.stop.is_stopped() {
                self.emit(AgentEvent::Status { text: "Stopped.".into() });
                break;
            }
            if turns >= max_turns {
                self.emit(AgentEvent::Error {
                    message: format!("Stopped after {max_turns} steps without a final answer."),
                    remediation: Some("Ask it to continue, or narrow the question.".into()),
                    log: None,
                });
                break;
            }
            turns += 1;
            let tools = self.tool_defs();
            let system = self.system.clone().unwrap_or_default();
            let Some(result) = self.one_turn(&system, &tools, effort, &mut msg_usage, &mut msg_cost) else { break };
            self.last_input = result.usage.input_tokens + result.usage.cache_read_tokens + result.usage.cache_write_tokens;
            self.last_history_estimate = context::estimate_tokens(&self.history).max(1);
            let (slot_name, local, model_id) = match self.chain.get(self.current) {
                Some(s) => (s.name.clone(), s.local, result.model.clone()),
                None => break,
            };
            let cost = price::cost(&model_id, &result.usage, &self.cfg.prices, local);
            self.meter.add(result.usage, cost);
            msg_usage.add(result.usage);
            msg_cost = match (msg_cost, cost) {
                (Some(a), Some(b)) => Some(a + b),
                _ => None,
            };
            self.host.journal(
                "model_turn",
                json!({
                    "backend": slot_name, "model": model_id, "input_tokens": result.usage.input_tokens,
                    "output_tokens": result.usage.output_tokens, "cache_read_tokens": result.usage.cache_read_tokens,
                    "cache_write_tokens": result.usage.cache_write_tokens, "cost_usd": cost, "stop_reason": result.stop.as_str(),
                    "effort": effort.as_str(),
                }),
            );
            let uses = result.message.tool_uses();
            match result.stop {
                StopReason::Refusal => {
                    self.emit(AgentEvent::Error {
                        message: "The model declined to answer this request.".into(),
                        remediation: Some("Rephrase the question, say why you're examining the software, or try another backend.".into()),
                        log: None,
                    });
                    self.host.journal("error", json!({"kind": "refusal", "backend": slot_name}));
                    // Drop the refused turn so the conversation stays usable.
                    break;
                }
                StopReason::PauseTurn => {
                    self.history.push(result.message);
                    self.record_unrun_tools(&uses);
                    if let Some(follow_up) = result.follow_up {
                        self.history.push(Msg::user_text(follow_up));
                    }
                    continue;
                }
                StopReason::ToolUse if !uses.is_empty() => {
                    self.history.push(result.message);
                    let results = self.run_tools(&uses);
                    self.history.push(Msg { role: Speaker::User, blocks: results });
                    self.save_transcript();
                    if self.failed_builds >= MAX_FAILED_BUILDS {
                        self.emit(AgentEvent::Error {
                            message: format!("The build command failed {} times in a row, so this step stopped before spending more.", self.failed_builds),
                            remediation: Some(
                                "Ask for a fix (say what to change), or try a stronger model. Try again carries on from the project as it is.".into(),
                            ),
                            log: None,
                        });
                        break;
                    }
                }
                StopReason::MaxTokens if !uses.is_empty() => {
                    // Tool input may be cut off: don't run anything, tell the model.
                    self.history.push(result.message);
                    let blocks = uses
                        .iter()
                        .map(|(id, _, _)| Block::ToolResult {
                            tool_use_id: id.clone(),
                            content: "Not run: the reply hit the output limit, so this call may be incomplete. Retry it with smaller input.".into(),
                            is_error: true,
                        })
                        .collect();
                    self.history.push(Msg { role: Speaker::User, blocks });
                }
                StopReason::MaxTokens => {
                    self.history.push(result.message);
                    self.emit(AgentEvent::Status { text: "The answer hit the length limit and was cut off.".into() });
                    break;
                }
                _ => {
                    self.history.push(result.message);
                    self.record_unrun_tools(&uses);
                    break;
                }
            }
        }
        self.save_transcript();
        let backend = self.chain.get(self.current).map(|s| s.name.clone()).unwrap_or_default();
        AgentEvent::TurnDone {
            backend,
            input_tokens: msg_usage.input_tokens.saturating_add(msg_usage.cache_read_tokens).saturating_add(msg_usage.cache_write_tokens),
            output_tokens: msg_usage.output_tokens,
            cost_usd: msg_cost,
            cost_is_estimate: false,
        }
    }

    /// Pair retained calls left unexecuted by a stop reason before reusing the history.
    fn record_unrun_tools(&mut self, uses: &[(String, String, Value)]) {
        if uses.is_empty() {
            return;
        }
        let blocks = uses
            .iter()
            .map(|(id, _, _)| Block::ToolResult {
                tool_use_id: id.clone(),
                content: "Not run: the reply stopped without requesting tool execution.".into(),
                is_error: true,
            })
            .collect();
        self.history.push(Msg { role: Speaker::User, blocks });
    }

    /// Keep the conversation in its transcript file, when it has one.
    fn save_transcript(&self) {
        if let Some(p) = &self.cfg.transcript
            && let Err(e) = context::save(p, &self.history, self.cfg.deep)
        {
            log::warn!("couldn't keep the conversation in {}: {}", p.display(), e.message);
        }
    }

    /// Input tokens the conversation may take before room is made.
    fn context_budget(&self) -> u64 {
        let slot = self.chain.get(self.current);
        let configured = if self.cfg.context_tokens == 0 { slot.map(|s| s.context_tokens).unwrap_or(0) } else { self.cfg.context_tokens };
        // A window the server reported replaces our guess (a budget the person set stays the most),
        // with a quarter of it left free for the answer.
        let reported = self.server_windows.get(&self.current).map(|window| window - window / 4);
        match (configured, reported) {
            (0, Some(room)) => room,
            (n, Some(room)) => n.min(room),
            (0, None) if slot.is_some_and(|s| s.local) => context::LOCAL_CONTEXT_TOKENS,
            (0, None) => context::CLOUD_CONTEXT_TOKENS,
            (n, None) => n,
        }
    }

    /// Make room when the conversation has outgrown its budget: trim old tool results first; when
    /// that isn't enough, replace the oldest part with a summary the model writes. Return the
    /// successful summary call's usage so the message and build account for it too.
    fn fit_context(&mut self, system: &str, tools: &[ToolDef]) -> Option<(Usage, Option<f64>)> {
        let budget = self.context_budget();
        let estimate = context::estimate_tokens(&self.history).max(1);
        // Without a reported count (a local server often sends none), the system prompt and tool
        // schemas are added from their text; with one, it already includes them.
        let unreported = (self.last_input == 0).then(|| context::estimate_overhead(system, tools));
        // Reported input includes prompt/tool overhead. Add the history accumulated since it
        // was measured; comparing only with the current estimate can miss that growth.
        let size = match unreported {
            None => self.last_input.saturating_add(estimate.saturating_sub(self.last_history_estimate)),
            Some(overhead) => estimate.saturating_add(overhead),
        };
        if size <= budget {
            return None;
        }
        // Use the same approximate scale to judge the trimmed history before the next turn.
        let scale = size as f64 / estimate as f64;
        let trimmed = context::trim_old(&mut self.history);
        let after = match unreported {
            None => (context::estimate_tokens(&self.history) as f64 * scale) as u64,
            Some(overhead) => context::estimate_tokens(&self.history).saturating_add(overhead),
        };
        self.last_input = 0;
        self.last_history_estimate = 0;
        self.host.journal("context", json!({"action": "trim", "blocks": trimmed, "tokens_before": size, "tokens_after": after, "budget": budget}));
        // floor(3 * budget / 4), without overflowing for a large configured budget.
        if trimmed > 0 && after <= budget - budget.div_ceil(4) {
            return None;
        }
        let cut = context::cut_point(&self.history)?;
        self.emit(AgentEvent::Status { text: "Making room: summarising the earlier work…".into() });
        let messages = context::with_summary_request(&self.history[..cut]);
        let req = TurnRequest { system, messages: &messages, tools: &[], effort: Effort::Low, max_tokens: crate::anthropic::MAX_TOKENS };
        let stop = self.stop.clone();
        let summary = self.chain.get_mut(self.current).map(|slot| slot.model.turn(&req, &mut |_| {}, &stop));
        let mut accounted = None;
        let text = match summary {
            Some(Ok(r)) => {
                let local = self.chain.get(self.current).is_some_and(|s| s.local);
                let backend = self.chain.get(self.current).map(|s| s.name.as_str()).unwrap_or_default();
                let cost = price::cost(&r.model, &r.usage, &self.cfg.prices, local);
                self.meter.add(r.usage, cost);
                accounted = Some((r.usage, cost));
                self.host.journal(
                    "model_turn",
                    json!({
                        "backend": backend, "model": r.model, "input_tokens": r.usage.input_tokens,
                        "output_tokens": r.usage.output_tokens, "cache_read_tokens": r.usage.cache_read_tokens,
                        "cache_write_tokens": r.usage.cache_write_tokens, "cost_usd": cost, "stop_reason": r.stop.as_str(),
                        "effort": Effort::Low.as_str(), "purpose": "context_summary",
                    }),
                );
                r.message.text()
            }
            Some(Err(e)) => {
                self.host.journal("error", json!({"kind": "summary", "message": e.message}));
                String::new()
            }
            None => String::new(),
        };
        let text = if text.trim().is_empty() {
            "A summary of the earlier work couldn't be made. Look at the project's files (build_files_list) to see what's done.".to_string()
        } else {
            text
        };
        let before = self.history.len();
        self.history = context::summarised(&self.history, cut, &text);
        self.host.journal("context", json!({"action": "summary", "messages_before": before, "messages_after": self.history.len()}));
        self.save_transcript();
        accounted
    }

    /// One model call, falling back along the chain when a backend fails before producing output.
    fn one_turn(
        &mut self,
        system: &str,
        tools: &[ToolDef],
        effort: Effort,
        msg_usage: &mut Usage,
        msg_cost: &mut Option<f64>,
    ) -> Option<crate::model::TurnResult> {
        // Chain positions already fitted to a window their server reported: each is asked once more.
        let mut refitted = BTreeSet::new();
        loop {
            // A fallback can have a smaller window than the backend that just failed.
            if let Some((usage, cost)) = self.fit_context(system, tools) {
                msg_usage.add(usage);
                *msg_cost = match (*msg_cost, cost) {
                    (Some(a), Some(b)) => Some(a + b),
                    _ => None,
                };
            }
            let events = self.events.clone();
            let mut produced = false;
            let stop = self.stop.clone();
            let req = TurnRequest { system, messages: &self.history, tools, effort, max_tokens: crate::anthropic::MAX_TOKENS };
            let slot = self.chain.get_mut(self.current)?;
            let r = slot.model.turn(
                &req,
                &mut |d| {
                    if let Delta::Text(t) = d {
                        produced = true;
                        let _ = events.send(AgentEvent::TextDelta { text: t });
                    }
                },
                &stop,
            );
            match r {
                Ok(r) => return Some(r),
                Err(e) if e.code == ErrorCode::Cancelled => {
                    self.emit(AgentEvent::Status { text: "Stopped.".into() });
                    return None;
                }
                Err(e) => {
                    let name = slot.name.clone();
                    self.host.journal("error", json!({"backend": name, "code": e.code, "message": e.message}));
                    // The request didn't fit the server's context window, and it said how large that
                    // is: make room to fit it, measured with the server's own count, and ask once more.
                    let detail = |k: &str| e.details.as_ref().and_then(|d| d.get(k)).and_then(Value::as_u64);
                    if !produced
                        && let Some(window) = detail("context_window")
                        && refitted.insert(self.current)
                    {
                        self.server_windows.insert(self.current, window);
                        // The refused request was larger than the window, whatever else is known.
                        self.last_input = detail("prompt_tokens").unwrap_or(0).max(window.saturating_add(1));
                        let before = context::estimate_tokens(&self.history);
                        self.last_history_estimate = before;
                        self.emit(AgentEvent::Status { text: format!("{name} has room for {window} tokens; making room…") });
                        if let Some((usage, cost)) = self.fit_context(system, tools) {
                            msg_usage.add(usage);
                            *msg_cost = match (*msg_cost, cost) {
                                (Some(a), Some(b)) => Some(a + b),
                                _ => None,
                            };
                        }
                        // The same request would be refused the same way: ask again only when room was made.
                        if context::estimate_tokens(&self.history) < before {
                            continue;
                        }
                    }
                    if !produced && self.current + 1 < self.chain.len() {
                        self.current += 1;
                        let next = self.chain.get(self.current).map(|s| s.name.clone()).unwrap_or_default();
                        self.emit(AgentEvent::Status { text: format!("{name} failed ({}). Trying {next}…", e.message) });
                        continue;
                    }
                    *self.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(TurnFailure::from_api(&e));
                    self.emit(AgentEvent::Error { message: format!("{name}: {}", e.message), remediation: e.remediation, log: None });
                    return None;
                }
            }
        }
    }

    /// Run every tool call of one assistant message; all results go back in one user message.
    fn run_tools(&mut self, uses: &[(String, String, Value)]) -> Vec<Block> {
        let mut out = Vec::with_capacity(uses.len());
        for (id, name, input) in uses {
            if self.stop.is_stopped() {
                out.push(Block::ToolResult { tool_use_id: id.clone(), content: "Not run: the user pressed stop.".into(), is_error: true });
                continue;
            }
            self.emit(AgentEvent::ToolStarted { call_id: id.clone(), tool: name.clone(), input: input.clone() });
            let (content, ok, summary, evidence_id) = self.run_one(id, name, input.clone());
            self.emit(AgentEvent::ToolFinished { call_id: id.clone(), tool: name.clone(), ok, summary, evidence_id });
            out.push(Block::ToolResult { tool_use_id: id.clone(), content, is_error: !ok });
        }
        out
    }

    fn run_one(&mut self, call_id: &str, name: &str, input: Value) -> (String, bool, String, Option<String>) {
        if input.get(INVALID_INPUT_KEY).is_some() {
            let e = ToolError::invalid("INVALID_JSON: the tool input wasn't valid JSON (it may have been cut off). Send the call again with complete input.");
            return (error_text(&e), false, e.message, None);
        }
        match name {
            "list_toolsets" => {
                let list: Vec<Value> = self
                    .host
                    .toolsets()
                    .iter()
                    .map(|t| json!({"id": t.id(), "title": t.title(), "enabled": self.enabled.contains(t), "tools": self.host.tools(&BTreeSet::from([*t])).len()}))
                    .collect();
                let text = json!({"toolsets": list}).to_string();
                return (text, true, "Listed toolsets".into(), None);
            }
            "enable_toolset" => {
                let id = input.get("toolset").and_then(Value::as_str).unwrap_or_default();
                return match self.host.toolsets().into_iter().find(|t| t.id() == id) {
                    Some(t) => {
                        self.enabled.insert(t);
                        let n = self.host.tools(&BTreeSet::from([t])).len();
                        (json!({"enabled": id, "tools_added": n}).to_string(), true, format!("Enabled {}", t.title()), None)
                    }
                    None => {
                        let e = ToolError::invalid(format!("No toolset {id:?}")).with_remediation("Call list_toolsets for the ids.");
                        (error_text(&e), false, e.message, None)
                    }
                };
            }
            _ => {}
        }
        match self.host.toolset_of(name) {
            None => {
                let e = ToolError::invalid(format!("No tool named {name}"));
                return (error_text(&e), false, e.message, None);
            }
            Some(t) if !self.enabled.contains(&t) => {
                let e = ToolError::invalid(format!("{name} is in the {} toolset, which isn't enabled.", t.id()))
                    .with_remediation(format!("Call enable_toolset with toolset={} first.", t.id()));
                return (error_text(&e), false, e.message, None);
            }
            Some(_) => {}
        }
        let cancel = self.cancel.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let mut r = self.host.call(name, input.clone(), false, cancel.clone());
        if let Err(e) = &r
            && e.code == ErrorCode::ConsentRequired
        {
            r = match self.ask_consent(call_id, name, &e.message) {
                Some(true) => self.host.call(name, input, true, cancel),
                Some(false) => Err(ToolError::new(ErrorCode::ConsentRequired, "The user said no to running this tool.")
                    .with_remediation("Find another way that doesn't run code or change a device, or explain why it's needed.")),
                None => Err(ToolError::cancelled()),
            };
        }
        match r {
            Ok(out) => {
                // A build that ran and failed is still an Ok result: `succeeded` says which.
                if name == BUILD_COMMAND_TOOL
                    && let ToolOutput::Value(v) = &out
                {
                    match v.get("succeeded").and_then(Value::as_bool) {
                        Some(true) => self.failed_builds = 0,
                        Some(false) => self.failed_builds = self.failed_builds.saturating_add(1),
                        None => {}
                    }
                }
                let (text, summary, ev) = compact(&out);
                (text, true, summary, ev)
            }
            Err(e) => (error_text(&e), false, e.message.clone(), None),
        }
    }

    /// Ask the UI and wait (this worker thread only) for the answer; `None` when stopped.
    fn ask_consent(&mut self, call_id: &str, tool: &str, explanation: &str) -> Option<bool> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.waiters.lock().unwrap_or_else(PoisonError::into_inner).insert(call_id.to_string(), tx);
        self.emit(AgentEvent::ConsentNeeded {
            call_id: call_id.to_string(),
            tool: tool.to_string(),
            title: self.host.title(tool),
            explanation: explanation.to_string(),
        });
        self.host.journal("consent", json!({"tool": tool, "call_id": call_id, "state": "asked"}));
        let answer = wait_answer(&rx, &self.stop);
        self.waiters.lock().unwrap_or_else(PoisonError::into_inner).remove(call_id);
        self.host.journal(
            "consent",
            json!({"tool": tool, "call_id": call_id, "state": match answer { Some(true) => "allowed", Some(false) => "declined", None => "stopped" }}),
        );
        answer
    }
}

fn wait_answer(rx: &Receiver<bool>, stop: &Stop) -> Option<bool> {
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(a) => return Some(a),
            Err(RecvTimeoutError::Timeout) if !stop.is_stopped() => {}
            _ => return None,
        }
    }
}

/// A conversation with an API backend: a worker thread runs [`AgentLoop`].
pub struct ApiConversation {
    to_worker: Sender<Option<String>>,
    events: Receiver<AgentEvent>,
    busy: Arc<AtomicBool>,
    stop: Stop,
    cancel: Arc<Mutex<CancelToken>>,
    waiters: Waiters,
    queued: Vec<AgentEvent>,
    resumed: bool,
    failure: Arc<Mutex<Option<TurnFailure>>>,
}

impl ApiConversation {
    pub fn start(host: Arc<dyn ToolHost>, chain: Vec<ModelSlot>, cfg: LoopConfig) -> Result<ApiConversation, ToolError> {
        ApiConversation::start_with(host, chain, cfg, BTreeSet::from([Toolset::Core]))
    }

    /// [`ApiConversation::start`] with these toolsets enabled from the start.
    pub fn start_with(host: Arc<dyn ToolHost>, chain: Vec<ModelSlot>, cfg: LoopConfig, enabled: BTreeSet<Toolset>) -> Result<ApiConversation, ToolError> {
        if chain.is_empty() {
            return Err(ToolError::unavailable("No AI backend could be started.").with_remediation("Add one in Settings → AI."));
        }
        let (etx, erx) = std::sync::mpsc::channel();
        let (wtx, wrx) = std::sync::mpsc::channel::<Option<String>>();
        let busy = Arc::new(AtomicBool::new(false));
        let stop = Stop::default();
        let cancel = Arc::new(Mutex::new(CancelToken::default()));
        let waiters: Waiters = Arc::default();
        let failure: Arc<Mutex<Option<TurnFailure>>> = Arc::default();
        // A kept conversation continues where it stopped, in the mode it ran in.
        let history = cfg.transcript.as_deref().and_then(|p| context::load(p, cfg.deep)).unwrap_or_default();
        let resumed = !history.is_empty();
        let mut agent = AgentLoop {
            host,
            chain,
            current: 0,
            history,
            system: None,
            enabled,
            cfg,
            events: etx.clone(),
            waiters: waiters.clone(),
            stop: stop.clone(),
            cancel: cancel.clone(),
            meter: Meter::default(),
            last_input: 0,
            last_history_estimate: 0,
            failure: failure.clone(),
            failed_builds: 0,
            server_windows: BTreeMap::new(),
        };
        let busy2 = busy.clone();
        std::thread::Builder::new()
            .name("ai-agent".into())
            .spawn(move || {
                while let Ok(text) = wrx.recv() {
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| agent.handle_turn(text.as_deref())));
                    let done = match r {
                        Ok(done) => done,
                        Err(_) => {
                            let _ = etx.send(AgentEvent::Error {
                                message: "The assistant hit a bug and stopped safely.".into(),
                                remediation: Some("Start a new chat.".into()),
                                log: None,
                            });
                            AgentEvent::TurnDone {
                                backend: agent.chain.get(agent.current).map(|slot| slot.name.clone()).unwrap_or_default(),
                                input_tokens: 0,
                                output_tokens: 0,
                                cost_usd: None,
                                cost_is_estimate: false,
                            }
                        }
                    };
                    busy2.store(false, Ordering::SeqCst);
                    let _ = etx.send(done);
                }
            })
            .map_err(|e| ToolError::internal(format!("couldn't start the AI worker: {e}")))?;
        Ok(ApiConversation { to_worker: wtx, events: erx, busy, stop, cancel, waiters, queued: Vec::new(), resumed, failure })
    }

    /// It continues a conversation kept in its transcript file.
    pub fn resumed(&self) -> bool {
        self.resumed
    }

    fn submit(&mut self, text: Option<String>) {
        if self.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        // Reset before enqueueing, so an immediate Stop cannot be erased by the worker.
        self.stop.reset();
        *self.failure.lock().unwrap_or_else(PoisonError::into_inner) = None;
        if self.to_worker.send(text).is_err() {
            self.busy.store(false, Ordering::SeqCst);
            self.queued.push(AgentEvent::Error {
                message: "The assistant isn't running any more.".into(),
                remediation: Some("Start a new chat.".into()),
                log: None,
            });
        }
    }
}

impl Conversation for ApiConversation {
    fn send(&mut self, text: &str) {
        self.submit(Some(text.to_string()));
    }

    fn retry(&mut self, _text: &str) {
        self.submit(None);
    }

    fn last_failure(&self) -> Option<TurnFailure> {
        *self.failure.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn poll(&mut self) -> Vec<AgentEvent> {
        let mut v = std::mem::take(&mut self.queued);
        v.extend(self.events.try_iter());
        v
    }

    fn answer_consent(&mut self, call_id: &str, allow: bool) {
        if let Some(tx) = self.waiters.lock().unwrap_or_else(PoisonError::into_inner).remove(call_id) {
            let _ = tx.send(allow);
        }
    }

    fn stop(&mut self) {
        self.stop.stop();
        self.cancel.lock().unwrap_or_else(PoisonError::into_inner).cancel();
    }

    fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::TurnResult;
    use std::collections::VecDeque;

    /// Paths of boolean sub-schemas in `v`.
    fn boolean_subschemas(v: &Value, path: &str, out: &mut Vec<String>) {
        let Value::Object(o) = v else { return };
        for (k, sub) in o {
            let subs: Vec<(String, &Value)> = match (k.as_str(), sub) {
                ("properties" | "$defs" | "definitions", Value::Object(m)) => m.iter().map(|(n, s)| (format!("{path}/{k}/{n}"), s)).collect(),
                ("anyOf" | "oneOf" | "allOf" | "prefixItems", Value::Array(a)) => a.iter().enumerate().map(|(i, s)| (format!("{path}/{k}/{i}"), s)).collect(),
                ("items" | "not", s) => vec![(format!("{path}/{k}"), s)],
                _ => Vec::new(),
            };
            for (p, s) in subs {
                if s.is_boolean() {
                    out.push(p);
                } else {
                    boolean_subschemas(s, &p, out);
                }
            }
        }
    }

    /// Every place a schema can sit holds an object, as LM Studio requires (#17).
    #[test]
    fn tool_schemas_have_only_object_subschemas() {
        let reg = Registry::with_all_tools();
        let all: BTreeSet<Toolset> = reg.toolsets().into_iter().collect();
        let raw = reg.get("app_call_path_find").map(|s| reg.input_schema(s)).unwrap_or_default();
        assert!(raw.to_string().contains("\"items\":true"), "the case from #17 is still covered");
        let mut bad = Vec::new();
        for t in registry_tools(&reg, &all) {
            boolean_subschemas(&t.schema, &t.name, &mut bad);
        }
        assert!(bad.is_empty(), "boolean sub-schemas left: {bad:?}");
    }

    #[test]
    fn boolean_subschemas_become_objects() {
        let s = clean_schema(json!({
            "type": "object", "additionalProperties": false,
            "properties": {"functions": {"type": "array", "items": true}, "never": false, "pick": {"anyOf": [true, {"items": false}]}}
        }));
        assert_eq!(s["properties"]["functions"]["items"], json!({}));
        assert_eq!(s["properties"]["never"], json!({"not": {}}));
        assert_eq!(s["properties"]["pick"]["anyOf"], json!([{}, {"items": {"not": {}}}]));
        assert_eq!(s["additionalProperties"], json!(false), "left as APIs expect");
        // Deeper than the limit: the rest is left as it is.
        let mut deep = json!(true);
        for _ in 0..(MAX_SCHEMA_DEPTH + 10) {
            deep = json!({"items": deep});
        }
        let mut bad = Vec::new();
        boolean_subschemas(&clean_schema(deep), "", &mut bad);
        assert_eq!(bad.len(), 1);
    }

    /// #417: a builder on a local model gets the Build tools and the question tool, not the whole
    /// Core set; other toolsets stay.
    #[test]
    fn a_local_builder_is_not_offered_the_study_tools() {
        let host = FakeHost { calls: Mutex::default(), journal: Mutex::default() };
        let all = host.tools(&BTreeSet::from([Toolset::Core, Toolset::Runtime]));
        let names = |v: &[ToolDef]| v.iter().map(|t| t.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&all), ["look", "danger"]);
        // `look` is a Core tool that isn't kept; `danger` belongs to another toolset.
        assert_eq!(names(&lean_for_local(all, &host)), ["danger"]);
        let ask = vec![ToolDef { name: "core_person_ask".into(), description: "d".into(), schema: json!({"type":"object"}) }];
        struct CoreHost;
        impl ToolHost for CoreHost {
            fn tools(&self, _: &BTreeSet<Toolset>) -> Vec<ToolDef> {
                Vec::new()
            }
            fn toolset_of(&self, _: &str) -> Option<Toolset> {
                Some(Toolset::Core)
            }
            fn call(&self, _: &str, _: Value, _: bool, _: CancelToken) -> Result<ToolOutput, ToolError> {
                Err(ToolError::internal("unused"))
            }
            fn journal(&self, _: &str, _: Value) {}
            fn investigation(&self) -> String {
                "{}".into()
            }
            fn clean_room(&self) -> bool {
                true
            }
            fn title(&self, id: &str) -> String {
                id.into()
            }
        }
        assert_eq!(names(&lean_for_local(ask, &CoreHost)), ["core_person_ask"], "the question tool stays");
    }

    /// #417, with the real tools: the tool descriptions a local builder is sent in Clean room fell
    /// from 33-41 tools (23,000-30,000 characters) to 13 (about 10,000).
    #[test]
    fn the_real_local_builder_tool_list_is_small_and_can_write_files() {
        let engine = craft_anything_engine::Engine::shared(craft_anything_engine::testing::session(), crate::tools());
        let host = EngineHost(engine);
        let enabled = BTreeSet::from([Toolset::Core, Toolset::Build]);
        let all = host.tools(&enabled);
        let lean = lean_for_local(all.clone(), &host);
        let names: Vec<&str> = lean.iter().map(|t| t.name.as_str()).collect();
        let size = |v: &[ToolDef]| v.iter().map(|t| t.description.len() + t.schema.to_string().len()).sum::<usize>();
        for needed in ["build_file_write", "build_file_edit", "build_file_read", "build_files_list", "build_command_run", "core_person_ask", "rebuild_check"] {
            assert!(names.contains(&needed), "{needed} in {names:?}");
        }
        assert!(!names.contains(&"core_journal_read") && !names.contains(&"core_session_show"), "{names:?}");
        assert!(lean.len() <= 20 && size(&lean) * 2 < size(&all), "{} tools, {} of {} characters", lean.len(), size(&lean), size(&all));
    }

    /// A scripted model: returns canned turns and records requests.
    pub struct Scripted {
        pub turns: VecDeque<Result<TurnResult, ToolError>>,
        pub seen: Arc<Mutex<Vec<Vec<Msg>>>>,
    }

    impl ChatModel for Scripted {
        fn turn(&mut self, req: &TurnRequest<'_>, on_delta: &mut dyn FnMut(Delta), _stop: &Stop) -> Result<TurnResult, ToolError> {
            self.seen.lock().unwrap().push(req.messages.to_vec());
            let r = self.turns.pop_front().unwrap_or_else(|| Err(ToolError::internal("script ended")));
            if let Ok(t) = &r {
                let text = t.message.text();
                if !text.is_empty() {
                    on_delta(Delta::Text(text));
                }
            }
            r
        }
        fn model(&self) -> String {
            "scripted".into()
        }
    }

    pub fn turn(blocks: Vec<Block>, stop: StopReason) -> Result<TurnResult, ToolError> {
        Ok(TurnResult {
            message: Msg { role: Speaker::Assistant, blocks },
            stop,
            usage: Usage { input_tokens: 10, output_tokens: 5, ..Default::default() },
            model: "claude-opus-5-5".into(),
            follow_up: None,
        })
    }

    /// A host whose `danger` tool needs consent.
    struct FakeHost {
        calls: Mutex<Vec<(String, bool)>>,
        journal: Mutex<Vec<(String, Value)>>,
    }

    impl ToolHost for FakeHost {
        fn tools(&self, enabled: &BTreeSet<Toolset>) -> Vec<ToolDef> {
            let mut v = Vec::new();
            if enabled.contains(&Toolset::Core) {
                v.push(ToolDef { name: "look".into(), description: "d".into(), schema: json!({"type":"object"}) });
            }
            if enabled.contains(&Toolset::Runtime) {
                v.push(ToolDef { name: "danger".into(), description: "d".into(), schema: json!({"type":"object"}) });
            }
            v
        }
        fn toolset_of(&self, id: &str) -> Option<Toolset> {
            match id {
                "look" => Some(Toolset::Core),
                "build_command_run" => Some(Toolset::Core),
                "danger" => Some(Toolset::Runtime),
                _ => None,
            }
        }
        fn call(&self, id: &str, input: Value, consent: bool, _c: CancelToken) -> Result<ToolOutput, ToolError> {
            self.calls.lock().unwrap().push((id.to_string(), consent));
            if id == "danger" && !consent {
                return Err(ToolError::new(ErrorCode::ConsentRequired, "needs your OK"));
            }
            if id == "build_command_run" {
                return Ok(ToolOutput::Value(
                    json!({"command": "cargo build", "exit_code": 0, "succeeded": input.get("ok").and_then(Value::as_bool) == Some(true)}),
                ));
            }
            if id == "look" && input.get("large").and_then(Value::as_bool) == Some(true) {
                return Ok(ToolOutput::Value(json!({"content": "n".repeat(8000)})));
            }
            Ok(ToolOutput::Value(json!({"ran": id})))
        }
        fn journal(&self, kind: &str, entry: Value) {
            self.journal.lock().unwrap().push((kind.to_string(), entry));
        }
        fn investigation(&self) -> String {
            "{}".into()
        }
        fn clean_room(&self) -> bool {
            true
        }
        fn title(&self, id: &str) -> String {
            id.into()
        }
    }

    fn drain_until_done(c: &mut ApiConversation, mut on: impl FnMut(&AgentEvent, &mut ApiConversation)) -> Vec<AgentEvent> {
        let mut all = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            for e in c.poll() {
                on(&e, c);
                let done = matches!(e, AgentEvent::TurnDone { .. });
                all.push(e);
                if done {
                    return all;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("timed out: {all:?}");
    }

    fn api_stop_fixture(partial_event: bool) {
        use std::io::{BufRead, BufReader, Read, Write};
        for anthropic in [false, true] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let (ready, waiting) = std::sync::mpsc::channel();
            let (closed, disconnected) = std::sync::mpsc::channel();
            let server = std::thread::spawn(move || {
                for turn in 0..2 {
                    let (mut socket, _) = listener.accept().unwrap();
                    socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                    let mut request = BufReader::new(&mut socket);
                    let mut length = 0;
                    loop {
                        let mut line = String::new();
                        assert!(request.read_line(&mut line).unwrap() > 0);
                        if line == "\r\n" {
                            break;
                        }
                        if let Some((name, value)) = line.split_once(':')
                            && name.eq_ignore_ascii_case("content-length")
                        {
                            length = value.trim().parse::<usize>().unwrap();
                        }
                    }
                    request.read_exact(&mut vec![0; length]).unwrap();
                    drop(request);
                    if turn == 0 {
                        if partial_event {
                            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {").unwrap();
                            socket.flush().unwrap();
                        }
                        ready.send(()).unwrap();
                        // A cancelled turn closes its real HTTP connection, rather than just
                        // hiding the spinner while a request carries on in another thread.
                        assert_eq!(socket.read(&mut [0; 1]).unwrap(), 0);
                        closed.send(()).unwrap();
                    } else {
                        let body = if anthropic {
                            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"model\":\"fixture\",\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"done\"}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
                        } else {
                            "data: {\"choices\":[{\"delta\":{\"content\":\"done\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"
                        };
                        write!(
                            socket,
                            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .unwrap();
                    }
                }
            });
            let model: Box<dyn ChatModel> = if anthropic {
                Box::new(crate::anthropic::Anthropic::new(Some(&base), "sk-test".into(), "fixture"))
            } else {
                Box::new(crate::openai::OpenAi::compatible("Fixture", &base, None, "fixture"))
            };
            let slot = ModelSlot { name: "Fixture".into(), model, local: true, effort: None, max_turns: 2, context_tokens: 0 };
            let host = Arc::new(FakeHost { calls: Mutex::default(), journal: Mutex::default() });
            let mut conversation = ApiConversation::start(host, vec![slot], LoopConfig::default()).unwrap();
            conversation.send("a request whose server never finishes");
            waiting.recv_timeout(Duration::from_secs(5)).unwrap();
            let started = std::time::Instant::now();
            conversation.stop();
            conversation.stop();
            let events = drain_until_done(&mut conversation, |event, _| assert!(!matches!(event, AgentEvent::Error { .. }), "{event:?}"));
            assert!(started.elapsed() < Duration::from_secs(1), "Stop must interrupt a silent API connection");
            assert!(!conversation.is_busy());
            assert!(events.iter().any(|event| matches!(event, AgentEvent::Status { text } if text == "Stopped.")));
            disconnected.recv_timeout(Duration::from_secs(1)).unwrap();
            conversation.send("next message");
            drain_until_done(&mut conversation, |event, _| assert!(!matches!(event, AgentEvent::Error { .. }), "{event:?}"));
            assert!(!conversation.is_busy());
            server.join().unwrap();
        }
    }

    #[test]
    fn stop_api_waiting_for_first_response_byte() {
        api_stop_fixture(false);
    }

    #[test]
    fn stop_api_waiting_in_a_partial_sse_event() {
        api_stop_fixture(true);
    }

    #[test]
    fn stop_before_api_worker_starts_is_preserved() {
        let host = Arc::new(FakeHost { calls: Mutex::default(), journal: Mutex::default() });
        let seen = Arc::new(Mutex::default());
        let script = Scripted { turns: VecDeque::from([turn(vec![Block::Text { text: "next answer".into() }], StopReason::EndTurn)]), seen: seen.clone() };
        let (events, _received) = std::sync::mpsc::channel();
        let mut agent = AgentLoop {
            host,
            chain: vec![ModelSlot { name: "Fixture".into(), model: Box::new(script), local: true, effort: None, max_turns: 2, context_tokens: 0 }],
            current: 0,
            history: Vec::new(),
            system: None,
            enabled: BTreeSet::from([Toolset::Core]),
            cfg: LoopConfig::default(),
            events,
            waiters: Arc::default(),
            stop: Stop::default(),
            cancel: Arc::new(Mutex::new(CancelToken::default())),
            meter: Meter::default(),
            last_input: 0,
            last_history_estimate: 0,
            failure: Arc::default(),
            failed_builds: 0,
            server_windows: BTreeMap::new(),
        };
        agent.stop.stop();
        let done = agent.handle_turn(Some("cancel before dequeue"));
        assert!(matches!(done, AgentEvent::TurnDone { .. }));
        assert!(seen.lock().unwrap().is_empty(), "a worker cannot revive a cancelled request");
        agent.handle("next message");
        assert_eq!(seen.lock().unwrap().len(), 1, "a new turn resets Stop");
    }

    #[test]
    fn consent_toolsets_and_one_result_message() {
        let host = Arc::new(FakeHost { calls: Mutex::default(), journal: Mutex::default() });
        let seen = Arc::new(Mutex::new(Vec::new()));
        let script = Scripted {
            turns: VecDeque::from([
                turn(vec![Block::ToolUse { id: "a".into(), name: "danger".into(), input: json!({}) }], StopReason::ToolUse),
                turn(
                    vec![
                        Block::ToolUse { id: "b".into(), name: "enable_toolset".into(), input: json!({"toolset": "runtime"}) },
                        Block::ToolUse { id: "c".into(), name: "look".into(), input: json!({}) },
                        Block::ToolUse { id: "d".into(), name: "look".into(), input: json!({"__invalid_json_input": "{"}) },
                    ],
                    StopReason::ToolUse,
                ),
                turn(vec![Block::ToolUse { id: "e".into(), name: "danger".into(), input: json!({}) }], StopReason::ToolUse),
                turn(vec![Block::ToolUse { id: "f".into(), name: "danger".into(), input: json!({}) }], StopReason::ToolUse),
                turn(vec![Block::Text { text: "done".into() }], StopReason::EndTurn),
            ]),
            seen: seen.clone(),
        };
        let slot = ModelSlot { name: "Fake".into(), model: Box::new(script), local: false, effort: None, max_turns: 10, context_tokens: 0 };
        let mut c = ApiConversation::start(host.clone(), vec![slot], LoopConfig { beginner: true, ..LoopConfig::default() }).unwrap();
        c.send("How does it work?");
        assert!(c.is_busy());
        let mut asked = 0;
        let events = drain_until_done(&mut c, |e, c| {
            if let AgentEvent::ConsentNeeded { call_id, .. } = e {
                asked += 1;
                c.answer_consent(call_id, call_id == "e");
            }
        });
        assert_eq!(asked, 2);
        let seen = seen.lock().unwrap();
        // turn 1: danger not enabled → error, no consent asked
        let r1 = seen[1].last().unwrap();
        assert!(matches!(&r1.blocks[0], Block::ToolResult { is_error: true, content, .. } if content.contains("enable_toolset")));
        // turn 2: three results in ONE user message
        let r2 = seen[2].last().unwrap();
        assert_eq!(r2.role, Speaker::User);
        assert_eq!(r2.blocks.len(), 3);
        assert!(matches!(&r2.blocks[2], Block::ToolResult { is_error: true, content, .. } if content.contains("INVALID_JSON")));
        // turn 3 allowed, turn 4 declined
        assert!(matches!(&seen[3].last().unwrap().blocks[0], Block::ToolResult { is_error: false, .. }));
        assert!(matches!(&seen[4].last().unwrap().blocks[0], Block::ToolResult { is_error: true, content, .. } if content.contains("said no")));
        let calls = host.calls.lock().unwrap().clone();
        assert!(calls.contains(&("danger".into(), true)));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::TextDelta { text } if text == "done")));
        let j = host.journal.lock().unwrap().clone();
        for kind in ["user_message", "model_turn", "consent"] {
            assert!(j.iter().any(|(k, _)| k == kind));
        }
        assert!(matches!(events.last(), Some(AgentEvent::TurnDone { input_tokens: 50, .. })));
        // cost known for the priced model id
        assert!(matches!(events.last(), Some(AgentEvent::TurnDone { cost_usd: Some(_), .. })));
    }

    /// #399: a model that never gets its code to compile kept going for 400 steps and 9.4M tokens.
    /// A step stops after [`MAX_FAILED_BUILDS`] failed build commands in a row, says why, and keeps
    /// what was done; a build that succeeds starts the count again.
    #[test]
    fn a_step_stops_after_many_failed_builds_in_a_row() {
        let build = |ok: bool, n: usize| {
            turn(vec![Block::ToolUse { id: format!("b{n}"), name: BUILD_COMMAND_TOOL.into(), input: json!({"ok": ok}) }], StopReason::ToolUse)
        };
        let run = |turns: Vec<Result<TurnResult, ToolError>>| {
            let host = Arc::new(FakeHost { calls: Mutex::default(), journal: Mutex::default() });
            let script = Scripted { turns: VecDeque::from(turns), seen: Arc::default() };
            let slot = ModelSlot { name: "Fake".into(), model: Box::new(script), local: false, effort: None, max_turns: 400, context_tokens: 0 };
            let mut c = ApiConversation::start(host, vec![slot], LoopConfig { beginner: true, ..LoopConfig::default() }).unwrap();
            c.send("build it");
            drain_until_done(&mut c, |_, _| {})
        };
        let n = MAX_FAILED_BUILDS as usize;
        // All failing: stopped at the limit with a reason, not by running out of script.
        let events = run((0..n + 5).map(|i| build(false, i)).collect());
        let stopped = events.iter().find_map(|e| match e {
            AgentEvent::Error { message, .. } => Some(message.clone()),
            _ => None,
        });
        assert!(stopped.as_deref().is_some_and(|m| m.contains(&format!("failed {n} times in a row"))), "{stopped:?}");
        assert_eq!(events.iter().filter(|e| matches!(e, AgentEvent::ToolFinished { .. })).count(), n, "no more builds after the limit");
        // A success in between starts the count again: the same number of failures, split, goes on.
        let mut turns: Vec<_> = (0..n - 1).map(|i| build(false, i)).collect();
        turns.push(build(true, 99));
        turns.extend((0..n - 1).map(|i| build(false, 100 + i)));
        turns.push(turn(vec![Block::Text { text: "done".into() }], StopReason::EndTurn));
        let events = run(turns);
        assert!(!events.iter().any(|e| matches!(e, AgentEvent::Error { .. })), "{events:?}");
        assert!(events.iter().any(|e| matches!(e, AgentEvent::TextDelta { text } if text == "done")));
    }

    #[test]
    fn the_build_tool_the_limit_counts_exists() {
        let e = craft_anything_engine::Engine::new(craft_anything_engine::testing::session());
        assert!(e.registry().get(BUILD_COMMAND_TOOL).is_some_and(|s| !s.effects.is_empty()));
    }

    /// A long conversation is summarised, saved after every message, and resumed in its mode.
    #[test]
    fn a_long_conversation_is_summarised_kept_and_resumed() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("transcript.json");
        let host = Arc::new(FakeHost { calls: Mutex::default(), journal: Mutex::default() });
        let seen = Arc::new(Mutex::new(Vec::new()));
        let look = |id: &str| turn(vec![Block::ToolUse { id: id.into(), name: "look".into(), input: json!({}) }], StopReason::ToolUse);
        let mut summary = turn(vec![Block::Text { text: "Made index.html; the menu is left.".into() }], StopReason::EndTurn).unwrap();
        summary.usage = Usage { input_tokens: 30, output_tokens: 4, cache_read_tokens: 7, cache_write_tokens: 3 };
        let script = Scripted {
            turns: VecDeque::from([look("a"), look("b"), Ok(summary), turn(vec![Block::Text { text: "done".into() }], StopReason::EndTurn)]),
            seen: seen.clone(),
        };
        let slot = ModelSlot { name: "Fake".into(), model: Box::new(script), local: false, effort: None, max_turns: 10, context_tokens: 0 };
        // Ordinary turns report 10 input tokens, over this budget, so room is made before the third.
        let cfg = || LoopConfig {
            transcript: Some(path.clone()),
            context_tokens: 5,
            prices: BTreeMap::from([(
                "claude-opus-5-5".into(),
                crate::config::Price { input_per_mtok: 1.0, output_per_mtok: 2.0, cache_read_per_mtok: Some(0.25), cache_write_per_mtok: Some(0.5) },
            )]),
            ..LoopConfig::default()
        };
        let mut c = ApiConversation::start(host.clone(), vec![slot], cfg()).unwrap();
        assert!(!c.resumed());
        c.send("Build it");
        let events = drain_until_done(&mut c, |_, _| {});
        assert!(events.iter().any(|e| matches!(e, AgentEvent::TextDelta { text } if text == "done")));
        {
            let seen = seen.lock().unwrap();
            assert_eq!(seen.len(), 4, "two steps, the summary, the last step");
            assert!(seen[2].last().unwrap().text().contains("summary of the work so far"), "the summary request");
            assert!(seen[3][0].text().contains("Made index.html"), "the summary replaced the oldest part");
            assert!(seen[3].len() < seen[1].len() + 2);
        }
        assert!(
            // Cached tokens count as input (R5.4: Ollama 0.40 reports them, and usage.json left them out).
            matches!(events.last(), Some(AgentEvent::TurnDone { input_tokens: 70, output_tokens: 19, cost_usd: Some(cost), .. }) if (cost - 0.00010125).abs() < 1e-12),
            "{events:?}"
        );
        {
            let journal = host.journal.lock().unwrap();
            assert!(journal.iter().any(|(kind, _)| kind == "context"));
            let turns: Vec<_> = journal.iter().filter(|(kind, _)| kind == "model_turn").map(|(_, entry)| entry).collect();
            assert_eq!(turns.len(), 4, "all model calls, including the summary, are journaled");
            let summary = turns.iter().find(|entry| entry["purpose"] == "context_summary").unwrap();
            assert_eq!(summary["input_tokens"], 30);
            assert_eq!(summary["output_tokens"], 4);
            assert_eq!(summary["cache_read_tokens"], 7);
            assert_eq!(summary["cache_write_tokens"], 3);
            assert_eq!(summary["effort"], "low");
            assert!((turns.iter().map(|entry| entry["cost_usd"].as_f64().unwrap()).sum::<f64>() - 0.00010125).abs() < 1e-12);
        }
        // A new conversation with the same file continues it.
        let seen2 = Arc::new(Mutex::new(Vec::new()));
        let script = Scripted { turns: VecDeque::from([turn(vec![Block::Text { text: "blue now".into() }], StopReason::EndTurn)]), seen: seen2.clone() };
        let slot = ModelSlot { name: "Fake".into(), model: Box::new(script), local: false, effort: None, max_turns: 10, context_tokens: 0 };
        let mut c = ApiConversation::start(host, vec![slot], LoopConfig { transcript: Some(path.clone()), deep: true, ..LoopConfig::default() }).unwrap();
        assert!(c.resumed());
        c.send("Make it blue");
        drain_until_done(&mut c, |_, _| {});
        let seen2 = seen2.lock().unwrap();
        assert!(seen2[0][0].text().contains("Made index.html") && seen2[0].last().unwrap().text().contains("Make it blue"));
        assert!(crate::context::load(&path, true).unwrap().last().unwrap().text().contains("blue now"), "kept after every message");
    }

    #[test]
    fn summary_accounting_keeps_unknown_prices_and_failed_calls_distinct() {
        for (local, failed) in [(false, false), (true, false), (false, true)] {
            let host = Arc::new(FakeHost { calls: Mutex::default(), journal: Mutex::default() });
            let seen = Arc::new(Mutex::new(Vec::new()));
            let look = |id: &str| turn(vec![Block::ToolUse { id: id.into(), name: "look".into(), input: json!({}) }], StopReason::ToolUse);
            let summary = if failed {
                Err(ToolError::internal("summary unavailable"))
            } else {
                let mut result = turn(vec![Block::Text { text: String::new() }], StopReason::EndTurn).unwrap();
                result.model = "unpriced-fixture-model".into();
                Ok(result)
            };
            let script = Scripted {
                turns: VecDeque::from([look("a"), look("b"), summary, turn(vec![Block::Text { text: "done".into() }], StopReason::EndTurn)]),
                seen: seen.clone(),
            };
            let slot = ModelSlot { name: "Fake".into(), model: Box::new(script), local, effort: None, max_turns: 10, context_tokens: 0 };
            let mut c = ApiConversation::start(host.clone(), vec![slot], LoopConfig { context_tokens: 5, ..LoopConfig::default() }).unwrap();
            c.send("Build it");
            let events = drain_until_done(&mut c, |_, _| {});
            assert_eq!(seen.lock().unwrap().len(), 4, "the summary was attempted");
            let Some(AgentEvent::TurnDone { input_tokens, output_tokens, cost_usd, .. }) = events.last() else { panic!("{events:?}") };
            assert_eq!((*input_tokens, *output_tokens), if failed { (30, 15) } else { (40, 20) });
            if local {
                assert_eq!(*cost_usd, Some(0.0));
            } else if failed {
                assert!(cost_usd.is_some(), "a failed call supplies no usage or unknown price");
            } else {
                assert_eq!(*cost_usd, None, "the successful summary's unknown price makes the total unknown");
            }
            let journal = host.journal.lock().unwrap();
            assert_eq!(journal.iter().filter(|(kind, _)| kind == "model_turn").count(), if failed { 3 } else { 4 });
        }
    }

    /// A huge backend budget can trim a kept conversation without overflow.
    #[test]
    fn a_large_backend_budget_can_trim_without_overflow() {
        let budget = i64::MAX as u64;
        let dir = tempfile::tempdir().unwrap();
        let transcript = dir.path().join("transcript.json");
        let host = Arc::new(FakeHost { calls: Mutex::default(), journal: Mutex::default() });
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut history = vec![
            Msg::user_text("Build it"),
            Msg { role: Speaker::Assistant, blocks: vec![Block::ToolUse { id: "old".into(), name: "look".into(), input: json!({}) }] },
            Msg { role: Speaker::User, blocks: vec![Block::ToolResult { tool_use_id: "old".into(), content: "x".repeat(1000), is_error: false }] },
        ];
        for _ in 0..context::KEEP_RECENT / 2 {
            history.push(Msg { role: Speaker::Assistant, blocks: vec![Block::Text { text: "checked".into() }] });
            history.push(Msg::user_text("continue"));
        }
        context::save(&transcript, &history, true).unwrap();
        let mut first = turn(vec![Block::ToolUse { id: "new".into(), name: "look".into(), input: json!({}) }], StopReason::ToolUse).unwrap();
        first.usage.input_tokens = budget + 1;
        let script = Scripted { turns: VecDeque::from([Ok(first), turn(vec![Block::Text { text: "done".into() }], StopReason::EndTurn)]), seen: seen.clone() };
        let slot = ModelSlot { name: "Large".into(), model: Box::new(script), local: false, effort: None, max_turns: 5, context_tokens: budget };
        let mut conversation =
            ApiConversation::start(host.clone(), vec![slot], LoopConfig { transcript: Some(transcript), deep: true, ..Default::default() }).unwrap();
        assert!(conversation.resumed());
        conversation.send("continue");
        let events = drain_until_done(&mut conversation, |event, _| {
            assert!(!matches!(event, AgentEvent::Error { .. }), "{event:?}");
        });
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "trimming made enough room without a summary call");
        assert!(matches!(&seen[1][2].blocks[0], Block::ToolResult { tool_use_id, content, .. } if tool_use_id == "old" && content.len() < 1000));
        assert!(events.iter().any(|event| matches!(event, AgentEvent::TextDelta { text } if text == "done")), "{events:?}");
        let journal = host.journal.lock().unwrap();
        assert!(journal.iter().any(|(kind, entry)| kind == "context" && entry["budget"] == budget && entry["blocks"] == 1));
    }

    /// #731: a local server that reports no usage left the system prompt and tool schemas out of the
    /// count, so a conversation past its budget was never trimmed. They are counted from their text
    /// now; with nothing large in them, nothing is trimmed.
    #[test]
    fn an_unreported_turn_counts_the_system_prompt_and_tools() {
        let host = Arc::new(FakeHost { calls: Mutex::default(), journal: Mutex::default() });
        let script = Scripted { turns: VecDeque::new(), seen: Arc::new(Mutex::new(Vec::new())) };
        let (events, _) = std::sync::mpsc::channel();
        let mut agent = AgentLoop {
            host: host.clone(),
            chain: vec![ModelSlot { name: "Local".into(), model: Box::new(script), local: true, effort: None, max_turns: 5, context_tokens: 1000 }],
            current: 0,
            history: vec![Msg::user_text("Build it")],
            system: None,
            enabled: BTreeSet::from([Toolset::Core]),
            cfg: LoopConfig::default(),
            events,
            waiters: Arc::default(),
            stop: Stop::default(),
            cancel: Arc::new(Mutex::new(CancelToken::default())),
            meter: Meter::default(),
            last_input: 0,
            last_history_estimate: 0,
            failure: Arc::default(),
            failed_builds: 0,
            server_windows: BTreeMap::new(),
        };
        let tools = [ToolDef { name: "look".into(), description: "x".repeat(4000), schema: json!({"type": "object"}) }];

        // A window the server reported (#962) replaces the local guess but not a smaller set budget.
        agent.server_windows.insert(0, 65_536);
        assert_eq!(agent.context_budget(), 1000, "a budget that was set stays the most");
        agent.chain[0].context_tokens = 0;
        assert_eq!(agent.context_budget(), 49_152, "three quarters of the reported window");
        agent.server_windows.clear();
        assert_eq!(agent.context_budget(), context::LOCAL_CONTEXT_TOKENS);
        agent.chain[0].context_tokens = 1000;

        assert!(agent.fit_context("A short system prompt.", &[]).is_none());
        assert!(host.journal.lock().unwrap().is_empty(), "a small prompt is not judged");

        agent.fit_context(&"s".repeat(4000), &tools);
        let journal = host.journal.lock().unwrap();
        let (kind, entry) = journal.first().expect("the oversized prompt is judged");
        assert_eq!(kind, "context");
        assert_eq!(entry["budget"], 1000);
        assert!(entry["tokens_before"].as_u64().is_some_and(|n| n > 2000), "{entry}");
    }

    #[test]
    fn backend_context_budgets_and_conversation_overrides() {
        for (backend_budget, override_budget, local, summarises) in [
            (5, 0, false, true),
            (5, 0, true, true),
            (0, 5, false, true),
            (100, 5, false, true),
            (5, 100, false, false),
            (0, 0, false, false),
            (0, 0, true, false),
        ] {
            let host = Arc::new(FakeHost { calls: Mutex::default(), journal: Mutex::default() });
            let seen = Arc::new(Mutex::new(Vec::new()));
            let look = |id: &str| turn(vec![Block::ToolUse { id: id.into(), name: "look".into(), input: json!({}) }], StopReason::ToolUse);
            let mut turns = VecDeque::from([look("a"), look("b")]);
            if summarises {
                turns.push_back(turn(vec![Block::Text { text: "Earlier work kept.".into() }], StopReason::EndTurn));
            }
            turns.push_back(turn(vec![Block::Text { text: "done".into() }], StopReason::EndTurn));
            let slot = ModelSlot {
                name: "Configured".into(),
                model: Box::new(Scripted { turns, seen: seen.clone() }),
                local,
                effort: None,
                max_turns: 5,
                context_tokens: backend_budget,
            };
            let mut conversation = ApiConversation::start(host, vec![slot], LoopConfig { context_tokens: override_budget, ..Default::default() }).unwrap();
            conversation.send("Build it");
            let events = drain_until_done(&mut conversation, |_, _| {});
            let seen = seen.lock().unwrap();
            assert_eq!(seen.len(), if summarises { 4 } else { 3 }, "backend {backend_budget}, override {override_budget}, local {local}");
            if summarises {
                assert!(seen[2].last().unwrap().text().contains("summary of the work so far"));
                assert!(seen[3][0].text().contains("Earlier work kept."));
            }
            assert!(events.iter().any(|event| matches!(event, AgentEvent::TextDelta { text } if text == "done")), "{events:?}");
            assert!(matches!(events.last(), Some(AgentEvent::TurnDone { input_tokens, .. }) if *input_tokens == if summarises { 40 } else { 30 }));
        }
    }

    #[test]
    fn fallback_refits_context_to_its_own_budget_and_accounts_for_the_summary() {
        let host = Arc::new(FakeHost { calls: Mutex::default(), journal: Mutex::default() });
        let seen = Arc::new(Mutex::new(Vec::new()));
        let look = |id: &str| turn(vec![Block::ToolUse { id: id.into(), name: "look".into(), input: json!({}) }], StopReason::ToolUse);
        let first = Scripted {
            turns: VecDeque::from([look("a"), look("b"), Err(ToolError::new(ErrorCode::EngineMissing, "temporarily unavailable"))]),
            seen: Arc::default(),
        };
        let fallback = Scripted {
            turns: VecDeque::from([
                turn(vec![Block::Text { text: "Earlier work kept.".into() }], StopReason::EndTurn),
                turn(vec![Block::Text { text: "done".into() }], StopReason::EndTurn),
            ]),
            seen: seen.clone(),
        };
        let chain = vec![
            ModelSlot { name: "Large".into(), model: Box::new(first), local: false, effort: None, max_turns: 5, context_tokens: 100 },
            ModelSlot { name: "Small".into(), model: Box::new(fallback), local: false, effort: None, max_turns: 5, context_tokens: 5 },
        ];
        let mut conversation = ApiConversation::start(host.clone(), chain, LoopConfig::default()).unwrap();
        conversation.send("Build it");
        let events = drain_until_done(&mut conversation, |_, _| {});
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "the fallback summarises before answering");
        assert!(seen[0].last().unwrap().text().contains("summary of the work so far"));
        assert!(seen[1][0].text().contains("Earlier work kept."));
        assert!(events.iter().any(|event| matches!(event, AgentEvent::TextDelta { text } if text == "done")), "{events:?}");
        assert!(matches!(events.last(), Some(AgentEvent::TurnDone { input_tokens: 40, output_tokens: 20, backend, .. }) if backend == "Small"));
        let journal = host.journal.lock().unwrap();
        assert!(journal.iter().any(|(kind, entry)| kind == "context" && entry["budget"] == 5));
        assert!(journal.iter().any(|(kind, entry)| kind == "model_turn" && entry["purpose"] == "context_summary" && entry["backend"] == "Small"));
    }

    /// A fallback fits its own budget, counting new tool results since the last request.
    #[test]
    fn fallback_budget_counts_new_results_since_the_last_request() {
        let last_input = 10_000;
        let budget = 11_000;
        let dir = tempfile::tempdir().unwrap();
        let transcript = dir.path().join("transcript.json");
        let host = Arc::new(FakeHost { calls: Mutex::default(), journal: Mutex::default() });
        let mut history = vec![
            Msg::user_text("Build it"),
            Msg { role: Speaker::Assistant, blocks: vec![Block::ToolUse { id: "old".into(), name: "look".into(), input: json!({}) }] },
            Msg { role: Speaker::User, blocks: vec![Block::ToolResult { tool_use_id: "old".into(), content: "x".repeat(8000), is_error: false }] },
        ];
        for _ in 0..context::KEEP_RECENT / 2 {
            history.push(Msg { role: Speaker::Assistant, blocks: vec![Block::Text { text: "checked".into() }] });
            history.push(Msg::user_text("continue"));
        }
        context::save(&transcript, &history, true).unwrap();
        let mut first = turn(vec![Block::ToolUse { id: "new".into(), name: "look".into(), input: json!({"large": true}) }], StopReason::ToolUse).unwrap();
        first.usage.input_tokens = last_input;
        let primary_seen = Arc::new(Mutex::new(Vec::new()));
        let primary = Scripted {
            turns: VecDeque::from([Ok(first), Err(ToolError::new(ErrorCode::EngineMissing, "temporarily unavailable"))]),
            seen: primary_seen.clone(),
        };
        let answer = |text: &str| {
            let mut result = turn(vec![Block::Text { text: text.into() }], StopReason::EndTurn).unwrap();
            result.usage.input_tokens = last_input;
            Ok(result)
        };
        let fallback_seen = Arc::new(Mutex::new(Vec::new()));
        let fallback = Scripted { turns: VecDeque::from([answer("done"), answer("next done")]), seen: fallback_seen.clone() };
        let chain = vec![
            ModelSlot { name: "Large".into(), model: Box::new(primary), local: false, effort: None, max_turns: 5, context_tokens: 30_000 },
            ModelSlot { name: "Small".into(), model: Box::new(fallback), local: false, effort: None, max_turns: 5, context_tokens: budget },
        ];
        let mut conversation =
            ApiConversation::start(host.clone(), chain, LoopConfig { transcript: Some(transcript), deep: true, ..Default::default() }).unwrap();
        conversation.send("continue");
        let events = drain_until_done(&mut conversation, |event, _| assert!(!matches!(event, AgentEvent::Error { .. }), "{event:?}"));
        assert!(events.iter().any(|event| matches!(event, AgentEvent::TextDelta { text } if text == "done")));
        {
            let seen = primary_seen.lock().unwrap();
            assert_eq!(seen.len(), 2);
            let before = context::estimate_tokens(&seen[0]);
            let grown = context::estimate_tokens(&seen[1]);
            assert!(grown < last_input && last_input < budget, "max(last_input, estimate) would still miss the growth");
            assert!(grown - before > budget - last_input, "the new result crosses the remaining budget");
        }
        let result_len = |history: &[Msg], id: &str| {
            history
                .iter()
                .flat_map(|message| &message.blocks)
                .find_map(|block| match block {
                    Block::ToolResult { tool_use_id, content, .. } if tool_use_id == id => Some(content.len()),
                    _ => None,
                })
                .unwrap()
        };
        {
            let seen = fallback_seen.lock().unwrap();
            assert_eq!(seen.len(), 1, "trimming makes room without a summary request");
            assert!(result_len(&seen[0], "old") < 8000, "the fallback must trim after new tool output grows the history");
            assert!(result_len(&seen[0], "new") >= 8000, "the latest tool result stays complete");
        }
        // The successful fallback records the history it actually received. A short follow-up
        // must not count the already-sent tool result as new growth and summarise it again.
        conversation.send("again");
        let events = drain_until_done(&mut conversation, |event, _| assert!(!matches!(event, AgentEvent::Error { .. }), "{event:?}"));
        assert!(events.iter().any(|event| matches!(event, AgentEvent::TextDelta { text } if text == "next done")));
        assert_eq!(fallback_seen.lock().unwrap().len(), 2);
        let journal = host.journal.lock().unwrap();
        let trims: Vec<_> = journal.iter().filter(|(kind, entry)| kind == "context" && entry["action"] == "trim").collect();
        assert_eq!(trims.len(), 1, "the short follow-up does not compact the same growth twice");
        assert_eq!(trims[0].1["budget"], budget);
        assert!(trims[0].1["tokens_before"].as_u64().unwrap() > budget);
    }

    /// A request refused for the server's window is fitted and asked once more; a second refusal
    /// stops with the way to raise the window.
    #[test]
    fn a_request_too_long_for_the_servers_window_is_fitted_and_asked_once_more() {
        // #962: Ollama on its 64k default refused a 70k request; our 24k-local guess counts
        // characters, so the server's own count is what the next fit is measured with.
        let overflow = || {
            let body = json!({"error": {"message": "request (69944 tokens) exceeds the available context size (65536 tokens), try increasing it"}});
            Err(crate::http::status_error("Ollama (local)", 400, &body.to_string()))
        };
        let answer = || turn(vec![Block::Text { text: "done".into() }], StopReason::EndTurn);
        for (turns, fits) in [(vec![overflow(), answer()], true), (vec![overflow(), overflow(), answer()], false)] {
            let dir = tempfile::tempdir().unwrap();
            let transcript = dir.path().join("transcript.json");
            let mut history = vec![
                Msg::user_text("Build it"),
                Msg { role: Speaker::Assistant, blocks: vec![Block::ToolUse { id: "old".into(), name: "look".into(), input: json!({}) }] },
                Msg { role: Speaker::User, blocks: vec![Block::ToolResult { tool_use_id: "old".into(), content: "x".repeat(8000), is_error: false }] },
            ];
            for _ in 0..context::KEEP_RECENT / 2 {
                history.push(Msg { role: Speaker::Assistant, blocks: vec![Block::Text { text: "checked".into() }] });
                history.push(Msg::user_text("continue"));
            }
            context::save(&transcript, &history, true).unwrap();
            let host = Arc::new(FakeHost { calls: Mutex::default(), journal: Mutex::default() });
            let seen = Arc::new(Mutex::new(Vec::new()));
            let model = Scripted { turns: VecDeque::from(turns), seen: seen.clone() };
            let chain = vec![ModelSlot { name: "Ollama (local)".into(), model: Box::new(model), local: true, effort: None, max_turns: 5, context_tokens: 0 }];
            let mut conversation =
                ApiConversation::start(host.clone(), chain, LoopConfig { transcript: Some(transcript), deep: true, ..Default::default() }).unwrap();
            conversation.send("continue");
            let events = drain_until_done(&mut conversation, |_, _| {});
            let seen = seen.lock().unwrap();
            assert_eq!(seen.len(), 2, "asked once more, never again (fits={fits})");
            let old = |h: &[Msg]| {
                h.iter()
                    .flat_map(|m| &m.blocks)
                    .find_map(|b| match b {
                        Block::ToolResult { tool_use_id, content, .. } if tool_use_id == "old" => Some(content.len()),
                        _ => None,
                    })
                    .unwrap()
            };
            assert_eq!(old(&seen[0]), 8000);
            assert!(old(&seen[1]) < 8000, "room was made before asking again");
            assert!(events.iter().any(|e| matches!(e, AgentEvent::Status { text } if text.contains("65536"))), "{events:?}");
            let journal = host.journal.lock().unwrap();
            assert!(journal.iter().any(|(kind, entry)| kind == "context" && entry["budget"] == 65536 - 65536 / 4 && entry["tokens_before"] == 69944));
            let error = events.iter().find_map(|e| match e {
                AgentEvent::Error { remediation, .. } => Some(remediation.clone().unwrap_or_default()),
                _ => None,
            });
            if fits {
                assert!(error.is_none(), "{events:?}");
                assert!(events.iter().any(|e| matches!(e, AgentEvent::TextDelta { text } if text == "done")));
            } else {
                assert!(error.is_some_and(|r| r.contains("OLLAMA_CONTEXT_LENGTH")), "{events:?}");
            }
        }
    }

    /// A conversation kept in `dir` whose old tool result (8000 characters) can be trimmed, or
    /// just one message when `long` is false.
    fn refit_conversation(dir: &std::path::Path, long: bool, chain: Vec<ModelSlot>) -> (ApiConversation, Arc<FakeHost>) {
        let transcript = dir.join("transcript.json");
        let mut history = vec![Msg::user_text("Build it")];
        if long {
            history.push(Msg { role: Speaker::Assistant, blocks: vec![Block::ToolUse { id: "old".into(), name: "look".into(), input: json!({}) }] });
            history
                .push(Msg { role: Speaker::User, blocks: vec![Block::ToolResult { tool_use_id: "old".into(), content: "x".repeat(8000), is_error: false }] });
            for _ in 0..context::KEEP_RECENT / 2 {
                history.push(Msg { role: Speaker::Assistant, blocks: vec![Block::Text { text: "checked".into() }] });
                history.push(Msg::user_text("continue"));
            }
        }
        context::save(&transcript, &history, true).unwrap();
        let host = Arc::new(FakeHost { calls: Mutex::default(), journal: Mutex::default() });
        let conversation = ApiConversation::start(host.clone(), chain, LoopConfig { transcript: Some(transcript), deep: true, ..Default::default() }).unwrap();
        (conversation, host)
    }

    fn old_result_len(history: &[Msg]) -> Option<usize> {
        history.iter().flat_map(|m| &m.blocks).find_map(|b| match b {
            Block::ToolResult { tool_use_id, content, .. } if tool_use_id == "old" => Some(content.len()),
            _ => None,
        })
    }

    #[test]
    fn a_refit_makes_room_first_and_each_backend_gets_its_own() {
        let refused = |message: &str| Err(crate::http::status_error("Server", 400, &json!({"error": {"message": message}}).to_string()));
        let window_only = "This model's maximum context length is 2048 tokens.";
        let answer = || turn(vec![Block::Text { text: "done".into() }], StopReason::EndTurn);
        let slot =
            |name: &str, model: Scripted| ModelSlot { name: name.into(), model: Box::new(model), local: true, effort: None, max_turns: 5, context_tokens: 0 };

        // No count of the refused request: it was larger than the window, so room is made all the same.
        let dir = tempfile::tempdir().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let model = Scripted { turns: VecDeque::from([refused(window_only), answer()]), seen: seen.clone() };
        let (mut conversation, _) = refit_conversation(dir.path(), true, vec![slot("A", model)]);
        conversation.send("continue");
        let events = drain_until_done(&mut conversation, |_, _| {});
        assert!(events.iter().any(|e| matches!(e, AgentEvent::TextDelta { text } if text == "done")), "{events:?}");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(old_result_len(&seen[1]).unwrap() < 8000, "trimmed before asking again");

        // Nothing to trim: the same request isn't sent again.
        let dir = tempfile::tempdir().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let model = Scripted { turns: VecDeque::from([refused(window_only), answer()]), seen: seen.clone() };
        let (mut conversation, _) = refit_conversation(dir.path(), false, vec![slot("A", model)]);
        conversation.send("continue");
        let events = drain_until_done(&mut conversation, |_, _| {});
        assert_eq!(seen.lock().unwrap().len(), 1, "an unchanged request isn't asked again");
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Error { .. })), "{events:?}");

        // A fallback refused for its own window is fitted and asked once more too.
        let dir = tempfile::tempdir().unwrap();
        let (first_seen, second_seen) = (Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(Vec::new())));
        let ollama = "request (69944 tokens) exceeds the available context size (65536 tokens)";
        let first = Scripted { turns: VecDeque::from([refused(ollama), refused(ollama)]), seen: first_seen.clone() };
        // B's 2048-token window is small enough that it summarises before asking again.
        let second = Scripted { turns: VecDeque::from([refused(window_only), answer(), answer()]), seen: second_seen.clone() };
        let (mut conversation, _) = refit_conversation(dir.path(), true, vec![slot("A", first), slot("B", second)]);
        conversation.send("continue");
        let events = drain_until_done(&mut conversation, |_, _| {});
        assert!(events.iter().any(|e| matches!(e, AgentEvent::TextDelta { text } if text == "done")), "{events:?}");
        assert_eq!((first_seen.lock().unwrap().len(), second_seen.lock().unwrap().len()), (2, 3), "A asked twice; B refused, summarised, answered");
        assert!(!events.iter().any(|e| matches!(e, AgentEvent::Error { .. })), "{events:?}");
    }

    #[test]
    fn fallback_chain_and_refusal() {
        let host = Arc::new(FakeHost { calls: Mutex::default(), journal: Mutex::default() });
        let failing = Scripted { turns: VecDeque::from([Err(ToolError::new(ErrorCode::EngineMissing, "down"))]), seen: Arc::default() };
        let refusing = Scripted {
            turns: VecDeque::from([turn(vec![], StopReason::Refusal), turn(vec![Block::Text { text: "ok".into() }], StopReason::EndTurn)]),
            seen: Arc::default(),
        };
        let chain = vec![
            ModelSlot { name: "A".into(), model: Box::new(failing), local: false, effort: None, max_turns: 5, context_tokens: 0 },
            ModelSlot { name: "B".into(), model: Box::new(refusing), local: true, effort: None, max_turns: 5, context_tokens: 0 },
        ];
        let mut c = ApiConversation::start(host, chain, LoopConfig::default()).unwrap();
        c.send("q");
        let ev = drain_until_done(&mut c, |_, _| {});
        assert!(ev.iter().any(|e| matches!(e, AgentEvent::Status { text } if text.contains("Trying B"))));
        assert!(ev.iter().any(|e| matches!(e, AgentEvent::Error { message, .. } if message.contains("declined"))));
        assert!(matches!(ev.last(), Some(AgentEvent::TurnDone { backend, .. }) if backend == "B"));
        c.send("again");
        let ev = drain_until_done(&mut c, |_, _| {});
        assert!(ev.iter().any(|e| matches!(e, AgentEvent::TextDelta { text } if text == "ok")));
        assert!(!c.is_busy() || c.poll().is_empty());
    }

    #[test]
    fn compaction_keeps_small_results_and_previews_large_ones() {
        use craft_anything_evidence::{Environment, EvidenceBuilder, Subject};
        let small = EvidenceBuilder::new("op", Subject::default(), Environment::builtin(true)).result(json!({"a": 1})).summary("small").build().unwrap();
        let (text, summary, id) = compact(&ToolOutput::evidence(small.clone()));
        assert!(text.contains("\"result\":{\"a\":1}"));
        assert_eq!(summary, "small");
        assert_eq!(id, Some(small.id));
        let big = EvidenceBuilder::new("op", Subject::default(), Environment::builtin(true)).result(json!({"s": "x".repeat(100_000)})).build().unwrap();
        let (text, _, _) = compact(&ToolOutput::evidence(big));
        assert!(text.len() < RESULT_BUDGET + 2048);
        assert!(text.contains("result_preview"));
        let (text, _, id) = compact(&ToolOutput::Value(json!({"k": "v"})));
        assert!(text.contains("\"k\""));
        assert!(id.is_none());
        assert!(truncate_json(&json!("é".repeat(50_000)), 1000).len() < 1100);
    }
}
