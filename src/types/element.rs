use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A single Elementor element (section, container, column, or widget).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Element {
    pub id: String,
    #[serde(rename = "elType")]
    pub el_type: String,
    #[serde(rename = "widgetType", skip_serializing_if = "Option::is_none")]
    pub widget_type: Option<String>,
    #[serde(default)]
    pub settings: Value,
    #[serde(default)]
    pub elements: Vec<Self>,
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, Value>,
}
