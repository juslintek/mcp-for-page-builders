//! Persistent E2E tests — creates content that stays in WordPress for visual inspection.
//! Run: WP_TEST_URL=http://localhost:8080 WP_TEST_USER=admin WP_TEST_PASS=xxx cargo test --test e2e_persistent -- --test-threads=1
//! Then browse to the WordPress site to see results.
//!
//! NO CLEANUP — everything persists.

mod helpers;

use elementor_mcp::elementor::{self, Element};
use elementor_mcp::wp::WpClient;
use helpers::*;
use serde_json::json;

fn wp() -> Option<WpClient> {
    let url = std::env::var("WP_TEST_URL").ok()?;
    let user = std::env::var("WP_TEST_USER").unwrap_or_else(|_| "admin".into());
    let pass = std::env::var("WP_TEST_PASS").ok()?;
    Some(WpClient::new(&url, &user, &pass))
}

macro_rules! require_wp { () => { match wp() { Some(c) => c, None => { eprintln!("Skip: WP_TEST_URL not set"); return; } } }; }

async fn publish_page(wp: &WpClient, title: &str, elements: Vec<Element>) -> u64 {
    let data = to_elementor_data(&elements);
    let body = json!({
        "title": title, "status": "publish",
        "meta": {"_elementor_data": data, "_elementor_edit_mode": "builder"}
    });
    let r = wp.post("wp/v2/pages", &body).await.expect(&format!("Failed to create: {title}"));
    let id = r["id"].as_u64().unwrap();
    wp.clear_elementor_cache().await.ok();
    let link = r["link"].as_str().unwrap_or("");
    println!("  [{id}] {title} → {link}");
    id
}

// ═══════════════════════════════════════════════════════════════════════════════
// 1. WIDGET SHOWCASE — one page per widget type
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn p01_heading_widget() {
    let wp = require_wp!();
    publish_page(&wp, "E2E: Heading Widget", vec![
        container(vec![
            heading_styled("Heading H1 — Centered, Red, 48px", "h1", "#e94560", 48),
            heading("Heading H2 — Default", "h2"),
            heading("Heading H3 — Default", "h3"),
            heading("Heading H4 — Default", "h4"),
        ]),
    ]).await;
}

#[tokio::test]
async fn p02_text_editor_widget() {
    let wp = require_wp!();
    publish_page(&wp, "E2E: Text Editor Widget", vec![
        container(vec![
            text("<h2>Rich Text Content</h2><p>This is a paragraph with <strong>bold</strong>, <em>italic</em>, <u>underline</u>, and <a href='#'>links</a>.</p><ul><li>Bullet one</li><li>Bullet two</li></ul><ol><li>Numbered one</li><li>Numbered two</li></ol><blockquote>A blockquote for emphasis.</blockquote>"),
        ]),
    ]).await;
}

#[tokio::test]
async fn p03_image_widget() {
    let wp = require_wp!();
    publish_page(&wp, "E2E: Image Widget", vec![
        container(vec![
            heading("Image Widget", "h2"),
            image("https://picsum.photos/800/400"),
            text("<p><em>Random image from picsum.photos</em></p>"),
        ]),
    ]).await;
}

#[tokio::test]
async fn p04_button_widget() {
    let wp = require_wp!();
    publish_page(&wp, "E2E: Button Widget", vec![
        container(vec![
            heading("Button Styles", "h2"),
            button("Default Button", "#"),
            button_styled("Primary Large", "#", "#0073aa", "#ffffff"),
            button_styled("Danger", "#", "#e94560", "#ffffff"),
            button_styled("Success", "#", "#4ade80", "#000000"),
        ]),
    ]).await;
}

#[tokio::test]
async fn p05_icon_list_widget() {
    let wp = require_wp!();
    publish_page(&wp, "E2E: Icon List Widget", vec![
        container(vec![
            heading("Feature List", "h2"),
            icon_list(vec![
                ("Drag & Drop Editor", "fas fa-mouse-pointer"),
                ("Responsive Design", "fas fa-mobile-alt"),
                ("SEO Optimized", "fas fa-search"),
                ("Fast Loading", "fas fa-bolt"),
                ("24/7 Support", "fas fa-headset"),
            ]),
        ]),
    ]).await;
}

#[tokio::test]
async fn p06_divider_spacer_widgets() {
    let wp = require_wp!();
    publish_page(&wp, "E2E: Divider & Spacer", vec![
        container(vec![
            heading("Content Above", "h3"),
            divider(),
            spacer(40),
            heading("Content Below (40px spacer)", "h3"),
            divider(),
        ]),
    ]).await;
}

