//! HTTP plumbing: one ureq agent (rustls), JSON POST with streamed responses, and an SSE parser.

use craft_anything_evidence::{ErrorCode, ToolError};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read};
use std::time::Duration;
mod cancel;

/// Generic, non-identifying User-Agent.
pub const USER_AGENT: &str = "CraftAnything-dev";

/// An agent for model calls: generous body time (streams can run for many minutes), bounded
/// connect and first-byte waits.
pub fn agent(connect: Duration, first_byte: Duration, body: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .user_agent(USER_AGENT)
        .timeout_connect(Some(connect))
        .timeout_recv_response(Some(first_byte))
        .timeout_recv_body(Some(body))
        .build()
        .new_agent()
}

/// How long a streamed answer may take unless its AI sets `timeout_secs`.
pub const STREAM_BODY: Duration = Duration::from_secs(60 * 60);
/// How long to wait for a streamed answer to start unless its AI sets `timeout_secs`.
pub const STREAM_FIRST_BYTE: Duration = Duration::from_secs(300);

/// The agent for streamed model turns. `limit` is the AI's own `timeout_secs`: it bounds the wait
/// for the answer to start as well as the whole answer, as a request queued behind another tool's
/// on a shared server gets no bytes until its turn (#1239). Without one, the defaults apply.
pub fn stream_agent(limit: Option<Duration>) -> ureq::Agent {
    agent(Duration::from_secs(15), limit.unwrap_or(STREAM_FIRST_BYTE), limit.unwrap_or(STREAM_BODY))
}

/// A streamed model request whose live socket closes when its conversation stops.
pub(crate) fn stream_agent_with_stop(limit: Option<Duration>, stop: &crate::model::Stop) -> Result<ureq::Agent, ToolError> {
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .user_agent(USER_AGENT)
        .timeout_resolve(Some(Duration::from_secs(15)))
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(limit.unwrap_or(STREAM_FIRST_BYTE)))
        .timeout_recv_body(Some(limit.unwrap_or(STREAM_BODY)))
        .build();
    cancel::agent(config, stop)
}

/// Whether a request's error says the server took it and didn't answer in time: its address is
/// right, and it may still be starting or loading a model.
pub fn was_reached(e: &ToolError) -> bool {
    e.details.as_ref().and_then(|d| d.get("reached")).and_then(Value::as_bool).unwrap_or(false)
}

/// The agent for quick calls (model lists, probes).
pub fn quick_agent(total: Duration) -> ureq::Agent {
    agent(total, total, total)
}

/// A request that failed before any answer came. A server on this computer (Ollama, LM Studio) is
/// told apart: the internet has nothing to do with it, and one that took the request but didn't
/// start answering was reached, so "Couldn't reach" misled (a large model still loading, #1239).
fn net_error(what: &str, url: &str, e: ureq::Error) -> ToolError {
    let local = is_this_computer(url);
    const LONGER: &str = "To wait longer, set timeout_secs (in seconds) for this AI under [[ai.backends]] in config.toml; the wait is five minutes unless set.";
    if matches!(e, ureq::Error::Timeout(ureq::Timeout::RecvResponse)) {
        let why = if local {
            "A large model may still be loading, or reading a long prompt: a smaller context length in the server's settings makes both faster."
        } else {
            "A server busy with another tool's work may need longer."
        };
        // `reached`: the server took the request, so its address is right (#1429).
        return ToolError::new(ErrorCode::TimedOut, format!("{what} took the request but didn't start answering in time."))
            .with_remediation(format!("{why} {LONGER}"))
            .with_details(serde_json::json!({ "reached": true }));
    }
    let code = if matches!(e, ureq::Error::Timeout(_)) { ErrorCode::TimedOut } else { ErrorCode::EngineMissing };
    let remediation = if local {
        "Check that the server is running on this computer at this AI's address, then try again."
    } else {
        "Check your internet connection or that the server is running, then try again."
    };
    ToolError::new(code, format!("Couldn't reach {what}: {e}")).with_remediation(remediation)
}

