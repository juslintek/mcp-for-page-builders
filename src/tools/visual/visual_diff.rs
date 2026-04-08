use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::str_arg;
use crate::cdp;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct VisualDiff;

#[async_trait]
impl Tool for VisualDiff {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "visual_diff",
            description: "Compare two pages element-by-element via CDP and return structured, actionable differences (not pixel diff).",
            input_schema: json!({"type":"object","required":["url_a","url_b","selectors"],"properties":{"url_a":{"type":"string"},"url_b":{"type":"string"},"selectors":{"type":"array","items":{"type":"string"}}}}),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let url_a = str_arg(&args, "url_a").ok_or_else(|| anyhow::anyhow!("url_a required"))?;
        let url_b = str_arg(&args, "url_b").ok_or_else(|| anyhow::anyhow!("url_b required"))?;
        let selectors: Vec<String> = args.get("selectors")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        if selectors.is_empty() { anyhow::bail!("selectors array required"); }

        let build_js = |sels: &[String]| -> String {
            let sels_js = sels.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(",");
            format!(r"(()=>{{const results={{}};[{sels_js}].forEach(sel=>{{const el=document.querySelector(sel);if(!el){{results[sel]=null;return;}}const s=getComputedStyle(el);const b=el.getBoundingClientRect();results[sel]={{box:{{x:Math.round(b.x),y:Math.round(b.y),w:Math.round(b.width),h:Math.round(b.height)}},styles:{{'background-color':s.backgroundColor,color:s.color,'font-size':s.fontSize,'font-weight':s.fontWeight,padding:s.padding,margin:s.margin,'border-radius':s.borderRadius,display:s.display,'flex-direction':s.flexDirection,gap:s.gap,opacity:s.opacity}},text:el.textContent?.trim()?.substring(0,100)||''}}}});return JSON.stringify(results)}})()
")
        };

        let page_a = cdp::open_page(&url_a, 1440, 900).await?;
        let result_a: String = page_a.evaluate(build_js(&selectors)).await?.into_value()?;
        let data_a: Value = serde_json::from_str(&result_a)?;

        let page_b = cdp::open_page(&url_b, 1440, 900).await?;
        let result_b: String = page_b.evaluate(build_js(&selectors)).await?.into_value()?;
        let data_b: Value = serde_json::from_str(&result_b)?;

        let mut diffs = Vec::new();
        let mut matched = 0u32;
        let mut total = 0u32;

        for sel in &selectors {
            let a = data_a.get(sel);
            let b = data_b.get(sel);
            match (a, b) {
                (Some(Value::Null) | None, Some(Value::Null) | None) => {}
                (Some(_), Some(Value::Null) | None) => { diffs.push(json!({"selector": sel, "issue": "missing on B", "severity": "high"})); total += 1; }
                (Some(Value::Null) | None, Some(_)) => { diffs.push(json!({"selector": sel, "issue": "missing on A", "severity": "high"})); total += 1; }
                (Some(va), Some(vb)) => {
                    if let (Some(ba), Some(bb)) = (va.get("box"), vb.get("box")) {
                        for dim in ["w", "h"] {
                            total += 1;
                            let da = ba.get(dim).and_then(serde_json::Value::as_i64).unwrap_or(0);
                            let db = bb.get(dim).and_then(serde_json::Value::as_i64).unwrap_or(0);
                            if (da - db).abs() > 5 {
                                let label = if dim == "w" { "width" } else { "height" };
                                diffs.push(json!({"selector": sel, "property": label, "value_a": da, "value_b": db, "severity": if (da-db).abs() > 20 {"high"} else {"medium"}}));
                            } else { matched += 1; }
                        }
                    }
                    if let (Some(sa), Some(sb)) = (va.get("styles"), vb.get("styles"))
                        && let (Some(oa), Some(ob)) = (sa.as_object(), sb.as_object()) {
                            for (k, va_val) in oa {
                                total += 1;
                                let vb_val = ob.get(k).unwrap_or(&Value::Null);
                                if va_val == vb_val { matched += 1; } else {
                                    diffs.push(json!({"selector": sel, "property": k, "value_a": va_val, "value_b": vb_val, "severity": "medium"}));
                                }
                            }
                        }
                    let ta = va.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    let tb = vb.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    if !ta.is_empty() || !tb.is_empty() {
                        total += 1;
                        if ta == tb { matched += 1; } else { diffs.push(json!({"selector": sel, "property": "text", "value_a": ta, "value_b": tb, "severity": "low"})); }
                    }
                }
            }
        }

        let score = if total > 0 { #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] { (f64::from(matched) / f64::from(total) * 100.0) as u32 } } else { 100 };
        let output = json!({"match_score": score, "total_checks": total, "differences": diffs});
        Ok(ToolResult::text(serde_json::to_string_pretty(&output)?))
    }
}
