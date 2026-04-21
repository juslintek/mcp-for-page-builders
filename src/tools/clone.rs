use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Map, Value};

use crate::cdp;
use crate::elementor::generate_id;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use super::Tool;

/// Converts live DOM elements to Elementor JSON by inspecting via CDP and mapping HTML tags to widget types.
///
/// **Weakness:** the tag-to-widget mapping is heuristic — complex layouts may not map perfectly.
/// For example, a `<div>` with a background image won't be detected as an image widget.
///
/// The CSS-to-Elementor mapping is a subset of what `css_to_elementor` provides,
/// inlined here to avoid a round-trip through the REST API.
///
/// Produces valid Elementor JSON ready for `create_page` or `update_element`.
pub struct CloneElement;

#[async_trait]
impl Tool for CloneElement {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "clone_element",
            description: "Clone a live DOM element as Elementor JSON. Inspects via CDP, maps HTML tags to widget types, converts CSS to Elementor settings.\n\nWorkflow: Use to replicate external designs into Elementor. Output is ready for create_page or add_element. For style-only matching (keeping existing structure), use match_styles instead.",
            input_schema: json!({
                "type": "object",
                "required": ["url", "selector"],
                "properties": {
                    "url": { "type": "string" },
                    "selector": { "type": "string" },
                    "max_depth": { "type": "integer", "default": 3 },
                    "pre_js": { "type": "string", "description": "JavaScript to execute before cloning (e.g. expand a section)" },
                    "wait_ms": { "type": "integer", "default": 0, "description": "Milliseconds to wait after pre_js" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let url = args["url"].as_str().ok_or_else(|| anyhow::anyhow!("url required"))?;
        let selector = args["selector"].as_str().ok_or_else(|| anyhow::anyhow!("selector required"))?;
        let max_depth = args.get("max_depth").and_then(serde_json::Value::as_u64).unwrap_or(3);

        let js = build_dom_inspect_js(selector, max_depth);
        let (page, _warning) = cdp::open_page_with_js(url, 1440, 900,
            args["pre_js"].as_str(),
            args.get("wait_ms").and_then(|v| v.as_u64()).unwrap_or(0),
        ).await?;
        let result: String = page.evaluate(js).await.context("CDP evaluate failed")?.into_value()?;
        let dom: Value = serde_json::from_str(&result)?;
        if let Some(err) = dom.get("error") {
            anyhow::bail!("{}", err.as_str().unwrap_or("not found"));
        }

        let elementor = dom_to_elementor(&dom, 0);
        Ok(ToolResult::text(serde_json::to_string_pretty(&json!([elementor]))?))
    }
}

fn build_dom_inspect_js(selector: &str, max_depth: u64) -> String {
    format!(
        r"(()=>{{function inspect(el,d){{if(!el||d<0)return null;const s=getComputedStyle(el);const b=el.getBoundingClientRect();const props=['background-color','color','font-size','font-weight','font-family','padding','margin','border-radius','border','gap','display','flex-direction','justify-content','align-items','width','height','min-height','max-width','opacity','text-transform','line-height','letter-spacing','box-shadow','overflow'];const styles={{}};props.forEach(p=>{{const v=s.getPropertyValue(p);if(v&&v!=='none'&&v!=='normal'&&v!=='auto'&&v!=='0px'&&v!=='rgba(0, 0, 0, 0)')styles[p]=v}});const children=[];if(d>0)for(const c of el.children){{const r=inspect(c,d-1);if(r)children.push(r)}}const text=el.childNodes.length===1&&el.childNodes[0].nodeType===3?el.textContent.trim():'';return{{tag:el.tagName,id:el.id||'',classes:el.className?.split?.(/\s+/)||[],src:el.src||el.getAttribute('src')||'',href:el.href||el.getAttribute('href')||'',text,styles,children,box:{{w:Math.round(b.width),h:Math.round(b.height)}}}}}}const el=document.querySelector('{selector}');if(!el)return JSON.stringify({{error:'not found'}});return JSON.stringify(inspect(el,{max_depth}))}})()"
    )
}

#[allow(clippy::only_used_in_recursion)]
fn dom_to_elementor(node: &Value, depth: u32) -> Value {
    let tag = node["tag"].as_str().unwrap_or("DIV").to_uppercase();
    let text = node["text"].as_str().unwrap_or("");
    let src = node["src"].as_str().unwrap_or("");
    let href = node["href"].as_str().unwrap_or("");
    let children = node["children"].as_array();
    let styles = node.get("styles").and_then(|v| v.as_object());

    let id = rand_id();
    let mut settings = Map::new();

    if let Some(css) = styles {
        apply_container_styles(&mut settings, css);
    }

    match tag.as_str() {
        "H1" | "H2" | "H3" | "H4" | "H5" | "H6" => {
            let level = &tag[1..];
            settings.insert("title".into(), json!(text));
            settings.insert("header_size".into(), json!(format!("h{level}")));
            if let Some(css) = styles { add_typography(&mut settings, css); }
            widget("heading", &id, settings)
        }
        "P" => {
            settings.insert("editor".into(), json!(format!("<p>{text}</p>")));
            if let Some(css) = styles { add_typography(&mut settings, css); }
            widget("text-editor", &id, settings)
        }
        "IMG" => {
            if !src.is_empty() { settings.insert("image".into(), json!({"url": src})); }
            widget("image", &id, settings)
        }
        "A" if !text.is_empty() && text.len() < 50 => {
            settings.insert("text".into(), json!(text));
            if !href.is_empty() { settings.insert("link".into(), json!({"url": href})); }
            if let Some(css) = styles { add_typography(&mut settings, css); }
            widget("button", &id, settings)
        }
        _ => {
            let child_elements: Vec<Value> = children
                .map(|arr| arr.iter().map(|c| dom_to_elementor(c, depth + 1)).collect())
                .unwrap_or_default();

            if child_elements.is_empty() && !text.is_empty() {
                settings.insert("editor".into(), json!(format!("<p>{text}</p>")));
                widget("text-editor", &id, settings)
            } else {
                container(&id, settings, &child_elements)
            }
        }
    }
}

#[allow(clippy::or_fun_call)]
fn apply_container_styles(settings: &mut Map<String, Value>, css: &Map<String, Value>) {
    if let Some(bg) = css.get("background-color").and_then(|v| v.as_str())
        && bg != "rgba(0, 0, 0, 0)" && bg != "transparent" {
            settings.insert("background_background".into(), json!("classic"));
            settings.insert("background_color".into(), json!(bg));
        }
    if let Some(br) = css.get("border-radius").and_then(|v| v.as_str())
        && br != "0px" {
            let n = br.replace("px", "").parse::<f64>().unwrap_or(0.0);
            settings.insert("border_radius".into(), json!({"top":n.to_string(),"right":n.to_string(),"bottom":n.to_string(),"left":n.to_string(),"unit":"px","isLinked":true}));
        }
    if let Some(p) = css.get("padding").and_then(|v| v.as_str())
        && p != "0px" {
            let parts: Vec<f64> = p.split_whitespace().map(|s| s.replace("px","").parse().unwrap_or(0.0)).collect();
            if let Some(&v) = parts.first() {
                settings.insert("padding".into(), json!({"top":v.to_string(),"right":parts.get(1).unwrap_or(&v).to_string(),"bottom":parts.get(2).unwrap_or(&v).to_string(),"left":parts.get(3).unwrap_or(parts.get(1).unwrap_or(&v)).to_string(),"unit":"px","isLinked":parts.len()==1}));
            }
        }
    if let Some(gap) = css.get("gap").and_then(|v| v.as_str()) {
        let n = gap.replace("px","").parse::<f64>().unwrap_or(0.0);
        if n > 0.0 { settings.insert("flex_gap".into(), json!({"size":n,"unit":"px"})); }
    }
    if let Some(fd) = css.get("flex-direction").and_then(|v| v.as_str()) {
        settings.insert("flex_direction".into(), json!(fd));
    }
    if let Some(jc) = css.get("justify-content").and_then(|v| v.as_str()) {
        settings.insert("flex_justify_content".into(), json!(jc));
    }
    if let Some(ai) = css.get("align-items").and_then(|v| v.as_str()) {
        settings.insert("flex_align_items".into(), json!(ai));
    }
}

fn add_typography(settings: &mut Map<String, Value>, css: &Map<String, Value>) {
    let mut has_typo = false;
    if let Some(v) = css.get("color").and_then(|v| v.as_str()) {
        settings.insert("title_color".into(), json!(v));
    }
    if let Some(v) = css.get("font-size").and_then(|v| v.as_str()) {
        let n = v.replace("px","").parse::<f64>().unwrap_or(0.0);
        if n > 0.0 { settings.insert("typography_font_size".into(), json!({"size":n,"unit":"px"})); has_typo = true; }
    }
    if let Some(v) = css.get("font-weight").and_then(|v| v.as_str()) {
        settings.insert("typography_font_weight".into(), json!(v)); has_typo = true;
    }
    if let Some(v) = css.get("text-transform").and_then(|v| v.as_str()) {
        settings.insert("typography_text_transform".into(), json!(v)); has_typo = true;
    }
    if has_typo {
        settings.insert("typography_typography".into(), json!("custom"));
    }
}

fn rand_id() -> String {
    generate_id()
}

fn widget(widget_type: &str, id: &str, settings: Map<String, Value>) -> Value {
    json!({
        "id": id,
        "elType": "widget",
        "widgetType": widget_type,
        "settings": Value::Object(settings),
        "elements": []
    })
}

fn container(id: &str, settings: Map<String, Value>, children: &[Value]) -> Value {
    json!({
        "id": id,
        "elType": "container",
        "settings": Value::Object(settings),
        "elements": children
    })
}
