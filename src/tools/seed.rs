use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::elementor::{self, generate_id};
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use super::Tool;

pub struct SeedContent;

#[async_trait]
impl Tool for SeedContent {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "seed_content",
            description: "Create sample pages with various Elementor widgets and layouts. Useful for testing and demos. Creates: landing page, features page, about page, contact page.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "prefix": { "type": "string", "description": "Page title prefix (default: 'Demo')", "default": "Demo" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let prefix = args.get("prefix").and_then(|v| v.as_str()).unwrap_or("Demo");
        let mut created = Vec::new();

        // 1. Landing page
        let landing = build_landing(prefix);
        let id = create_page(wp, &format!("{prefix} - Landing Page"), &landing).await?;
        created.push(format!("[{id}] {prefix} - Landing Page"));

        // 2. Features page
        let features = build_features(prefix);
        let id = create_page(wp, &format!("{prefix} - Features"), &features).await?;
        created.push(format!("[{id}] {prefix} - Features"));

        // 3. About page
        let about = build_about(prefix);
        let id = create_page(wp, &format!("{prefix} - About"), &about).await?;
        created.push(format!("[{id}] {prefix} - About"));

        // 4. Contact page
        let contact = build_contact(prefix);
        let id = create_page(wp, &format!("{prefix} - Contact"), &contact).await?;
        created.push(format!("[{id}] {prefix} - Contact"));

        // 5. All widgets showcase
        let showcase = build_widget_showcase();
        let id = create_page(wp, &format!("{prefix} - Widget Showcase"), &showcase).await?;
        created.push(format!("[{id}] {prefix} - Widget Showcase"));

        Ok(ToolResult::text(format!("Created {} pages:\n{}", created.len(), created.join("\n"))))
    }
}

async fn create_page(wp: &WpClient, title: &str, elements: &[elementor::Element]) -> Result<u64> {
    let data = elementor::serialize_data(elements)?;
    let body = json!({
        "title": title, "status": "publish",
        "meta": { "_elementor_data": data, "_elementor_edit_mode": "builder" }
    });
    let result = wp.post("wp/v2/pages", &body).await?;
    wp.clear_elementor_cache().await?;
    Ok(result["id"].as_u64().unwrap_or(0))
}

fn w(wt: &str, s: Value) -> elementor::Element {
    elementor::Element {
        id: generate_id(), el_type: "widget".into(), widget_type: Some(wt.into()),
        settings: s, elements: vec![], extra: Default::default(),
    }
}

fn c(settings: Value, children: Vec<elementor::Element>) -> elementor::Element {
    elementor::Element {
        id: generate_id(), el_type: "container".into(), widget_type: None,
        settings, elements: children, extra: Default::default(),
    }
}

fn row(children: Vec<elementor::Element>) -> elementor::Element {
    c(json!({"flex_direction": "row", "flex_gap": {"unit": "px", "size": 20}}), children)
}

fn col(children: Vec<elementor::Element>) -> elementor::Element {
    c(json!({"flex_grow": 1}), children)
}