/// Whether `url` points at this computer: `localhost`, a `127.x.x.x` address or `::1`.
fn is_this_computer(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = match host_port.strip_prefix('[') {
        // `[::1]:11434`
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => host_port.rsplit_once(':').map_or(host_port, |(h, _)| h),
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "localhost" || host.ends_with(".localhost") || host == "::1" || host.parse::<std::net::Ipv4Addr>().is_ok_and(|ip| ip.is_loopback())
}

/// Turn a non-2xx response into a plain error. Keys are never echoed.
pub fn status_error(what: &str, status: u16, body: &str) -> ToolError {
    // Gemini's OpenAI-compatible endpoint wraps its error in a one-item array.
    let parsed = serde_json::from_str::<Value>(body).ok().map(|v| if let Value::Array(a) = &v { a.first().cloned().unwrap_or(v) } else { v });
    let full = parsed.as_ref().map(|value| response_detail(value).unwrap_or_else(|| "The service didn't provide an error message.".into()));
    // The window is read from the whole message: the shortened one may cut it off.
    let overflow = context_overflow(full.as_deref().unwrap_or(body));
    // Ollama fits a long prompt to its context by dropping the oldest messages; once the person's
    // request is among them, a Qwen 3.8 model refuses what's left with this, and no retry fits (#1245).
    let lower = full.as_deref().unwrap_or(body).to_ascii_lowercase();
    let dropped_request = overflow.is_none() && lower.contains("no user query found in messages");
    let detail: String = full.unwrap_or_else(|| body.chars().take(300).collect()).chars().take(1000).collect();
    let (code, remediation) = match status {
        _ if dropped_request => (ErrorCode::BadSettings, ""),
        401 | 403 => (ErrorCode::NoPermission, "The API key was rejected. Check it in Settings → AI."),
        404 => (ErrorCode::BadSettings, "The model or address wasn't found. Check the model name and base URL."),
        408 | 504 => (ErrorCode::TimedOut, "The service took too long. Try again."),
        429 => (ErrorCode::OverLimit, "You've hit a rate or spending limit. Wait a moment or check your plan."),
        500..=599 => (ErrorCode::EngineMissing, "The service is having trouble. Try again soon, or use another backend."),
        _ => (ErrorCode::BadInput, "The request was rejected; see the message."),
    };
    let provider = parsed
        .as_ref()
        .and_then(|v| v.pointer("/error/code").and_then(Value::as_str).or_else(|| v.pointer("/error/type").and_then(Value::as_str)).map(str::to_string));
    const RAISE: &str = "Give the model a larger context in its server (Ollama: the Context length setting or OLLAMA_CONTEXT_LENGTH; \
                         LM Studio: the model's Context Length), or set context_tokens for this AI in Settings → AI so less is sent.";
    let remediation = match overflow {
        Some((window, _)) => format!("The conversation is longer than the model's context window ({window} tokens). {RAISE}"),
        None if dropped_request => format!(
            "The conversation is longer than the context the server gives this model, so the server left out its earliest \
             messages, the request among them. {RAISE}"
        ),
        None => remediation.to_string(),
    };
    ToolError::new(code, format!("{what} answered {status}: {detail}")).with_remediation(remediation).with_details(serde_json::json!({
        "status": status,
        "provider_error": provider,
        "context_window": overflow.map(|o| o.0),
        "prompt_tokens": overflow.and_then(|o| o.1),
        "context_overflow": overflow.is_some() || dropped_request,
        "affordable_tokens": if status == 402 { affordable_tokens(&lower) } else { None },
    }))
}

/// The answer length OpenRouter says a key can still pay for: "You requested up to 131072 tokens,
/// but can only afford 35063" (#1213).
fn affordable_tokens(lower: &str) -> Option<u64> {
    let (_, rest) = lower.split_once("can only afford ")?;
    rest.chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok()
}

/// The context window a server says a request didn't fit in, and the request's size when it says
/// that too (#962). Local servers word it as llama.cpp does ("request (69944 tokens) exceeds the
/// available context size (65536 tokens)"), OpenAI-style ones as "maximum context length is 8192
/// tokens ... you requested 9000 tokens (8000 in the messages, 1000 in the completion)". The size
/// is the prompt's: OpenAI's total counts the answer it asked room for, so only the part "in the
/// messages" is taken, and none when it isn't given.
pub fn context_overflow(message: &str) -> Option<(u64, Option<u64>)> {
    let m = message.to_ascii_lowercase();
    let number = |text: &str| -> Option<u64> {
        let digits: String = text.trim_start().chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok().filter(|n| *n > 0)
    };
    let after = |marker: &str| m.split_once(marker).and_then(|(_, rest)| number(rest));
    let window = after("available context size (").or_else(|| after("maximum context length is"))?;
    let in_messages = || {
        let (_, requested) = m.split_once("requested")?;
        let (_, inner) = requested.split_once('(')?;
        inner.split_once(" in the messages").and_then(|(n, _)| number(n))
    };
    Some((window, after("request (").or_else(in_messages)))
}

/// Read only the human-readable fields, never serialise an arbitrary provider payload.
fn error_message<'a>(value: &'a Value, outer: Option<&str>) -> Option<&'a str> {
    let mut messages = [
        value.as_str(),
        value.pointer("/error/message").and_then(Value::as_str),
        value.get("message").and_then(Value::as_str),
        value.get("error").and_then(Value::as_str),
        value.get("detail").and_then(Value::as_str),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .filter(|message| !message.is_empty());
    let first = messages.next()?;
    // A repeated outer message must not hide a distinct detail later in the payload.
    if Some(first) == outer { messages.find(|message| Some(*message) != outer).or(Some(first)) } else { Some(first) }
}

