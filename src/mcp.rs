use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub use crate::types::{Request, Response, ToolDef, ToolResult};

pub struct Stdio {
    reader: BufReader<tokio::io::Stdin>,
    writer: tokio::io::Stdout,
}

impl Default for Stdio {
    fn default() -> Self {
        Self::new()
    }
}

impl Stdio {
    pub fn new() -> Self {
        Self {
            reader: BufReader::new(tokio::io::stdin()),
            writer: tokio::io::stdout(),
        }
    }

    /// Reads the next JSON-RPC request from stdin, skipping blank lines.
    ///
    /// Returns `Ok(None)` only on true EOF (stdin closed) — a blank line
    /// between messages must NOT be treated as EOF, or the server exits
    /// silently on any stray newline, which the client sees as a dropped
    /// connection.
    pub async fn read_request(&mut self) -> Result<Option<Request>> {
        loop {
            let mut line = String::new();
            let n = self.reader.read_line(&mut line).await?;
            if n == 0 {
                return Ok(None); // true EOF
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue; // blank line — keep reading, not EOF
            }
            return match serde_json::from_str(trimmed) {
                Ok(req) => Ok(Some(req)),
                Err(e) => {
                    // Malformed line: log it and keep the connection alive
                    // instead of propagating an error that tears down the
                    // read loop (and thus the whole server). Cap the raw
                    // content logged — a client payload could embed
                    // secrets/tokens/PII, and this now persists to disk.
                    let preview: String = trimmed.chars().take(200).collect();
                    let truncated_note = if trimmed.len() > preview.len() { " (truncated)" } else { "" };
                    tracing::error!(
                        "Failed to parse incoming JSON-RPC line: {e} — raw{truncated_note}: {preview:?}"
                    );
                    continue;
                }
            };
        }
    }

    pub async fn write_response(&mut self, resp: &Response) -> Result<()> {
        let mut json = serde_json::to_string(resp)?;
        json.push('\n');
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.flush().await?;
        Ok(())
    }

    /// Send a JSON-RPC notification (no id, no response expected).
    pub async fn write_notification(&mut self, method: &str, params: serde_json::Value) -> Result<()> {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let mut json = serde_json::to_string(&msg)?;
        json.push('\n');
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.flush().await?;
        Ok(())
    }
}
