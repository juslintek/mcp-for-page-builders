//! Builder helpers for constructing Elementor JSON in tests.

use elementor_mcp::elementor::{Element, generate_id};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Build a widget element.
pub fn widget(widget_type: &str, settings: Value) -> Element {
    Element {
        id: generate_id(),
        el_type: "widget".into(),
        widget_type: Some(widget_type.into()),
        settings,
        elements: vec![],
        extra: HashMap::new(),
    }
}

/// Build a container with children.
pub fn container(children: Vec<Element>) -> Element {
    container_with(json!({}), children)
}

/// Build a container with settings and children.
pub fn container_with(settings: Value, children: Vec<Element>) -> Element {
    Element {
        id: generate_id(),
        el_type: "container".into(),
        widget_type: None,
        settings,
        elements: children,
        extra: HashMap::new(),
    }
}

/// Build a multi-column layout: N containers side by side inside a parent.
pub fn columns(n: usize, children_per_col: Vec<Vec<Element>>) -> Element {
    let cols: Vec<Element> = children_per_col.into_iter().map(|kids| {
        container_with(json!({"_column_size": 100 / n}), kids)
    }).collect();
    container_with(json!({"flex_direction": "row"}), cols)
}

// ── Widget shortcuts ──────────────────────────────────────────────────────────

pub fn heading(text: &str, tag: &str) -> Element {
    widget("heading", json!({"title": text, "header_size": tag}))
}

pub fn heading_styled(text: &str, tag: &str, color: &str, size_px: u32) -> Element {
    widget("heading", json!({
        "title": text, "header_size": tag, "title_color": color, "align": "center",
        "typography_typography": "custom", "typography_font_size": {"unit": "px", "size": size_px}
    }))
}

pub fn text(html: &str) -> Element {
    widget("text-editor", json!({"editor": html}))
}

pub fn image(url: &str) -> Element {
    widget("image", json!({"image": {"url": url, "id": ""}, "image_size": "full"}))
}

pub fn button(label: &str, url: &str) -> Element {
    widget("button", json!({"text": label, "link": {"url": url, "is_external": false}}))
}

pub fn button_styled(label: &str, url: &str, bg: &str, color: &str) -> Element {
    widget("button", json!({
        "text": label, "link": {"url": url}, "size": "lg",
        "background_color": bg, "button_text_color": color
    }))
}

pub fn icon_list(items: Vec<(&str, &str)>) -> Element {
    let list: Vec<Value> = items.into_iter().map(|(text, icon)| {
        json!({"text": text, "selected_icon": {"value": icon, "library": "fa-solid"}})
    }).collect();
    widget("icon-list", json!({"icon_list": list}))
}

pub fn nav_menu(menu_name: &str) -> Element {
    widget("nav-menu", json!({"menu": menu_name, "layout": "horizontal", "pointer": "underline", "submenu_icon": {"value":"fas fa-angle-down","library":"fa-solid"}}))
}

pub fn nav_menu_styled(menu_name: &str, text_color: &str, pointer_color: &str) -> Element {
    widget("nav-menu", json!({
        "menu": menu_name, "layout": "horizontal", "pointer": "underline",
        "color_menu_item": text_color, "pointer_color": pointer_color,
        "submenu_icon": {"value":"fas fa-angle-down","library":"fa-solid"}
    }))
}

pub fn divider() -> Element { widget("divider", json!({"style": "solid"})) }
pub fn spacer(px: u32) -> Element { widget("spacer", json!({"space": {"unit": "px", "size": px}})) }
pub fn html_widget(code: &str) -> Element { widget("html", json!({"html": code})) }

pub fn video(youtube_url: &str) -> Element {
    widget("video", json!({"video_type": "youtube", "youtube_url": youtube_url}))
}

pub fn icon_box(icon: &str, title: &str, desc: &str) -> Element {
    widget("icon-box", json!({
        "selected_icon": {"value": icon, "library": "fa-solid"},
        "title_text": title, "description_text": desc
    }))
}

pub fn image_box(img_url: &str, title: &str, desc: &str) -> Element {
    widget("image-box", json!({
        "image": {"url": img_url}, "title_text": title, "description_text": desc
    }))
}

pub fn counter(number: u32, title: &str) -> Element {
    widget("counter", json!({"ending_number": number, "title": title}))
}

pub fn progress_bar(title: &str, percent: u32) -> Element {
    widget("progress-bar", json!({"title": title, "percent": percent}))
}

pub fn testimonial(content: &str, name: &str, job: &str) -> Element {
    widget("testimonial", json!({
        "testimonial_content": content, "testimonial_name": name, "testimonial_job": job
    }))
}

pub fn star_rating(score: f32, title: &str) -> Element {
    widget("star-rating", json!({"rating": score, "title": title}))
}

