use base64::Engine;
use serde::Serialize;

#[derive(Serialize, Clone)]
#[serde(tag = "type")]
pub enum ToolContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
}

#[derive(Serialize, Clone)]
pub struct ToolResult {
    pub content: Vec<ToolContent>,
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

impl ToolResult {
    pub fn text(s: impl Into<String>) -> Self {
        Self { content: vec![ToolContent::Text { text: s.into() }], is_error: None }
    }

    pub fn error(s: impl Into<String>) -> Self {
        Self { content: vec![ToolContent::Text { text: s.into() }], is_error: Some(true) }
    }

    pub fn image(bytes: &[u8], mime_type: &str) -> Self {
        Self {
            content: vec![ToolContent::Image {
                data: base64::engine::general_purpose::STANDARD.encode(bytes),
                mime_type: mime_type.to_string(),
            }],
            is_error: None,
        }
    }

    pub fn mixed(content: Vec<ToolContent>) -> Self {
        Self { content, is_error: None }
    }

    pub fn text_and_image(text: impl Into<String>, bytes: &[u8], mime_type: &str) -> Self {
        Self {
            content: vec![
                ToolContent::Text { text: text.into() },
                ToolContent::Image {
                    data: base64::engine::general_purpose::STANDARD.encode(bytes),
                    mime_type: mime_type.to_string(),
                },
            ],
            is_error: None,
        }
    }
}
