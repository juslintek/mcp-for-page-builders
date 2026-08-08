use serde::{Deserialize, Serialize};

/// High-performance frame metadata for Portal live streaming
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FramePayload {
    pub shadow_id: String,
    pub timestamp_ms: u64,
    pub width: u32,
    pub height: u32,
    pub format: String,
    pub data_b64: String,
}

/// Event payload broadcast to the Portal UI viewer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortalEvent {
    pub event_type: String, // e.g. "click", "type", "navigate", "fork", "promote"
    pub shadow_id: String,
    pub description: String,
    pub timestamp_ms: u64,
}

pub struct StreamerService;

impl StreamerService {
    pub fn stream_endpoint(shadow_id: &str) -> String {
        format!("ws://localhost:9100/portal/stream/{shadow_id}")
    }
}
