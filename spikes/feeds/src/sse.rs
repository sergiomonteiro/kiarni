//! Proposed SSE framing, not an HTTP handler. See proposed ADR 0005.
use crate::{Event, Kind};
use kiarni_core::DEFAULT_MAX_DOCUMENT_BYTES;

pub const MAX_FRAME_BYTES: usize = 3 * 1024 * 1024;
pub const READY: &str = "event: ready\ndata: {\"snapshot\":false,\"replay\":false}\n\n";
pub const HEARTBEAT: &str = ": heartbeat\n\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireError {
    InvalidEvent,
    FrameTooLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Terminal {
    Overflow,
    Shutdown,
    Interrupted,
}

pub fn encode_terminal(reason: Terminal) -> &'static str {
    match reason {
        Terminal::Overflow => "event: stream_end\ndata: {\"code\":\"overflow\"}\n\n",
        Terminal::Shutdown => "event: stream_end\ndata: {\"code\":\"shutdown\"}\n\n",
        Terminal::Interrupted => "event: stream_end\ndata: {\"code\":\"interrupted\"}\n\n",
    }
}

fn raw_state(state: Option<&[u8]>) -> Result<&str, WireError> {
    let Some(bytes) = state else {
        return Ok("null");
    };
    if bytes.len() > DEFAULT_MAX_DOCUMENT_BYTES {
        return Err(WireError::FrameTooLarge);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| WireError::InvalidEvent)?;
    serde_json::from_str::<&serde_json::value::RawValue>(text)
        .map_err(|_| WireError::InvalidEvent)?;
    Ok(text)
}

pub fn encode_change(event: &Event) -> Result<String, WireError> {
    if event.collection.len() > 128 || event.id.len() > 128 {
        return Err(WireError::InvalidEvent);
    }
    let kind = match event.kind {
        Kind::Insert => "insert",
        Kind::Update => "update",
        Kind::Delete => "delete",
    };
    let collection =
        serde_json::to_string(&event.collection).map_err(|_| WireError::InvalidEvent)?;
    let id = serde_json::to_string(&event.id).map_err(|_| WireError::InvalidEvent)?;
    // Sequence is a decimal string to avoid rounding in JavaScript clients.
    // Raw JSON states preserve number tokens; no intermediate f64 conversion.
    let json = format!(
        "{{\"type\":\"{kind}\",\"collection\":{collection},\"id\":{id},\"sequence\":\"{}\",\"before\":{},\"after\":{}}}",
        event.sequence,
        raw_state(event.before.as_deref())?,
        raw_state(event.after.as_deref())?,
    );
    let mut frame = String::from("event: change\n");
    // JSON whitespace can include CR/LF. Every physical line must use data:.
    for line in json.split(['\r', '\n']) {
        if frame.len() + line.len() + 8 > MAX_FRAME_BYTES {
            return Err(WireError::FrameTooLarge);
        }
        frame.push_str("data: ");
        frame.push_str(line);
        frame.push('\n');
    }
    frame.push('\n');
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn event(after: &[u8]) -> Event {
        Event {
            sequence: 9_007_199_254_740_993,
            kind: Kind::Insert,
            collection: "orders".into(),
            id: "ord_1".into(),
            before: None,
            after: Some(Arc::from(after)),
        }
    }

    #[test]
    fn rust_encoder_matches_the_shared_client_fixture() {
        let event =
            event(br#"{"_id":"ord_1","status":"paid","total":123456789012345678901234567890}"#);
        assert_eq!(
            encode_change(&event).unwrap(),
            include_str!("../../streaming/fixtures/change.sse")
        );
        assert_eq!(READY, include_str!("../../streaming/fixtures/ready.sse"));
        assert_eq!(
            encode_terminal(Terminal::Overflow),
            include_str!("../../streaming/fixtures/overflow.sse")
        );
    }

    #[test]
    fn multiline_raw_json_cannot_inject_sse_fields() {
        let mut event = event(b"{\r\n\"note\":\"event: stream_end\\ndata: injected\"\n}");
        event.id = "id\nevent: stream_end".into();
        let frame = encode_change(&event).unwrap();
        assert_eq!(
            frame
                .lines()
                .filter(|line| line.starts_with("event:"))
                .count(),
            1
        );
        let json = frame
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .collect::<Vec<_>>()
            .join("\n");
        let decoded: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded["id"], "id\nevent: stream_end");
        assert_eq!(decoded["sequence"], "9007199254740993");
        assert!(
            !frame
                .lines()
                .any(|line| line.starts_with("id:") || line.starts_with("retry:"))
        );
    }

    #[test]
    fn invalid_payloads_and_expanded_frames_are_bounded() {
        assert_eq!(
            encode_change(&event(b"not JSON")),
            Err(WireError::InvalidEvent)
        );
        assert_eq!(encode_change(&event(b"\xff")), Err(WireError::InvalidEvent));
        let whitespace = format!("{{{}\"x\":1}}", "\n".repeat(DEFAULT_MAX_DOCUMENT_BYTES / 2));
        assert_eq!(
            encode_change(&event(whitespace.as_bytes())),
            Err(WireError::FrameTooLarge)
        );
    }
}
