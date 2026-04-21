use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::types::{Tool, ToolDef, ToolResult};
use crate::wp::WpClient;

pub struct UploadMedia;

#[async_trait]
impl Tool for UploadMedia {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "upload_media",
            description: "Upload a file to WordPress media library via multipart upload",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": {"type": "string", "description": "Local file path to upload"},
                    "file_data": {"type": "string", "description": "Base64-encoded file data (alternative to file_path)"},
                    "filename": {"type": "string", "description": "Filename (required with file_data)"},
                    "title": {"type": "string", "description": "Media title"},
                    "alt_text": {"type": "string", "description": "Alt text for the media"}
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        wp.require_configured()?;

        let (bytes, filename) = if let Some(path) = args.get("file_path").and_then(Value::as_str) {
            let data = tokio::fs::read(path).await?;
            let name = std::path::Path::new(path)
                .file_name()
                .map_or("upload".to_string(), |n| n.to_string_lossy().to_string());
            (data, name)
        } else if let Some(b64) = args.get("file_data").and_then(Value::as_str) {
            let data = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)?;
            let name = args.get("filename").and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("filename required with file_data"))?
                .to_string();
            (data, name)
        } else {
            anyhow::bail!("Provide either file_path or file_data + filename");
        };

        let mime = mime_from_ext(&filename);
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(filename)
            .mime_str(&mime)?;
        let mut form = reqwest::multipart::Form::new().part("file", part);
        if let Some(t) = args.get("title").and_then(Value::as_str) {
            form = form.text("title", t.to_string());
        }
        if let Some(a) = args.get("alt_text").and_then(Value::as_str) {
            form = form.text("alt_text", a.to_string());
        }

        let result = wp.post_multipart("wp/v2/media", form).await?;
        let id = result["id"].as_i64().unwrap_or(0);
        let url = result["source_url"].as_str().unwrap_or("");
        Ok(ToolResult::text(serde_json::to_string_pretty(&json!({
            "id": id,
            "url": url,
            "type": result["mime_type"],
        }))?))
    }
}

fn mime_from_ext(filename: &str) -> String {
    match filename.rsplit('.').next().map(str::to_lowercase).as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("pdf") => "application/pdf",
        Some("mp4") => "video/mp4",
        Some("mp3") => "audio/mpeg",
        _ => "application/octet-stream",
    }.to_string()
}
