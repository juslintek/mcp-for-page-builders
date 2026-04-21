use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::str_arg;
use crate::cdp;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct VisualDiff;

/// JS that discovers top-level semantic selectors from a page.
const DISCOVER_SELECTORS_JS: &str = r"(()=>{const sels=new Set();const semantic=['header','nav','main','footer','section','article','aside','h1','h2'];semantic.forEach(t=>{if(document.querySelector(t))sels.add(t)});document.querySelectorAll('[class*=hero],[class*=header],[class*=footer],[class*=banner],[class*=nav]').forEach(el=>{if(el.id)sels.add('#'+el.id);else if(el.className){const c=el.className.split(/\s+/).find(c=>/hero|header|footer|banner|nav/.test(c));if(c)sels.add('.'+c)}});for(const c of document.body.children){if(c.id)sels.add('#'+c.id);else if(c.className&&c.className.split){const cls=c.className.split(/\s+/)[0];if(cls)sels.add('.'+cls)}}return JSON.stringify([...sels])})()";

#[async_trait]
impl Tool for VisualDiff {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "visual_diff",
            description: "Compare two pages element-by-element via CDP. Returns structured differences with match score.\n\nWorkflow: Use after making changes to verify visual parity. Omit selectors for auto-discovery of page sections. Use after match_styles or update_element to verify results.",
            input_schema: json!({"type":"object","required":["url_a","url_b"],"properties":{
                "url_a":{"type":"string"},
                "url_b":{"type":"string"},
                "selectors":{"type":"array","items":{"type":"string"},"description":"CSS selectors to compare. Omit for auto-discovery of page sections."},
                "pre_js_a":{"type":"string","description":"JavaScript to execute on page A before comparison"},
                "pre_js_b":{"type":"string","description":"JavaScript to execute on page B before comparison"},
                "wait_ms":{"type":"integer","default":0,"description":"Milliseconds to wait after pre_js"}
            }}),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let url_a = str_arg(&args, "url_a").ok_or_else(|| anyhow::anyhow!("url_a required"))?;
        let url_b = str_arg(&args, "url_b").ok_or_else(|| anyhow::anyhow!("url_b required"))?;
        let mut selectors: Vec<String> = args.get("selectors")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        let pre_js_a = str_arg(&args, "pre_js_a");
        let pre_js_b = str_arg(&args, "pre_js_b");
        let wait_ms = args.get("wait_ms").and_then(|v| v.as_u64()).unwrap_or(0);

        let (page_a, warn_a) = cdp::open_page_with_js(&url_a, 1440, 900, pre_js_a.as_deref(), wait_ms).await?;
        let (page_b, warn_b) = cdp::open_page_with_js(&url_b, 1440, 900, pre_js_b.as_deref(), wait_ms).await?;

        // Auto-discover selectors if none provided
        if selectors.is_empty() {
            let sels_a: String = page_a.evaluate(DISCOVER_SELECTORS_JS).await?.into_value()?;
            let sels_b: String = page_b.evaluate(DISCOVER_SELECTORS_JS).await?.into_value()?;
            let mut all: Vec<String> = serde_json::from_str::<Vec<String>>(&sels_a)?;
            let extra: Vec<String> = serde_json::from_str(&sels_b)?;
            for s in extra { if !all.contains(&s) { all.push(s); } }
            selectors = all;
        }

        if selectors.is_empty() { anyhow::bail!("No selectors found on either page"); }

        let build_js = |sels: &[String]| -> String {
            let sels_js = sels.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(",");
            format!(r"(()=>{{const results={{}};[{sels_js}].forEach(sel=>{{const el=document.querySelector(sel);if(!el){{results[sel]=null;return;}}const s=getComputedStyle(el);const b=el.getBoundingClientRect();results[sel]={{box:{{x:Math.round(b.x),y:Math.round(b.y),w:Math.round(b.width),h:Math.round(b.height)}},styles:{{'background-color':s.backgroundColor,color:s.color,'font-size':s.fontSize,'font-weight':s.fontWeight,padding:s.padding,margin:s.margin,'border-radius':s.borderRadius,display:s.display,'flex-direction':s.flexDirection,gap:s.gap,opacity:s.opacity}},text:el.textContent?.trim()?.substring(0,100)||''}}}});return JSON.stringify(results)}})()
")
        };

        let result_a: String = page_a.evaluate(build_js(&selectors)).await?.into_value()?;
        let data_a: Value = serde_json::from_str(&result_a)?;

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
        let mut output = json!({"match_score": score, "total_checks": total, "selectors_used": selectors, "differences": diffs});
        if let Some(w) = warn_a { output["warning_a"] = json!(w); }
        if let Some(w) = warn_b { output["warning_b"] = json!(w); }
        Ok(ToolResult::text(serde_json::to_string_pretty(&output)?))
    }
}
