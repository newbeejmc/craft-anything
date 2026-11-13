//! OpenAI chat completions (and every OpenAI-compatible server: OpenRouter, Ollama, LM Studio,
//! llama.cpp, vLLM…): streaming, function tools, and a ReAct text fallback for models without
//! tool calling.

use crate::http::{self, SseEvent};
use crate::model::{Block, ChatModel, Delta, Msg, Speaker, Stop, StopReason, ToolDef, TurnRequest, TurnResult, Usage};
use craft_anything_evidence::{ErrorCode, ToolError};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const OPENAI_BASE: &str = "https://api.openai.com/v1";
pub const OPENROUTER_BASE: &str = "https://openrouter.ai/api/v1";
/// Gemini's OpenAI-compatible endpoint, for a Google AI Studio key.
pub const GEMINI_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/openai";
pub const OPENAI_DEFAULT_MODEL: &str = "gpt-5";
pub const OPENROUTER_DEFAULT_MODEL: &str = "anthropic/claude-opus-5-5";
/// Kimi K3.1 Pro (Moonshot AI) on OpenRouter: 1M-token context, tool calls.
pub const KIMI_K3_1: &str = "moonshotai/kimi-k3.1-pro";
/// OpenRouter models offered when adding it.
pub const OPENROUTER_MODELS: &[&str] = &[OPENROUTER_DEFAULT_MODEL, KIMI_K3_1];
/// Gemini models offered when adding a Google AI Studio key; the live list comes from the key.
pub const GEMINI_DEFAULT_MODEL: &str = "gemini-3.8-flash";
pub const GEMINI_MODELS: &[&str] = &[GEMINI_DEFAULT_MODEL, "gemini-4-argon"];
/// Generic app identification for OpenRouter (no personal data).
pub const APP_REFERER: &str = "https://github.com/storytold/craft-anything";
pub const APP_TITLE: &str = "Craft Anything";
/// OpenCode Go (a subscription): an OpenAI-compatible base, the key from its console, and a session
/// header the service asks every client to send so routing and prompt caching follow a conversation.
pub const OPENCODE_GO_BASE: &str = "https://opencode.ai/inference/go/openai/v1";
/// The header OpenCode Go asks every client to send.
pub const OPENCODE_GO_SESSION_HEADER: &str = "x-opencode-session";
/// The model a new OpenCode Go backend starts on, before its own list is fetched.
pub const OPENCODE_GO_DEFAULT_MODEL: &str = "kimi-k3.1-pro";

/// Whether this base URL is OpenCode Go's: https, host opencode.ai, path under /inference/go/.
/// A substring test would also match a look-alike host or a proxy path.
pub fn is_opencode_go(base: &str) -> bool {
    https_host_path(base).is_some_and(|(host, path)| host.eq_ignore_ascii_case("opencode.ai") && (path == "inference/go" || path.starts_with("inference/go/")))
}

/// An https address's host (without user or port) and path; `None` for any other scheme.
pub(crate) fn https_host_path(base: &str) -> Option<(&str, &str)> {
    let base = base.trim().trim_end_matches('/');
    let (scheme, rest) = base.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.rsplit('@').next().unwrap_or(host).split(':').next().unwrap_or(host);
    Some((host, path))
}

/// Well-known local servers: (name, base URL).
pub const LOCAL_SERVERS: &[(&str, &str)] = &[
    ("Ollama", "http://127.0.0.1:11434/v1"),
    ("LM Studio", "http://127.0.0.1:1234/v1"),
    ("llama.cpp server", "http://127.0.0.1:8080/v1"),
    ("vLLM", "http://127.0.0.1:8000/v1"),
];

#[derive(Debug, Clone)]
pub struct OpenAi {
    /// For messages ("OpenAI", "OpenRouter", "Ollama"…).
    pub label: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub extra_headers: Vec<(&'static str, String)>,
    /// Use the ReAct text protocol instead of function tools.
    pub react: bool,
    /// Recover JSON text calls automatically, and switch to ReAct when the server rejects tools.
    pub auto_react: bool,
    /// Talking to OpenRouter, which routes each request to one of several upstream providers.
    pub openrouter: bool,
    /// The AI's own limit on one streamed answer (a slow local model can need hours); none uses
    /// the defaults.
    pub stream_limit: Option<std::time::Duration>,
    /// The longest answer to ask for, once OpenRouter has said the key can't afford more (#1213).
    pub answer_cap: Option<u64>,
}

/// An answer cap smaller than this would cut a builder's file writes short: such a refusal stays one.
const MIN_AFFORDABLE_ANSWER: u64 = 4096;

impl OpenAi {
    pub fn official(base: Option<&str>, key: String, model: &str) -> Self {
        Self::compatible("OpenAI", base.unwrap_or(OPENAI_BASE), Some(key), if model.is_empty() { OPENAI_DEFAULT_MODEL } else { model })
    }

    pub fn openrouter(base: Option<&str>, key: String, model: &str) -> Self {
        let mut o = Self::compatible("OpenRouter", base.unwrap_or(OPENROUTER_BASE), Some(key), if model.is_empty() { OPENROUTER_DEFAULT_MODEL } else { model });
        o.extra_headers = vec![("HTTP-Referer", APP_REFERER.to_string()), ("X-Title", APP_TITLE.to_string())];
        o.openrouter = true;
        o
    }

    /// A Google AI Studio (Gemini API) key, through Gemini's OpenAI-compatible endpoint.
    pub fn gemini(base: Option<&str>, key: String, model: &str) -> Self {
        Self::compatible("Gemini", base.unwrap_or(GEMINI_BASE), Some(key), if model.is_empty() { GEMINI_DEFAULT_MODEL } else { model })
    }

    pub fn compatible(label: &str, base: &str, key: Option<String>, model: &str) -> Self {
        OpenAi {
            label: label.to_string(),
            base_url: base.trim_end_matches('/').to_string(),
            api_key: key.filter(|k| !k.is_empty()),
            model: model.to_string(),
            extra_headers: Vec::new(),
            react: false,
            auto_react: true,
            openrouter: false,
            stream_limit: None,
            answer_cap: None,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{path}", self.base_url)
    }

    pub fn headers(&self) -> Vec<(&'static str, String)> {
        let mut h = self.extra_headers.clone();
        if let Some(k) = &self.api_key {
            h.push(("authorization", format!("Bearer {k}")));
        }
        h
    }

    pub fn body(&self, req: &TurnRequest<'_>) -> Value {
        let mut system = req.system.to_string();
        if self.react && !req.tools.is_empty() {
            system.push_str(&react_instructions(req.tools));
        }
        let mut messages = vec![json!({"role": "system", "content": system})];
        let mut dropped_empty_assistant = false;
        for m in req.messages {
            let next_wire = messages_json(m, self.react);
            if m.role == Speaker::Assistant && next_wire.is_empty() {
                dropped_empty_assistant = true;
                continue;
            }
            for msg in next_wire {
                let can_merge = dropped_empty_assistant
                    && msg.get("role").and_then(Value::as_str) == Some("user")
                    && messages.last().and_then(|m| m.get("role")).and_then(Value::as_str) == Some("user")
                    && messages.last().and_then(|m| m.get("content")).and_then(Value::as_str).is_some()
                    && msg.get("content").and_then(Value::as_str).is_some();
                if can_merge {
                    let new_text = msg.get("content").and_then(Value::as_str).unwrap_or_default();
                    if let Some(last_val) = messages.last_mut().and_then(|m| m.get_mut("content"))
                        && let Some(last_text) = last_val.as_str()
                    {
                        *last_val = Value::String(format!("{last_text}\n\n{new_text}"));
                    }
                } else {
                    messages.push(msg);
                }
                dropped_empty_assistant = false;
            }
        }
        let mut body = json!({
            "model": self.model,
            "stream": true,
            "stream_options": {"include_usage": true},
            "messages": messages,
        });
        if let (Some(cap), Some(o)) = (self.answer_cap, body.as_object_mut()) {
            o.insert("max_tokens".into(), json!(cap));
        }
        if !self.react
            && !req.tools.is_empty()
            && let Some(o) = body.as_object_mut()
        {
            let tools: Vec<Value> = req
                .tools
                .iter()
                .map(|t| json!({"type": "function", "function": {"name": t.name, "description": t.description, "parameters": t.schema}}))
                .collect();
            o.insert("tools".into(), Value::Array(tools));
            o.insert("tool_choice".into(), json!("auto"));
            // OpenRouter picks an upstream provider per request, and one that can't do tool calls
            // answers "400: Provider returned error" (#369). Only a provider that supports every
            // parameter in the request is used.
            if self.openrouter {
                o.insert("provider".into(), json!({"require_parameters": true}));
            }
        }
        body
    }

    /// The models the server says are loaded now, read at the root of a `…/v1` address: Ollama's
    /// `/api/ps` (Ollama and FastFlowLM answer it), else LM Studio's `/api/v0/models`, whose
    /// entries say `state: "loaded"` (embedding models left out). Empty when the address doesn't
    /// end in `/v1`, or the server answers neither: `/models` may list every model a server has
    /// downloaded or can download, and asking for another loads or downloads it (#1434).
    pub fn loaded_models(&self, timeout: std::time::Duration) -> Vec<String> {
        let Some(root) = self.base_url.strip_suffix("/v1") else { return Vec::new() };
        let agent = http::quick_agent(timeout);
        let get = |path: &str| http::get_json(&agent, &self.label, &format!("{root}{path}"), &self.headers()).ok();
        let ids = |entries: Vec<&Value>, keys: &[&str]| {
            let mut out: Vec<String> = Vec::new();
            for m in entries {
                let id = keys.iter().find_map(|k| m.get(*k)).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
                if let Some(id) = id
                    && !out.iter().any(|seen| seen == id)
                {
                    out.push(id.to_string());
                }
            }
            out
        };
        let running =
            get("/api/ps").map(|v| ids(v.get("models").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default(), &["model", "name"]));
        if let Some(found) = running.filter(|r| !r.is_empty()) {
            return found;
        }
        let Some(v) = get("/api/v0/models") else { return Vec::new() };
        let loaded = v
            .get("data")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter(|m| m.get("state").and_then(Value::as_str) == Some("loaded"))
                    .filter(|m| m.get("type").and_then(Value::as_str) != Some("embeddings"))
                    .collect()
            })
            .unwrap_or_default();
        ids(loaded, &["id"])
    }