/// Unwrap a bounded number of JSON strings before selecting known message fields.
fn provider_reason(raw: &Value, outer: Option<&str>, remaining: u8) -> Option<String> {
    let Some(text) = raw.as_str() else { return error_message(raw, outer).map(str::to_string) };
    match serde_json::from_str::<Value>(text) {
        Ok(parsed) if remaining > 0 => provider_reason(&parsed, outer, remaining - 1),
        Ok(_) => None,
        // Broken or excessively wrapped structured metadata must not become a plain message.
        Err(_) if text.trim_start().starts_with(['{', '[', '"']) => None,
        Err(_) => error_message(raw, outer).map(str::to_string),
    }
}

fn response_detail(value: &Value) -> Option<String> {
    let outer = error_message(value, None);
    let reason = value.pointer("/error/metadata/raw").and_then(|raw| provider_reason(raw, outer, 4));
    let Some(reason) = reason else { return outer.map(str::to_string) };
    let provider = value.pointer("/error/metadata/provider_name").and_then(Value::as_str).map(str::trim).filter(|name| !name.is_empty());
    let detail = match provider {
        Some(name) => format!("{}: {reason}", name.chars().take(80).collect::<String>()),
        None => reason.clone(),
    };
    match outer {
        Some(message) if message != reason => Some(format!("{}: {detail}", message.chars().take(300).collect::<String>())),
        _ => Some(detail),
    }
}

/// POST JSON and return a reader over the (streamed) body; errors on non-2xx.
pub fn post_json(agent: &ureq::Agent, what: &str, url: &str, headers: &[(&str, String)], body: &Value) -> Result<Box<dyn Read + Send>, ToolError> {
    let mut req = agent.post(url).header("content-type", "application/json");
    for (k, v) in headers {
        req = req.header(*k, v.as_str());
    }
    let resp = req.send(body.to_string()).map_err(|e| net_error(what, url, e))?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        let retry_after = resp.headers().get("retry-after").and_then(|h| h.to_str().ok()).and_then(|s| s.trim().parse::<u64>().ok());
        let text = resp.into_body().read_to_string().unwrap_or_default();
        let mut error = status_error(what, status, &text);
        if let (Some(seconds), Some(Value::Object(details))) = (retry_after, error.details.as_mut()) {
            details.insert("retry_after_secs".into(), Value::from(seconds.min(crate::failure::MAX_RETRY_WAIT)));
        }
        return Err(error);
    }
    Ok(Box::new(resp.into_body().into_reader()))
}

/// GET JSON.
pub fn get_json(agent: &ureq::Agent, what: &str, url: &str, headers: &[(&str, String)]) -> Result<Value, ToolError> {
    let mut req = agent.get(url);
    for (k, v) in headers {
        req = req.header(*k, v.as_str());
    }
    let resp = req.call().map_err(|e| net_error(what, url, e))?;
    let status = resp.status().as_u16();
    let text = resp.into_body().read_to_string().map_err(|e| net_error(what, url, e))?;
    if !(200..300).contains(&status) {
        return Err(status_error(what, status, &text));
    }
    serde_json::from_str(&text).map_err(|e| ToolError::new(ErrorCode::GarbledOutput, format!("{what} sent JSON we couldn't read: {e}")))
}

