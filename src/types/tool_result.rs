use serde::Serialize;

#[derive(Serialize)]
pub struct ToolContent {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub text: String,
}

#[derive(Serialize)]
pub struct ToolResult {
    pub content: Vec<ToolContent>,
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

impl ToolResult {
    pub fn text(s: impl Into<String>) -> Self {
        Self { content: vec![ToolContent { kind: "text", text: s.into() }], is_error: None }
    }

    pub fn error(s: impl Into<String>) -> Self {
        Self { content: vec![ToolContent { kind: "text", text: s.into() }], is_error: Some(true) }
    }
}