    /// Model ids from `GET /models`.
    pub fn list_models(&self, timeout: std::time::Duration) -> Result<Vec<String>, ToolError> {
        self.list_models_within(timeout, timeout)
    }

    /// Like [`OpenAi::list_models`], with its own limit for reaching the server (`connect`) and for
    /// its answer (`wait`): a server still starting or loading a model is reached at once but may
    /// take a while to answer, and an address nothing listens at shouldn't hold the person as long.
    pub fn list_models_within(&self, connect: std::time::Duration, wait: std::time::Duration) -> Result<Vec<String>, ToolError> {
        let agent = http::agent(connect, wait, wait);
        let v = http::get_json(&agent, &self.label, &self.url("models"), &self.headers())?;
        let arr = v.get("data").or_else(|| v.get("models")).and_then(Value::as_array).cloned().unwrap_or_default();
        // A catalogue may list a model only for another API (Responses, Messages): such an entry
        // would fail on a chat-completions client, so skip it when the entry says which APIs it
        // serves. Empty and repeated ids are dropped too.
        let mut out: Vec<String> = Vec::new();
        for m in arr {
            if m.get("api").is_some_and(|a| a.get("chat-completions").is_none()) {
                continue;
            }
            let Some(id) = m.get("id").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) else { continue };
            if !out.iter().any(|seen| seen == id) {
                out.push(id.to_string());
            }
        }
        Ok(out)
    }

    fn stream_once(&self, req: &TurnRequest<'_>, on_delta: &mut dyn FnMut(Delta), stop: &Stop) -> Result<TurnResult, ToolError> {
        let agent = http::stream_agent_with_stop(self.stream_limit, stop)?;
        let response = http::post_json(&agent, &self.label, &self.url("chat/completions"), &self.headers(), &self.body(req));
        stop.check()?;
        let reader = response?;
        let mut st = ChatStream::default();
        let parsed = http::parse_sse(reader, |ev| {
            stop.check()?;
            st.apply(&ev, on_delta)?;
            Ok(!st.done)
        });
        stop.check()?;
        parsed?;
        let mut r = st.finish(&self.model)?;
        if self.react || self.auto_react {
            react_postprocess(&mut r, req.tools, self.react);
        }
        Ok(r)
    }
}

/// The answer length a 402 says the key can still pay for.
fn affordable(e: &ToolError) -> Option<u64> {
    e.details.as_ref().and_then(|d| d.get("affordable_tokens")).and_then(Value::as_u64)
}

fn rejects_tools(e: &ToolError) -> bool {
    let m = e.message.to_ascii_lowercase();
    let status = e.details.as_ref().and_then(|d| d.get("status")).and_then(Value::as_u64).unwrap_or(0);
    (status == 400 || status == 422 || status == 500 || status == 404) && (m.contains("tool") || m.contains("function"))
}

impl ChatModel for OpenAi {
    fn turn(&mut self, req: &TurnRequest<'_>, on_delta: &mut dyn FnMut(Delta), stop: &Stop) -> Result<TurnResult, ToolError> {
        match self.stream_once(req, on_delta, stop) {
            Err(e) if !self.react && self.auto_react && !req.tools.is_empty() && (rejects_tools(&e) || self.no_provider_for_tools(&e)) => {
                log::info!("{} rejected tool calling; switching to the text protocol", self.label);
                self.react = true;
                self.stream_once(req, on_delta, stop)
            }
            // With no max_tokens, OpenRouter holds the model's whole output limit against the key's
            // credit; it says what it can afford, so ask once more for an answer that long (#1213).
            Err(e) if self.openrouter && affordable(&e).is_some_and(|n| n >= MIN_AFFORDABLE_ANSWER && self.answer_cap.is_none_or(|c| n < c)) => {
                self.answer_cap = affordable(&e);
                log::info!("{} can afford an answer of {:?} tokens; asking for that", self.label, self.answer_cap);
                self.stream_once(req, on_delta, stop)
            }
            other => other,
        }
    }

    fn model(&self) -> String {
        self.model.clone()
    }
}

impl OpenAi {
    /// OpenRouter answers 404 when no provider supports all the request's parameters (see
    /// [`OpenAi::body`]): tool calling isn't available for this model, so the text protocol is.
    fn no_provider_for_tools(&self, e: &ToolError) -> bool {
        self.openrouter && e.details.as_ref().and_then(|d| d.get("status")).and_then(Value::as_u64) == Some(404)
    }
}

/// Wire messages for one neutral message (a user message with several tool results becomes
/// several `tool` messages).
pub fn messages_json(m: &Msg, react: bool) -> Vec<Value> {
    match m.role {
        Speaker::User => {
            let mut out = Vec::new();
            let mut text = String::new();
            for b in &m.blocks {
                match b {
                    Block::Text { text: t } => text.push_str(t),
                    Block::ToolResult { tool_use_id, content, is_error } => {
                        if react {
                            let tag = if *is_error { "Observation (error)" } else { "Observation" };
                            text.push_str(&format!("{tag}: {content}\n"));
                        } else {
                            out.push(json!({"role": "tool", "tool_call_id": tool_use_id, "content": content}));
                        }
                    }
                    _ => {}
                }
            }
            if !text.is_empty() {
                out.push(json!({"role": "user", "content": text}));
            }
            out
        }
        Speaker::Assistant => {
            // Some providers (Kimi, #849) refuse an assistant message with nothing in it; blank text
            // counts as nothing.
            let text = m.text();
            let mut text = if text.trim().is_empty() { String::new() } else { text };
            if react {
                // Calls made natively before the switch to the text protocol are written as the
                // protocol's own lines, so the turn isn't empty and the model sees what it did.
                // A call the text already names (one `Action:` line each) isn't written twice; any
                // other is, so its result never arrives without it.
                let mut named: Vec<String> = text.lines().filter_map(|l| l.trim().strip_prefix("Action:")).map(|n| n.trim().to_string()).collect();
                for (_, name, input) in m.tool_uses() {
                    if let Some(i) = named.iter().position(|n| *n == name) {
                        named.swap_remove(i);
                        continue;
                    }
                    if !text.is_empty() && !text.ends_with('\n') {
                        text.push('\n');
                    }
                    text.push_str(&format!("Action: {name}\nAction Input: {input}"));
                }
                return if text.is_empty() { Vec::new() } else { vec![json!({"role": "assistant", "content": text})] };
            }
            let calls: Vec<Value> = m
                .tool_uses()
                .into_iter()
                .map(|(id, name, input)| {
                    let mut call = json!({"id": id, "type": "function", "function": {"name": name, "arguments": input.to_string()}});
                    let extra = m.blocks.iter().find_map(|b| match b {
                        Block::ToolExtra { tool_use_id, extra } if *tool_use_id == id => Some(extra.clone()),
                        _ => None,
                    });
                    if let (Some(extra), Some(o)) = (extra, call.as_object_mut()) {
                        o.insert("extra_content".into(), extra);
                    }
                    call
                })
                .collect();
            if text.is_empty() && calls.is_empty() {
                return Vec::new();
            }
            let mut v = json!({"role": "assistant"});
            if let Some(o) = v.as_object_mut() {
                if !text.is_empty() {
                    o.insert("content".into(), Value::String(text));
                }
                if !calls.is_empty() {
                    o.insert("tool_calls".into(), Value::Array(calls));
                }
            }
            vec![v]
        }
    }
}

#[derive(Debug, Default)]
struct CallAcc {
    id: String,
    name: String,
    args: String,
    extra: Option<Value>,
}

/// Accumulates a streamed chat completion.
#[derive(Debug, Default)]
pub struct ChatStream {
    text: String,
    calls: BTreeMap<u64, CallAcc>,
    pub usage: Usage,
    pub finish: Option<String>,
    pub model: String,
    pub done: bool,
}

