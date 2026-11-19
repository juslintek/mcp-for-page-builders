use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtomicVariant {
    pub meta: AtomicVariantMeta,
    pub props: Map<String, Value>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub clear_all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AtomicVariantMeta {
    pub breakpoint: String,
    pub state: Option<String>,
}

/// Parse a plain CSS string (supporting `&:hover`/`&:focus`/`&:active` and `@media(--breakpoint)`)
/// into Elementor V4 atomic style variants.
pub fn parse_css_to_variants(css: &str) -> Result<Vec<AtomicVariant>> {
    let cleaned = strip_css_comments(css);
    let mut variants: BTreeMap<AtomicVariantMeta, (Map<String, Value>, bool)> = BTreeMap::new();
    parse_block(&cleaned, "desktop", None, &mut variants)?;
    Ok(variants
        .into_iter()
        .map(|(meta, (props, clear_all))| AtomicVariant {
            meta,
            props,
            clear_all,
        })
        .collect())
}

/// Convert parsed variants into Elementor V4 style `variants` JSON array (for `styles` / `Global_Classes_Repository`).
pub fn variants_to_elementor_json(variants: &[AtomicVariant]) -> Vec<Value> {
    variants
        .iter()
        .map(|v| {
            json!({
                "meta": {
                    "breakpoint": v.meta.breakpoint,
                    "state": v.meta.state,
                },
                "props": v.props,
            })
        })
        .collect()
}

fn strip_css_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
        } else {
            let ch = input[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn parse_block(
    body: &str,
    breakpoint: &str,
    state: Option<&str>,
    variants: &mut BTreeMap<AtomicVariantMeta, (Map<String, Value>, bool)>,
) -> Result<()> {
    let mut i = 0;
    let len = body.len();
    while i < len {
        // Skip whitespace and stray semicolons
        while i < len {
            let ch = body[i..].chars().next().unwrap();
            if ch.is_whitespace() || ch == ';' {
                i += ch.len_utf8();
            } else {
                break;
            }
        }
        if i >= len {
            break;
        }

        // Find next '{' or ';' at depth 0 (respecting quotes and parens)
        let start = i;
        let mut paren_depth = 0i32;
        let mut quote: Option<char> = None;
        let mut found_brace = false;
        let mut found_semi = false;

        while i < len {
            let ch = body[i..].chars().next().unwrap();
            if let Some(q) = quote {
                if ch == '\\' {
                    i += ch.len_utf8();
                    if i < len {
                        i += body[i..].chars().next().unwrap().len_utf8();
                    }
                    continue;
                }
                if ch == q {
                    quote = None;
                }
                i += ch.len_utf8();
                continue;
            }
            match ch {
                '\'' | '"' => {
                    quote = Some(ch);
                    i += ch.len_utf8();
                }
                '(' => {
                    paren_depth += 1;
                    i += 1;
                }
                ')' => {
                    paren_depth = (paren_depth - 1).max(0);
                    i += 1;
                }
                '{' if paren_depth == 0 => {
                    found_brace = true;
                    break;
                }
                ';' if paren_depth == 0 => {
                    found_semi = true;
                    break;
                }
                _ => {
                    i += ch.len_utf8();
                }
            }
        }

        if found_brace {
            let header = body[start..i].trim();
            i += 1; // skip '{'
            let block_start = i;
            let mut depth = 1i32;
            let mut q: Option<char> = None;
            while i < len && depth > 0 {
                let ch = body[i..].chars().next().unwrap();
                if let Some(qc) = q {
                    if ch == '\\' {
                        i += ch.len_utf8();
                        if i < len {
                            i += body[i..].chars().next().unwrap().len_utf8();
                        }
                        continue;
                    }
                    if ch == qc {
                        q = None;
                    }
                    i += ch.len_utf8();
                    continue;
                }
                match ch {
                    '\'' | '"' => q = Some(ch),
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                if depth > 0 {
                    i += ch.len_utf8();
                }
            }
            if depth != 0 {
                bail!("Unclosed '{{' in CSS block '{header}'");
            }
            let inner = &body[block_start..i];
            i += 1; // skip '}'

            if let Some(bp) = parse_media_breakpoint(header) {
                parse_block(inner, &bp, state, variants)?;
            } else if let Some(st) = parse_pseudo_state(header) {
                parse_block(inner, breakpoint, Some(&st), variants)?;
            } else {
                bail!("Unsupported nested CSS block '{header}'; use &:hover/&:focus/&:active or @media(--breakpoint)");
            }
        } else {
            let decl = body[start..i].trim();
            if found_semi {
                i += 1;
            }
            if !decl.is_empty() {
                apply_declaration(decl, breakpoint, state, variants)?;
            }
        }
    }
    Ok(())
}

fn parse_media_breakpoint(header: &str) -> Option<String> {
    let trimmed = header.trim();
    if !trimmed.starts_with("@media") {
        return None;
    }
    let rest = trimmed["@media".len()..].trim();
    let inner = rest.strip_prefix('(')?.strip_suffix(')')?.trim();
    if let Some(bp) = inner.strip_prefix("--") {
        return Some(bp.trim().to_ascii_lowercase());
    }
    // Support standard max-width breakpoints as convenience
    let lower = inner.to_ascii_lowercase();
    if lower.contains("767") {
        return Some("mobile".to_string());
    }
    if lower.contains("1024") || lower.contains("1023") {
        return Some("tablet".to_string());
    }
    None
}

fn parse_pseudo_state(header: &str) -> Option<String> {
    let trimmed = header.trim();
    let rest = trimmed.strip_prefix("&:")?.trim().to_ascii_lowercase();
    match rest.as_str() {
        "hover" | "focus" | "active" | "hover-within" | "focus-visible" => Some(rest),
        _ => None,
    }
}

fn apply_declaration(
    decl: &str,
    breakpoint: &str,
    state: Option<&str>,
    variants: &mut BTreeMap<AtomicVariantMeta, (Map<String, Value>, bool)>,
) -> Result<()> {
    let (prop_raw, val_raw) = decl
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("Invalid CSS declaration '{decl}'"))?;
    let prop = prop_raw.trim().to_ascii_lowercase();
    let val = val_raw.trim();
    if prop.is_empty() || val.is_empty() {
        bail!("Empty CSS property or value in '{decl}'");
    }

    let meta = AtomicVariantMeta {
        breakpoint: breakpoint.to_string(),
        state: state.map(str::to_string),
    };
    let entry = variants.entry(meta).or_insert_with(|| (Map::new(), false));

    if prop == "all" && val == "null" {
        entry.0.clear();
        entry.1 = true;
        return Ok(());
    }
    if val == "null" {
        entry.0.insert(normalize_prop_name(&prop), Value::Null);
        return Ok(());
    }

    let normalized = normalize_prop_name(&prop);
    let typed = css_value_to_atomic(&normalized, val);
    entry.0.insert(normalized, typed);
    Ok(())
}

fn normalize_prop_name(prop: &str) -> String {
    match prop {
        "background" => "background-color".to_string(),
        other => other.to_string(),
    }
}

pub fn css_value_to_atomic(prop: &str, val: &str) -> Value {
    let trimmed = val.trim();

    // var(--label) reference
    if let Some(inner) = trimmed.strip_prefix("var(").and_then(|s| s.strip_suffix(')')) {
        let var_name = inner.split(',').next().unwrap_or(inner).trim();
        if let Some(label) = var_name.strip_prefix("--") {
            let var_type = if is_color_prop(prop) {
                "global-color-variable"
            } else if prop == "font-family" {
                "global-font-variable"
            } else {
                "global-size-variable"
            };
            return json!({
                "$$type": var_type,
                "value": label,
            });
        }
    }

    if is_color_prop(prop) {
        return json!({
            "$$type": "color",
            "value": trimmed,
        });
    }

    if matches!(prop, "padding" | "margin") {
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if let Some((top, right, bottom, left)) = expand_box_sides(&parts) {
            return json!({
                "$$type": "dimensions",
                "value": {
                    "block-start": parse_size_or_string(top),
                    "inline-end": parse_size_or_string(right),
                    "block-end": parse_size_or_string(bottom),
                    "inline-start": parse_size_or_string(left),
                }
            });
        }
    }

    if is_size_prop(prop)
        && let Some(size_val) = try_parse_size(trimmed)
    {
        return size_val;
    }

    json!({
        "$$type": "string",
        "value": trimmed,
    })
}

fn is_color_prop(prop: &str) -> bool {
    matches!(
        prop,
        "color" | "background-color" | "border-color" | "outline-color" | "fill" | "stroke"
    )
}

fn is_size_prop(prop: &str) -> bool {
    matches!(
        prop,
        "font-size"
            | "line-height"
            | "letter-spacing"
            | "width"
            | "height"
            | "min-width"
            | "min-height"
            | "max-width"
            | "max-height"
            | "gap"
            | "row-gap"
            | "column-gap"
            | "border-width"
            | "border-radius"
            | "top"
            | "right"
            | "bottom"
            | "left"
            | "padding-top"
            | "padding-right"
            | "padding-bottom"
            | "padding-left"
            | "margin-top"
            | "margin-right"
            | "margin-bottom"
            | "margin-left"
            | "flex-basis"
    )
}

fn expand_box_sides<'a>(parts: &[&'a str]) -> Option<(&'a str, &'a str, &'a str, &'a str)> {
    match parts.len() {
        1 => Some((parts[0], parts[0], parts[0], parts[0])),
        2 => Some((parts[0], parts[1], parts[0], parts[1])),
        3 => Some((parts[0], parts[1], parts[2], parts[1])),
        4 => Some((parts[0], parts[1], parts[2], parts[3])),
        _ => None,
    }
}