fn build_landing(prefix: &str) -> Vec<elementor::Element> {
    vec![
        // Hero
        c(json!({"background_background": "classic", "background_color": "#1a1a2e",
            "padding": {"unit":"px","top":"100","bottom":"100","left":"20","right":"20","isLinked":false}}), vec![
            w("heading", json!({"title": format!("Welcome to {prefix}"), "header_size": "h1",
                "title_color": "#ffffff", "align": "center",
                "typography_typography": "custom", "typography_font_size": {"unit":"px","size":52}})),
            w("text-editor", json!({"editor": "<p style='text-align:center;color:#aaa;font-size:18px'>Build beautiful websites with the power of Elementor</p>"})),
            c(json!({"align_items": "center", "justify_content": "center"}), vec![
                w("button", json!({"text": "Get Started", "size": "lg",
                    "background_color": "#e94560", "button_text_color": "#ffffff",
                    "link": {"url": "#features"}})),
            ]),
        ]),
        // Stats
        c(json!({"padding": {"unit":"px","top":"60","bottom":"60","isLinked":false}}), vec![
            row(vec![
                col(vec![w("counter", json!({"ending_number": 1500, "title": "Projects Completed"}))]),
                col(vec![w("counter", json!({"ending_number": 200, "title": "Happy Clients"}))]),
                col(vec![w("counter", json!({"ending_number": 50, "title": "Team Members"}))]),
                col(vec![w("counter", json!({"ending_number": 99, "title": "% Satisfaction"}))]),
            ]),
        ]),
        // Features preview
        c(json!({"padding": {"unit":"px","top":"60","bottom":"60","isLinked":false}}), vec![
            w("heading", json!({"title": "Why Choose Us", "header_size": "h2", "align": "center"})),
            w("divider", json!({"style": "solid", "color": "#e94560", "width": {"unit":"%","size":10}})),
            row(vec![
                col(vec![w("icon-box", json!({"selected_icon": {"value":"fas fa-rocket","library":"fa-solid"}, "title_text": "Lightning Fast", "description_text": "Optimized for speed and performance"}))]),
                col(vec![w("icon-box", json!({"selected_icon": {"value":"fas fa-shield-alt","library":"fa-solid"}, "title_text": "Secure", "description_text": "Enterprise-grade security built in"}))]),
                col(vec![w("icon-box", json!({"selected_icon": {"value":"fas fa-code","library":"fa-solid"}, "title_text": "Developer Friendly", "description_text": "Clean code and great documentation"}))]),
            ]),
        ]),
        // CTA
        c(json!({"background_color": "#16213e", "padding": {"unit":"px","top":"80","bottom":"80","isLinked":false}}), vec![
            w("heading", json!({"title": "Ready to Get Started?", "header_size": "h2", "title_color": "#ffffff", "align": "center"})),
            c(json!({"align_items": "center"}), vec![
                w("button", json!({"text": "Start Free Trial", "size": "lg", "background_color": "#e94560", "button_text_color": "#fff", "link": {"url": "#"}})),
            ]),
        ]),
    ]
}

fn build_features(_prefix: &str) -> Vec<elementor::Element> {
    vec![
        c(json!({"padding": {"unit":"px","top":"60","bottom":"40","isLinked":false}}), vec![
            w("heading", json!({"title": "Features", "header_size": "h1", "align": "center"})),
            w("text-editor", json!({"editor": "<p style='text-align:center;max-width:600px;margin:auto'>Everything you need to build amazing websites</p>"})),
        ]),
        // Feature rows
        c(json!({"padding": {"unit":"px","top":"40","bottom":"40","isLinked":false}}), vec![
            row(vec![
                col(vec![w("image", json!({"image": {"url": "https://via.placeholder.com/500x300/e94560/fff?text=Visual+Editor"}, "image_size": "full"}))]),
                col(vec![
                    w("heading", json!({"title": "Visual Drag & Drop", "header_size": "h3"})),
                    w("text-editor", json!({"editor": "<p>Build pages visually with our intuitive drag and drop editor. No coding required.</p>"})),
                    w("icon-list", json!({"icon_list": [
                        {"text": "Real-time preview", "selected_icon": {"value": "fas fa-check", "library": "fa-solid"}},
                        {"text": "Responsive controls", "selected_icon": {"value": "fas fa-check", "library": "fa-solid"}},
                        {"text": "Undo/redo support", "selected_icon": {"value": "fas fa-check", "library": "fa-solid"}}
                    ]})),
                ]),
            ]),
        ]),
        c(json!({"padding": {"unit":"px","top":"40","bottom":"40","isLinked":false}, "background_color": "#f8f9fa"}), vec![
            row(vec![
                col(vec![
                    w("heading", json!({"title": "Responsive Design", "header_size": "h3"})),
                    w("text-editor", json!({"editor": "<p>Every element adapts perfectly to any screen size.</p>"})),
                    w("progress-bar", json!({"title": "Desktop", "percent": 100})),
                    w("progress-bar", json!({"title": "Tablet", "percent": 95})),
                    w("progress-bar", json!({"title": "Mobile", "percent": 90})),
                ]),
                col(vec![w("image", json!({"image": {"url": "https://via.placeholder.com/500x300/16213e/fff?text=Responsive"}, "image_size": "full"}))]),
            ]),
        ]),
    ]
}