impl ChatStream {
    pub fn apply(&mut self, ev: &SseEvent, on_delta: &mut dyn FnMut(Delta)) -> Result<(), ToolError> {
        let data = ev.data.trim();
        if data.is_empty() {
            return Ok(());
        }
        if data == "[DONE]" {
            self.done = true;
            return Ok(());
        }
        let v: Value =
            serde_json::from_str(data).map_err(|e| ToolError::new(ErrorCode::GarbledOutput, format!("The server sent a chunk we couldn't read: {e}")))?;
        if let Some(err) = v.get("error") {
            let msg = err.get("message").and_then(Value::as_str).unwrap_or("unknown error");
            let provider = err.get("code").and_then(Value::as_str).or_else(|| err.get("type").and_then(Value::as_str));
            let status = err.get("code").and_then(Value::as_u64);
            return Err(
                ToolError::new(ErrorCode::ProgramFailed, format!("Stream error: {msg}")).with_details(json!({"provider_error": provider, "status": status}))
            );
        }
        if let Some(m) = v.get("model").and_then(Value::as_str) {
            self.model = m.to_string();
        }
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
            self.usage.output_tokens = u.get("completion_tokens").and_then(Value::as_u64).unwrap_or(self.usage.output_tokens);
            let cached = u.pointer("/prompt_tokens_details/cached_tokens").and_then(Value::as_u64);
            if let Some(c) = cached {
                self.usage.cache_read_tokens = c;
            }
            // prompt_tokens includes the cached ones; input_tokens is the rest, as Anthropic reports
            // it, so the context budget and the cost count cached tokens once (#252).
            if let Some(p) = u.get("prompt_tokens").and_then(Value::as_u64) {
                self.usage.input_tokens = p.saturating_sub(cached.unwrap_or(0));
            }
        }
        let Some(choice) = v.get("choices").and_then(Value::as_array).and_then(|a| a.first()) else { return Ok(()) };
        let d = choice.get("delta").cloned().unwrap_or(Value::Null);
        if let Some(t) = d.get("content").and_then(Value::as_str).filter(|t| !t.is_empty()) {
            self.text.push_str(t);
            on_delta(Delta::Text(t.to_string()));
        }
        if let Some(t) = d.get("reasoning_content").or_else(|| d.get("reasoning")).and_then(Value::as_str).filter(|t| !t.is_empty()) {
            on_delta(Delta::Thinking(t.to_string()));
        }
        if let Some(calls) = d.get("tool_calls").and_then(Value::as_array) {
            for (pos, c) in calls.iter().enumerate() {
                let idx = c.get("index").and_then(Value::as_u64).unwrap_or(pos as u64);
                if self.calls.len() >= 256 && !self.calls.contains_key(&idx) {
                    continue;
                }
                let acc = self.calls.entry(idx).or_default();
                if let Some(id) = c.get("id").and_then(Value::as_str) {
                    acc.id = id.to_string();
                }
                if let Some(n) = c.pointer("/function/name").and_then(Value::as_str) {
                    acc.name.push_str(n);
                }
                if let Some(a) = c.pointer("/function/arguments").and_then(Value::as_str) {
                    acc.args.push_str(a);
                }
                if let Some(extra) = c.get("extra_content") {
                    acc.extra = Some(extra.clone());
                }
            }
        }
        if let Some(f) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish = Some(f.to_string());
        }
        Ok(())
    }

    pub fn finish(self, fallback_model: &str) -> Result<TurnResult, ToolError> {
        let mut blocks = Vec::new();
        if !self.text.is_empty() {
            blocks.push(Block::Text { text: self.text });
        }
        let has_calls = !self.calls.is_empty();
        for (i, (_, c)) in self.calls.into_iter().enumerate() {
            let input =
                if c.args.trim().is_empty() { json!({}) } else { serde_json::from_str(&c.args).unwrap_or_else(|_| crate::anthropic::invalid_input(&c.args)) };
            let id = if c.id.is_empty() { format!("call_{i}") } else { c.id };
            blocks.push(Block::ToolUse { id: id.clone(), name: c.name, input });
            if let Some(extra) = c.extra {
                blocks.push(Block::ToolExtra { tool_use_id: id, extra });
            }
        }
        let stop = match self.finish.as_deref() {
            Some(f) => StopReason::parse(f),
            None if has_calls => StopReason::ToolUse,
            None if self.done => StopReason::EndTurn,
            None => {
                return Err(ToolError::new(ErrorCode::Truncated, "The answer stream ended early.")
                    .with_remediation("The server ended the answer early (a local server may still be loading the model). Try again in a minute."));
            }
        };
        // Some servers say "stop" for tool calls. A limit or refusal must still prevent execution.
        let stop = if has_calls && stop == StopReason::EndTurn { StopReason::ToolUse } else { stop };
        let model = if self.model.is_empty() { fallback_model.to_string() } else { self.model };
        Ok(TurnResult { message: Msg { role: Speaker::Assistant, blocks }, stop, usage: self.usage, model, follow_up: None })
    }
}

/// Extra system text for the ReAct protocol.
pub fn react_instructions(tools: &[ToolDef]) -> String {
    let mut s = String::from(
        "\n\n## Using tools (text protocol)\nYou can't call tools directly. To use one, reply with exactly two lines and nothing after them:\nAction: <tool name>\nAction Input: <JSON object with the parameters>\nYou'll get the result back as an Observation. When you have the answer, start a line with \"Final Answer:\" and write it.\n\nTools:\n",
    );
    for t in tools {
        s.push_str(&format!("- {}: {} Parameters: {}\n", t.name, t.description, t.schema));
    }
    s
}

/// A ReAct step parsed from model text.
#[derive(Debug, Clone, PartialEq)]
pub enum ReactStep {
    Action {
        tool: String,
        input: Value,
    },
    Final(String),
    /// Plain text with no protocol markers (treated as the answer).
    Text(String),
}

/// Parse a ReAct step, accepting a complete JSON call as an alternative.
pub fn parse_react(text: &str) -> ReactStep {
    if let Some(step) = json_action(text) {
        return step;
    }
    // Preserve a whole fenced ReAct step, but do not treat markers inside a JSON answer or
    // ordinary code example as instructions.
    let lead = text.trim();
    let fenced_protocol = backtick_fence(lead).is_some_and(|(_, body)| {
        let body = body.trim_start();
        ["Action:", "Thought:", "Final Answer:"].into_iter().any(|marker| body.starts_with(marker))
    });
    if lead.starts_with(['{', '[']) || (lead.starts_with("```") && !fenced_protocol) {
        return ReactStep::Text(text.trim().to_string());
    }
    if let Some(pos) = text.find("Final Answer:") {
        return ReactStep::Final(text.get(pos + "Final Answer:".len()..).unwrap_or_default().trim().to_string());
    }
    let Some(apos) = text.find("Action:") else { return ReactStep::Text(text.trim().to_string()) };
    let after = text.get(apos + "Action:".len()..).unwrap_or_default();
    let tool = after.lines().next().unwrap_or_default().trim().trim_matches('`').to_string();
    let input = match after.find("Action Input:") {
        Some(ipos) => {
            let raw = after.get(ipos + "Action Input:".len()..).unwrap_or_default().trim();
            let raw = raw.trim_start_matches("```json").trim_start_matches("```").trim();
            extract_json_object(raw).unwrap_or_else(|| json!({}))
        }
        None => json!({}),
    };
    if tool.is_empty() { ReactStep::Text(text.trim().to_string()) } else { ReactStep::Action { tool, input } }
}

/// A complete backtick fence. Its standalone closing line can be longer than its opener,
/// but an earlier closing line means the remaining text is outside that fence.
fn backtick_fence(text: &str) -> Option<(&str, &str)> {
    let (opening, rest) = text.trim().split_once('\n')?;
    let length = opening.bytes().take_while(|byte| *byte == b'`').count();
    if length < 3 {
        return None;
    }
    let language = opening.get(length..)?.trim();
    if language.contains('`') {
        return None;
    }
    let (body, closing) = rest.rsplit_once('\n')?;
    let closes = |line: &str| {
        let line = line.trim();
        line.len() >= length && line.bytes().all(|byte| byte == b'`')
    };
    if !closes(closing) || body.lines().any(closes) {
        return None;
    }
    Some((language, body))
}

/// Accept a complete call, not a JSON object quoted somewhere in an explanation.
fn json_action(text: &str) -> Option<ReactStep> {
    let text = text.trim();
    let raw = if text.starts_with("```") {
        let (language, body) = backtick_fence(text)?;
        if !matches!(language, "" | "json") {
            return None;
        }
        body.trim()
    } else {
        text
    };
    let value: Value = serde_json::from_str(raw).ok()?;
    let envelope = value.as_object()?;
    if envelope.contains_key("function")
        && (envelope.keys().any(|key| !matches!(key.as_str(), "function" | "type" | "id"))
            || envelope.get("type").is_some_and(|kind| kind.as_str() != Some("function")))
    {
        return None;
    }
    let function = value.get("function").unwrap_or(&value);
    let fields = function.as_object()?;
    if fields.keys().any(|key| !matches!(key.as_str(), "name" | "arguments" | "parameters"))
        || fields.contains_key("arguments") == fields.contains_key("parameters")
    {
        return None;
    }
    let tool = function.get("name")?.as_str()?.trim();
    if tool.is_empty() {
        return None;
    }
    let arguments = function.get("arguments").or_else(|| function.get("parameters"))?;
    let input: Value = match arguments.as_str() {
        Some(encoded) => serde_json::from_str(encoded).ok()?,
        None => arguments.clone(),
    };
    input.is_object().then(|| ReactStep::Action { tool: tool.to_string(), input })
}

