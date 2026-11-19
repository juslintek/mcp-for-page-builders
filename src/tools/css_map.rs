use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{Map, Value, json};
use std::collections::HashSet;

use super::Tool;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;

pub struct CssToElementor;

#[async_trait]
impl Tool for CssToElementor {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "css_to_elementor",
            description: "Map CSS declarations to verified Elementor settings. Unsupported declarations are returned as residuals.",
            input_schema: json!({
                "type": "object",
                "required": ["css", "widget_type"],
                "properties": {
                    "css": { "type": "object" },
                    "widget_type": { "type": "string" },
                    "target": { "type": "string", "enum": ["classic", "atomic"] }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let css = args
            .get("css")
            .and_then(Value::as_object)
            .context("css object required")?;
        let target = args
            .get("target")
            .and_then(Value::as_str)
            .unwrap_or("classic");
        if target != "classic" {
            if target != "atomic" {
                anyhow::bail!("target must be classic or atomic");
            }
            let response = wp
                .post(
                    "elementor/v1/css-to-atomic",
                    &json!({"blocks":{"probe":css}}),
                )
                .await
                .context("Elementor native CSS conversion request failed")?;
            let converted = response
                .pointer("/data/probe")
                .context("Elementor native CSS conversion response omitted data.probe")?;
            let mut output = json!({"target":"atomic", "props":converted["props"]});
            if !converted["customCss"].is_null() && converted["customCss"] != "" {
                output["customCss"] = converted["customCss"].clone();
            }
            output["rejected"] = converted
                .get("rejected")
                .cloned()
                .unwrap_or_else(|| json!([]));
            return Ok(ToolResult::text(serde_json::to_string_pretty(&output)?));
        }

        let widget_type = args
            .get("widget_type")
            .and_then(Value::as_str)
            .context("widget_type required for target-aware mapping")?;
        let schema = crate::elementor::ElementorService::new(wp)
            .element_schema(widget_type)
            .await
            .context("failed to load the registered Elementor widget schema")?;
        let allowed = schema_setting_keys(&schema);
        let (settings, residual) = map_css_to_elementor_for_widget(css, widget_type, &allowed);
        let mut result = json!({"target":"classic", "settings":settings});
        result["widget_type"] = json!(widget_type);
        if !residual.is_empty() {
            result["residual"] = json!(residual);
        }
        Ok(ToolResult::text(serde_json::to_string_pretty(&result)?))
    }
}

fn schema_setting_keys(schema: &Value) -> HashSet<String> {
    schema["controls"]
        .as_object()
        .into_iter()
        .flat_map(|controls| controls.keys().cloned())
        .chain(
            schema["props_schema"]
                .as_object()
                .into_iter()
                .flat_map(|props| props.keys().cloned()),
        )
        .chain(
            schema["style_schema"]
                .as_object()
                .into_iter()
                .flat_map(|styles| styles.keys().cloned()),
        )
        .collect()
}

/// Result from Elementor's native Atomic CSS conversion endpoint.
/// The planner accepts this only when its caller has obtained it from the active editor runtime.
#[derive(Debug, Clone)]
pub struct AtomicCssConversion {
    pub settings: Value,
    pub custom_css: Option<Value>,
    pub rejected: Vec<String>,
    pub capability: String,
}

pub fn map_css_to_elementor_target(
    css: &Map<String, Value>,
    target: &str,
    widget_type: Option<&str>,
    classic_allowed_keys: Option<&HashSet<String>>,
    atomic: Option<&AtomicCssConversion>,
) -> Result<(Value, Vec<String>)> {
    match target {
        "classic" => {
            let widget_type =
                widget_type.context("classic CSS mapping requires a target widget type")?;
            let allowed = classic_allowed_keys
                .context("classic CSS mapping requires the registered widget schema")?;
            let (settings, residual) = map_css_to_elementor_for_widget(css, widget_type, allowed);
            Ok((Value::Object(settings), residual))
        }
        "atomic" => {
            let conversion =
                atomic.context("Atomic target needs a native editor conversion capability")?;
            if conversion.capability != "elementor-native-css-converter" {
                anyhow::bail!(
                    "Atomic conversion was not produced by Elementor's native CSS converter"
                );
            }
            Ok((conversion.settings.clone(), conversion.rejected.clone()))
        }
        _ => anyhow::bail!("target must be classic or atomic"),
    }
}

/// Map only classic settings with known value shapes. Invalid and unsupported declarations remain residual.
pub fn map_css_to_elementor(css: &Map<String, Value>) -> (Map<String, Value>, Vec<String>) {
    map_css(css, None, None)
}

pub fn map_css_to_elementor_for_widget(
    css: &Map<String, Value>,
    widget_type: &str,
    allowed_keys: &HashSet<String>,
) -> (Map<String, Value>, Vec<String>) {
    map_css(css, Some(widget_type), Some(allowed_keys))
}

fn map_css(
    css: &Map<String, Value>,
    widget_type: Option<&str>,
    allowed_keys: Option<&HashSet<String>>,
) -> (Map<String, Value>, Vec<String>) {
    let mut out = Map::new();
    let mut residual = Vec::new();
    for (prop, value) in css {
        let Some(raw) = value.as_str() else {
            residual.push(format!("{prop}: non-string value"));
            continue;
        };
        let value = raw.trim();
        if value.is_empty() {
            continue;
        }
        let previous = out.clone();
        let accepted = match prop.as_str() {
            "background-color" if !is_keyword(value) && !is_symbolic(value) => {
                out.insert("background_background".into(), json!("classic"));
                out.insert("background_color".into(), json!(value));
                true
            }
            "background-image" if is_gradient(value) => {
                // CSS gradients carry angle, stops and color spaces that this classic control cannot preserve.
                false
            }
            "color" if !is_keyword(value) && !is_symbolic(value) => {
                out.insert("color".into(), json!(value));
                true
            }
            "font-size" | "line-height" | "letter-spacing" => {
                if let Some(size) = parse_size(value) {
                    out.insert("typography_typography".into(), json!("custom"));
                    let key = match prop.as_str() {
                        "font-size" => "typography_font_size",
                        "line-height" => "typography_line_height",
                        _ => "typography_letter_spacing",
                    };
                    out.insert(key.into(), size);
                    true
                } else {
                    false
                }
            }
            "font-weight" if parse_weight(value) => {
                out.insert("typography_typography".into(), json!("custom"));
                out.insert("typography_font_weight".into(), json!(value));
                true
            }
            "font-family" if !value.is_empty() => {
                out.insert("typography_typography".into(), json!("custom"));
                out.insert(
                    "typography_font_family".into(),
                    json!(
                        value
                            .split(',')
                            .next()
                            .unwrap_or(value)
                            .trim()
                            .trim_matches(['"', '\''])
                    ),
                );
                true
            }
            "text-transform"
                if matches!(value, "none" | "capitalize" | "uppercase" | "lowercase") =>
            {
                out.insert("typography_typography".into(), json!("custom"));
                out.insert("typography_text_transform".into(), json!(value));
                true
            }
            "padding" | "margin" | "border-radius" => {
                if let Some(mapped) = parse_trbl(value) {
                    let key = if prop == "border-radius" {
                        "border_radius"
                    } else {
                        prop
                    };
                    out.insert(key.into(), mapped);
                    true
                } else {
                    false
                }
            }
            "padding-top" | "padding-right" | "padding-bottom" | "padding-left" | "margin-top"
            | "margin-right" | "margin-bottom" | "margin-left" => {
                if let Some(size) = parse_size(value) {
                    let key = if prop.starts_with("padding") {
                        "padding"
                    } else {
                        "margin"
                    };
                    set_trbl_part(
                        &mut out,
                        key,
                        prop.rsplit('-').next().unwrap_or_default(),
                        size,
                    );
                    true
                } else {
                    false
                }
            }
            "flex-direction"
                if matches!(value, "row" | "row-reverse" | "column" | "column-reverse") =>
            {
                out.insert("flex_direction".into(), json!(value));
                true
            }
            "justify-content"
                if matches!(
                    value,
                    "flex-start"
                        | "start"
                        | "flex-end"
                        | "end"
                        | "center"
                        | "space-between"
                        | "space-around"
                        | "space-evenly"
                ) =>
            {
                out.insert("flex_justify_content".into(), json!(flex_val(value)));
                true
            }
            "align-items"
                if matches!(
                    value,
                    "flex-start" | "start" | "flex-end" | "end" | "center" | "stretch" | "baseline"
                ) =>
            {
                out.insert("flex_align_items".into(), json!(flex_val(value)));
                true
            }
            "width" | "height" | "min-height" | "max-width" | "gap" => {
                if let Some(size) = parse_size(value) {
                    let key = match prop.as_str() {
                        "gap" => "flex_gap",
                        "min-height" => "min_height",
                        "max-width" => "max_width",
                        _ => prop,
                    };
                    if prop == "width" {
                        out.insert("_element_width".into(), json!("initial"));
                        out.insert("_element_custom_width".into(), size);
                    } else {
                        out.insert(key.into(), size);
                    }
                    true
                } else {
                    false
                }
            }
            "opacity" => {
                if let Ok(n) = value.parse::<f64>() {
                    if (0.0..=1.0).contains(&n) {
                        out.insert("_opacity".into(), json!({"size": n, "unit": ""}));
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            "z-index" => {
                if let Ok(n) = value.parse::<i64>() {
                    out.insert("z_index".into(), json!({"size": n, "unit": ""}));
                    true
                } else {
                    false
                }
            }
            "position" if matches!(value, "absolute" | "fixed" | "relative" | "sticky") => {
                out.insert("position".into(), json!(value));
                true
            }
            "overflow" if matches!(value, "hidden" | "visible" | "scroll" | "auto") => {
                out.insert("overflow".into(), json!(value));
                true
            }
            // display, states, gradients, borders and shadows are not represented without widget-specific context.
            _ => false,
        };
        if accepted {
            remap_widget_keys(&mut out, prop, widget_type, allowed_keys);
        }
        if accepted && allowed_keys.is_some_and(|keys| out.keys().any(|key| !keys.contains(key))) {
            out = previous;
            residual.push(format!("{prop}: {value}"));
        } else if !accepted {
            residual.push(format!("{prop}: {value}"));
        }
    }
    (out, residual)
}

fn remap_widget_keys(
    out: &mut Map<String, Value>,
    property: &str,
    widget: Option<&str>,
    allowed: Option<&HashSet<String>>,
) {
    if property == "background-color" && widget == Some("button") {
        out.remove("background_background");
    }
    let alternatives: &[(&str, &str)] = match property {
        "background-color" if widget == Some("button") => &[
            ("background_background", ""),
            ("background_color", "background_color"),
        ],
        "background-color" => &[
            ("background_background", "_background_background"),
            ("background_color", "_background_color"),
        ],
        "color" => match widget {
            Some("heading") => &[("color", "title_color")],
            Some("text-editor") => &[("color", "text_color")],
            Some("button") => &[("color", "button_text_color")],
            _ => &[("color", "")],
        },
        "padding" | "padding-top" | "padding-right" | "padding-bottom" | "padding-left" => {
            &[("padding", "_padding")]
        }
        "margin" | "margin-top" | "margin-right" | "margin-bottom" | "margin-left" => {
            &[("margin", "_margin")]
        }
        "border-radius" => &[("border_radius", "_border_radius")],
        "z-index" => &[("z_index", "_z_index")],
        "position" => &[("position", "_position")],
        "opacity" if widget == Some("image") => &[("_opacity", "opacity")],
        _ => &[],
    };
    for (from, to) in alternatives {
        if to.is_empty() {
            continue;
        }
        if let Some(value) = out.remove(*from) {
            let chosen = if allowed.is_some_and(|keys| !keys.contains(*to) && keys.contains(*from))
            {
                *from
            } else {
                *to
            };
            out.insert(chosen.into(), value);
        }
    }
}

fn is_keyword(value: &str) -> bool {
    matches!(
        value,
        "inherit" | "initial" | "unset" | "revert" | "revert-layer"
    )
}
fn is_symbolic(value: &str) -> bool {
    value.contains("var(")
        || value.contains("calc(")
        || value.contains("env(")
        || value == "currentColor"
}
fn is_gradient(value: &str) -> bool {
    value.starts_with("linear-gradient(")
        || value.starts_with("radial-gradient(")
        || value.starts_with("conic-gradient(")
}
fn parse_weight(value: &str) -> bool {
    matches!(value, "normal" | "bold" | "bolder" | "lighter")
        || value.parse::<u16>().is_ok_and(|n| (1..=1000).contains(&n))
}

fn parse_size(value: &str) -> Option<Value> {
    if matches!(value, "auto" | "normal" | "none") {
        return None;
    }
    let split = value
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '+' | '-')))
        .unwrap_or(value.len());
    let (number, unit) = value.split_at(split);
    let number = number.parse::<f64>().ok()?;
    if !number.is_finite()
        || !matches!(
            unit,
            "px" | "rem" | "em" | "%" | "vw" | "vh" | "" | "ch" | "ex"
        )
    {
        return None;
    }
    Some(json!({"size": number, "unit": unit}))
}

fn parse_trbl(value: &str) -> Option<Value> {
    let parts: Vec<Value> = value
        .split_whitespace()
        .map(parse_size)
        .collect::<Option<_>>()?;
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    let first_unit = parts[0]["unit"].as_str()?;
    if parts.iter().any(|part| part["unit"] != first_unit) {
        return None;
    }
    let n = |i: usize| parts[i]["size"].as_f64().map(|f| f.to_string());
    let top = n(0)?;
    let right = n(if parts.len() > 1 { 1 } else { 0 })?;
    let bottom = n(if parts.len() > 2 { 2 } else { 0 })?;
    let left = n(if parts.len() > 3 {
        3
    } else if parts.len() > 1 {
        1
    } else {
        0
    })?;
    Some(
        json!({"top":top,"right":right,"bottom":bottom,"left":left,"unit":first_unit,"isLinked":parts.len()==1}),
    )
}

fn set_trbl_part(out: &mut Map<String, Value>, key: &str, side: &str, size: Value) {
    let entry = out.entry(key.to_string()).or_insert_with(|| json!({"top":"0","right":"0","bottom":"0","left":"0","unit":size["unit"],"isLinked":false}));
    if let Some(object) = entry.as_object_mut() {
        object.insert(
            side.to_string(),
            json!(
                size["size"]
                    .as_f64()
                    .map_or_else(|| "0".into(), |n| n.to_string())
            ),
        );
        object.insert("unit".into(), size["unit"].clone());
        object.insert("isLinked".into(), json!(false));
    }
}

fn flex_val(value: &str) -> &str {
    match value {
        "start" => "flex-start",
        "end" => "flex-end",
        _ => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_and_symbolic_css_stays_residual() {
        let css = serde_json::from_value::<Map<String, Value>>(json!({
            "font-size":"1.25rem", "width":"100%", "background-image":"linear-gradient(90deg, red, blue)",
            "display":"grid", "color":"var(--brand-color)"
        })).unwrap();
        let (settings, residual) = map_css_to_elementor(&css);
        assert_eq!(settings["typography_font_size"]["unit"], "rem");
        assert_eq!(settings["_element_custom_width"]["size"], 100.0);
        assert_eq!(settings["_element_custom_width"]["unit"], "%");
        assert_eq!(residual.len(), 3);
        assert!(
            residual
                .iter()
                .any(|item| item.starts_with("background-image:"))
        );
        assert!(residual.iter().any(|item| item.starts_with("display:")));
        assert!(residual.iter().any(|item| item.starts_with("color:")));

        let heading_keys = HashSet::from(["title_color".to_string()]);
        let heading_css = serde_json::from_value::<Map<String, Value>>(
            json!({"color":"#123456","font-size":"18px"}),
        )
        .unwrap();
        let (heading, heading_residual) =
            map_css_to_elementor_for_widget(&heading_css, "heading", &heading_keys);
        assert_eq!(heading["title_color"], "#123456");
        assert_eq!(heading_residual, vec!["font-size: 18px"]);
    }
}
