use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use super::Tool;

fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)?.as_str().map(|s| s.to_string())
}

// ── Schema types ──────────────────────────────────────────────────────────────

pub struct WidgetSchema {
    widget_type: &'static str,
    category: &'static str,
    /// Known valid setting keys for this widget.
    settings: &'static [&'static str],
    /// Common mistakes: wrong_key → correct_key.
    aliases: &'static [(&'static str, &'static str)],
}

// ── Common settings shared by ALL widgets ─────────────────────────────────────

const COMMON_SETTINGS: &[&str] = &[
    // Layout
    "_element_width", "_element_custom_width", "_flex_size", "_position",
    // Background
    "_background_background", "_background_color", "_background_image",
    "_background_hover_background", "_background_hover_color",
    // Border
    "_border_border", "_border_width", "_border_color", "_border_radius",
    "_border_hover_border", "_border_hover_color",
    // Spacing
    "_margin", "_margin_tablet", "_margin_mobile",
    "_padding", "_padding_tablet", "_padding_mobile",
    // Responsive
    "hide_desktop", "hide_tablet", "hide_mobile",
    // Motion
    "_animation", "_animation_delay", "_hover_animation",
    // Advanced
    "_css_classes", "_element_id", "_z_index",
    // Custom CSS (Pro)
    "custom_css",
];

// ── Typography settings (shared prefix pattern) ───────────────────────────────

const TYPOGRAPHY_KEYS: &[&str] = &[
    "typography_typography", "typography_font_family", "typography_font_size",
    "typography_font_size_tablet", "typography_font_size_mobile",
    "typography_font_weight", "typography_text_transform", "typography_font_style",
    "typography_text_decoration", "typography_line_height", "typography_letter_spacing",
    "typography_word_spacing",
];

// ── Common aliases (mistakes → correct) ───────────────────────────────────────

const COMMON_ALIASES: &[(&str, &str)] = &[
    ("font_size", "typography_font_size"),
    ("font_family", "typography_font_family"),
    ("font_weight", "typography_font_weight"),
    ("line_height", "typography_line_height"),
    ("letter_spacing", "typography_letter_spacing"),
    ("text_transform", "typography_text_transform"),
    ("background", "_background_background"),
    ("background_color", "_background_color"),
    ("margin", "_margin"),
    ("padding", "_padding"),
    ("border", "_border_border"),
    ("border_radius", "_border_radius"),
    ("css_classes", "_css_classes"),
    ("element_id", "_element_id"),
    ("animation", "_animation"),
    ("z_index", "_z_index"),
    ("class", "_css_classes"),
    ("id", "_element_id"),
];

// ── Widget schemas ────────────────────────────────────────────────────────────