/// The calls in a reply that ends in fenced JSON calls after a sentence or between them
/// ("Let's list the files." then a ```json fence), as small local models write them (#417).
/// Every fence must be a complete `json` (or unlabelled) call, and nothing but blank lines may
/// follow the last one; anything else (other code, a `~~~` fence, a fence inside a fence, an
/// unclosed fence, words after the call, a JSON object quoted inline) means the reply is text,
/// as before. Protocol markers before the first call keep the reply as text; after a call they
/// require a retry even if another fence follows (it could be an example after a final answer).
/// Inside a call's arguments they're only content.
fn parse_fenced_calls(text: &str) -> Option<(Vec<(String, Value)>, bool)> {
    let mut calls = Vec::new();
    let mut lines = text.lines();
    let mut words_after = false;
    let mut protocol_after = false;
    while let Some(line) = lines.next() {
        let opening = line.trim();
        // Anything in a tilde fence is an example, whatever it holds.
        if opening.starts_with("~~~") {
            return None;
        }
        let length = opening.bytes().take_while(|byte| *byte == b'`').count();
        if length < 3 {
            if ["Final Answer:", "Action:"].into_iter().any(|marker| line.contains(marker)) {
                if calls.is_empty() {
                    return None;
                }
                // A later fence may be an example after a final answer, not another call.
                protocol_after = true;
            }
            words_after |= !opening.is_empty() && !calls.is_empty();
            continue;
        }
        let closes = |line: &str| {
            let line = line.trim();
            line.len() >= length && line.bytes().all(|byte| byte == b'`')
        };
        let mut body = String::new();
        loop {
            let next = lines.next()?;
            if closes(next) {
                break;
            }
            body.push_str(next);
            body.push('\n');
        }
        let language = opening.get(length..)?.trim();
        // The body is the call itself: a fence inside it is a quoted example.
        if !matches!(language, "" | "json") || body.trim_start().starts_with(['`', '~']) {
            return None;
        }
        let Some(ReactStep::Action { tool, input }) = json_action(&body) else { return None };
        calls.push((tool, input));
        words_after = false;
    }
    (!calls.is_empty()).then_some((calls, words_after || protocol_after))
}

/// Returns fenced calls only when no nonblank text follows the final call.
fn fenced_calls(text: &str) -> Option<Vec<(String, Value)>> {
    let (calls, words_after) = parse_fenced_calls(text)?;
    (!words_after).then_some(calls)
}

/// One call written in Qwen's tool-call markup, its parameters as written.
#[derive(Debug, Clone, PartialEq)]
struct MarkupCall {
    tool: String,
    params: Vec<(String, String)>,
}

/// The calls in a reply written in the markup Qwen's chat template teaches:
///
/// ```text
/// <tool_call>
/// <function=name>
/// <parameter=key>
/// value
/// </parameter>
/// </function>
/// </tool_call>
/// ```
///
/// A llama.cpp server turns this into a real call, but hands it back as text when it names a
/// tool the model wasn't offered, and the loop then took it for the final answer. Words may
/// come before the first call; the flag is true when anything but blank lines follows the
/// last one. The last `</tool_call>` may be missing (the server may stop on it). Anything
/// else (a call inside a code fence, `<tool_call>` in the middle of a line, a missing
/// `</parameter>`) means the reply is text.
fn parse_markup_calls(text: &str) -> Option<(Vec<MarkupCall>, bool)> {
    const OPEN: &str = "<tool_call>";
    const CLOSE: &str = "</tool_call>";
    // The first call starts a line, and no code fence comes before it.
    let mut start = None;
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            return None;
        }
        if trimmed.starts_with(OPEN) {
            start = Some(offset.checked_add(line.len().checked_sub(trimmed.len())?)?);
            break;
        }
        offset = offset.checked_add(line.len())?;
    }
    let mut rest = text.get(start?..)?;
    let mut calls = Vec::new();
    loop {
        rest = rest.trim_start().strip_prefix(OPEN)?.trim_start();
        let after_name = rest.strip_prefix("<function=")?;
        let (tool, after) = after_name.split_once('>')?;
        let tool = tool.trim();
        if tool.is_empty() || tool.contains(['<', '\n']) {
            return None;
        }
        rest = after;
        let mut params = Vec::new();
        loop {
            rest = rest.trim_start();
            if let Some(after) = rest.strip_prefix("</function>") {
                rest = after;
                break;
            }
            let (key, after) = rest.strip_prefix("<parameter=")?.split_once('>')?;
            let key = key.trim();
            if key.is_empty() || key.contains(['<', '\n']) {
                return None;
            }
            let (value, after) = after.split_once("</parameter>")?;
            // The template puts the value on its own lines; the text inside is kept as written.
            let value = value.strip_prefix('\n').unwrap_or(value);
            let value = value.strip_suffix('\n').unwrap_or(value);
            params.push((key.to_string(), value.to_string()));
            rest = after;
        }
        calls.push(MarkupCall { tool: tool.to_string(), params });
        rest = rest.trim_start();
        rest = rest.strip_prefix(CLOSE).unwrap_or(rest);
        let next = rest.trim_start();
        if next.starts_with(OPEN) {
            continue;
        }
        return Some((calls, !next.is_empty()));
    }
}

/// A markup call's arguments as JSON: a parameter the tool's schema says is a string stays as
/// written; any other is read as JSON when it parses (numbers, booleans, lists, objects), as
/// the template writes them, and stays a string when it doesn't, for the tool to refuse.
fn markup_input(call: &MarkupCall, tool: Option<&ToolDef>) -> Value {
    let mut input = serde_json::Map::new();
    for (key, raw) in &call.params {
        let kind = tool.and_then(|t| t.schema.get("properties")).and_then(|p| p.get(key)).and_then(|p| p.get("type"));
        let is_string = match kind {
            Some(Value::String(kind)) => kind == "string",
            Some(Value::Array(kinds)) => kinds.iter().any(|k| k == "string"),
            _ => false,
        };
        let value = if is_string { None } else { serde_json::from_str::<Value>(raw.trim()).ok() };
        input.insert(key.clone(), value.unwrap_or_else(|| Value::String(raw.clone())));
    }
    Value::Object(input)
}

/// What the model is told when its markup names tools it doesn't have.
fn not_offered_message(missing: &[&str], tools: &[ToolDef]) -> String {
    let names = missing.iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(", ");
    let (verb, noun) = if missing.len() == 1 { ("isn't", "a tool") } else { ("aren't", "tools") };
    let offered = tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", ");
    format!(
        "Nothing in your previous response ran: {names} {verb} {noun} you can use here. Your tools are: {offered}. Call one of these instead, or if you're finished, answer in plain text."
    )
}

/// The first balanced `{…}` in `s`, parsed.
fn extract_json_object(s: &str) -> Option<Value> {
    let start = s.find('{')?;
    let mut depth = 0usize;
    let mut in_str = false;
    let mut esc = false;
    for (i, c) in s.char_indices().skip_while(|(i, _)| *i < start) {
        if in_str {
            match (esc, c) {
                (true, _) => esc = false,
                (false, '\\') => esc = true,
                (false, '"') => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return serde_json::from_str(s.get(start..=i)?).ok();
                }
            }
            _ => {}
        }
    }
    None
}