/// One server-sent event.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// Longest single SSE line we accept (protects memory against a hostile server).
const MAX_SSE_LINE: usize = 8 * 1024 * 1024;
/// Most data retained for one event, including newlines between its `data:` fields.
const MAX_SSE_EVENT_DATA: usize = 8 * 1024 * 1024;

/// A stream that stopped mid-answer. One that ran out of time says how to give it more (#1096).
fn stream_broke_off(e: std::io::Error) -> ToolError {
    let e = ureq::Error::from(e);
    let err = ToolError::new(ErrorCode::EngineMissing, format!("The stream broke off: {e}"));
    match e {
        ureq::Error::Timeout(ureq::Timeout::RecvBody) => err.with_remediation(
            "The answer took longer than this AI's time limit (one hour unless set). A slow local model may need longer: set \
             timeout_secs (in seconds) for this AI under [[ai.backends]] in config.toml.",
        ),
        _ => err,
    }
}

/// Parse an SSE stream, calling `f` for each event. `f` returns false to stop early.
/// Handles CRLF, multi-line `data:`, comments (`:`), and a final event without a blank line.
pub fn parse_sse<R: Read>(r: R, mut f: impl FnMut(SseEvent) -> Result<bool, ToolError>) -> Result<(), ToolError> {
    let mut reader = BufReader::new(r);
    let mut cur = SseEvent::default();
    let mut has_data = false;
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = (&mut reader).take(MAX_SSE_LINE as u64).read_until(b'\n', &mut line).map_err(stream_broke_off)?;
        if n == 0 {
            break;
        }
        if line.last() != Some(&b'\n') && line.len() >= MAX_SSE_LINE {
            return Err(ToolError::new(ErrorCode::OverLimit, "The stream sent a line that was too long."));
        }
        let text = String::from_utf8_lossy(&line);
        let l = text.trim_end_matches(['\n', '\r']);
        if l.is_empty() {
            if has_data || cur.event.is_some() {
                let ev = std::mem::take(&mut cur);
                has_data = false;
                if !f(ev)? {
                    return Ok(());
                }
            }
            continue;
        }
        if l.starts_with(':') {
            continue;
        }
        let (field, value) = match l.split_once(':') {
            Some((k, v)) => (k, v.strip_prefix(' ').unwrap_or(v)),
            None => (l, ""),
        };
        match field {
            "event" => cur.event = Some(value.to_string()),
            "data" => {
                let size = cur.data.len().saturating_add(usize::from(has_data)).saturating_add(value.len());
                if size > MAX_SSE_EVENT_DATA {
                    return Err(ToolError::new(ErrorCode::OverLimit, "The stream sent an event whose data was too large."));
                }
                if has_data {
                    cur.data.push('\n');
                }
                cur.data.push_str(value);
                has_data = true;
            }
            _ => {}
        }
    }
    if has_data {
        f(cur)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #1096: a stream that ran out of time says which setting gives it more; another break doesn't.
    #[test]
    fn a_stream_that_ran_out_of_time_says_how_to_give_it_more() {
        struct Fails(fn() -> ureq::Error);
        impl Read for Fails {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err((self.0)().into_io())
            }
        }
        let timed_out = parse_sse(Fails(|| ureq::Error::Timeout(ureq::Timeout::RecvBody)), |_| Ok(true)).unwrap_err();
        assert!(timed_out.message.contains("timeout"), "{}", timed_out.message);
        assert!(timed_out.remediation.as_deref().is_some_and(|r| r.contains("timeout_secs")), "{timed_out:?}");
        let cut = parse_sse(Fails(|| ureq::Error::ConnectionFailed), |_| Ok(true)).unwrap_err();
        assert!(cut.remediation.is_none(), "{cut:?}");
    }

    /// #1239: an answer that never started (a server busy with another tool's work) says which
    /// setting gives it longer; a server that can't be reached still says to check it.
    #[test]
    fn an_answer_that_never_started_says_how_to_wait_longer() {
        for url in ["http://127.0.0.1:11434/v1/chat/completions", "https://ollama.example.com/v1/chat/completions"] {
            let queued = net_error("Ollama", url, ureq::Error::Timeout(ureq::Timeout::RecvResponse));
            assert_eq!(queued.code, ErrorCode::TimedOut);
            assert!(queued.remediation.as_deref().is_some_and(|r| r.contains("timeout_secs")), "{queued:?}");
            // It was reached: not "Couldn't reach", and nothing about the internet.
            assert_eq!(queued.message, "Ollama took the request but didn't start answering in time.");
            assert!(!queued.remediation.as_deref().unwrap_or_default().contains("internet"), "{queued:?}");
            let down = net_error("Ollama", url, ureq::Error::ConnectionFailed);
            assert!(down.message.starts_with("Couldn't reach Ollama"), "{down:?}");
            assert!(down.remediation.as_deref().is_some_and(|r| r.contains("server is running")), "{down:?}");
        }
    }

    /// An AI on this computer: a model may still be loading, and the internet isn't involved.
    #[test]
    fn a_server_on_this_computer_is_not_told_to_check_the_internet() {
        let local = "http://localhost:11434/v1/chat/completions";
        let queued = net_error("Ollama (local)", local, ureq::Error::Timeout(ureq::Timeout::RecvResponse));
        let advice = queued.remediation.unwrap_or_default();
        assert!(advice.contains("still be loading") && advice.contains("context length") && advice.contains("timeout_secs"), "{advice}");
        for e in [ureq::Error::ConnectionFailed, ureq::Error::Timeout(ureq::Timeout::Connect)] {
            let down = net_error("Ollama (local)", local, e);
            let advice = down.remediation.unwrap_or_default();
            assert!(advice.contains("on this computer") && !advice.contains("internet"), "{advice}");
        }
        let remote = net_error("OpenRouter", "https://openrouter.ai/api/v1/chat/completions", ureq::Error::ConnectionFailed);
        assert!(remote.remediation.unwrap_or_default().contains("internet connection"));
        let remote = net_error("Ollama", "https://ollama.example.com/v1", ureq::Error::Timeout(ureq::Timeout::RecvResponse));
        assert!(!remote.remediation.unwrap_or_default().contains("still be loading"), "a remote server gets the queue advice");
    }

    #[test]
    fn loopback_addresses_are_this_computer() {
        for url in [
            "http://localhost:11434/v1",
            "http://LOCALHOST/api",
            "http://127.0.0.1:1234",
            "http://127.1.2.3:8080/x",
            "http://[::1]:11434/v1",
            "http://user:pw@localhost:11434",
            "http://ollama.localhost:11434",
            "localhost:11434",
        ] {
            assert!(is_this_computer(url), "{url}");
        }
        for url in [
            "https://api.openai.com/v1",
            "http://192.168.1.20:11434/v1",
            "http://localhost.example.com/v1",
            "http://[::2]:11434",
            "http://example.com/?next=http://localhost",
            "http://127.0.0.1.example.com",
            "",
        ] {
            assert!(!is_this_computer(url), "{url}");
        }
    }

    fn collect(s: &str) -> Vec<SseEvent> {
        let mut v = Vec::new();
        parse_sse(s.as_bytes(), |e| {
            v.push(e);
            Ok(true)
        })
        .unwrap();
        v
    }

    #[test]
    fn a_full_context_window_is_read_from_the_servers_words() {
        let ollama = "request (69944 tokens) exceeds the available context size (65536 tokens), try increasing it";
        assert_eq!(context_overflow(ollama), Some((65536, Some(69944))));
        // OpenAI's "requested" counts the answer's room too: only the part in the messages is the prompt.
        let openai = "This model's maximum context length is 8192 tokens. However, you requested 9000 tokens (8000 in the messages, 1000 in the completion).";
        assert_eq!(context_overflow(openai), Some((8192, Some(8000))));
        assert_eq!(context_overflow("maximum context length is 8192 tokens. However, you requested 9000 tokens."), Some((8192, None)));
        assert_eq!(context_overflow("maximum context length is 4096 tokens"), Some((4096, None)));
        for other in ["Invalid request: the message at position 64 must not be empty", "context size (abc tokens)", "", "available context size ("] {
            assert_eq!(context_overflow(other), None, "{other:?}");
        }
        let body = serde_json::json!({"error": {"message": ollama}}).to_string();
        let e = status_error("Ollama (local)", 400, &body);
        let details = e.details.unwrap();
        assert_eq!((details["context_window"].as_u64(), details["prompt_tokens"].as_u64()), (Some(65536), Some(69944)));
        assert!(e.remediation.unwrap().contains("OLLAMA_CONTEXT_LENGTH"));
        // A long message is read whole, though only its first 1000 characters are shown.
        let long = format!("{} {ollama}", "x".repeat(2000));
        let e = status_error("Ollama (local)", 400, &serde_json::json!({"error": {"message": long}}).to_string());
        assert!(!e.message.contains("65536"), "the shown message is shortened");
        assert_eq!(e.details.unwrap()["context_window"].as_u64(), Some(65536));
        // The shown part ends in "(655": the last ten characters, "36 tokens)", are cut.
        let tail = "request (69944 tokens) exceeds the available context size (65536 tokens)";
        let cut = format!("{}{tail}", "x".repeat(1000 + 10 - tail.len()));
        let e = status_error("Ollama (local)", 400, &serde_json::json!({"error": {"message": cut}}).to_string());
        assert_eq!(e.details.unwrap()["context_window"].as_u64(), Some(65536), "not the number cut short");
        let plain = status_error("Ollama (local)", 400, r#"{"error":{"message":"bad"}}"#);
        assert!(plain.details.unwrap()["context_window"].is_null());
        assert_eq!(plain.remediation.as_deref(), Some("The request was rejected; see the message."));
    }

    #[test]
    fn a_request_the_server_dropped_to_fit_is_a_context_overflow_not_a_server_fault() {
        // #1245: Ollama's own words when its truncation left a Qwen 3.8 model no user message.
        let e = status_error("Ollama (local)", 500, r#"{"error":{"message":"no user query found in messages"}}"#);
        assert_eq!(e.code, ErrorCode::BadSettings);
        assert!(e.message.contains("no user query found in messages"), "the server's words are kept");
        let remediation = e.remediation.clone().unwrap();
        assert!(remediation.contains("OLLAMA_CONTEXT_LENGTH") && !remediation.contains("Try again soon"), "{remediation}");
        let details = e.details.clone().unwrap();
        assert_eq!(details["context_overflow"], true);
        assert!(details["context_window"].is_null(), "the server didn't say its size");
        assert!(!crate::TurnFailure::from_api(&e).retryable(), "the same request would be refused again");
        // A window the server names still counts as an overflow, and a plain 500 is still worth retrying.
        let sized = status_error("Ollama (local)", 500, r#"{"error":{"message":"request (9000 tokens) exceeds the available context size (8192 tokens)"}}"#);
        assert!(!crate::TurnFailure::from_api(&sized).retryable());
        let busy = status_error("Ollama (local)", 500, r#"{"error":{"message":"model runner has unexpectedly stopped"}}"#);
        assert_eq!(busy.details.clone().unwrap()["context_overflow"], false);
        assert!(crate::TurnFailure::from_api(&busy).retryable());
    }

    #[test]
    fn sse_edge_cases() {
        let evs = collect(": comment\r\nevent: a\r\ndata: 1\r\ndata: 2\r\n\r\ndata:x\n\n\n\ndata: tail");
        assert_eq!(evs.len(), 3);
        assert_eq!(evs[0], SseEvent { event: Some("a".into()), data: "1\n2".into() });
        assert_eq!(evs[1].data, "x");
        assert_eq!(evs[2].data, "tail");
    }

    #[test]
    fn early_stop() {
        let mut n = 0;
        parse_sse("data: 1\n\ndata: 2\n\n".as_bytes(), |_| {
            n += 1;
            Ok(false)
        })
        .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn sse_event_data_is_bounded_across_lines() {
        let first = "é".repeat(2 * 1024 * 1024);
        let full = format!("data:{first}\ndata:{}\n", "x".repeat(4 * 1024 * 1024 - 1));
        for stream in [format!("data:{first}\ndata:{first}\n\n"), format!("data:{first}\ndata:{first}"), format!("{full}data:\n\n")] {
            let mut emitted = 0;
            let error = parse_sse(stream.as_bytes(), |_| {
                emitted += 1;
                Ok(true)
            })
            .unwrap_err();
            assert_eq!(error.code, ErrorCode::OverLimit);
            assert_eq!(emitted, 0, "an oversized event must not reach the provider parser");
            assert!(!error.message.contains('é'), "the error must not echo response data");
        }
    }

    #[test]
    fn sse_event_data_limit_preserves_boundaries_and_eof() {
        let first = "é".repeat(2 * 1024 * 1024);
        let full = format!("data:{first}\r\ndata:{}\r\n", "x".repeat(4 * 1024 * 1024 - 1));
        let stream = format!("{full}\r\ndata:\r\n\r\n{full}");
        let mut lengths = Vec::new();
        parse_sse(stream.as_bytes(), |event| {
            lengths.push(event.data.len());
            Ok(true)
        })
        .unwrap();
        assert_eq!(lengths, [8 * 1024 * 1024, 0, 8 * 1024 * 1024]);
    }

    #[test]
    fn provider_errors_include_the_nested_reason() {
        let reason = "The selected model does not accept this request parameter.";
        for raw in [
            serde_json::json!({"error": {"message": reason}}),
            serde_json::json!({"message": reason}),
            serde_json::json!({"detail": reason}),
            serde_json::json!({"error": {"message": " \t "}, "message": "\n", "detail": reason}),
            serde_json::json!({"error": reason}),
            serde_json::json!(serde_json::json!({"error": {"message": reason}}).to_string()),
            serde_json::json!(reason),
        ] {
            let body = serde_json::json!({"error": {
                "message": "Provider returned error",
                "metadata": {"provider_name": "Example provider", "raw": raw}
            }});
            let error = status_error("OpenRouter", 400, &body.to_string());
            assert_eq!(error.code, ErrorCode::BadInput);
            assert_eq!(error.details.as_ref().unwrap()["status"], 400);
            assert!(error.message.contains("Provider returned error"), "{error:?}");
            assert!(error.message.contains(reason), "{error:?}");
            assert!(error.message.contains("Example provider"), "{error:?}");
            assert_eq!(error.remediation.as_deref(), Some("The request was rejected; see the message."));
        }
    }

    #[test]
    fn provider_review_prefers_a_distinct_nested_reason() {
        let outer = "Provider returned error";
        let reason = "The selected model does not accept this request parameter.";
        for raw in [
            serde_json::json!({"message": outer, "detail": reason}),
            serde_json::json!({"error": {"message": outer}, "message": outer, "detail": reason}),
            serde_json::json!(serde_json::json!({"message": outer, "detail": reason}).to_string()),
        ] {
            let body = serde_json::json!({"error": {"message": outer, "metadata": {"provider_name": "Example provider", "raw": raw}}});
            let error = status_error("OpenRouter", 400, &body.to_string());
            assert!(error.message.contains(reason), "{error:?}");
            assert_eq!(error.message.matches(outer).count(), 1, "{error:?}");
            assert!(error.message.contains("Example provider"), "{error:?}");
        }
        let body = serde_json::json!({"error": {"message": outer, "metadata": {"provider_name": "Example provider", "raw": {"message": outer}}}});
        assert_eq!(status_error("OpenRouter", 400, &body.to_string()).message, format!("OpenRouter answered 400: Example provider: {outer}"));
    }

    #[test]
    fn provider_review_decodes_raw_strings_without_dumping_metadata() {
        let outer = "Provider returned error";
        let mut raw = serde_json::json!({"headers": {"authorization": "Bearer sk-test"}});
        for _ in 0..8 {
            let body = serde_json::json!({"error": {"message": outer, "metadata": {"raw": raw}}});
            let error = status_error("OpenRouter", 400, &body.to_string());
            assert_eq!(error.message, format!("OpenRouter answered 400: {outer}"));
            raw = serde_json::json!(raw.to_string());
        }
        let reason = "The model is temporarily unavailable.";
        let mut raw = serde_json::json!({"message": reason, "headers": {"authorization": "Bearer sk-test"}});
        for _ in 0..4 {
            let body = serde_json::json!({"error": {"message": outer, "metadata": {"raw": raw}}});
            let error = status_error("OpenRouter", 400, &body.to_string());
            assert!(error.message.contains(reason), "{error:?}");
            assert!(!error.message.contains("headers") && !error.message.contains("sk-test"), "{error:?}");
            raw = serde_json::json!(raw.to_string());
        }
        let raw = serde_json::to_string(reason).unwrap();
        let body = serde_json::json!({"error": {"message": outer, "metadata": {"raw": raw}}});
        assert!(status_error("OpenRouter", 400, &body.to_string()).message.contains(reason));
        let raw = serde_json::to_string(r#"{"headers":{"authorization":"Bearer sk-test""#).unwrap();
        let body = serde_json::json!({"error": {"message": outer, "metadata": {"raw": raw}}});
        assert_eq!(status_error("OpenRouter", 400, &body.to_string()).message, format!("OpenRouter answered 400: {outer}"));
    }

    #[test]
    fn provider_details_are_bounded_and_do_not_dump_metadata() {
        let outer = "Provider returned error";
        for raw in [
            serde_json::json!({"headers": {"authorization": "Bearer sk-test"}}),
            serde_json::json!(r#"{"headers":{"authorization":"Bearer sk-test"}}"#),
            serde_json::json!(r#"{"headers":{"authorization":"Bearer sk-test""#),
            Value::Null,
            serde_json::json!(" "),
        ] {
            let body = serde_json::json!({"error": {"message": outer, "metadata": {"raw": raw}}});
            let error = status_error("OpenRouter", 400, &body.to_string());
            assert_eq!(error.message, format!("OpenRouter answered 400: {outer}"));
        }
        let reason = "é".repeat(2000);
        let body = serde_json::json!({"error": {"message": outer, "metadata": {"raw": {"message": reason}}}});
        let error = status_error("OpenRouter", 400, &body.to_string());
        assert!(error.message.contains('é'));
        assert!(error.message.strip_prefix("OpenRouter answered 400: ").unwrap().chars().count() <= 1000);
        let body = serde_json::json!({"error": {"message": outer, "metadata": {"raw": {"message": outer}}}});
        assert_eq!(status_error("OpenRouter", 400, &body.to_string()).message.matches(outer).count(), 1);
        let body = serde_json::json!({"error": {"metadata": {"raw": {"headers": {"authorization": "Bearer sk-test"}}}}});
        assert_eq!(status_error("OpenRouter", 400, &body.to_string()).message, "OpenRouter answered 400: The service didn't provide an error message.");
    }

    #[test]
    fn status_errors_are_plain() {
        let e = status_error("Gemini", 400, r#"[{"error":{"code":400,"message":"Function call is missing a thought_signature"}}]"#);
        assert_eq!(e.message, "Gemini answered 400: Function call is missing a thought_signature");
        let e = status_error("Anthropic", 401, r#"{"error":{"message":"invalid x-api-key"}}"#);
        assert_eq!(e.code, ErrorCode::NoPermission);
        assert!(e.message.contains("invalid x-api-key"));
        assert_eq!(status_error("x", 429, "slow down").code, ErrorCode::OverLimit);
    }

    #[test]
    fn a_post_keeps_a_structured_retry_after_header() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut reader = BufReader::new(&mut stream);
            let mut len = 0;
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                if let Some((name, value)) = line.split_once(':')
                    && name.eq_ignore_ascii_case("content-length")
                {
                    len = value.trim().parse::<usize>().unwrap();
                }
            }
            reader.read_exact(&mut vec![0; len]).unwrap();
            let body = r#"{"error":{"type":"rate_limit_error","message":"服务限流"}}"#;
            write!(stream, "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 7\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        });
        let result = post_json(&quick_agent(Duration::from_secs(5)), "Stand-in", &url, &[], &serde_json::json!({}));
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("a rate limit is not a response stream"),
        };
        assert_eq!(crate::TurnFailure::from_api(&error), crate::TurnFailure { kind: crate::FailureKind::RateLimited, retry_after_secs: Some(7) });
        server.join().unwrap();
    }
}
