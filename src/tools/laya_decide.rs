use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::OnceCell;

use crate::{
    laya::{LayaClient, LayaDecision, LayaRequest},
    mcp::{ToolDef, ToolResult},
    types::Tool,
    wp::WpClient,
};

const MAX_STATE_BYTES: usize = 24_576;
const MAX_QUESTION_BYTES: usize = 2_048;
const MAX_LABEL_BYTES: usize = 128;
const MAX_OPTIONS: usize = 8;

static CLIENT: OnceCell<LayaClient> = OnceCell::const_new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub struct LayaDecideElementor;

#[async_trait]
impl Tool for LayaDecideElementor {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "laya_decide_elementor",
            description: "Use the local read-only Laya model to classify Elementor CSS mappings or repeated patterns. Returns a prediction or an explicit abstention; never applies changes.",
            input_schema: json!({
                "type": "object",
                "required": ["task", "state", "question", "options"],
                "properties": {
                    "task": {"type":"string", "enum":["css_mapping", "pattern_category"]},
                    "state": {"description":"Bounded Elementor or CSS context", "type":["string", "object", "array"]},
                    "question": {"type":"string", "minLength":1, "maxLength":2048},
                    "options": {"type":"array", "minItems":2, "maxItems":8, "items":{"type":"string", "minLength":1, "maxLength":128}}
                },
                "additionalProperties":false
            }),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let task = args
            .get("task")
            .and_then(Value::as_str)
            .context("task must be css_mapping or pattern_category")?;
        if !matches!(task, "css_mapping" | "pattern_category") {
            bail!("task must be css_mapping or pattern_category");
        }
        let state = args
            .get("state")
            .filter(|state| state.is_string() || state.is_object() || state.is_array())
            .context("state must be text, an object, or an array")?;
        let question = args
            .get("question")
            .and_then(Value::as_str)
            .filter(|question| !question.trim().is_empty())
            .context("question must be nonempty text")?;
        if question.len() > MAX_QUESTION_BYTES {
            bail!("question exceeds {MAX_QUESTION_BYTES} UTF-8 bytes");
        }
        let options = args
            .get("options")
            .and_then(Value::as_array)
            .context("options must contain 2 to 8 labels")?;
        if !(2..=MAX_OPTIONS).contains(&options.len()) {
            bail!("options must contain 2 to {MAX_OPTIONS} labels");
        }
        let options = options
            .iter()
            .map(|option| {
                option
                    .as_str()
                    .filter(|label| !label.trim().is_empty())
                    .context("every option must be nonempty text")
                    .map(str::to_owned)
            })
            .collect::<Result<Vec<_>>>()?;
        if options.iter().any(|label| label.len() > MAX_LABEL_BYTES) {
            bail!("each option must be at most {MAX_LABEL_BYTES} UTF-8 bytes");
        }
        if options
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != options.len()
        {
            bail!("option labels must be unique");
        }

        let bounded_state = json!({"task":task, "context":state});
        let state_bytes = serde_json::to_vec(&bounded_state)?;
        if state_bytes.len() > MAX_STATE_BYTES {
            bail!("state exceeds {MAX_STATE_BYTES} UTF-8 bytes");
        }
        let request = LayaRequest {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed).to_string(),
            state: bounded_state,
            question: question.to_owned(),
            options,
        };
        let client = CLIENT
            .get_or_try_init(|| async { LayaClient::new() })
            .await
            .context("configure local Laya with LAYA_PYTHON and LAYA_MODEL_PATH; defaults use the user's local install and pinned cache")?;
        let decision = client.predict(&request).await?;
        Ok(ToolResult::text(serde_json::to_string_pretty(&decision)?))
    }
}

pub async fn decide_elementor(request: &LayaRequest) -> Result<LayaDecision> {
    CLIENT
        .get_or_try_init(|| async { LayaClient::new() })
        .await
        .context("configure local Laya with LAYA_PYTHON and LAYA_MODEL_PATH; defaults use the user's local install and pinned cache")?
        .predict(request)
        .await
}