fn all_schemas() -> Vec<WidgetSchema> {
    vec![
        WidgetSchema {
            widget_type: "heading",
            category: "basic",
            settings: &[
                "title", "link", "header_size", "align", "align_tablet", "align_mobile",
                "title_color", "title_color_hover",
                "blend_mode", "text_stroke_stroke_type", "text_stroke_color",
            ],
            aliases: &[
                ("text", "title"), ("content", "title"), ("size", "header_size"),
                ("color", "title_color"), ("tag", "header_size"),
                ("text_align", "align"), ("alignment", "align"),
            ],
        },
        WidgetSchema {
            widget_type: "text-editor",
            category: "basic",
            settings: &[
                "editor", "align", "align_tablet", "align_mobile",
                "text_color", "columns", "column_gap", "drop_cap",
            ],
            aliases: &[
                ("content", "editor"), ("text", "editor"), ("html", "editor"),
                ("color", "text_color"),
            ],
        },
        WidgetSchema {
            widget_type: "image",
            category: "basic",
            settings: &[
                "image", "image_size", "align", "align_tablet", "align_mobile",
                "caption_source", "caption", "link_to", "link",
                "width", "max_width", "height", "opacity", "hover_opacity",
                "hover_animation", "border_radius", "image_border_radius",
                "object_fit", "object_position",
            ],
            aliases: &[
                ("src", "image"), ("url", "image"), ("alt", "image"),
                ("size", "image_size"),
            ],
        },
        WidgetSchema {
            widget_type: "button",
            category: "basic",
            settings: &[
                "text", "link", "align", "align_tablet", "align_mobile",
                "size", "icon", "icon_align", "icon_indent",
                "button_type", "button_text_color", "background_color",
                "button_text_color_hover", "button_background_hover_color",
                "border_radius", "text_padding",
                "button_css_id",
            ],
            aliases: &[
                ("label", "text"), ("title", "text"), ("href", "link"),
                ("color", "button_text_color"), ("bg_color", "background_color"),
            ],
        },
        WidgetSchema {
            widget_type: "icon-list",
            category: "basic",
            settings: &[
                "icon_list", "view", "size", "icon_color", "icon_color_hover",
                "text_color", "text_color_hover", "text_indent",
                "icon_size", "icon_self_align", "divider", "divider_style",
                "divider_color", "divider_weight",
            ],
            aliases: &[("items", "icon_list"), ("list", "icon_list")],
        },
        WidgetSchema {
            widget_type: "divider",
            category: "basic",
            settings: &[
                "style", "weight", "color", "width", "width_tablet", "width_mobile",
                "align", "gap", "gap_tablet", "gap_mobile",
                "look", "text", "icon", "element_tag",
            ],
            aliases: &[],
        },
        WidgetSchema {
            widget_type: "spacer",
            category: "basic",
            settings: &["space", "space_tablet", "space_mobile"],
            aliases: &[("height", "space"), ("size", "space")],
        },
        WidgetSchema {
            widget_type: "html",
            category: "basic",
            settings: &["html"],
            aliases: &[("content", "html"), ("code", "html")],
        },
        WidgetSchema {
            widget_type: "video",
            category: "basic",
            settings: &[
                "video_type", "youtube_url", "vimeo_url", "dailymotion_url",
                "hosted_url", "external_url", "start", "end",
                "autoplay", "mute", "loop", "controls", "modestbranding",
                "lazy_load", "aspect_ratio", "lightbox",
            ],
            aliases: &[("url", "youtube_url"), ("src", "youtube_url")],
        },
        WidgetSchema {
            widget_type: "icon-box",
            category: "basic",
            settings: &[
                "selected_icon", "view", "shape", "title_text", "description_text",
                "link", "position", "title_size",
                "primary_color", "secondary_color", "hover_primary_color",
                "icon_space", "icon_size", "title_color", "title_bottom_space",
                "description_color",
            ],
            aliases: &[
                ("icon", "selected_icon"), ("title", "title_text"),
                ("description", "description_text"),
            ],
        },
        WidgetSchema {
            widget_type: "image-box",
            category: "basic",
            settings: &[
                "image", "image_size", "title_text", "description_text",
                "link", "position", "title_size",
                "title_color", "title_bottom_space", "description_color",
                "image_space", "content_vertical_alignment",
            ],
            aliases: &[
                ("title", "title_text"), ("description", "description_text"),
            ],
        },
        WidgetSchema {
            widget_type: "toggle",
            category: "basic",
            settings: &[
                "tabs", "border_color", "border_width",
                "title_background", "title_color", "title_active_color",
                "tab_active_color", "content_background_color",
                "icon", "icon_active", "icon_color", "icon_active_color",
                "icon_align", "icon_space",
            ],
            aliases: &[("items", "tabs")],
        },
        WidgetSchema {
            widget_type: "accordion",
            category: "basic",
            settings: &[
                "tabs", "selected_icon", "selected_active_icon",
                "title_background", "title_color", "tab_active_color",
                "title_padding", "icon_align", "icon_color", "icon_active_color",
                "icon_space", "content_background_color", "content_color",
                "content_padding", "border_color", "border_width",
            ],
            aliases: &[("items", "tabs"), ("icon", "selected_icon")],
        },
        WidgetSchema {
            widget_type: "tabs",
            category: "basic",
            settings: &[
                "tabs", "type", "border_color", "border_width",
                "background_color", "heading_color", "active_color",
                "heading_width", "content_color",
            ],
            aliases: &[("items", "tabs")],
        },
        WidgetSchema {
            widget_type: "counter",
            category: "basic",
            settings: &[
                "starting_number", "ending_number", "prefix", "suffix",
                "duration", "thousand_separator", "thousand_separator_char",
                "title", "number_color", "title_color",
            ],
            aliases: &[("number", "ending_number"), ("value", "ending_number")],
        },
        WidgetSchema {
            widget_type: "progress-bar",
            category: "basic",
            settings: &[
                "title", "percent", "display_percentage", "inner_text",
                "bar_color", "bar_bg_color", "bar_height", "bar_border_radius",
                "title_color",
            ],
            aliases: &[("value", "percent"), ("progress", "percent")],
        },
        WidgetSchema {
            widget_type: "testimonial",
            category: "basic",
            settings: &[
                "testimonial_content", "testimonial_image", "testimonial_name",
                "testimonial_job", "testimonial_image_position",
                "testimonial_alignment", "content_content_color",
                "name_text_color", "job_text_color",
            ],
            aliases: &[
                ("content", "testimonial_content"), ("name", "testimonial_name"),
                ("image", "testimonial_image"), ("job", "testimonial_job"),
            ],
        },
        WidgetSchema {
            widget_type: "social-icons",
            category: "basic",
            settings: &[
                "social_icon_list", "shape", "columns",
                "icon_color", "icon_primary_color", "icon_secondary_color",
                "icon_size", "icon_padding", "icon_spacing",
                "hover_primary_color", "hover_secondary_color",
                "hover_border_color", "hover_animation",
            ],
            aliases: &[("icons", "social_icon_list"), ("items", "social_icon_list")],
        },
        WidgetSchema {
            widget_type: "alert",
            category: "basic",
            settings: &[
                "alert_type", "alert_title", "alert_description",
                "show_dismiss", "dismiss",
            ],
            aliases: &[
                ("type", "alert_type"), ("title", "alert_title"),
                ("description", "alert_description"),
            ],
        },
        WidgetSchema {
            widget_type: "star-rating",
            category: "basic",
            settings: &[
                "rating_scale", "rating", "star_style", "title",
                "align", "star_size", "star_space", "star_color",
                "star_unmarked_color", "title_color",
            ],
            aliases: &[("value", "rating"), ("score", "rating")],
        },
        // ── Pro / ProElements widgets ─────────────────────────────────────────
        WidgetSchema {
            widget_type: "nav-menu",
            category: "pro",
            settings: &[
                "menu", "layout", "align_items", "pointer", "indicator",
                "submenu_icon", "heading", "dropdown", "toggle",
                "color_menu_item", "color_menu_item_hover", "color_menu_item_active",
                "pointer_color", "pointer_width",
            ],
            aliases: &[("items", "menu")],
        },
        WidgetSchema {
            widget_type: "posts",
            category: "pro",
            settings: &[
                "skin", "columns", "posts_per_page", "post_type", "orderby", "order",
                "show_title", "show_excerpt", "show_read_more", "show_date",
                "show_author", "show_comments", "show_badge", "show_avatar",
                "meta_separator", "thumbnail", "thumbnail_size", "masonry",
                "read_more_text", "title_tag", "excerpt_length",
                "pagination_type", "pagination_numbers_shorten",
            ],
            aliases: &[("query", "post_type"), ("count", "posts_per_page")],
        },
        WidgetSchema {
            widget_type: "slides",
            category: "pro",
            settings: &[
                "slides", "slides_per_view", "slides_to_scroll",
                "effect", "speed", "autoplay", "autoplay_speed",
                "loop", "pause_on_hover", "pause_on_interaction",
                "navigation", "pagination", "content_animation",
                "height", "height_tablet", "height_mobile",
            ],
            aliases: &[("items", "slides")],
        },
        WidgetSchema {
            widget_type: "flip-box",
            category: "pro",
            settings: &[
                "front_title_text", "front_description_text", "front_icon",
                "back_title_text", "back_description_text", "back_button_text",
                "back_link", "flip_effect", "flip_direction",
                "front_background_color", "back_background_color",
                "front_title_color", "back_title_color",
            ],
            aliases: &[
                ("title", "front_title_text"), ("description", "front_description_text"),
            ],
        },
        WidgetSchema {
            widget_type: "call-to-action",
            category: "pro",
            settings: &[
                "skin", "title", "description", "button_text", "link",
                "ribbon_title", "bg_image", "bg_color",
                "title_color", "description_color", "button_color",
                "min_height", "alignment", "vertical_position",
            ],
            aliases: &[("label", "button_text"), ("text", "title")],
        },
        WidgetSchema {
            widget_type: "price-table",
            category: "pro",
            settings: &[
                "heading", "sub_heading", "price", "currency_symbol",
                "currency_format", "period", "features_list",
                "button_text", "link", "ribbon_title",
                "heading_color", "price_color", "features_color",
                "button_text_color", "button_background_color",
            ],
            aliases: &[("title", "heading"), ("items", "features_list")],
        },
        WidgetSchema {
            widget_type: "price-list",
            category: "pro",
            settings: &[
                "price_list", "title_color", "price_color",
                "description_color", "separator_color",
                "image_size", "min_height",
            ],
            aliases: &[("items", "price_list")],
        },
        WidgetSchema {
            widget_type: "countdown",
            category: "pro",
            settings: &[
                "countdown_type", "due_date", "evergreen_counter_hours",
                "evergreen_counter_minutes", "show_days", "show_hours",
                "show_minutes", "show_seconds", "show_labels",
                "custom_labels", "label_days", "label_hours",
                "label_minutes", "label_seconds",
                "digits_color", "label_color", "background_color",
            ],
            aliases: &[("date", "due_date"), ("type", "countdown_type")],
        },
        WidgetSchema {
            widget_type: "form",
            category: "pro",
            settings: &[
                "form_name", "form_fields", "input_size",
                "show_labels", "mark_required", "label_position",
                "button_text", "button_size", "button_width",
                "email_to", "email_subject", "email_content",
                "email_from", "email_from_name",
                "success_message", "error_message", "required_message",
                "button_background_color", "button_text_color",
                "field_background_color", "field_text_color",
            ],
            aliases: &[("fields", "form_fields"), ("submit_text", "button_text")],
        },
        WidgetSchema {
            widget_type: "animated-headline",
            category: "pro",
            settings: &[
                "headline_style", "before_text", "highlighted_text",
                "after_text", "rotating_text", "tag",
                "animation_type", "loop", "highlight_animation_duration",
                "title_color", "words_color",
            ],
            aliases: &[("text", "highlighted_text"), ("title", "before_text")],
        },
        WidgetSchema {
            widget_type: "blockquote",
            category: "pro",
            settings: &[
                "blockquote_content", "tweet_button_text",
                "author_name", "tweet_button_skin",
                "content_color", "border_color", "border_width",
            ],
            aliases: &[("content", "blockquote_content"), ("text", "blockquote_content")],
        },
        WidgetSchema {
            widget_type: "gallery",
            category: "pro",
            settings: &[
                "gallery", "gallery_type", "columns", "gap",
                "link_to", "aspect_ratio", "image_size",
                "overlay_background", "overlay_title", "overlay_description",
                "animation_duration", "hover_animation",
            ],
            aliases: &[("images", "gallery"), ("items", "gallery")],
        },
        WidgetSchema {
            widget_type: "hotspot",
            category: "pro",
            settings: &[
                "image", "hotspot", "tooltip_position", "tooltip_trigger",
                "tooltip_animation", "sequenced_animation",
                "hotspot_color", "hotspot_size",
            ],
            aliases: &[("spots", "hotspot"), ("points", "hotspot")],
        },
        WidgetSchema {
            widget_type: "lottie",
            category: "pro",
            settings: &[
                "source_type", "source_url", "source_json",
                "link_to", "caption_source", "loop", "reverse",
                "speed", "trigger", "viewport",
                "renderer", "lazyload",
            ],
            aliases: &[("url", "source_url"), ("json", "source_json")],
        },
        WidgetSchema {
            widget_type: "table-of-contents",
            category: "pro",
            settings: &[
                "title", "html_tag", "headings_by_tags",
                "container", "exclude_headings_by_selector",
                "marker_view", "icon", "min_height",
                "minimize_box", "minimized_on",
                "title_color", "title_background_color",
                "item_text_color", "item_text_color_hover",
            ],
            aliases: &[("heading", "title"), ("tags", "headings_by_tags")],
        },
        WidgetSchema {
            widget_type: "search-form",
            category: "pro",
            settings: &[
                "skin", "placeholder", "heading",
                "button_type", "button_text", "icon",
                "size", "toggle_button_content",
                "input_text_color", "input_background_color",
                "button_text_color", "button_background_color",
            ],
            aliases: &[("text", "placeholder")],
        },
        WidgetSchema {
            widget_type: "author-box",
            category: "pro",
            settings: &[
                "source", "show_avatar", "show_name", "show_biography",
                "show_link", "link_text", "link_to",
                "author_name_tag", "layout",
                "name_color", "biography_color", "link_color",
                "avatar_size",
            ],
            aliases: &[],
        },
        WidgetSchema {
            widget_type: "breadcrumbs",
            category: "pro",
            settings: &[
                "align", "text_color", "link_color",
                "separator_color", "separator_type",
            ],
            aliases: &[],
        },
        WidgetSchema {
            widget_type: "post-info",
            category: "pro",
            settings: &[
                "icon_list", "view", "icon_color", "text_color",
                "icon_size", "text_indent", "divider",
            ],
            aliases: &[("items", "icon_list")],
        },
        WidgetSchema {
            widget_type: "post-navigation",
            category: "pro",
            settings: &[
                "show_label", "prev_label", "next_label",
                "show_arrow", "arrow", "show_title", "show_borders",
                "label_color", "title_color", "arrow_color",
            ],
            aliases: &[],
        },
    ]
}