fn build_about(_prefix: &str) -> Vec<elementor::Element> {
    vec![
        c(json!({"background_color": "#1a1a2e", "padding": {"unit":"px","top":"80","bottom":"80","isLinked":false}}), vec![
            w("heading", json!({"title": "About Us", "header_size": "h1", "title_color": "#fff", "align": "center"})),
        ]),
        c(json!({"padding": {"unit":"px","top":"60","bottom":"60","isLinked":false}}), vec![
            row(vec![
                col(vec![w("image", json!({"image": {"url": "https://via.placeholder.com/400x400/e94560/fff?text=Team"}, "image_size": "full"}))]),
                col(vec![
                    w("heading", json!({"title": "Our Story", "header_size": "h2"})),
                    w("text-editor", json!({"editor": "<p>We started with a simple mission: make web development accessible to everyone. Today, we serve thousands of customers worldwide.</p><p>Our team of passionate developers and designers work tirelessly to bring you the best tools for building websites.</p>"})),
                ]),
            ]),
        ]),
        // Team
        c(json!({"padding": {"unit":"px","top":"40","bottom":"60","isLinked":false}}), vec![
            w("heading", json!({"title": "Meet the Team", "header_size": "h2", "align": "center"})),
            row(vec![
                col(vec![w("image-box", json!({"image": {"url": "https://via.placeholder.com/150/333/fff?text=JD"}, "title_text": "John Doe", "description_text": "CEO & Founder"}))]),
                col(vec![w("image-box", json!({"image": {"url": "https://via.placeholder.com/150/333/fff?text=JS"}, "title_text": "Jane Smith", "description_text": "CTO"}))]),
                col(vec![w("image-box", json!({"image": {"url": "https://via.placeholder.com/150/333/fff?text=BJ"}, "title_text": "Bob Johnson", "description_text": "Lead Designer"}))]),
            ]),
        ]),
        // Testimonial
        c(json!({"background_color": "#f8f9fa", "padding": {"unit":"px","top":"60","bottom":"60","isLinked":false}}), vec![
            w("heading", json!({"title": "What Our Clients Say", "header_size": "h2", "align": "center"})),
            w("testimonial", json!({"testimonial_content": "This tool completely transformed how we build websites. The speed and flexibility are unmatched.", "testimonial_name": "Sarah Wilson", "testimonial_job": "Marketing Director"})),
            w("star-rating", json!({"rating": 5, "title": "Overall Rating"})),
        ]),
    ]
}

fn build_contact(_prefix: &str) -> Vec<elementor::Element> {
    vec![
        c(json!({"padding": {"unit":"px","top":"60","bottom":"60","isLinked":false}}), vec![
            w("heading", json!({"title": "Contact Us", "header_size": "h1", "align": "center"})),
            w("text-editor", json!({"editor": "<p style='text-align:center'>We'd love to hear from you</p>"})),
        ]),
        c(json!({"padding": {"unit":"px","top":"20","bottom":"60","isLinked":false}}), vec![
            row(vec![
                col(vec![
                    w("icon-box", json!({"selected_icon": {"value":"fas fa-envelope","library":"fa-solid"}, "title_text": "Email", "description_text": "hello@example.com"})),
                    w("icon-box", json!({"selected_icon": {"value":"fas fa-phone","library":"fa-solid"}, "title_text": "Phone", "description_text": "+1 (555) 123-4567"})),
                    w("icon-box", json!({"selected_icon": {"value":"fas fa-map-marker-alt","library":"fa-solid"}, "title_text": "Address", "description_text": "123 Main St, City, Country"})),
                    w("social-icons", json!({"social_icon_list": [
                        {"social_icon": {"value": "fab fa-facebook", "library": "fa-brands"}, "link": {"url": "#"}},
                        {"social_icon": {"value": "fab fa-twitter", "library": "fa-brands"}, "link": {"url": "#"}},
                        {"social_icon": {"value": "fab fa-linkedin", "library": "fa-brands"}, "link": {"url": "#"}}
                    ]})),
                ]),
                col(vec![
                    w("html", json!({"html": "<div style='background:#f8f9fa;padding:40px;border-radius:8px'><h3>Send us a message</h3><p>Use the form below or email us directly.</p><p><em>Form widget requires Elementor Pro or ProElements</em></p></div>"})),
                ]),
            ]),
        ]),
    ]
}