pub fn alert(title: &str, desc: &str, alert_type: &str) -> Element {
    widget("alert", json!({"alert_title": title, "alert_description": desc, "alert_type": alert_type}))
}

pub fn toggle(items: Vec<(&str, &str)>) -> Element {
    let tabs: Vec<Value> = items.into_iter().map(|(title, content)| {
        json!({"tab_title": title, "tab_content": content})
    }).collect();
    widget("toggle", json!({"tabs": tabs}))
}

pub fn accordion(items: Vec<(&str, &str)>) -> Element {
    let tabs: Vec<Value> = items.into_iter().map(|(title, content)| {
        json!({"tab_title": title, "tab_content": content})
    }).collect();
    widget("accordion", json!({"tabs": tabs}))
}

pub fn tabs(items: Vec<(&str, &str)>) -> Element {
    let t: Vec<Value> = items.into_iter().map(|(title, content)| {
        json!({"tab_title": title, "tab_content": content})
    }).collect();
    widget("tabs", json!({"tabs": t}))
}

pub fn social_icons(networks: Vec<&str>) -> Element {
    let list: Vec<Value> = networks.into_iter().map(|n| {
        json!({"social_icon": {"value": format!("fab fa-{n}"), "library": "fa-brands"}, "link": {"url": "#"}})
    }).collect();
    widget("social-icons", json!({"social_icon_list": list}))
}

// ── HTML to Elementor conversion ──────────────────────────────────────────────

/// Convert simple HTML into Elementor elements.
/// Maps: h1-h6 → heading, p → text-editor, img → image, a.button → button,
/// everything else → html widget.
pub fn html_to_elements(html: &str) -> Vec<Element> {
    let mut elements = Vec::new();
    let mut remaining = html.trim();

    while !remaining.is_empty() {
        remaining = remaining.trim_start();
        if remaining.is_empty() { break; }

        if let Some(el) = try_parse_heading(remaining) {
            elements.push(el.0);
            remaining = el.1;
        } else if let Some(el) = try_parse_paragraph(remaining) {
            elements.push(el.0);
            remaining = el.1;
        } else if let Some(el) = try_parse_img(remaining) {
            elements.push(el.0);
            remaining = el.1;
        } else if let Some(el) = try_parse_tag(remaining) {
            // Fallback: wrap unknown block in html widget
            elements.push(html_widget(el.0));
            remaining = el.1;
        } else {
            // Plain text remainder → text-editor
            elements.push(text(&format!("<p>{remaining}</p>")));
            break;
        }
    }
    elements
}

fn try_parse_heading(s: &str) -> Option<(Element, &str)> {
    for n in 1..=6 {
        let open = format!("<h{n}");
        let close = format!("</h{n}>");
        if s.starts_with(&open)
            && let Some(end) = s.find(&close) {
                let full = &s[..end + close.len()];
                // Extract inner text (strip tags)
                let inner = full.split('>').nth(1).unwrap_or("").split('<').next().unwrap_or("").trim();
                return Some((heading(inner, &format!("h{n}")), &s[end + close.len()..]));
            }
    }
    None
}

fn try_parse_paragraph(s: &str) -> Option<(Element, &str)> {
    if !s.starts_with("<p") { return None; }
    let close = "</p>";
    let end = s.find(close)?;
    let full = &s[..end + close.len()];
    Some((text(full), &s[end + close.len()..]))
}

fn try_parse_img(s: &str) -> Option<(Element, &str)> {
    if !s.starts_with("<img") { return None; }
    let end = s.find('>')? + 1;
    let tag = &s[..end];
    let src = extract_attr(tag, "src").unwrap_or_default();
    Some((image(&src), &s[end..]))
}

fn try_parse_tag(s: &str) -> Option<(&str, &str)> {
    if !s.starts_with('<') { return None; }
    let tag_name_end = s[1..].find([' ', '>', '/'])? + 1;
    let tag_name = &s[1..tag_name_end];
    // Self-closing
    if let Some(end) = s.find("/>") {
        let e = end + 2;
        return Some((&s[..e], &s[e..]));
    }
    let close = format!("</{tag_name}>");
    if let Some(end) = s.find(&close) {
        let e = end + close.len();
        return Some((&s[..e], &s[e..]));
    }
    // No closing tag — take until next tag
    let next = s[1..].find('<').map_or(s.len(), |i| i + 1);
    Some((&s[..next], &s[next..]))
}

fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let pattern = format!("{attr}=\"");
    let start = tag.find(&pattern)? + pattern.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_string())
}

// ── Page helpers ──────────────────────────────────────────────────────────────

/// Serialize elements to the JSON string format `WordPress` expects.
pub fn to_elementor_data(elements: &[Element]) -> String {
    serde_json::to_string(elements).unwrap()
}