static REACT_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Recover complete text calls as neutral tool-use blocks.
fn react_postprocess(r: &mut TurnResult, tools: &[ToolDef], react: bool) {
    if !matches!(r.stop, StopReason::EndTurn | StopReason::ToolUse) || !r.message.tool_uses().is_empty() {
        return;
    }
    let text = r.message.text();
    let json_call = json_action(&text);
    let from_json = json_call.is_some();
    // Calls after a sentence run only when every one names a tool the model was offered.
    let offered = |tool: &str| tools.iter().any(|definition| definition.name == tool);
    if !from_json && let Some((calls, words_after)) = parse_markup_calls(&text) {
        let missing = calls.iter().map(|call| call.tool.as_str()).filter(|tool| !offered(tool)).collect::<Vec<_>>();
        if !missing.is_empty() {
            let mut unique = Vec::new();
            for tool in missing {
                if !unique.contains(&tool) {
                    unique.push(tool);
                }
            }
            r.follow_up = Some(not_offered_message(&unique, tools));
            r.stop = StopReason::PauseTurn;
            return;
        }
        if words_after {
            r.follow_up = Some("The tool call in your previous response was not run because text followed it. Send the tool call again without adding a summary or other text after it.".into());
            r.stop = StopReason::PauseTurn;
            return;
        }
        for call in calls {
            let n = REACT_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let input = markup_input(&call, tools.iter().find(|definition| definition.name == call.tool));
            r.message.blocks.push(Block::ToolUse { id: format!("react_{n}"), name: call.tool, input });
        }
        r.stop = StopReason::ToolUse;
        return;
    }
    if !from_json && let Some(calls) = fenced_calls(&text).filter(|calls| calls.iter().all(|(tool, _)| offered(tool))) {
        for (tool, input) in calls {
            let n = REACT_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            r.message.blocks.push(Block::ToolUse { id: format!("react_{n}"), name: tool, input });
        }
        r.stop = StopReason::ToolUse;
        return;
    }
    if !from_json
        && let Some((calls, true)) = parse_fenced_calls(&text)
        && calls.iter().all(|(tool, _)| offered(tool))
    {
        r.follow_up = Some("The fenced tool call in your previous response was not run because text followed the closing fence. Send the tool call again without adding a summary or other text after it.".into());
        r.stop = StopReason::PauseTurn;
        return;
    }
    let step = json_call.or_else(|| react.then(|| parse_react(&text)));
    if let Some(ReactStep::Action { tool, input }) = step
        && (!from_json || tools.iter().any(|definition| definition.name == tool))
    {
        let n = REACT_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        r.message.blocks.push(Block::ToolUse { id: format!("react_{n}"), name: tool, input });
        r.stop = StopReason::ToolUse;
    } else if react {
        r.stop = StopReason::EndTurn;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Effort;

    #[test]
    fn qwen_markup_calls_are_read_and_examples_are_not() {
        let names = |text: &str| parse_markup_calls(text).map(|(calls, after)| (calls.into_iter().map(|c| c.tool).collect::<Vec<_>>(), after));
        // What a 512k-window Qwen model wrote in live builds, as the server handed it back.
        let read = "<tool_call>\n<function=build_command_read>\n<parameter=path>\napp/src/main.rs\n</parameter>\n<parameter=offset>\n1580\n</parameter>\n<parameter=limit>\n45\n</parameter>\n</function>\n</tool_call>";
        let (calls, after) = parse_markup_calls(read).unwrap();
        assert!(!after);
        assert_eq!(
            calls,
            [MarkupCall {
                tool: "build_command_read".into(),
                params: vec![("path".into(), "app/src/main.rs".into()), ("offset".into(), "1580".into()), ("limit".into(), "45".into())],
            }]
        );
        let two = "<tool_call>\n<function=web_doc_fetch>\n<parameter=url>\nhttps://docs.rs/eframe\n</parameter>\n</function>\n</tool_call>\n<tool_call>\n<function=web_search>\n<parameter=query>\neframe screenshot example\n</parameter>\n</function>\n</tool_call>";
        assert_eq!(names(two), Some((vec!["web_doc_fetch".into(), "web_search".into()], false)));
        let after_words = "Now I'll rewrite main.rs with all API fixes.\n\n<tool_call>\n<function=write_output>\n</function>\n</tool_call>";
        assert_eq!(names(after_words), Some((vec!["write_output".into()], false)));
        // The server may stop on the closing tag, so it can be missing at the end.
        assert_eq!(names("<tool_call>\n<function=look>\n</function>\n"), Some((vec!["look".into()], false)));
        assert_eq!(names("<tool_call>\n<function=look>\n</function>\n</tool_call>\nDone."), Some((vec!["look".into()], true)));
        // A value keeps its own lines and markup-like text; only the template's newlines go.
        let (calls, _) =
            parse_markup_calls("<tool_call>\n<function=write>\n<parameter=text>\n\nfn a() {}\n<b>\n\n</parameter>\n</function>\n</tool_call>").unwrap();
        assert_eq!(calls[0].params, [("text".to_string(), "\nfn a() {}\n<b>\n".to_string())]);
        for text in [
            "No calls here.",
            "Use <tool_call><function=look></function></tool_call> like this.",
            "```xml\n<tool_call>\n<function=look>\n</function>\n</tool_call>\n```",
            "~~~\n<tool_call>\n<function=look>\n</function>\n</tool_call>\n~~~",
            "<tool_call>\n<function=look>\n<parameter=path>\napp\n</function>\n</tool_call>",
            "<tool_call>\n<function=>\n</function>\n</tool_call>",
            "<tool_call>\n<function=look>\nsome words\n</function>\n</tool_call>",
            "<tool_call>\n{\"name\":\"look\",\"arguments\":{}}\n</tool_call>",
        ] {
            assert_eq!(names(text), None, "{text}");
        }

        // Arguments follow the schema: strings as written, everything else as JSON.
        let schema = json!({"type":"object","properties":{"path":{"type":"string"},"offset":{"type":"integer"},"tags":{"type":["array","null"]},"note":{"type":["string","null"]}}});
        let tool = ToolDef { name: "build_command_read".into(), description: String::new(), schema };
        let call = MarkupCall {
            tool: tool.name.clone(),
            params: vec![
                ("path".into(), "123".into()),
                ("offset".into(), "1580".into()),
                ("tags".into(), "[\"a\"]".into()),
                ("note".into(), "true".into()),
                ("other".into(), "false".into()),
                ("bad".into(), "{not json".into()),
            ],
        };
        assert_eq!(markup_input(&call, Some(&tool)), json!({"path":"123","offset":1580,"tags":["a"],"note":"true","other":false,"bad":"{not json"}));
        assert_eq!(markup_input(&call, None)["path"], json!(123));
    }

    #[test]
    fn a_markup_call_to_a_tool_that_isnt_offered_is_answered_not_ended() {
        let tool = |name: &str| ToolDef {
            name: name.into(),
            description: String::new(),
            schema: json!({"type":"object","properties":{"path":{"type":"string"},"offset":{"type":"integer"}}}),
        };
        let tools = [tool("build_file_read"), tool("build_file_write")];
        let turn = |text: &str| TurnResult {
            message: Msg { role: Speaker::Assistant, blocks: vec![Block::Text { text: text.into() }] },
            stop: StopReason::EndTurn,
            usage: Usage::default(),
            model: String::new(),
            follow_up: None,
        };
        let call = |name: &str| {
            format!(
                "<tool_call>\n<function={name}>\n<parameter=path>\napp/src/main.rs\n</parameter>\n<parameter=offset>\n10\n</parameter>\n</function>\n</tool_call>"
            )
        };
        for react in [false, true] {
            // Offered: it runs, with typed arguments.
            let mut r = turn(&format!("Reading it.\n{}\n{}", call("build_file_read"), call("build_file_write")));
            react_postprocess(&mut r, &tools, react);
            assert_eq!(r.stop, StopReason::ToolUse, "react={react}");
            let uses = r.message.tool_uses();
            assert_eq!(uses.iter().map(|(_, name, _)| name.to_string()).collect::<Vec<_>>(), ["build_file_read", "build_file_write"]);
            assert_eq!(uses[0].2, json!({"path":"app/src/main.rs","offset":10}));

            // Not offered (alone, twice, or beside one that is): nothing runs, and the model is
            // told which names it got wrong and which it has, once each.
            for text in [
                call("build_command_read"),
                format!("{}\n{}", call("web_search"), call("web_search")),
                format!("{}\n{}", call("build_file_read"), call("web_fetch")),
            ] {
                let mut r = turn(&text);
                react_postprocess(&mut r, &tools, react);
                assert_eq!(r.stop, StopReason::PauseTurn, "react={react}: {text}");
                assert!(r.message.tool_uses().is_empty(), "react={react}: {text}");
                let told = r.follow_up.unwrap();
                let wrong = ["build_command_read", "web_search", "web_fetch"].into_iter().find(|name| text.contains(name)).unwrap();
                assert_eq!(told.matches(&format!("`{wrong}`")).count(), 1, "{told}");
                assert!(told.contains("isn't a tool you can use here") && told.contains("Your tools are: build_file_read, build_file_write."), "{told}");
                assert!(!told.contains("`build_file_read`"), "{told}");
            }
            let mut r = turn(&format!("{}\n{}", call("web_search"), call("web_fetch")));
            react_postprocess(&mut r, &tools, react);
            assert!(r.follow_up.unwrap().contains("`web_search`, `web_fetch` aren't tools"));

            // Words after the call: not run, and asked again without them.
            let mut r = turn(&format!("{}\nThat should show the file.", call("build_file_read")));
            react_postprocess(&mut r, &tools, react);
            assert_eq!(r.stop, StopReason::PauseTurn, "react={react}");
            assert!(r.message.tool_uses().is_empty());
            assert!(r.follow_up.unwrap().contains("text followed it"));
        }
    }

    #[test]
    fn calls_after_a_sentence_are_read_and_examples_are_not() {
        let call = |tool: &str| format!("```json\n{{\"name\":\"{tool}\",\"arguments\":{{\"path\":\"app\"}}}}\n```");
        let names = |text: &str| fenced_calls(text).map(|calls| calls.into_iter().map(|(tool, _)| tool).collect::<Vec<_>>());
        // What qwen2.5-coder:14b wrote in a live build.
        let after = format!("It looks like the file is missing. Let's list the files.\n\n{}", call("look"));
        assert_eq!(names(&after), Some(vec!["look".to_string()]));
        let several = format!("{}\n\nThen:\n{}\n\n", call("look"), call("write"));
        assert_eq!(names(&several), Some(vec!["look".to_string(), "write".to_string()]));
        assert_eq!(names("```\n{\"name\":\"look\",\"parameters\":{}}\n````"), Some(vec!["look".to_string()]), "unlabelled, longer close");
        for text in [
            format!("{}\nThis is an example.", call("look")),
            format!("Here's Rust:\n```rust\nfn main() {{}}\n```\n{}", call("look")),
            format!("{}\n```json\n{{\"name\":\"look\",\"arguments\":{{}}}}", call("look")),
            "For example, use {\"name\":\"look\",\"arguments\":{}}.".to_string(),
            "```json\n{\"name\":\"look\",\"arguments\":{},\"description\":\"x\"}\n```".to_string(),
            "```json\n[{\"name\":\"look\",\"arguments\":{}}]\n```".to_string(),
            "No calls here.".to_string(),
            // An example in a tilde fence, then a real-looking call.
            format!("~~~markdown\n{}\n~~~\n{}", call("write"), call("look")),
            // A four-backtick fence quoting a three-backtick call.
            format!("Like this:\n````\n{}\n````", call("write")),
        ] {
            assert_eq!(names(&text), None, "{text}");
        }

        // Only tools that were offered run, and then all of them.
        let tool = |name: &str| ToolDef { name: name.into(), description: String::new(), schema: json!({"type": "object"}) };
        let turn = |text: &str| TurnResult {
            message: Msg { role: Speaker::Assistant, blocks: vec![Block::Text { text: text.into() }] },
            stop: StopReason::EndTurn,
            usage: Usage::default(),
            model: String::new(),
            follow_up: None,
        };
        for react in [false, true] {
            let mut r = turn(&several);
            react_postprocess(&mut r, &[tool("look"), tool("write")], react);
            assert_eq!(r.stop, StopReason::ToolUse, "react={react}");
            assert_eq!(r.message.tool_uses().into_iter().map(|(_, name, _)| name.to_string()).collect::<Vec<_>>(), ["look", "write"]);
        }
        let mut r = turn(&several);
        react_postprocess(&mut r, &[tool("look")], false);
        assert!(r.message.tool_uses().is_empty() && r.stop == StopReason::EndTurn, "one isn't offered: none run");
        // The markers inside a call's arguments are only what it writes.
        let notes = "Writing the notes.\n```json\n{\"name\":\"write\",\"arguments\":{\"content\":\"Action: save\\nFinal Answer: done\"}}\n```";
        let mut r = turn(notes);
        react_postprocess(&mut r, &[tool("write")], false);
        assert_eq!(r.message.tool_uses().len(), 1, "{notes}");
        // A final answer that ends in an example stays the answer.
        let answer = format!("Final Answer: it's built. To list the files again:\n{}", call("look"));
        for react in [false, true] {
            let mut r = turn(&answer);
            react_postprocess(&mut r, &[tool("look")], react);
            assert!(r.message.tool_uses().is_empty() && r.stop == StopReason::EndTurn, "react={react}");
        }
        for trailing in ["### Summary:\nI listed the files.", "Action: look\nAction Input: {}", "Final Answer: I listed the files."] {
            let text = format!("{}\n{}", call("look"), trailing);
            for react in [false, true] {
                let mut r = turn(&text);
                react_postprocess(&mut r, &[tool("look")], react);
                assert!(r.message.tool_uses().is_empty(), "react={react}, trailing={trailing}");
                assert_eq!(r.stop, StopReason::PauseTurn, "react={react}, trailing={trailing}");
                assert_eq!(
                    r.follow_up.as_deref(),
                    Some(
                        "The fenced tool call in your previous response was not run because text followed the closing fence. Send the tool call again without adding a summary or other text after it."
                    ),
                    "react={react}, trailing={trailing}"
                );
            }
        }
        // Protocol text between calls must not be cleared by another fence (PR #1031).
        for marker in ["Action: look\nAction Input: {}", "Final Answer: Here's an example:"] {
            let text = format!("{}\n{marker}\n{}\nThen:\n{}", call("look"), call("write"), call("look"));
            assert!(fenced_calls(&text).is_none(), "{marker}");
            for react in [false, true] {
                let mut r = turn(&text);
                react_postprocess(&mut r, &[tool("look"), tool("write")], react);
                assert!(r.message.tool_uses().is_empty(), "react={react}, marker={marker}");
                assert_eq!(r.stop, StopReason::PauseTurn, "react={react}, marker={marker}");
                assert_eq!(
                    r.follow_up.as_deref(),
                    Some(
                        "The fenced tool call in your previous response was not run because text followed the closing fence. Send the tool call again without adding a summary or other text after it."
                    ),
                    "react={react}, marker={marker}"
                );
            }
        }
    }

    /// Only the real OpenCode Go base gets its session header: not a look-alike host, path or scheme.
    #[test]
    fn opencode_go_base_is_recognised_exactly() {
        assert!(is_opencode_go(OPENCODE_GO_BASE));
        assert!(is_opencode_go("https://opencode.ai/inference/go/openai/v1/models"));
        assert!(is_opencode_go("https://OpenCode.AI/inference/go/openai/v1"));
        assert!(!is_opencode_go("https://opencode.ai.evil.test/inference/go/v1"));
        assert!(!is_opencode_go("http://127.0.0.1:4000/opencode.ai/inference/go/v1"));
        assert!(!is_opencode_go("https://opencode.ai/inference/gopher/v1"));
        assert!(!is_opencode_go("https://opencode.ai/inference/go-v2/v1"));
        assert!(!is_opencode_go("http://opencode.ai/inference/go/v1"));
        assert!(!is_opencode_go("https://openrouter.ai/api/v1"));
    }

    #[test]
    fn json_text_calls_and_non_calls() {
        for text in [
            r#"{"name":"look","arguments":{"path":"app"}}"#,
            r#"{"name":"look","parameters":{"path":"app"}}"#,
            r#"{"name":"look","arguments":"{\"path\":\"app\"}"}"#,
            r#"{"type":"function","function":{"name":"look","arguments":{"path":"app"}}}"#,
            "```json\n{\"name\":\"look\",\"arguments\":{\"path\":\"app\"}}\n```",
            "```\n{\"name\":\"look\",\"arguments\":{\"path\":\"app\"}}\n```",
            "```json\n{\"name\":\"look\",\"arguments\":{\"path\":\"app\"}}\n````",
            "````json\n{\"name\":\"look\",\"arguments\":{\"path\":\"app\"}}\n````",
            "````\n{\"name\":\"look\",\"arguments\":{\"path\":\"app\"}}\n`````",
        ] {
            assert_eq!(parse_react(text), ReactStep::Action { tool: "look".into(), input: json!({"path": "app"}) }, "{text}");
        }
        for text in [
            r#"{"name":"look","arguments":{"path":"app"}"#,
            r#"{"name":"look","arguments":"not JSON"}"#,
            r#"{"name":"look","arguments":["app"]}"#,
            r#"{"name":"look"}"#,
            r#"{"name":"look","arguments":{},"description":"This is an example"}"#,
            r#"{"name":"look","arguments":{},"parameters":{"path":"different"}}"#,
            r#"{"type":"example","function":{"name":"look","arguments":{}}}"#,
            r#"{"name":"","arguments":{}}"#,
            r#"{"answer":"Action: look\nAction Input: {}"}"#,
            r#"[{"name":"look","arguments":{}}]"#,
            r#"For example, use {"name":"look","arguments":{}}."#,
            "```json\n{\"name\":\"look\",\"arguments\":{}}\n```\nThis is an example.",
            "```json\n{\"name\":\"look\",\"arguments\":{}}```",
            "```json`example\n{\"name\":\"look\",\"arguments\":{}}\n```",
            "````json\n{\"name\":\"look\",\"arguments\":{}}\n```",
            "```json\n{\"name\":\"look\",\"arguments\":{}}\n```\nThis is only an example.\n```",
        ] {
            assert!(matches!(parse_react(text), ReactStep::Text(_)), "{text}");
        }
        assert_eq!(
            parse_react(r#"{"name":"look","arguments":{"text":"Final Answer: is a label"}}"#),
            ReactStep::Action { tool: "look".into(), input: json!({"text":"Final Answer: is a label"}) }
        );
    }

    #[test]
    fn fenced_legacy_react_calls_still_parse() {
        for text in [
            "```\nAction: look\nAction Input: {\"path\":\"app\"}\n```",
            "```text\nAction: look\nAction Input: {\"path\":\"app\"}\n```",
            "```json\nThought: inspect the app\nAction: look\nAction Input: {\"path\":\"app\"}\n```",
            " \n```\r\nAction: look\r\nAction Input: {\"path\":\"app\"}\r\n```\n ",
            "```text\nAction: look\nAction Input: {\"path\":\"app\"}\n  ``` \t",
            "```text\nAction: look\nAction Input: {\"path\":\"app\"}\n````",
            "````text\nAction: look\nAction Input: {\"path\":\"app\"}\n````",
            "````text\nAction: look\nAction Input: ```json\n{\"path\":\"app\"}\n```\n````",
        ] {
            assert_eq!(parse_react(text), ReactStep::Action { tool: "look".into(), input: json!({"path": "app"}) }, "{text}");
        }
        for text in [
            "```json\n{\"answer\":\"Action: look\\nAction Input: {}\"}\n```",
            "```\n[\"Action: look\\nAction Input: {}\"]\n```",
            "```python\nprint(\"Action: look\\nAction Input: {}\")\n```",
            "```\nThis is an example.\nAction: look\nAction Input: {}\n```",
            "```\nAction: look\nAction Input: {}",
            "```\nAction: look\nAction Input: {}\n```\nThis is only an example.",
            "```text\nAction: look\nAction Input: {}\n```not a close```",
            "```text\nAction: look\nAction Input: {}```",
            "````text\nAction: look\nAction Input: {}\n```",
            "```text\nAction: look\nAction Input: {}\n```\nThis is only an example.\n```",
            "```text\nAction: look\nAction Input: {}\n````\nThis is only an example.\n```",
            "````text\nAction: look\nAction Input: {}\n````\nThis is only an example.\n````",
            "```json\n{\"name\":\"look\",\"arguments\":{}}```",
        ] {
            assert!(matches!(parse_react(text), ReactStep::Text(_)), "{text}");
        }
    }

    #[test]
    fn react_parsing() {
        assert_eq!(
            parse_react("Thought: look\nAction: core_session_show\nAction Input: {\"a\": \"}\"}\nmore"),
            ReactStep::Action { tool: "core_session_show".into(), input: json!({"a": "}"}) }
        );
        assert_eq!(parse_react("Action: core_targets_list\n"), ReactStep::Action { tool: "core_targets_list".into(), input: json!({}) });
        assert_eq!(parse_react("blah\nFinal Answer: it saves JSON"), ReactStep::Final("it saves JSON".into()));
        assert_eq!(parse_react("just words"), ReactStep::Text("just words".into()));
        assert_eq!(parse_react("Action: x\nAction Input: ```json\n{\"k\":1}\n```"), ReactStep::Action { tool: "x".into(), input: json!({"k": 1}) });
    }

    #[test]
    fn tool_call_accumulation() {
        let chunks = [
            json!({"model":"gpt-x","choices":[{"delta":{"role":"assistant","content":"Let me look"}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"core_session_","arguments":""}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"show","arguments":"{\"x\""}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":1,"id":"c2","function":{"name":"core_targets_list","arguments":"{}"}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":":1}"}}]}}]}),
            json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            json!({"choices":[],"usage":{"prompt_tokens":11,"completion_tokens":5}}),
        ];
        let mut st = ChatStream::default();
        let mut text = String::new();
        for c in chunks {
            st.apply(&SseEvent { event: None, data: c.to_string() }, &mut |d| {
                if let Delta::Text(t) = d {
                    text.push_str(&t)
                }
            })
            .unwrap();
        }
        st.apply(&SseEvent { event: None, data: "[DONE]".into() }, &mut |_| {}).unwrap();
        let r = st.finish("m").unwrap();
        assert_eq!(text, "Let me look");
        assert_eq!(r.stop, StopReason::ToolUse);
        assert_eq!(r.usage.input_tokens, 11);
        // #252: cached prompt tokens are part of prompt_tokens, and are counted once.
        let mut st = ChatStream::default();
        let usage = json!({"choices": [], "usage": {"prompt_tokens": 100_000, "completion_tokens": 10, "prompt_tokens_details": {"cached_tokens": 65_536}}});
        let end = json!({"choices": [{"delta": {}, "finish_reason": "stop"}]});
        for data in [end.to_string(), usage.to_string(), "[DONE]".into()] {
            st.apply(&SseEvent { event: None, data }, &mut |_| {}).unwrap();
        }
        let u = st.finish("m").unwrap().usage;
        assert_eq!((u.input_tokens, u.cache_read_tokens, u.output_tokens), (34_464, 65_536, 10));
        assert_eq!(
            r.message.tool_uses(),
            vec![("c1".into(), "core_session_show".into(), json!({"x": 1})), ("c2".into(), "core_targets_list".into(), json!({}))]
        );
    }

    #[test]
    fn native_tool_calls_preserve_terminal_stop_reasons() {
        for (finish, expected) in [
            (Some("length"), StopReason::MaxTokens),
            (Some("content_filter"), StopReason::Refusal),
            (Some("pause_turn"), StopReason::PauseTurn),
            (Some("provider_stop"), StopReason::Other("provider_stop".into())),
            (Some("tool_calls"), StopReason::ToolUse),
            (Some("stop"), StopReason::ToolUse),
            (None, StopReason::ToolUse),
        ] {
            let mut stream = ChatStream::default();
            let call = json!({"choices":[{"delta":{"tool_calls":[{
                "index":0,"id":"call_a","function":{"name":"core_session_show","arguments":"{}"}
            }]}}]});
            stream.apply(&SseEvent { event: None, data: call.to_string() }, &mut |_| {}).unwrap();
            if let Some(reason) = finish {
                let end = json!({"choices":[{"delta":{},"finish_reason":reason}]});
                stream.apply(&SseEvent { event: None, data: end.to_string() }, &mut |_| {}).unwrap();
            }
            stream.apply(&SseEvent { event: None, data: "[DONE]".into() }, &mut |_| {}).unwrap();
            let result = stream.finish("m").unwrap();
            assert_eq!(result.stop, expected, "finish_reason={finish:?}");
            assert_eq!(result.message.tool_uses(), vec![("call_a".into(), "core_session_show".into(), json!({}))]);
        }
    }

    #[test]
    fn early_stream_end_is_reported_with_remediation() {
        let st = ChatStream::default();
        let err = st.finish("m").unwrap_err();
        assert_eq!(err.code, ErrorCode::Truncated);
        assert_eq!(err.message, "The answer stream ended early.");
        let fix = err.remediation.as_deref().expect("remediation is set");
        assert!(fix.contains("loading the model"));
        assert!(fix.contains("Try again in a minute."));
    }

    /// #369: OpenRouter routes to an upstream provider per request, and one without tool calling
    /// answered "400: Provider returned error". With tools in the request, only providers that
    /// support every parameter are used; nothing else changes.
    /// #978: Gemini refuses a tool call sent back without the thought signature it came with.
    #[test]
    fn a_tool_calls_extra_content_goes_back_with_it() {
        let extra = json!({"google": {"thought_signature": "sig"}});
        let mut chat = ChatStream::default();
        for call in [
            json!({"index": 0, "id": "a", "extra_content": extra, "function": {"name": "look", "arguments": "{\"pa"}}),
            json!({"index": 0, "function": {"arguments": "th"}}),
            json!({"index": 1, "id": "b", "function": {"name": "look", "arguments": "{}"}}),
        ] {
            let data = json!({"choices": [{"delta": {"tool_calls": [call]}}]}).to_string();
            chat.apply(&SseEvent { event: None, data }, &mut |_| {}).unwrap();
        }
        let message = chat.finish("m").unwrap().message;
        let wire = messages_json(&message, false);
        let calls = wire[0]["tool_calls"].as_array().unwrap();
        assert_eq!(calls[0]["extra_content"], extra, "kept with the call whose arguments were cut off");
        assert_eq!(calls[0]["function"]["name"], "look");
        assert!(calls[1].get("extra_content").is_none(), "only on the call it came with");
    }

    #[test]
    fn gemini_uses_the_google_endpoint_with_a_bearer_key() {
        let g = OpenAi::gemini(None, "k".into(), "");
        assert_eq!(g.url("chat/completions"), format!("{GEMINI_BASE}/chat/completions"));
        assert_eq!(g.model, GEMINI_DEFAULT_MODEL);
        assert!(g.headers().contains(&("authorization", "Bearer k".to_string())));
        assert!(!g.openrouter);
    }

    #[test]
    fn openrouter_only_routes_a_tool_request_to_providers_that_support_tools() {
        use crate::model::{Effort, ToolDef, TurnRequest};
        let tools = [ToolDef { name: "look".into(), description: "d".into(), schema: json!({"type": "object"}) }];
        let messages = [Msg::user_text("hi")];
        let with_tools = TurnRequest { system: "S", messages: &messages, tools: &tools, effort: Effort::Low, max_tokens: 100 };
        let without = TurnRequest { system: "S", messages: &messages, tools: &[], effort: Effort::Low, max_tokens: 100 };
        let router = OpenAi::openrouter(None, "k".into(), "moonshotai/kimi-k3");
        assert_eq!(router.body(&with_tools)["provider"], json!({"require_parameters": true}));
        assert!(router.body(&without).get("provider").is_none(), "nothing to require without tools");
        let mut text_protocol = router.clone();
        text_protocol.react = true;
        assert!(text_protocol.body(&with_tools).get("provider").is_none(), "the text protocol sends no tools");
        let other = OpenAi::compatible("Ollama", "http://127.0.0.1:11434/v1", None, "llama3");
        assert!(other.body(&with_tools).get("provider").is_none(), "only OpenRouter knows `provider`");
        assert_eq!(router.body(&with_tools)["tool_choice"], "auto");
    }

    #[test]
    fn wire_messages() {
        let o = OpenAi::openrouter(None, "k".into(), "");
        assert!(o.headers().iter().any(|(k, _)| *k == "X-Title"));
        assert!(o.headers().iter().any(|(k, v)| *k == "authorization" && v == "Bearer k"));
        let local = OpenAi::compatible("Ollama", "http://127.0.0.1:11434/v1/", None, "llama3");
        assert!(local.headers().is_empty());
        assert_eq!(local.url("models"), "http://127.0.0.1:11434/v1/models");
        let assistant = Msg { role: Speaker::Assistant, blocks: vec![Block::ToolUse { id: "c1".into(), name: "t".into(), input: json!({"a":1}) }] };
        let results = Msg {
            role: Speaker::User,
            blocks: vec![
                Block::ToolResult { tool_use_id: "c1".into(), content: "ok".into(), is_error: false },
                Block::ToolResult { tool_use_id: "c2".into(), content: "bad".into(), is_error: true },
            ],
        };
        let msgs = vec![Msg::user_text("q"), assistant, results];
        let tools = vec![ToolDef { name: "t".into(), description: "d".into(), schema: json!({"type":"object"}) }];
        let b = local.body(&TurnRequest { system: "S", messages: &msgs, tools: &tools, effort: Effort::Low, max_tokens: 100 });
        let m = b["messages"].as_array().unwrap();
        assert_eq!(m[0]["role"], "system");
        assert_eq!(m[2]["tool_calls"][0]["function"]["arguments"], "{\"a\":1}");
        assert!(m[2].get("content").is_none());
        assert_eq!(m[3]["role"], "tool");
        assert_eq!(m[4]["tool_call_id"], "c2");
        assert_eq!(b["tool_choice"], "auto");
        let mut react = local.clone();
        react.react = true;
        let b = react.body(&TurnRequest { system: "S", messages: &msgs, tools: &tools, effort: Effort::Low, max_tokens: 100 });
        assert!(b.get("tools").is_none());
        assert!(b["messages"][0]["content"].as_str().unwrap().contains("Action Input:"));
        // A call made natively before the switch is sent as the protocol's lines, never as an empty
        // assistant message (#849).
        assert_eq!(b["messages"][2]["role"], "assistant");
        assert_eq!(b["messages"][2]["content"], "Action: t\nAction Input: {\"a\":1}");
        assert!(b["messages"][3]["content"].as_str().unwrap().contains("Observation (error): bad"));
    }

    #[test]
    fn tool_only_assistant_omits_content_key() {
        let tool_only = Msg { role: Speaker::Assistant, blocks: vec![Block::ToolUse { id: "c1".into(), name: "look".into(), input: json!({"path": "."}) }] };
        let m = messages_json(&tool_only, false);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0]["role"], "assistant");
        assert!(m[0].get("content").is_none(), "tool-only assistant must omit empty content");
        assert_eq!(m[0]["tool_calls"][0]["function"]["name"], "look");
    }

    #[test]
    fn empty_assistant_message_is_omitted() {
        let empty = Msg { role: Speaker::Assistant, blocks: vec![] };
        assert!(messages_json(&empty, false).is_empty(), "empty assistant turn must produce no wire messages");
    }

    #[test]
    fn blank_assistant_message_is_omitted_in_both_protocols() {
        let blank = Msg { role: Speaker::Assistant, blocks: vec![Block::Text { text: " \n".into() }] };
        let empty = Msg { role: Speaker::Assistant, blocks: vec![] };
        for react in [false, true] {
            assert!(messages_json(&blank, react).is_empty(), "react={react}");
            assert!(messages_json(&empty, react).is_empty(), "react={react}");
        }
        // Blank text beside a tool call is left out, the call kept.
        let tool = Msg {
            role: Speaker::Assistant,
            blocks: vec![Block::Text { text: "  ".into() }, Block::ToolUse { id: "c1".into(), name: "look".into(), input: json!({}) }],
        };
        let m = messages_json(&tool, false);
        assert!(m.len() == 1 && m[0].get("content").is_none() && m[0]["tool_calls"].is_array(), "{m:?}");
        assert_eq!(messages_json(&tool, true)[0]["content"], "Action: look\nAction Input: {}");
        // A turn written in the protocol already keeps its own lines, once.
        let written = Msg {
            role: Speaker::Assistant,
            blocks: vec![
                Block::Text { text: "Thought: look\nAction: look\nAction Input: {}".into() },
                Block::ToolUse { id: "react_0".into(), name: "look".into(), input: json!({}) },
            ],
        };
        assert_eq!(messages_json(&written, true)[0]["content"], "Thought: look\nAction: look\nAction Input: {}");
        // Text naming one call beside a native call of another keeps both, so no result arrives
        // without its action; a second call to the same tool is written too.
        let mixed = Msg {
            role: Speaker::Assistant,
            blocks: vec![
                Block::Text { text: "Action: look\nAction Input: {}".into() },
                Block::ToolUse { id: "react_0".into(), name: "look".into(), input: json!({}) },
                Block::ToolUse { id: "c2".into(), name: "read".into(), input: json!({"p": 1}) },
                Block::ToolUse { id: "c3".into(), name: "look".into(), input: json!({"deep": true}) },
            ],
        };
        assert_eq!(
            messages_json(&mixed, true)[0]["content"],
            "Action: look\nAction Input: {}\nAction: read\nAction Input: {\"p\":1}\nAction: look\nAction Input: {\"deep\":true}"
        );
        // In a whole request (text protocol), the user turns either side are joined into one.
        let user = |t: &str| Msg { role: Speaker::User, blocks: vec![Block::Text { text: t.into() }] };
        let mut o = OpenAi::compatible("Kimi", "http://127.0.0.1:9", Some("sk-test".into()), "kimi");
        o.react = true;
        let msgs = [user("first"), blank, user("second")];
        let b = o.body(&TurnRequest { system: "s", messages: &msgs, tools: &[], effort: Effort::Low, max_tokens: 100 });
        assert_eq!(b["messages"].as_array().map(Vec::len), Some(2), "system, then the joined user turn");
        let wire = b["messages"].as_array().unwrap();
        assert!(wire.iter().all(|m| m["role"] != "assistant"), "{wire:?}");
        assert!(wire.iter().all(|m| m["content"].as_str().is_some_and(|c| !c.trim().is_empty())), "{wire:?}");
    }

    #[test]
    fn text_only_assistant_retains_content_key() {
        let text_only = Msg { role: Speaker::Assistant, blocks: vec![Block::Text { text: "Hello".into() }] };
        let m = messages_json(&text_only, false);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0]["role"], "assistant");
        assert_eq!(m[0]["content"], "Hello");
        assert!(m[0].get("tool_calls").is_none());
    }

    #[test]
    fn text_and_tools_assistant_retains_both_content_and_tools() {
        let text_and_tool = Msg {
            role: Speaker::Assistant,
            blocks: vec![
                Block::Text { text: "Checking directory".into() },
                Block::ToolUse { id: "c2".into(), name: "look".into(), input: json!({"path": "."}) },
            ],
        };
        let m = messages_json(&text_and_tool, false);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0]["role"], "assistant");
        assert_eq!(m[0]["content"], "Checking directory");
        assert_eq!(m[0]["tool_calls"][0]["id"], "c2");
    }

    #[test]
    fn dropped_empty_assistant_merges_adjacent_user_text_messages() {
        let client = OpenAi::compatible("test", "http://127.0.0.1", None, "test-model");
        let msgs = vec![Msg::user_text("First instruction"), Msg { role: Speaker::Assistant, blocks: vec![] }, Msg::user_text("Second instruction")];
        let b = client.body(&TurnRequest { system: "S", messages: &msgs, tools: &[], effort: Effort::Low, max_tokens: 100 });
        let m = b["messages"].as_array().unwrap();
        assert_eq!(m.len(), 2, "system + 1 merged user message expected, got: {m:?}");
        assert_eq!(m[0]["role"], "system");
        assert_eq!(m[1]["role"], "user");
        assert_eq!(m[1]["content"], "First instruction\n\nSecond instruction");
    }

    #[test]
    fn non_empty_assistant_keeps_user_messages_distinct() {
        let client = OpenAi::compatible("test", "http://127.0.0.1", None, "test-model");
        let msgs = vec![
            Msg::user_text("First instruction"),
            Msg { role: Speaker::Assistant, blocks: vec![Block::Text { text: "Acknowledged".into() }] },
            Msg::user_text("Second instruction"),
        ];
        let b = client.body(&TurnRequest { system: "S", messages: &msgs, tools: &[], effort: Effort::Low, max_tokens: 100 });
        let m = b["messages"].as_array().unwrap();
        assert_eq!(m.len(), 4, "system + user + assistant + user expected: {m:?}");
        assert_eq!(m[1]["content"], "First instruction");
        assert_eq!(m[2]["content"], "Acknowledged");
        assert_eq!(m[3]["content"], "Second instruction");
    }

    #[test]
    fn adjacent_user_messages_without_dropped_assistant_stay_separate() {
        let client = OpenAi::compatible("test", "http://127.0.0.1", None, "test-model");
        let msgs = vec![Msg::user_text("First instruction"), Msg::user_text("Second instruction")];
        let b = client.body(&TurnRequest { system: "S", messages: &msgs, tools: &[], effort: Effort::Low, max_tokens: 100 });
        let m = b["messages"].as_array().unwrap();
        assert_eq!(m.len(), 3, "system + 2 separate user messages expected: {m:?}");
        assert_eq!(m[1]["content"], "First instruction");
        assert_eq!(m[2]["content"], "Second instruction");
    }
}