// ── Schema lookup ─────────────────────────────────────────────────────────────

pub fn build_schema_map() -> HashMap<&'static str, &'static WidgetSchema> {
    // Leak the vec so we get 'static references — this runs once at startup
    let schemas: &'static Vec<WidgetSchema> = Box::leak(Box::new(all_schemas()));
    let mut map = HashMap::new();
    for s in schemas.iter() {
        map.insert(s.widget_type, s);
    }
    map
}

pub fn all_valid_keys(schema: &WidgetSchema) -> Vec<&str> {
    let mut keys: Vec<&str> = Vec::new();
    keys.extend_from_slice(schema.settings);
    keys.extend_from_slice(COMMON_SETTINGS);
    // Add typography keys for widgets that typically have text
    let text_widgets = ["heading", "text-editor", "button", "icon-box", "image-box",
        "counter", "progress-bar", "testimonial", "alert", "star-rating", "icon-list",
        "animated-headline", "blockquote", "call-to-action", "flip-box",
        "price-table", "table-of-contents", "form"];
    if text_widgets.contains(&schema.widget_type) {
        keys.extend_from_slice(TYPOGRAPHY_KEYS);
    }
    keys
}

pub fn suggest_fix(key: &str, schema: &WidgetSchema) -> Option<String> {
    // Check widget-specific aliases
    for (wrong, right) in schema.aliases {
        if *wrong == key { return Some(right.to_string()); }
    }
    // Check common aliases
    for (wrong, right) in COMMON_ALIASES {
        if *wrong == key { return Some(right.to_string()); }
    }
    // Fuzzy: check if key is a substring of any valid key
    let valid = all_valid_keys(schema);
    for v in &valid {
        if v.contains(key) || key.contains(v) {
            return Some(v.to_string());
        }
    }
    None
}

