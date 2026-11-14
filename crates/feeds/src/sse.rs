//! Bounded SSE framing, preserving raw JSON number tokens without a full-frame copy.
use crate::{Event, Kind, Terminal};
pub const MAX_FRAME_BYTES: usize = 3 * 1024 * 1024;
pub const READY: &[u8] = b"event: ready\ndata: {\"snapshot\":false,\"replay\":false}\n\n";
pub const HEARTBEAT: &[u8] = b": heartbeat\n\n";
pub fn terminal(reason: Terminal) -> &'static [u8] {
    match reason {
        Terminal::Overflow => b"event: stream_end\ndata: {\"code\":\"overflow\"}\n\n",
        Terminal::Shutdown => b"event: stream_end\ndata: {\"code\":\"shutdown\"}\n\n",
        Terminal::Interrupted => b"event: stream_end\ndata: {\"code\":\"interrupted\"}\n\n",
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameTooLarge;
pub struct Frame<'a> {
    prefix: String,
    before: &'a [u8],
    after: &'a [u8],
}
impl<'a> Frame<'a> {
    pub fn new(event: &'a Event) -> Result<Self, FrameTooLarge> {
        let kind = match event.kind() {
            Kind::Insert => "insert",
            Kind::Update => "update",
            Kind::Delete => "delete",
        };
        // Event identifiers are already restricted to ASCII alphanumeric/_/-.
        let prefix = format!(
            "{{\"type\":\"{kind}\",\"collection\":\"{}\",\"id\":\"{}\",\"sequence\":\"{}\",\"before\":",
            event.collection(),
            event.id(),
            event.sequence()
        );
        let before = event.before().unwrap_or(b"null");
        let after = event.after().unwrap_or(b"null");
        let extra = before
            .iter()
            .chain(after)
            .filter(|byte| matches!(byte, b'\r' | b'\n'))
            .count()
            * 6;
        let size = b"event: change\ndata: ".len()
            + prefix.len()
            + before.len()
            + b",\"after\":".len()
            + after.len()
            + 3
            + extra;
        if size > MAX_FRAME_BYTES {
            return Err(FrameTooLarge);
        }
        Ok(Self {
            prefix,
            before,
            after,
        })
    }
    pub fn write_with<E>(&self, mut write: impl FnMut(&[u8]) -> Result<(), E>) -> Result<(), E> {
        write(b"event: change\ndata: ")?;
        write(self.prefix.as_bytes())?;
        for (index, state) in [self.before, self.after].into_iter().enumerate() {
            if index == 1 {
                write(b",\"after\":")?;
            }
            for (index, line) in state
                .split(|byte| matches!(byte, b'\r' | b'\n'))
                .enumerate()
            {
                if index > 0 {
                    write(b"\ndata: ")?;
                }
                write(line)?;
            }
        }
        write(b"}\n\n")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::Change;
    fn event(after: &[u8]) -> Event {
        Event {
            sequence: 9_007_199_254_740_993,
            change: Change::new("orders", "doc", Kind::Insert, None, Some(after)).unwrap(),
        }
    }
    #[test]
    fn framing_preserves_numbers_and_multiline_cannot_inject_events() {
        let event = event(
            b"{\r\n\"_id\":\"doc\",\n\"n\":9007199254740993,\"s\":\"event: fake\\ndata: nope\"}",
        );
        let mut bytes = Vec::new();
        Frame::new(&event)
            .unwrap()
            .write_with(|part| {
                bytes.extend_from_slice(part);
                Ok::<_, ()>(())
            })
            .unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert_eq!(
            text.lines()
                .filter(|line| line.starts_with("event:"))
                .count(),
            1
        );
        assert!(text.contains("\"sequence\":\"9007199254740993\""));
        assert!(text.contains("\"n\":9007199254740993"));
        assert!(
            !text
                .lines()
                .any(|line| line.starts_with("id:") || line.starts_with("retry:"))
        );
        assert!(text.ends_with("}\n\n"));
    }
    #[test]
    fn expansion_is_rejected_before_writing_any_bytes() {
        let raw = format!("{{{}\"_id\":\"doc\"}}", "\n".repeat(500_000));
        assert!(matches!(
            Frame::new(&event(raw.as_bytes())),
            Err(FrameTooLarge)
        ));
    }
}
