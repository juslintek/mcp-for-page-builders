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

    pub async fn read_request(&mut self) -> Result<Option<Request>> {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).await?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_str(trimmed)?))
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
