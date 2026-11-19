use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "check", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanCheck {
    ElementPresent {
        element_id: String,
    },
    ComputedStyle {
        selector: String,
        property: String,
        expected: String,
        #[serde(default)]
        viewport: Option<PlanViewport>,
    },
    NoHorizontalOverflow {
        #[serde(default)]
        viewport: Option<PlanViewport>,
    },
    PageHeight {
        #[serde(default)]
        viewport: Option<PlanViewport>,
        expected: u32,
        max_delta_percent: f64,
    },
    BoundingBox {
        selector: String,
        width: f64,
        height: f64,
        tolerance_px: f64,
        #[serde(default)]
        viewport: Option<PlanViewport>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanViewport {
    #[serde(default = "default_viewport_width")]
    pub width: u32,
    #[serde(default = "default_viewport_height")]
    pub height: u32,
    #[serde(default)]
    pub mobile: bool,
}

fn default_viewport_width() -> u32 {
    900
}
fn default_viewport_height() -> u32 {
    900
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementorPlan {
    pub site_url: String,
    pub page_id: u64,
    pub expected_revision: String,
    pub operation_id: String,
    #[serde(default)]
    pub document_patch: Option<Value>,
    #[serde(default)]
    pub resource_patches: Vec<ResourcePatch>,
    #[serde(default)]
    pub retained: Vec<RetainedElement>,
    #[serde(default)]
    pub source: Option<Value>,
    #[serde(default)]
    pub checks: Vec<PlanCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourcePatch {
    pub kind: ResourceKind,
    pub id: String,
    #[serde(default)]
    pub expected_revision: Option<String>,
    #[serde(default)]
    pub before: Option<Value>,
    pub after: Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Class,
    Variable,
    Template,
    Kit,
    PageSettings,
    PostMetadata,
    Page,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetainedElement {
    pub element_id: String,
    pub reason: String,
}
