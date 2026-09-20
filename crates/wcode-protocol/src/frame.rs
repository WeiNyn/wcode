//! Newline-delimited JSON framing for the protocol's [`Frame`]s.
//!
//! One frame is one JSON object on one line, exactly as the design doc lays
//! out (§5.2). The envelope's body is `#[serde(flatten)]`ed, so a frame is a
//! single flat object on the wire.

use std::io;

use serde::{Serialize, de::DeserializeOwned};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use wcode_harness::protocol::Frame;

/// Write one frame as a single line and flush.
pub async fn write_frame<W, P>(writer: &mut W, frame: &Frame<P>) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
    P: Serialize,
{
    let mut line = serde_json::to_vec(frame).map_err(json_error)?;
    line.push(b'\n');
    writer.write_all(&line).await?;
    writer.flush().await
}

/// Maximum size (bytes, trailing newline excluded) of a single frame's JSON
/// body. A peer that sends a longer line is rejected with `InvalidData` rather
/// than buffered unboundedly. 16 MiB, matching jcode — far above any legitimate
/// frame.
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// Read one frame. Returns `Ok(None)` on a clean EOF. Blank lines are skipped,
/// so a frame boundary never depends on a caller flushing an empty line.
pub async fn read_frame<R, P>(reader: &mut R) -> io::Result<Option<Frame<P>>>
where
    R: AsyncBufRead + Unpin,
    P: DeserializeOwned,
{
    let mut line = String::new();
    loop {
        line.clear();
        // Cap the read: `+ 1` lets a payload of exactly `MAX_FRAME_BYTES` still
        // consume its trailing '\n'.
        let mut limited = (&mut *reader).take(MAX_FRAME_BYTES as u64 + 1);
        let n = limited.read_line(&mut line).await?;
        // No newline within the budget => the frame exceeds the cap.
        if n > MAX_FRAME_BYTES && !line.ends_with('\n') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("frame exceeds {MAX_FRAME_BYTES} bytes"),
            ));
        }
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        return serde_json::from_str(trimmed).map(Some).map_err(json_error);
    }
}

fn json_error(e: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use tokio::io::BufReader;
    use wcode_harness::event::AgentEvent;
    use wcode_harness::protocol::{PROTOCOL_VERSION, Request, SessionId};

    #[tokio::test]
    async fn frame_round_trips_through_ndjson() {
        let frame = Frame::new(7, SessionId::new("s1"), Request::Cancel);
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, &frame).await.unwrap();
        assert_eq!(buf.last(), Some(&b'\n'), "one frame is one line");

        let mut reader = BufReader::new(buf.as_slice());
        let back: Frame<Request> = read_frame(&mut reader).await.unwrap().unwrap();
        assert_eq!(back, frame);
        // A second read at EOF is a clean `None`.
        assert!(
            read_frame::<_, Request>(&mut reader)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn blank_lines_are_skipped() {
        let mut src = BufReader::new(&b"\n\n  \n"[..]);
        assert!(read_frame::<_, Request>(&mut src).await.unwrap().is_none());
    }

    #[derive(Debug, Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum Only {
        Marker,
    }

    #[derive(Debug, Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum Padded {
        Marker { pad: String },
    }

    /// A valid `Frame<Padded>` JSON body of exactly `total` bytes (newline
    /// excluded), and the `pad` length it was built with.
    fn padded_frame_json(total: usize) -> (String, usize) {
        const PREFIX: &str = r#"{"v":1,"id":0,"session":"s1","type":"marker","pad":""#;
        const SUFFIX: &str = r#""}"#;
        let pad = total - PREFIX.len() - SUFFIX.len();
        (format!("{PREFIX}{}{SUFFIX}", "a".repeat(pad)), pad)
    }

    #[tokio::test]
    async fn a_frame_of_exactly_the_cap_is_accepted() {
        // Positive control: a legal max-size frame (valid JSON, EXACTLY the cap
        // with its newline) is ACCEPTED. Serde is not the gate here — the point
        // is that the cap does not reject a frame at the boundary.
        let (json, pad_len) = padded_frame_json(MAX_FRAME_BYTES);
        assert_eq!(json.len(), MAX_FRAME_BYTES);
        let mut bytes = json.into_bytes();
        bytes.push(b'\n');
        let mut src = BufReader::new(bytes.as_slice());
        let frame: Frame<Padded> = read_frame(&mut src).await.unwrap().unwrap();
        match frame.body {
            Padded::Marker { pad } => assert_eq!(pad.len(), pad_len),
        }
    }

    #[tokio::test]
    async fn a_frame_one_byte_over_the_cap_is_rejected() {
        let (json, _) = padded_frame_json(MAX_FRAME_BYTES + 1);
        let mut bytes = json.into_bytes();
        bytes.push(b'\n');
        let mut src = BufReader::new(bytes.as_slice());
        let err = read_frame::<_, Padded>(&mut src).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn an_unterminated_frame_is_refused_rather_than_buffered_forever() {
        // A reader that never yields a newline: the ONLY way for `read_frame` to
        // return is the cap. Without it this would buffer forever.
        let mut src = BufReader::new(tokio::io::repeat(b'A'));
        let err = read_frame::<_, Only>(&mut src).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn a_non_frame_line_is_an_invalid_data_error() {
        let mut src = BufReader::new(&b"not json\n"[..]);
        let err = read_frame::<_, Only>(&mut src).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn an_event_frame_is_flat() {
        let frame = Frame::new(0, SessionId::new("s1"), AgentEvent::Ack);
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, &frame).await.unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert_eq!(
            text.trim(),
            format!(r#"{{"v":{PROTOCOL_VERSION},"id":0,"session":"s1","type":"ack"}}"#)
        );
    }
}