fn parse_size_or_string(s: &str) -> Value {
    try_parse_size(s).unwrap_or_else(|| json!({ "$$type": "string", "value": s }))
}

pub fn try_parse_size(raw: &str) -> Option<Value> {
    let s = raw.trim();
    if s == "0" {
        return Some(json!({
            "$$type": "size",
            "value": { "size": 0, "unit": "px" }
        }));
    }
    let unit_start = s
        .char_indices()
        .find(|&(idx, c)| !c.is_ascii_digit() && c != '.' && !(idx == 0 && c == '-'))
        .map(|(idx, _)| idx)?;
    let num_str = &s[..unit_start];
    let unit = &s[unit_start..];
    if num_str.is_empty() || unit.is_empty() {
        return None;
    }
    if !matches!(
        unit,
        "px" | "em" | "rem" | "%" | "vh" | "vw" | "vmin" | "vmax" | "ch" | "svh" | "dvh"
    ) {
        return None;
    }
    let num: f64 = num_str.parse().ok()?;
    let size_json = if num.fract() == 0.0 && num.abs() < (i64::MAX as f64) {
        json!(num as i64)
    } else {
        json!(num)
    };
    Some(json!({
        "$$type": "size",
        "value": { "size": size_json, "unit": unit }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_hover_and_media_breakpoints() {
        let css = r#"
            padding: 24px 16px;
            color: var(--primary);
            &:hover {
                background-color: #111111;
            }
            @media(--mobile) {
                padding: 12px;
                &:hover {
                    color: #ffffff;
                }
            }
        "#;
        let variants = parse_css_to_variants(css).unwrap();
        assert_eq!(variants.len(), 4);

        let desktop = variants
            .iter()
            .find(|v| v.meta.breakpoint == "desktop" && v.meta.state.is_none())
            .unwrap();
        assert_eq!(desktop.props["color"]["$$type"], "global-color-variable");
        assert_eq!(desktop.props["color"]["value"], "primary");
        assert_eq!(desktop.props["padding"]["$$type"], "dimensions");
        assert_eq!(desktop.props["padding"]["value"]["block-start"]["value"]["size"], 24);
        assert_eq!(desktop.props["padding"]["value"]["inline-end"]["value"]["size"], 16);

        let desktop_hover = variants
            .iter()
            .find(|v| v.meta.breakpoint == "desktop" && v.meta.state.as_deref() == Some("hover"))
            .unwrap();
        assert_eq!(desktop_hover.props["background-color"]["value"], "#111111");

        let mobile = variants
            .iter()
            .find(|v| v.meta.breakpoint == "mobile" && v.meta.state.is_none())
            .unwrap();
        assert_eq!(mobile.props["padding"]["value"]["block-start"]["value"]["size"], 12);
    }

    #[test]
    fn supports_null_and_all_null_deletions() {
        let css = "color: null; &:hover { all: null; }";
        let variants = parse_css_to_variants(css).unwrap();
        let desktop = variants
            .iter()
            .find(|v| v.meta.breakpoint == "desktop" && v.meta.state.is_none())
            .unwrap();
        assert_eq!(desktop.props["color"], Value::Null);
        let hover = variants
            .iter()
            .find(|v| v.meta.breakpoint == "desktop" && v.meta.state.as_deref() == Some("hover"))
            .unwrap();
        assert!(hover.clear_all);
    }
}
