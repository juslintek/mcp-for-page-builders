use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Map, Value};

use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use super::Tool;

/// Static lookup table mapping ~40 CSS properties to Elementor widget settings.
///
/// Handles shorthand properties (`padding`, `margin`, `border-radius`) by parsing
/// them into TRBL (top/right/bottom/left) format that Elementor expects.
///
/// **Weakness:** not all CSS properties are mapped — unmapped properties are returned
/// in an `unmapped` array so callers know what was ignored.
///
/// The `widget_type` parameter is reserved for future schema-aware mapping
/// (e.g. heading vs container have different setting keys) but is not yet implemented.
pub struct CssToElementor;

#[async_trait]
impl Tool for CssToElementor {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "css_to_elementor",
            description: "Convert CSS properties to Elementor widget settings JSON. Eliminates trial-and-error when mapping computed styles to Elementor controls.",
            input_schema: json!({
                "type": "object",
                "required": ["css"],
                "properties": {
                    "css": { "type": "object", "description": "CSS property→value map, e.g. {\"background-color\":\"#fff\",\"padding\":\"10px 20px\"}" },
                    "widget_type": { "type": "string", "description": "Optional widget type for schema-aware mapping" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let css = args.get("css").and_then(|v| v.as_object())
            .ok_or_else(|| anyhow::anyhow!("css object required"))?;

        let mut out = Map::new();
        let mut unmapped = Vec::new();

        for (prop, val) in css {
            let v = val.as_str().unwrap_or("");
            if v.is_empty() || v == "none" || v == "normal" || v == "auto" { continue; }

            match prop.as_str() {
                "background-color" => {
                    out.insert("background_background".into(), json!("classic"));
                    out.insert("background_color".into(), json!(v));
                }
                "background-image" if v.starts_with("linear-gradient") || v.starts_with("radial-gradient") => {
                    out.insert("background_background".into(), json!("gradient"));
                }
                "color" => { out.insert("color".into(), json!(v)); }
                "font-size" => { out.insert("typography_typography".into(), json!("custom")); out.insert("typography_font_size".into(), parse_size(v)); }
                "font-weight" => { out.insert("typography_typography".into(), json!("custom")); out.insert("typography_font_weight".into(), json!(v)); }
                "font-family" => { out.insert("typography_typography".into(), json!("custom")); out.insert("typography_font_family".into(), json!(v.split(',').next().unwrap_or(v).trim().trim_matches('\"').trim_matches('\''))); }
                "line-height" => { out.insert("typography_typography".into(), json!("custom")); out.insert("typography_line_height".into(), parse_size(v)); }
                "letter-spacing" => { out.insert("typography_typography".into(), json!("custom")); out.insert("typography_letter_spacing".into(), parse_size(v)); }
                "text-transform" => { out.insert("typography_typography".into(), json!("custom")); out.insert("typography_text_transform".into(), json!(v)); }
                "padding" => { out.insert("padding".into(), parse_trbl(v)); }
                "padding-top" => { set_trbl_part(&mut out, "padding", "top", v); }
                "padding-right" => { set_trbl_part(&mut out, "padding", "right", v); }
                "padding-bottom" => { set_trbl_part(&mut out, "padding", "bottom", v); }
                "padding-left" => { set_trbl_part(&mut out, "padding", "left", v); }
                "margin" => { out.insert("margin".into(), parse_trbl(v)); }
                "margin-top" => { set_trbl_part(&mut out, "margin", "top", v); }
                "margin-right" => { set_trbl_part(&mut out, "margin", "right", v); }
                "margin-bottom" => { set_trbl_part(&mut out, "margin", "bottom", v); }
                "margin-left" => { set_trbl_part(&mut out, "margin", "left", v); }
                "border-radius" => { out.insert("border_radius".into(), parse_trbl_radius(v)); }
                "border" => {
                    let parts: Vec<&str> = v.splitn(3, ' ').collect();
                    if parts.len() >= 2 {
                        out.insert("border_border".into(), json!(parts[1]));
                        if let Some(w) = parts.first() { out.insert("border_width".into(), parse_trbl(w)); }
                        if let Some(c) = parts.get(2) { out.insert("border_color".into(), json!(c)); }
                    }
                }
                "gap" => { out.insert("flex_gap".into(), parse_size(v)); }
                "display" => {}
                "flex-direction" => { out.insert("flex_direction".into(), json!(v)); }
                "justify-content" => { out.insert("flex_justify_content".into(), json!(flex_val(v))); }
                "align-items" => { out.insert("flex_align_items".into(), json!(flex_val(v))); }
                "width" => {
                    let s = parse_size(v);
                    if v.ends_with('%') {
                        out.insert("_element_width".into(), json!("initial"));
                        out.insert("_element_custom_width".into(), json!({"size": s["size"], "unit": "%"}));
                    } else if v != "auto" {
                        out.insert("_element_width".into(), json!("initial"));
                        out.insert("_element_custom_width".into(), s);
                    }
                }
                "min-height" => { out.insert("min_height".into(), parse_size(v)); }
                "max-width" => { out.insert("max_width".into(), parse_size(v)); }
                "height" if v != "auto" => { out.insert("height".into(), parse_size(v)); }
                "opacity" => {
                    if let Ok(f) = v.parse::<f64>() {
                        out.insert("_opacity".into(), json!({"size": f, "unit": ""}));
                    }
                }
                "z-index" => {
                    if let Ok(n) = v.parse::<i64>() {
                        out.insert("z_index".into(), json!({"size": n, "unit": ""}));
                    }
                }
                "position" if v != "static" => { out.insert("position".into(), json!(v)); }
                "box-shadow" if v != "none" => {
                    out.insert("box_shadow_box_shadow_type".into(), json!("yes"));
                    let parts: Vec<&str> = v.split_whitespace().collect();
                    if parts.len() >= 3 {
                        let mut shadow = Map::new();
                        if let Some(x) = parts.first() { shadow.insert("horizontal".into(), json!(parse_num(x))); }
                        if let Some(y) = parts.get(1) { shadow.insert("vertical".into(), json!(parse_num(y))); }
                        if let Some(b) = parts.get(2) { shadow.insert("blur".into(), json!(parse_num(b))); }
                        if let Some(s) = parts.get(3) {
                            if s.starts_with('#') || s.starts_with("rgb") {
                                shadow.insert("color".into(), json!(s));
                            } else {
                                shadow.insert("spread".into(), json!(parse_num(s)));
                                if let Some(c) = parts.get(4) { shadow.insert("color".into(), json!(c)); }
                            }
                        }
                        out.insert("box_shadow_box_shadow".into(), json!(shadow));
                    }
                }
                "overflow" if v == "hidden" => { out.insert("overflow".into(), json!("hidden")); }
                _ => { unmapped.push(format!("{prop}: {v}")); }
            }
        }

        let mut result = json!({"settings": Value::Object(out)});
        if !unmapped.is_empty() {
            result["unmapped"] = json!(unmapped);
        }
        Ok(ToolResult::text(serde_json::to_string_pretty(&result)?))
    }
}

fn parse_num(s: &str) -> f64 {
    s.trim_end_matches(|c: char| c.is_alphabetic() || c == '%').parse().unwrap_or(0.0)
}

fn parse_unit(s: &str) -> &str {
    if s.ends_with("px") { "px" }
    else if s.ends_with("em") { "em" }
    else if s.ends_with("rem") { "rem" }
    else if s.ends_with('%') { "%" }
    else if s.ends_with("vw") { "vw" }
    else if s.ends_with("vh") { "vh" }
    else { "px" }
}

fn parse_size(s: &str) -> Value {
    json!({"size": parse_num(s), "unit": parse_unit(s)})
}

fn parse_trbl(s: &str) -> Value {
    let parts: Vec<&str> = s.split_whitespace().collect();
    let unit = parts.first().map_or("px", |p| parse_unit(p));
    match parts.len() {
        1 => { let v = parse_num(parts[0]); json!({"top": v.to_string(), "right": v.to_string(), "bottom": v.to_string(), "left": v.to_string(), "unit": unit, "isLinked": true}) }
        2 => { let tb = parse_num(parts[0]); let lr = parse_num(parts[1]); json!({"top": tb.to_string(), "right": lr.to_string(), "bottom": tb.to_string(), "left": lr.to_string(), "unit": unit, "isLinked": false}) }
        3 => { json!({"top": parse_num(parts[0]).to_string(), "right": parse_num(parts[1]).to_string(), "bottom": parse_num(parts[2]).to_string(), "left": parse_num(parts[1]).to_string(), "unit": unit, "isLinked": false}) }
        _ => { json!({"top": parse_num(parts[0]).to_string(), "right": parse_num(parts.get(1).unwrap_or(&"0")).to_string(), "bottom": parse_num(parts.get(2).unwrap_or(&"0")).to_string(), "left": parse_num(parts.get(3).unwrap_or(&"0")).to_string(), "unit": unit, "isLinked": false}) }
    }
}

fn parse_trbl_radius(s: &str) -> Value {
    let parts: Vec<&str> = s.split_whitespace().collect();
    let unit = parts.first().map_or("px", |p| parse_unit(p));
    match parts.len() {
        1 => { let v = parse_num(parts[0]); json!({"top": v.to_string(), "right": v.to_string(), "bottom": v.to_string(), "left": v.to_string(), "unit": unit, "isLinked": true}) }
        _ => { json!({"top": parse_num(parts[0]).to_string(), "right": parse_num(parts.get(1).unwrap_or(&"0")).to_string(), "bottom": parse_num(parts.get(2).unwrap_or(&"0")).to_string(), "left": parse_num(parts.get(3).unwrap_or(&"0")).to_string(), "unit": unit, "isLinked": false}) }
    }
}

fn set_trbl_part(out: &mut Map<String, Value>, key: &str, side: &str, v: &str) {
    let entry = out.entry(key.to_string()).or_insert_with(|| json!({"top":"","right":"","bottom":"","left":"","unit":"px","isLinked":false}));
    if let Some(obj) = entry.as_object_mut() {
        obj.insert(side.to_string(), json!(parse_num(v).to_string()));
        obj.insert("unit".to_string(), json!(parse_unit(v)));
    }
}

fn flex_val(css: &str) -> &str {
    match css {
        "flex-start" | "start" => "flex-start",
        "flex-end" | "end" => "flex-end",
        "center" => "center",
        "space-between" => "space-between",
        "space-around" => "space-around",
        "space-evenly" => "space-evenly",
        "stretch" => "stretch",
        _ => css,
    }
}