#[tokio::test]
async fn p07_html_widget() {
    let wp = require_wp!();
    publish_page(&wp, "E2E: HTML Widget", vec![
        container(vec![
            heading("Custom HTML", "h2"),
            html_widget("<div style='background:linear-gradient(135deg,#667eea,#764ba2);color:#fff;padding:40px;border-radius:12px;text-align:center'><h2 style='margin:0 0 10px'>Custom HTML Block</h2><p>Styled with inline CSS gradient</p></div>"),
        ]),
    ]).await;
}

#[tokio::test]
async fn p08_video_widget() {
    let wp = require_wp!();
    publish_page(&wp, "E2E: Video Widget", vec![
        container(vec![
            heading("Embedded Video", "h2"),
            video("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
        ]),
    ]).await;
}

#[tokio::test]
async fn p09_icon_box_widget() {
    let wp = require_wp!();
    publish_page(&wp, "E2E: Icon Box Widget", vec![
        container(vec![
            heading("Icon Boxes", "h2"),
            columns(3, vec![
                vec![icon_box("fas fa-rocket", "Performance", "Blazing fast load times with optimized code")],
                vec![icon_box("fas fa-shield-alt", "Security", "Enterprise-grade protection for your data")],
                vec![icon_box("fas fa-code", "Developer API", "Full REST API and webhook support")],
            ]),
        ]),
    ]).await;
}

#[tokio::test]
async fn p10_image_box_widget() {
    let wp = require_wp!();
    publish_page(&wp, "E2E: Image Box Widget", vec![
        container(vec![
            heading("Team Members", "h2"),
            columns(3, vec![
                vec![image_box("https://picsum.photos/150/150?random=1", "Alice Johnson", "CEO & Founder")],
                vec![image_box("https://picsum.photos/150/150?random=2", "Bob Smith", "CTO")],
                vec![image_box("https://picsum.photos/150/150?random=3", "Carol Williams", "Lead Designer")],
            ]),
        ]),
    ]).await;
}

#[tokio::test]
async fn p11_counter_progress_widgets() {
    let wp = require_wp!();
    publish_page(&wp, "E2E: Counter & Progress Bar", vec![
        container(vec![
            heading("Statistics", "h2"),
            columns(4, vec![
                vec![counter(1500, "Projects")],
                vec![counter(200, "Clients")],
                vec![counter(50, "Team Members")],
                vec![counter(99, "% Uptime")],
            ]),
            spacer(30),
            heading("Skills", "h2"),
            progress_bar("WordPress", 95),
            progress_bar("Elementor", 90),
            progress_bar("PHP", 85),
            progress_bar("JavaScript", 80),
        ]),
    ]).await;
}

#[tokio::test]
async fn p12_testimonial_rating_widgets() {
    let wp = require_wp!();
    publish_page(&wp, "E2E: Testimonial & Rating", vec![
        container(vec![
            heading("Client Reviews", "h2"),
            testimonial("This tool completely transformed our workflow. The speed and flexibility are unmatched.", "Sarah Wilson", "Marketing Director at TechCorp"),
            spacer(20),
            testimonial("Best investment we've made this year. Our team productivity doubled.", "James Chen", "CTO at StartupXYZ"),
            spacer(20),
            star_rating(4.8, "Average Customer Rating"),
        ]),
    ]).await;
}

#[tokio::test]
async fn p13_toggle_accordion_tabs_widgets() {
    let wp = require_wp!();
    publish_page(&wp, "E2E: Toggle, Accordion & Tabs", vec![
        container(vec![
            heading("FAQ — Toggle", "h2"),
            toggle(vec![
                ("What is Elementor?", "Elementor is a drag-and-drop page builder plugin for WordPress."),
                ("Is it free?", "Elementor has a free version with many features. Pro adds advanced widgets."),
                ("Does it work with any theme?", "Yes, Elementor works with most WordPress themes."),
            ]),
            spacer(30),
            heading("Documentation — Accordion", "h2"),
            accordion(vec![
                ("Getting Started", "Install the plugin, activate it, and click 'Edit with Elementor' on any page."),
                ("Adding Widgets", "Drag widgets from the left panel onto your page canvas."),
                ("Responsive Design", "Use the responsive mode buttons to adjust layouts per device."),
            ]),
            spacer(30),
            heading("Pricing — Tabs", "h2"),
            tabs(vec![
                ("Free", "Basic widgets, templates, and responsive editing. Perfect for personal sites."),
                ("Pro", "Advanced widgets, theme builder, popup builder, and WooCommerce integration."),
                ("Enterprise", "Custom solutions, dedicated support, and SLA guarantees."),
            ]),
        ]),
    ]).await;
}

#[tokio::test]
async fn p15_nav_menu_widget() {
    let wp = require_wp!();
    publish_page(&wp, "E2E: Nav Menu Widget", vec![
        // Horizontal menu (light bg)
        container_with(json!({"background_color":"#ffffff","padding":{"unit":"px","top":"15","bottom":"15","left":"20","right":"20","isLinked":false}}), vec![
            heading("Horizontal Menu (Light)", "h4"),
            nav_menu("Main Menu"),
        ]),
        spacer(20),
        // Horizontal menu (dark bg)
        container_with(json!({"background_color":"#1a1a2e","padding":{"unit":"px","top":"15","bottom":"15","left":"20","right":"20","isLinked":false}}), vec![
            heading_styled("Horizontal Menu (Dark)", "h4", "#ffffff", 18),
            nav_menu_styled("Main Menu", "#ffffff", "#e94560"),
        ]),
        spacer(20),
        // Full header bar with logo + menu + CTA
        container_with(json!({"flex_direction":"row","background_color":"#16213e",
            "padding":{"unit":"px","top":"12","bottom":"12","left":"20","right":"20","isLinked":false},
            "align_items":"center"}), vec![
            widget("heading", json!({"title":"Brand","header_size":"h5","title_color":"#e94560"})),
            container_with(json!({"flex_grow":1}), vec![
                nav_menu_styled("Main Menu", "#cccccc", "#e94560"),
            ]),
            button_styled("Sign Up", "#", "#e94560", "#ffffff"),
        ]),
    ]).await;
}

#[tokio::test]
async fn p14_social_alert_widgets() {
    let wp = require_wp!();
    publish_page(&wp, "E2E: Social Icons & Alerts", vec![
        container(vec![
            heading("Follow Us", "h2"),
            social_icons(vec!["facebook", "twitter", "linkedin", "github", "youtube", "instagram"]),
            spacer(30),
            heading("Notifications", "h2"),
            alert("Success!", "Your changes have been saved successfully.", "success"),
            alert("Warning", "Please review your settings before publishing.", "warning"),
            alert("Info", "A new version is available. Update when ready.", "info"),
            alert("Error", "Something went wrong. Please try again.", "danger"),
        ]),
    ]).await;
}

// ═══════════════════════════════════════════════════════════════════════════════
// 2. LAYOUT SHOWCASE
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn p20_layout_two_columns() {
    let wp = require_wp!();
    publish_page(&wp, "E2E: Two Column Layout", vec![
        columns(2, vec![
            vec![
                heading("Left Column", "h2"),
                text("<p>This is the left column content. It takes up 50% of the width.</p>"),
                image("https://picsum.photos/500/300?random=10"),
            ],
            vec![
                heading("Right Column", "h2"),
                text("<p>This is the right column content. It also takes up 50% of the width.</p>"),
                button_styled("Learn More", "#", "#0073aa", "#fff"),
            ],
        ]),
    ]).await;
}

#[tokio::test]
async fn p21_layout_three_columns() {
    let wp = require_wp!();
    publish_page(&wp, "E2E: Three Column Layout", vec![
        container(vec![heading("Our Services", "h1")]),
        columns(3, vec![
            vec![icon_box("fas fa-paint-brush", "Design", "Beautiful, modern designs tailored to your brand")],
            vec![icon_box("fas fa-laptop-code", "Development", "Clean, performant code built to scale")],
            vec![icon_box("fas fa-chart-line", "Marketing", "Data-driven strategies that deliver results")],
        ]),
    ]).await;
}

#[tokio::test]
async fn p22_layout_sidebar() {
    let wp = require_wp!();
    publish_page(&wp, "E2E: Sidebar Layout (70/30)", vec![
        container_with(json!({"flex_direction": "row", "flex_gap": {"unit":"px","size":30}}), vec![
            container_with(json!({"flex_grow": 7}), vec![
                heading("Main Article", "h1"),
                text("<p>This is the main content area taking 70% width. Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.</p>"),
                image("https://picsum.photos/700/350?random=20"),
                text("<p>Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat.</p>"),
            ]),
            container_with(json!({"flex_grow": 3, "background_background": "classic", "background_color": "#f8f9fa", "padding": {"unit":"px","top":"20","bottom":"20","left":"20","right":"20","isLinked":false}}), vec![
                heading("Sidebar", "h3"),
                text("<p><strong>Recent Posts</strong></p><ul><li>Getting Started Guide</li><li>Advanced Techniques</li><li>Best Practices</li></ul>"),
                divider(),
                text("<p><strong>Categories</strong></p><ul><li>Tutorials</li><li>News</li><li>Updates</li></ul>"),
            ]),
        ]),
    ]).await;
}

#[tokio::test]
async fn p23_layout_nested_deep() {
    let wp = require_wp!();
    publish_page(&wp, "E2E: Nested Containers (3 levels)", vec![
        container_with(json!({"background_color": "#1a1a2e", "padding": {"unit":"px","top":"40","bottom":"40","left":"40","right":"40","isLinked":false}}), vec![
            heading_styled("Level 1 — Dark Background", "h2", "#ffffff", 32),
            container_with(json!({"background_color": "#16213e", "padding": {"unit":"px","top":"30","bottom":"30","left":"30","right":"30","isLinked":false}}), vec![
                heading_styled("Level 2 — Darker", "h3", "#e0e0e0", 24),
                container_with(json!({"background_color": "#0f3460", "padding": {"unit":"px","top":"20","bottom":"20","left":"20","right":"20","isLinked":false}}), vec![
                    heading_styled("Level 3 — Deepest", "h4", "#e94560", 20),
                    text("<p style='color:#aaa'>Three levels of nested containers with different backgrounds.</p>"),
                ]),
            ]),
        ]),
    ]).await;
}

// ═══════════════════════════════════════════════════════════════════════════════
// 3. FULL PAGES
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn p30_landing_page() {
    let wp = require_wp!();
    publish_page(&wp, "E2E: Complete Landing Page", vec![
        // Navigation bar
        container_with(json!({"flex_direction":"row","background_color":"#0f0f23",
            "padding":{"unit":"px","top":"12","bottom":"12","left":"20","right":"20","isLinked":false},
            "align_items":"center","position":"sticky","z_index":100}), vec![
            widget("heading", json!({"title":"MCP Demo","header_size":"h5","title_color":"#ffffff"})),
            container_with(json!({"flex_grow":1}), vec![
                nav_menu_styled("Main Menu", "#cccccc", "#e94560"),
            ]),
            button_styled("Get Started", "#pricing", "#e94560", "#ffffff"),
        ]),
        // Hero
        container_with(json!({"background_background":"classic","background_color":"#0f0f23","padding":{"unit":"px","top":"120","bottom":"120","left":"20","right":"20","isLinked":false}}), vec![
            heading_styled("Build Faster with Elementor MCP", "h1", "#ffffff", 52),
            text("<p style='text-align:center;color:#888;font-size:20px;max-width:600px;margin:auto'>The AI-powered tool that creates WordPress pages programmatically. 41 tools, zero manual work.</p>"),
            spacer(20),
            container_with(json!({"align_items":"center","justify_content":"center","flex_direction":"row","flex_gap":{"unit":"px","size":16}}), vec![
                button_styled("Get Started Free", "#", "#e94560", "#fff"),
                button_styled("View Demo", "#showcase", "#333", "#fff"),
            ]),
        ]),
        // Stats
        container_with(json!({"padding":{"unit":"px","top":"60","bottom":"60","isLinked":false}}), vec![
            columns(4, vec![
                vec![counter(41, "MCP Tools")],
                vec![counter(78, "Tests Passing")],
                vec![counter(40, "Widget Schemas")],
                vec![counter(5, "MB Binary Size")],
            ]),
        ]),
        // Features
        container_with(json!({"padding":{"unit":"px","top":"60","bottom":"60","isLinked":false}}), vec![
            heading("Everything You Need", "h2"),
            divider(),
            spacer(20),
            columns(3, vec![
                vec![icon_box("fas fa-puzzle-piece", "Element Operations", "Add, remove, move, duplicate widgets without replacing entire pages")],
                vec![icon_box("fas fa-palette", "Global Design Tokens", "Manage colors and typography across your entire site")],
                vec![icon_box("fas fa-check-circle", "Schema Validation", "Catch mistakes before they happen with 'did you mean?' suggestions")],
            ]),
            spacer(20),
            columns(3, vec![
                vec![icon_box("fas fa-camera", "Visual Comparison", "Side-by-side screenshots to verify changes")],
                vec![icon_box("fas fa-database", "File I/O", "Download, upload, and backup page data as JSON")],
                vec![icon_box("fas fa-key", "Auto Authentication", "One-click WordPress connection via browser")],
            ]),
        ]),
        // Testimonial
        container_with(json!({"background_color":"#f8f9fa","padding":{"unit":"px","top":"60","bottom":"60","isLinked":false}}), vec![
            heading("Trusted by Developers", "h2"),
            testimonial("Finally, an MCP server that actually uses Elementor's REST API. The schema validation alone saved us hours of debugging.", "DevOps Engineer", "WordPress Agency"),
            star_rating(4.9, "Developer Satisfaction"),
        ]),
        // CTA
        container_with(json!({"background_color":"#e94560","padding":{"unit":"px","top":"80","bottom":"80","isLinked":false}}), vec![
            heading_styled("Start Building Today", "h2", "#ffffff", 36),
            text("<p style='text-align:center;color:#fff;opacity:0.9'>docker run -p 8080:8080 juslintek/wp-sqlite-elementor-server:latest</p>"),
            container_with(json!({"align_items":"center"}), vec![
                button_styled("View on GitHub", "https://github.com/juslintek/elementor-mcp-rs", "#ffffff", "#e94560"),
            ]),
        ]),
    ]).await;
}

// ═══════════════════════════════════════════════════════════════════════════════
// 4. HTML → ELEMENTOR CONVERSION
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn p40_html_converted_page() {
    let wp = require_wp!();
    let html = r#"<h1>Company Name</h1>
<p>We build amazing products for the modern web. Our team of experts delivers quality solutions.</p>
<h2>Our Services</h2>
<p>We offer <strong>consulting</strong>, <em>development</em>, and design services for businesses of all sizes.</p>
<img src="https://picsum.photos/800/400?random=40">
<h2>Why Choose Us</h2>
<p>With over 10 years of experience, we've helped hundreds of companies achieve their goals.</p>
<h3>Contact</h3>
<p>Email us at hello@example.com or call +1 (555) 123-4567.</p>"#;

    let elements = html_to_elements(html);
    publish_page(&wp, "E2E: HTML → Elementor Conversion", vec![
        container(elements),
    ]).await;
}

// ═══════════════════════════════════════════════════════════════════════════════
// 5. POSTS
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn p50_create_posts() {
    let wp = require_wp!();
    for (title, content) in [
        ("E2E: First Blog Post", "<p>This is the first blog post created by the MCP integration tests.</p><p>It demonstrates the <code>create_post</code> tool.</p>"),
        ("E2E: Second Blog Post", "<p>Another post showing that the MCP can create multiple posts programmatically.</p>"),
        ("E2E: Technical Article", "<h2>Getting Started</h2><p>Follow these steps to set up your environment...</p><h2>Configuration</h2><p>Edit the config file to customize behavior.</p>"),
    ] {
        let r = wp.post("wp/v2/posts", &json!({"title": title, "content": content, "status": "publish"})).await.unwrap();
        let id = r["id"].as_u64().unwrap();
        let link = r["link"].as_str().unwrap_or("");
        println!("  [{id}] {title} → {link}");
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 6. ELEMENT OPERATIONS (visible result)
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn p60_element_operations_demo() {
    let wp = require_wp!();

    // Create base page
    let el = vec![container(vec![
        heading("Element Operations Demo", "h1"),
        text("<p>This page was built step-by-step using element operations.</p>"),
    ])];
    let id = publish_page(&wp, "E2E: Element Operations", el).await;

    // Add more widgets via element operations
    let mut tree = elementor::get_page_elements(&wp, id).await.unwrap();
    let parent_id = tree[0].id.clone();

    // add_element
    elementor::insert_at(&mut tree, Some(&parent_id), 2,
        widget("divider", json!({"style": "solid"})));
    elementor::insert_at(&mut tree, Some(&parent_id), 3,
        widget("heading", json!({"title": "Added via add_element", "header_size": "h3"})));
    elementor::insert_at(&mut tree, Some(&parent_id), 4,
        widget("text-editor", json!({"editor": "<p>This text was inserted at position 4.</p>"})));

    // duplicate_element — clone the heading
    let h_id = tree[0].elements[3].id.clone();
    let mut clone = elementor::find_by_id(&tree, &h_id).unwrap();
    elementor::regenerate_ids(&mut clone);
    clone.settings["title"] = json!("Duplicated heading (new ID)");
    elementor::insert_at(&mut tree, Some(&parent_id), 5, clone);

    // update_element — change color
    elementor::mutate_by_id(&mut tree, &h_id, &|el| {
        elementor::merge_settings(&mut el.settings, &json!({"title_color": "#e94560"}));
    });

    elementor::set_page_elements(&wp, id, &tree).await.unwrap();
    println!("  Page {id} updated with 6 element operations");
}

// ═══════════════════════════════════════════════════════════════════════════════
// 7. SEED CONTENT (uses the seed_content tool)
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn p70_seed_content() {
    let wp = require_wp!();
    // Call seed via the tool's underlying logic
    use elementor_mcp::tools::Tool;
    let tool = elementor_mcp::tools::seed::SeedContent;
    let result = tool.run(json!({"prefix": "E2E Seed"}), &wp).await.unwrap();
    println!("  {}", result.content[0].text);
}

// ═══════════════════════════════════════════════════════════════════════════════
// 8. TEMPLATES — header, footer, single, archive, popup, loop-item
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn p80_template_header() {
    let wp = require_wp!();
    let data = to_elementor_data(&vec![
        container_with(json!({"flex_direction":"row","background_color":"#1a1a2e",
            "padding":{"unit":"px","top":"15","bottom":"15","left":"20","right":"20","isLinked":false},
            "align_items":"center"}), vec![
            widget("heading", json!({"title":"MySite","header_size":"h4","title_color":"#ffffff"})),
            container_with(json!({"flex_grow":1}), vec![
                nav_menu_styled("Main Menu", "#ffffff", "#e94560"),
            ]),
            container_with(json!({"flex_direction":"row","flex_gap":{"unit":"px","size":10}}), vec![
                button_styled("Login", "/wp-login.php", "transparent", "#ffffff"),
                button_styled("Sign Up", "#", "#e94560", "#ffffff"),
            ]),
        ]),
    ]);
    let body = json!({"title":"E2E: Header Template","status":"publish","meta":{
        "_elementor_template_type":"header","_elementor_data":data,"_elementor_edit_mode":"builder"
    }});
    let r = wp.post("wp/v2/elementor_library", &body).await.unwrap();
    println!("  [{}] Header template with nav-menu created", r["id"]);
}

#[tokio::test]
async fn p81_template_footer() {
    let wp = require_wp!();
    let data = to_elementor_data(&vec![
        container_with(json!({"background_color":"#0f0f23","padding":{"unit":"px","top":"40","bottom":"40","isLinked":false}}), vec![
            columns(3, vec![
                vec![
                    widget("heading", json!({"title":"Company","header_size":"h5","title_color":"#fff"})),
                    widget("text-editor", json!({"editor":"<p style='color:#888'>Building the future of web development.</p>"})),
                ],
                vec![
                    widget("heading", json!({"title":"Links","header_size":"h5","title_color":"#fff"})),
                    widget("icon-list", json!({"icon_list":[
                        {"text":"Home","selected_icon":{"value":"fas fa-home","library":"fa-solid"}},
                        {"text":"Blog","selected_icon":{"value":"fas fa-pen","library":"fa-solid"}},
                        {"text":"Contact","selected_icon":{"value":"fas fa-envelope","library":"fa-solid"}}
                    ]})),
                ],
                vec![
                    widget("heading", json!({"title":"Follow Us","header_size":"h5","title_color":"#fff"})),
                    widget("social-icons", json!({"social_icon_list":[
                        {"social_icon":{"value":"fab fa-github","library":"fa-brands"},"link":{"url":"#"}},
                        {"social_icon":{"value":"fab fa-twitter","library":"fa-brands"},"link":{"url":"#"}},
                        {"social_icon":{"value":"fab fa-linkedin","library":"fa-brands"},"link":{"url":"#"}}
                    ]})),
                ],
            ]),
            widget("divider", json!({"style":"solid","color":"#333"})),
            widget("text-editor", json!({"editor":"<p style='text-align:center;color:#666;font-size:13px'>© 2026 MySite. All rights reserved.</p>"})),
        ]),
    ]);
    let body = json!({"title":"E2E: Footer Template","status":"publish","meta":{
        "_elementor_template_type":"footer","_elementor_data":data,"_elementor_edit_mode":"builder"
    }});
    let r = wp.post("wp/v2/elementor_library", &body).await.unwrap();
    println!("  [{}] Footer template created", r["id"]);
}

#[tokio::test]
async fn p82_template_single_post() {
    let wp = require_wp!();
    let data = to_elementor_data(&vec![
        container_with(json!({"padding":{"unit":"px","top":"40","bottom":"40","isLinked":false}}), vec![
            widget("heading", json!({"title":"[Post Title Placeholder]","header_size":"h1"})),
            widget("text-editor", json!({"editor":"<p style='color:#888'>By Author | Date | Category</p>"})),
            widget("divider", json!({"style":"solid"})),
            widget("text-editor", json!({"editor":"<p>[Post content would appear here via dynamic tags]</p><p>This is a single post template created by the MCP. In production, dynamic tags would pull the actual post content.</p>"})),
            widget("divider", json!({"style":"solid"})),
            widget("heading", json!({"title":"Related Posts","header_size":"h3"})),
            widget("text-editor", json!({"editor":"<p>[Related posts grid would appear here]</p>"})),
        ]),
    ]);
    let body = json!({"title":"E2E: Single Post Template","status":"publish","meta":{
        "_elementor_template_type":"single-post","_elementor_data":data,"_elementor_edit_mode":"builder"
    }});
    let r = wp.post("wp/v2/elementor_library", &body).await.unwrap();
    println!("  [{}] Single post template created", r["id"]);
}

#[tokio::test]
async fn p83_template_archive() {
    let wp = require_wp!();
    let data = to_elementor_data(&vec![
        container_with(json!({"padding":{"unit":"px","top":"40","bottom":"40","isLinked":false}}), vec![
            widget("heading", json!({"title":"[Archive Title]","header_size":"h1","align":"center"})),
            widget("text-editor", json!({"editor":"<p style='text-align:center;color:#888'>[Archive description]</p>"})),
            widget("divider", json!({"style":"solid"})),
            columns(3, vec![
                vec![widget("text-editor", json!({"editor":"<div style='border:1px solid #eee;padding:20px;border-radius:8px'><h3>Post 1</h3><p>Excerpt...</p></div>"}))],
                vec![widget("text-editor", json!({"editor":"<div style='border:1px solid #eee;padding:20px;border-radius:8px'><h3>Post 2</h3><p>Excerpt...</p></div>"}))],
                vec![widget("text-editor", json!({"editor":"<div style='border:1px solid #eee;padding:20px;border-radius:8px'><h3>Post 3</h3><p>Excerpt...</p></div>"}))],
            ]),
        ]),
    ]);
    let body = json!({"title":"E2E: Archive Template","status":"publish","meta":{
        "_elementor_template_type":"archive","_elementor_data":data,"_elementor_edit_mode":"builder"
    }});
    let r = wp.post("wp/v2/elementor_library", &body).await.unwrap();
    println!("  [{}] Archive template created", r["id"]);
}

#[tokio::test]
async fn p84_template_section_reusable() {
    let wp = require_wp!();
    // Reusable section template — can be inserted into any page
    let data = to_elementor_data(&vec![
        container_with(json!({"background_color":"#e94560","padding":{"unit":"px","top":"60","bottom":"60","isLinked":false}}), vec![
            widget("heading", json!({"title":"Ready to Get Started?","header_size":"h2","title_color":"#fff","align":"center"})),
            widget("text-editor", json!({"editor":"<p style='text-align:center;color:#fff;opacity:0.9'>Join thousands of happy customers today.</p>"})),
            container_with(json!({"align_items":"center"}), vec![
                widget("button", json!({"text":"Start Free Trial","size":"lg","background_color":"#fff","button_text_color":"#e94560"})),
            ]),
        ]),
    ]);
    let body = json!({"title":"E2E: Reusable CTA Section","status":"publish","meta":{
        "_elementor_template_type":"section","_elementor_data":data,"_elementor_edit_mode":"builder"
    }});
    let r = wp.post("wp/v2/elementor_library", &body).await.unwrap();
    println!("  [{}] Reusable section template created", r["id"]);
}

#[tokio::test]
async fn p85_list_all_templates() {
    let wp = require_wp!();
    let r = wp.get("wp/v2/elementor_library?per_page=50&status=any&context=edit").await.unwrap();
    let items = r.as_array().unwrap();
    println!("  {} templates found:", items.len());
    for item in items {
        let id = item["id"].as_u64().unwrap_or(0);
        let title = item["title"]["rendered"].as_str().unwrap_or("?");
        let ttype = item["meta"]["_elementor_template_type"].as_str().unwrap_or("?");
        println!("    [{id}] {title} ({ttype})");
    }
    assert!(items.len() >= 5, "Should have at least 5 templates");
}

// ═══════════════════════════════════════════════════════════════════════════════
// 9. STYLING REPRODUCTION — fetch HTML, convert to Elementor, create page
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn p90_reproduce_page_from_html() {
    let wp = require_wp!();
    let wp_url = std::env::var("WP_TEST_URL").unwrap();

    // Fetch the landing page HTML we created earlier
    let client = reqwest::Client::new();
    let resp = client.get(format!("{wp_url}/e2e-complete-landing-page/"))
        .send().await;

    let source_html = match resp {
        Ok(r) if r.status().is_success() => r.text().await.unwrap_or_default(),
        _ => {
            // Fallback: use a static HTML page
            String::from(r#"<h1>Reproduced Page</h1>
<p>This page was created by fetching HTML and converting it to Elementor widgets.</p>
<h2>Features</h2>
<p>Each HTML element is mapped to the appropriate Elementor widget type.</p>
<img src="https://picsum.photos/800/400?random=90">
<h2>How It Works</h2>
<p>The converter parses HTML tags and maps them: h1-h6 → heading, p → text-editor, img → image, unknown → html widget.</p>
<h3>Supported Tags</h3>
<p>Headings, paragraphs, images, and any other HTML via the fallback html widget.</p>"#)
        }
    };

    // Extract meaningful content (strip WP chrome, get body content)
    let content = extract_body_content(&source_html);
    let elements = html_to_elements(&content);

    println!("  Converted {} HTML elements to Elementor widgets", elements.len());

    // Build the reproduced page with styling
    let styled_elements = vec![
        container_with(json!({"background_color":"#f0f4f8","padding":{"unit":"px","top":"40","bottom":"40","isLinked":false}}), vec![
            widget("heading", json!({"title":"Reproduced from HTML","header_size":"h1","align":"center","title_color":"#1a1a2e"})),
            widget("text-editor", json!({"editor":"<p style='text-align:center;color:#666'>This page was automatically generated by fetching HTML and converting to Elementor.</p>"})),
            widget("divider", json!({"style":"solid","color":"#ddd"})),
        ]),
        container_with(json!({"padding":{"unit":"px","top":"20","bottom":"40","isLinked":false}}), elements),
    ];

    publish_page(&wp, "E2E: HTML Reproduction (Styled)", styled_elements).await;
}

#[tokio::test]
async fn p91_reproduce_with_full_styling() {
    let wp = require_wp!();

    // Simulate reproducing a styled marketing section
    let source_html = r#"
<h1>Premium WordPress Hosting</h1>
<p>Lightning-fast servers optimized for WordPress. 99.9% uptime guaranteed.</p>
<h2>Plans</h2>
<p>Starting at $9.99/month. Free SSL, daily backups, and 24/7 support included.</p>
<img src="https://picsum.photos/800/400?random=91">
<h2>Trusted by 50,000+ Sites</h2>
<p>Join the community of developers and businesses who trust us with their WordPress sites.</p>
<h3>Get Started Today</h3>
<p>No credit card required. 30-day money-back guarantee.</p>
"#;

    let raw_elements = html_to_elements(source_html);

    // Enhance with Elementor styling (what a human designer would add)
    let styled = vec![
        // Hero with gradient background
        container_with(json!({
            "background_background":"gradient",
            "background_color":"#667eea",
            "background_color_b":"#764ba2",
            "padding":{"unit":"px","top":"100","bottom":"100","left":"20","right":"20","isLinked":false}
        }), vec![
            heading_styled(&raw_elements[0].settings["title"].as_str().unwrap_or("Title"), "h1", "#ffffff", 52),
            widget("text-editor", json!({"editor":"<p style='text-align:center;color:rgba(255,255,255,0.9);font-size:20px;max-width:600px;margin:auto'>Lightning-fast servers optimized for WordPress. 99.9% uptime guaranteed.</p>"})),
            spacer(20),
            container_with(json!({"align_items":"center"}), vec![
                button_styled("Start Free Trial", "#", "#ffffff", "#667eea"),
            ]),
        ]),
        // Plans section
        container_with(json!({"padding":{"unit":"px","top":"60","bottom":"60","isLinked":false}}), vec![
            heading("Plans", "h2"),
            columns(3, vec![
                vec![
                    container_with(json!({"background_color":"#f8f9fa","padding":{"unit":"px","top":"30","bottom":"30","left":"20","right":"20","isLinked":false}}), vec![
                        heading("Starter", "h3"),
                        widget("text-editor", json!({"editor":"<p style='font-size:36px;font-weight:700'>$9.99<span style='font-size:16px;color:#888'>/mo</span></p>"})),
                        icon_list(vec![("1 Website", "fas fa-check"), ("10GB Storage", "fas fa-check"), ("Free SSL", "fas fa-check")]),
                        button("Choose Plan", "#"),
                    ]),
                ],
                vec![
                    container_with(json!({"background_color":"#667eea","padding":{"unit":"px","top":"30","bottom":"30","left":"20","right":"20","isLinked":false}}), vec![
                        heading_styled("Professional", "h3", "#ffffff", 24),
                        widget("text-editor", json!({"editor":"<p style='font-size:36px;font-weight:700;color:#fff'>$24.99<span style='font-size:16px;color:rgba(255,255,255,0.8)'>/mo</span></p>"})),
                        widget("text-editor", json!({"editor":"<p style='color:#fff'>10 Websites • 50GB • Priority Support</p>"})),
                        button_styled("Choose Plan", "#", "#ffffff", "#667eea"),
                    ]),
                ],
                vec![
                    container_with(json!({"background_color":"#f8f9fa","padding":{"unit":"px","top":"30","bottom":"30","left":"20","right":"20","isLinked":false}}), vec![
                        heading("Enterprise", "h3"),
                        widget("text-editor", json!({"editor":"<p style='font-size:36px;font-weight:700'>$49.99<span style='font-size:16px;color:#888'>/mo</span></p>"})),
                        icon_list(vec![("Unlimited Sites", "fas fa-check"), ("200GB Storage", "fas fa-check"), ("24/7 Phone Support", "fas fa-check")]),
                        button("Choose Plan", "#"),
                    ]),
                ],
            ]),
        ]),
        // Social proof
        container_with(json!({"background_color":"#1a1a2e","padding":{"unit":"px","top":"60","bottom":"60","isLinked":false}}), vec![
            heading_styled("Trusted by 50,000+ Sites", "h2", "#ffffff", 36),
            columns(4, vec![
                vec![counter(50000, "Active Sites")],
                vec![counter(99, "% Uptime")],
                vec![counter(24, "/7 Support")],
                vec![counter(30, "Day Guarantee")],
            ]),
        ]),
    ];

    publish_page(&wp, "E2E: Styled Reproduction (Marketing Page)", styled).await;
}

/// Extract body content from full HTML page, stripping WP chrome.
fn extract_body_content(html: &str) -> String {
    // Try to find main content area
    if let Some(start) = html.find("<main") {
        if let Some(end) = html[start..].find("</main>") {
            return html[start..start + end + 7].to_string();
        }
    }
    if let Some(start) = html.find("<article") {
        if let Some(end) = html[start..].find("</article>") {
            return html[start..start + end + 10].to_string();
        }
    }
    // Fallback: return as-is (the converter handles unknown tags)
    html.to_string()
}
