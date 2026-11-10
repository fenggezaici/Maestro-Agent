//! Generic Server-Sent Events byte/frame decoder. Payload interpretation is
//! left to each provider's `FrameParser`.
use serde_json::Value;

use crate::domain::AppResult;

#[derive(Debug)]
pub enum Frame {
    /// One complete `data:` payload parsed as JSON (provider `error` bodies are
    /// already rejected here with their own message).
    Data(Value),
    /// The literal `[DONE]` sentinel used by OpenAI-compatible streams.
    Done,
}

const MAX_FRAME_BYTES: usize = 1048576;

#[derive(Default)]
pub struct SseDecoder {
    bytes: Vec<u8>,
    data: Vec<String>,
    done: bool,
    first_line: bool,
    skip_lf: bool,
}

impl SseDecoder {
    pub fn new() -> Self {
        Self {
            first_line: true,
            ..Self::default()
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> AppResult<Vec<Frame>> {
        if self.done {
            return Ok(Vec::new());
        }
        let mut frames = Vec::new();
        for &byte in chunk {
            if self.skip_lf {
                self.skip_lf = false;
                if byte == b'\n' {
                    continue;
                }
            }
            if byte == b'\n' || byte == b'\r' {
                let line = std::mem::take(&mut self.bytes);
                self.line(&line, &mut frames)?;
                self.skip_lf = byte == b'\r';
                if self.done {
                    break;
                }
            } else {
                self.bytes.push(byte);
                if self.bytes.len() > MAX_FRAME_BYTES {
                    return Err("SSE frame 超过大小限制".into());
                }
            }
        }
        Ok(frames)
    }

    fn line(&mut self, bytes: &[u8], frames: &mut Vec<Frame>) -> AppResult<()> {
        let line = std::str::from_utf8(bytes)
            .map_err(|_| "SSE 包含无效 UTF-8".to_string())?
            .trim_end_matches('\r');
        let line = if self.first_line {
            self.first_line = false;
            line.trim_start_matches('\u{feff}')
        } else {
            line
        };
        if line.is_empty() {
            return self.dispatch(frames);
        }
        if let Some(data) = line.strip_prefix("data:") {
            self.data
                .push(data.strip_prefix(' ').unwrap_or(data).to_string());
            if self.data.iter().map(String::len).sum::<usize>() > MAX_FRAME_BYTES {
                return Err("SSE data 超过大小限制".into());
            }
        }
        Ok(())
    }

    fn dispatch(&mut self, frames: &mut Vec<Frame>) -> AppResult<()> {
        if self.data.is_empty() || self.done {
            return Ok(());
        }
        let data = std::mem::take(&mut self.data).join("\n");
        if data.trim() == "[DONE]" {
            self.done = true;
            frames.push(Frame::Done);
            return Ok(());
        }
        let value: Value =
            serde_json::from_str(&data).map_err(|_| "模型返回了无法解析的 SSE JSON".to_string())?;
        if let Some(error) = value.get("error") {
            return Err(error
                .get("message")
                .and_then(|message| message.as_str())
                .unwrap_or("模型服务返回错误")
                .to_string());
        }
        frames.push(Frame::Data(value));
        Ok(())
    }

    /// Flush any trailing line; providers decide whether a stream that ended
    /// without their finish marker is an error.
    pub fn finish(&mut self) -> AppResult<Vec<Frame>> {
        let mut frames = Vec::new();
        if !self.bytes.is_empty() && !self.done {
            let line = std::mem::take(&mut self.bytes);
            self.line(&line, &mut frames)?;
        }
        self.dispatch(&mut frames)?;
        Ok(frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(frames: Vec<Frame>) -> Vec<String> {
        frames
            .into_iter()
            .filter_map(|frame| match frame {
                Frame::Done => Some("[DONE]".into()),
                Frame::Data(value) => Some(value.to_string()),
            })
            .collect()
    }

    #[test]
    fn parses_utf8_across_every_byte_boundary_and_crlf_frames() {
        let wire = "\u{feff}: heartbeat\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"你好🌍\"}}]}\r\n\r\ndata: [DONE]\r\n\r\n";
        let mut decoder = SseDecoder::new();
        let mut frames = Vec::new();
        for byte in wire.as_bytes() {
            frames.extend(decoder.push(&[*byte]).unwrap());
        }
        let out = tokens(frames);
        assert_eq!(out.len(), 2);
        assert!(out[0].contains("你好🌍") && out[1] == "[DONE]");
        assert!(decoder.finish().unwrap().is_empty());
    }

    #[test]
    fn rejects_corrupt_provider_events() {
        assert!(SseDecoder::new().push(b"data: invalid\n\n").is_err());
        assert!(SseDecoder::new()
            .push(b"data: {\"error\":{\"message\":\"denied\"}}\n\n")
            .is_err());
    }

    #[test]
    fn accepts_all_sse_line_endings_and_multiline_data() {
        for ending in ["\r", "\n", "\r\n"] {
            let wire = format!("data: {{\"choices\":{ending}data: [{{\"delta\":{{\"content\":\"split\"}}}}]}}{ending}{ending}data: [DONE]{ending}{ending}");
            let mut decoder = SseDecoder::new();
            let mut frames = Vec::new();
            for byte in wire.bytes() {
                frames.extend(decoder.push(&[byte]).unwrap());
            }
            assert_eq!(tokens(frames).len(), 2);
        }
    }
}