fn build_widget_showcase() -> Vec<elementor::Element> {
    vec![
        c(json!({"padding": {"unit":"px","top":"40","bottom":"20","isLinked":false}}), vec![
            w("heading", json!({"title": "Widget Showcase", "header_size": "h1", "align": "center"})),
            w("text-editor", json!({"editor": "<p style='text-align:center'>Every Elementor widget type demonstrated</p>"})),
        ]),
        // Text widgets
        c(json!({"padding": {"unit":"px","top":"20","bottom":"20","isLinked":false}}), vec![
            w("heading", json!({"title": "Typography", "header_size": "h2"})),
            w("heading", json!({"title": "Heading H1", "header_size": "h1"})),
            w("heading", json!({"title": "Heading H2", "header_size": "h2"})),
            w("heading", json!({"title": "Heading H3", "header_size": "h3"})),
            w("text-editor", json!({"editor": "<p>Regular paragraph with <strong>bold</strong>, <em>italic</em>, and <a href='#'>links</a>.</p>"})),
            w("divider", json!({"style": "solid"})),
        ]),
        // Media
        c(json!({"padding": {"unit":"px","top":"20","bottom":"20","isLinked":false}}), vec![
            w("heading", json!({"title": "Media", "header_size": "h2"})),
            row(vec![
                col(vec![w("image", json!({"image": {"url": "https://via.placeholder.com/400x250/e94560/fff?text=Image"}, "image_size": "full"}))]),
                col(vec![w("video", json!({"video_type": "youtube", "youtube_url": "https://www.youtube.com/watch?v=dQw4w9WgXcQ"}))]),
            ]),
        ]),
        // Interactive
        c(json!({"padding": {"unit":"px","top":"20","bottom":"20","isLinked":false}}), vec![
            w("heading", json!({"title": "Interactive", "header_size": "h2"})),
            w("toggle", json!({"tabs": [
                {"tab_title": "What is Elementor?", "tab_content": "Elementor is a drag-and-drop page builder for WordPress."},
                {"tab_title": "Is it free?", "tab_content": "Elementor has a free version with many features."}
            ]})),
            w("accordion", json!({"tabs": [
                {"tab_title": "Getting Started", "tab_content": "Install the plugin and start building."},
                {"tab_title": "Advanced Usage", "tab_content": "Use custom CSS and dynamic content."}
            ]})),
            w("tabs", json!({"tabs": [
                {"tab_title": "Tab 1", "tab_content": "First tab content"},
                {"tab_title": "Tab 2", "tab_content": "Second tab content"}
            ]})),
        ]),
        // Data display
        c(json!({"padding": {"unit":"px","top":"20","bottom":"20","isLinked":false}}), vec![
            w("heading", json!({"title": "Data Display", "header_size": "h2"})),
            row(vec![
                col(vec![w("counter", json!({"ending_number": 42, "title": "Answer"}))]),
                col(vec![w("progress-bar", json!({"title": "Progress", "percent": 73}))]),
                col(vec![w("star-rating", json!({"rating": 4.5, "title": "Rating"}))]),
            ]),
        ]),
        // Alerts & buttons
        c(json!({"padding": {"unit":"px","top":"20","bottom":"40","isLinked":false}}), vec![
            w("heading", json!({"title": "Actions & Alerts", "header_size": "h2"})),
            w("alert", json!({"alert_title": "Success", "alert_description": "Operation completed.", "alert_type": "success"})),
            w("alert", json!({"alert_title": "Warning", "alert_description": "Please review.", "alert_type": "warning"})),
            row(vec![
                col(vec![w("button", json!({"text": "Primary", "size": "md", "background_color": "#0073aa", "button_text_color": "#fff"}))]),
                col(vec![w("button", json!({"text": "Secondary", "size": "md", "background_color": "#e94560", "button_text_color": "#fff"}))]),
                col(vec![w("button", json!({"text": "Outline", "size": "md", "button_type": "info"}))]),
            ]),
        ]),
    ]
}