// ── ListWidgets ───────────────────────────────────────────────────────────────

pub struct ListWidgets;

#[async_trait]
impl Tool for ListWidgets {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "list_widgets",
            description: "List all known widget types with their categories. These are the widgets with bundled schema validation.",
            input_schema: json!({ "type": "object", "properties": {} }),
        }
    }

    async fn run(&self, _args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let schemas = all_schemas();
        let mut lines = vec![format!("{} widget schemas available:", schemas.len())];
        for s in &schemas {
            lines.push(format!("  {} [{}] — {} settings", s.widget_type, s.category, s.settings.len()));
        }
        lines.push(String::new());
        lines.push(format!("Plus {} common settings shared by all widgets.", COMMON_SETTINGS.len()));
        Ok(ToolResult::text(lines.join("\n")))
    }
}

// ── GetWidgetSchema ───────────────────────────────────────────────────────────

pub struct GetWidgetSchema;

#[async_trait]
impl Tool for GetWidgetSchema {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_widget_schema",
            description: "Get the full schema for a widget type — all valid settings, common mistakes, and their corrections.",
            input_schema: json!({
                "type": "object",
                "required": ["widget_type"],
                "properties": {
                    "widget_type": { "type": "string", "description": "e.g. 'heading', 'text-editor', 'button'" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let wt = str_arg(&args, "widget_type").ok_or_else(|| anyhow::anyhow!("widget_type required"))?;
        let map = build_schema_map();

        match map.get(wt.as_str()) {
            Some(schema) => {
                let valid = all_valid_keys(schema);
                let aliases: HashMap<&str, &str> = schema.aliases.iter()
                    .chain(COMMON_ALIASES.iter())
                    .map(|(k, v)| (*k, *v))
                    .collect();

                let result = json!({
                    "widget_type": schema.widget_type,
                    "category": schema.category,
                    "widget_settings": schema.settings,
                    "common_settings": COMMON_SETTINGS,
                    "typography_settings": TYPOGRAPHY_KEYS,
                    "all_valid_keys": valid,
                    "common_mistakes": aliases,
                });
                Ok(ToolResult::text(serde_json::to_string_pretty(&result)?))
            }
            None => {
                let known: Vec<&str> = map.keys().copied().collect();
                Ok(ToolResult::error(format!(
                    "Unknown widget type '{wt}'. Known types: {}",
                    known.join(", ")
                )))
            }
        }
    }
}

// ── ValidateElement ───────────────────────────────────────────────────────────

pub struct ValidateElement;

#[async_trait]
impl Tool for ValidateElement {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "validate_element",
            description: "Validate an element's JSON against its widget schema. Reports invalid settings with 'did you mean?' suggestions.",
            input_schema: json!({
                "type": "object",
                "required": ["element"],
                "properties": {
                    "element": { "type": "object", "description": "Full element JSON to validate" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let el = args.get("element").ok_or_else(|| anyhow::anyhow!("element required"))?;

        let el_type = el.get("elType").and_then(|v| v.as_str()).unwrap_or("");
        let widget_type = el.get("widgetType").and_then(|v| v.as_str());

        // Structural checks
        let mut errors: Vec<String> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();

        if el.get("id").and_then(|v| v.as_str()).unwrap_or("").is_empty() {
            warnings.push("Missing 'id' — will be auto-generated on add_element".into());
        }

        if el_type.is_empty() {
            errors.push("Missing 'elType' — must be 'widget', 'container', 'section', or 'column'".into());
        }

        if el_type == "widget" {
            let wt = match widget_type {
                Some(wt) => wt,
                None => {
                    errors.push("Widget element missing 'widgetType'".into());
                    return Ok(format_validation(errors, warnings));
                }
            };

            let map = build_schema_map();
            match map.get(wt) {
                Some(schema) => {
                    let valid = all_valid_keys(schema);
                    if let Some(settings) = el.get("settings").and_then(|s| s.as_object()) {
                        for key in settings.keys() {
                            if !valid.iter().any(|v| v == key) {
                                match suggest_fix(key, schema) {
                                    Some(suggestion) => errors.push(format!(
                                        "Invalid setting '{key}' on widget '{wt}' — did you mean '{suggestion}'?"
                                    )),
                                    None => warnings.push(format!(
                                        "Unknown setting '{key}' on widget '{wt}' — not in bundled schema (may be valid for addons)"
                                    )),
                                }
                            }
                        }
                    }
                }
                None => {
                    warnings.push(format!("Widget type '{wt}' not in bundled schema — settings not validated"));
                }
            }
        }

        if errors.is_empty() && warnings.is_empty() {
            Ok(ToolResult::text("✓ Element is valid."))
        } else {
            Ok(format_validation(errors, warnings))
        }
    }
}

fn format_validation(errors: Vec<String>, warnings: Vec<String>) -> ToolResult {
    let mut lines = Vec::new();
    if !errors.is_empty() {
        lines.push(format!("✗ {} error(s):", errors.len()));
        for e in &errors { lines.push(format!("  ERROR: {e}")); }
    }
    if !warnings.is_empty() {
        lines.push(format!("⚠ {} warning(s):", warnings.len()));
        for w in &warnings { lines.push(format!("  WARN: {w}")); }
    }
    if errors.is_empty() {
        ToolResult::text(lines.join("\n"))
    } else {
        ToolResult::error(lines.join("\n"))
    }
}
