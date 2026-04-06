//! E2E tests — create real pages on WordPress, verify structure, clean up.
//! Requires: WP_TEST_URL, WP_TEST_USER, WP_TEST_PASS env vars.

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

/// Create a page, verify element count, return page ID for cleanup.
async fn create_and_verify(wp: &WpClient, title: &str, elements: Vec<Element>, expected_count: usize) -> u64 {
    let data = to_elementor_data(&elements);
    let body = json!({
        "title": title, "status": "draft",
        "meta": {"_elementor_data": data, "_elementor_edit_mode": "builder"}
    });
    let result = wp.post("wp/v2/pages", &body).await.expect("create failed");
    let id = result["id"].as_u64().unwrap();

    let tree = elementor::get_page_elements(wp, id).await.expect("read failed");
    assert_eq!(tree.len(), expected_count, "{title}: expected {expected_count} root elements, got {}", tree.len());
    id
}

async fn cleanup(wp: &WpClient, id: u64) {
    wp.delete(&format!("wp/v2/pages/{id}?force=true")).await.ok();
}

// ═══════════════════════════════════════════════════════════════════════════════
// WIDGET TESTS — every widget type with full settings
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn widget_heading() {
    let wp = require_wp!();
    let el = vec![container(vec![heading_styled("Test Heading", "h1", "#FF0000", 48)])];
    let id = create_and_verify(&wp, "E2E: heading", el, 1).await;
    let tree = elementor::get_page_elements(&wp, id).await.unwrap();
    let w = &tree[0].elements[0];
    assert_eq!(w.settings["title"], "Test Heading");
    assert_eq!(w.settings["title_color"], "#FF0000");
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn widget_text_editor() {
    let wp = require_wp!();
    let html = "<p>Hello <strong>bold</strong> and <em>italic</em></p>";
    let el = vec![container(vec![text(html)])];
    let id = create_and_verify(&wp, "E2E: text-editor", el, 1).await;
    let tree = elementor::get_page_elements(&wp, id).await.unwrap();
    assert_eq!(tree[0].elements[0].settings["editor"], html);
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn widget_image() {
    let wp = require_wp!();
    let el = vec![container(vec![image("https://via.placeholder.com/800x400")])];
    let id = create_and_verify(&wp, "E2E: image", el, 1).await;
    let tree = elementor::get_page_elements(&wp, id).await.unwrap();
    assert_eq!(tree[0].elements[0].widget_type.as_deref(), Some("image"));
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn widget_button() {
    let wp = require_wp!();
    let el = vec![container(vec![button_styled("Click Me", "https://example.com", "#0073aa", "#ffffff")])];
    let id = create_and_verify(&wp, "E2E: button", el, 1).await;
    let tree = elementor::get_page_elements(&wp, id).await.unwrap();
    assert_eq!(tree[0].elements[0].settings["text"], "Click Me");
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn widget_icon_list() {
    let wp = require_wp!();
    let el = vec![container(vec![icon_list(vec![
        ("Feature One", "fas fa-check"), ("Feature Two", "fas fa-star"),
    ])])];
    let id = create_and_verify(&wp, "E2E: icon-list", el, 1).await;
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn widget_divider_spacer() {
    let wp = require_wp!();
    let el = vec![container(vec![divider(), spacer(40), divider()])];
    let id = create_and_verify(&wp, "E2E: divider+spacer", el, 1).await;
    let tree = elementor::get_page_elements(&wp, id).await.unwrap();
    assert_eq!(tree[0].elements.len(), 3);
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn widget_html_raw() {
    let wp = require_wp!();
    let code = "<div style='background:#f0f0f0;padding:20px'>Custom HTML</div>";
    let el = vec![container(vec![html_widget(code)])];
    let id = create_and_verify(&wp, "E2E: html", el, 1).await;
    let tree = elementor::get_page_elements(&wp, id).await.unwrap();
    assert_eq!(tree[0].elements[0].settings["html"], code);
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn widget_video() {
    let wp = require_wp!();
    let el = vec![container(vec![video("https://www.youtube.com/watch?v=dQw4w9WgXcQ")])];
    let id = create_and_verify(&wp, "E2E: video", el, 1).await;
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn widget_icon_box() {
    let wp = require_wp!();
    let el = vec![container(vec![icon_box("fas fa-rocket", "Fast", "Lightning speed")])];
    let id = create_and_verify(&wp, "E2E: icon-box", el, 1).await;
    let tree = elementor::get_page_elements(&wp, id).await.unwrap();
    assert_eq!(tree[0].elements[0].settings["title_text"], "Fast");
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn widget_counter_progress() {
    let wp = require_wp!();
    let el = vec![container(vec![counter(1500, "Projects"), progress_bar("Completion", 85)])];
    let id = create_and_verify(&wp, "E2E: counter+progress", el, 1).await;
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn widget_testimonial() {
    let wp = require_wp!();
    let el = vec![container(vec![testimonial("Great product!", "John Doe", "CEO")])];
    let id = create_and_verify(&wp, "E2E: testimonial", el, 1).await;
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn widget_toggle_accordion_tabs() {
    let wp = require_wp!();
    let items = vec![("Section 1", "Content 1"), ("Section 2", "Content 2")];
    let el = vec![container(vec![
        toggle(items.clone()), accordion(items.clone()), tabs(items),
    ])];
    let id = create_and_verify(&wp, "E2E: toggle+accordion+tabs", el, 1).await;
    let tree = elementor::get_page_elements(&wp, id).await.unwrap();
    assert_eq!(tree[0].elements.len(), 3);
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn widget_social_icons() {
    let wp = require_wp!();
    let el = vec![container(vec![social_icons(vec!["facebook", "twitter", "linkedin", "github"])])];
    let id = create_and_verify(&wp, "E2E: social-icons", el, 1).await;
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn widget_alert_star_rating() {
    let wp = require_wp!();
    let el = vec![container(vec![
        alert("Notice", "Important information", "info"),
        star_rating(4.5, "Our Rating"),
    ])];
    let id = create_and_verify(&wp, "E2E: alert+star-rating", el, 1).await;
    cleanup(&wp, id).await;
}

// ═══════════════════════════════════════════════════════════════════════════════
// LAYOUT TESTS — containers, nesting, columns
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn layout_nested_containers_3_levels() {
    let wp = require_wp!();
    let el = vec![container(vec![
        container(vec![
            container(vec![heading("Deep nested", "h3")])
        ])
    ])];
    let id = create_and_verify(&wp, "E2E: nested-3", el, 1).await;
    let tree = elementor::get_page_elements(&wp, id).await.unwrap();
    // Verify 3 levels deep
    let level3 = &tree[0].elements[0].elements[0];
    assert_eq!(level3.elements[0].widget_type.as_deref(), Some("heading"));
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn layout_two_columns() {
    let wp = require_wp!();
    let el = vec![columns(2, vec![
        vec![heading("Left Column", "h2"), text("<p>Left content</p>")],
        vec![heading("Right Column", "h2"), text("<p>Right content</p>")],
    ])];
    let id = create_and_verify(&wp, "E2E: 2-col", el, 1).await;
    let tree = elementor::get_page_elements(&wp, id).await.unwrap();
    assert_eq!(tree[0].elements.len(), 2); // 2 column containers
    assert_eq!(tree[0].elements[0].elements.len(), 2); // heading + text in each
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn layout_three_columns() {
    let wp = require_wp!();
    let el = vec![columns(3, vec![
        vec![icon_box("fas fa-bolt", "Fast", "Speed")],
        vec![icon_box("fas fa-shield-alt", "Secure", "Safety")],
        vec![icon_box("fas fa-code", "Clean", "Code")],
    ])];
    let id = create_and_verify(&wp, "E2E: 3-col", el, 1).await;
    let tree = elementor::get_page_elements(&wp, id).await.unwrap();
    assert_eq!(tree[0].elements.len(), 3);
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn layout_sidebar() {
    let wp = require_wp!();
    // 70/30 sidebar layout
    let el = vec![container_with(json!({"flex_direction": "row"}), vec![
        container_with(json!({"_column_size": 70}), vec![
            heading("Main Content", "h1"), text("<p>Article body...</p>"),
        ]),
        container_with(json!({"_column_size": 30}), vec![
            heading("Sidebar", "h3"), text("<p>Widget area</p>"),
        ]),
    ])];
    let id = create_and_verify(&wp, "E2E: sidebar", el, 1).await;
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn layout_multiple_sections() {
    let wp = require_wp!();
    let el = vec![
        container_with(json!({"background_background": "classic", "background_color": "#1a1a2e"}), vec![
            heading_styled("Hero Section", "h1", "#ffffff", 48),
        ]),
        container(vec![heading("Content Section", "h2"), text("<p>Body</p>")]),
        container_with(json!({"background_color": "#f5f5f5"}), vec![
            heading("Footer", "h4"),
        ]),
    ];
    let id = create_and_verify(&wp, "E2E: multi-section", el, 3).await;
    cleanup(&wp, id).await;
}

// ═══════════════════════════════════════════════════════════════════════════════
// PAGE BUILDING TESTS — complete pages
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn page_landing() {
    let wp = require_wp!();
    let el = vec![
        // Hero
        container_with(json!({"background_background": "classic", "background_color": "#0073aa",
            "padding": {"unit":"px","top":"80","bottom":"80","left":"0","right":"0","isLinked":false}}), vec![
            heading_styled("Welcome to Our Product", "h1", "#ffffff", 48),
            text("<p style='color:#ccc;text-align:center'>The best solution for your needs</p>"),
            button_styled("Get Started", "#pricing", "#ff6600", "#ffffff"),
        ]),
        // Features
        container(vec![
            heading("Features", "h2"),
            columns(3, vec![
                vec![icon_box("fas fa-rocket", "Fast", "Lightning performance")],
                vec![icon_box("fas fa-lock", "Secure", "Enterprise security")],
                vec![icon_box("fas fa-cloud", "Scalable", "Grows with you")],
            ]),
        ]),
        // Testimonial
        container_with(json!({"background_color": "#f9f9f9"}), vec![
            heading("What People Say", "h2"),
            testimonial("Amazing product!", "Jane Smith", "CTO at TechCorp"),
        ]),
        // CTA
        container(vec![
            heading("Ready to Start?", "h2"),
            button_styled("Sign Up Now", "/signup", "#0073aa", "#ffffff"),
        ]),
    ];
    let id = create_and_verify(&wp, "E2E: landing-page", el, 4).await;

    // Verify structure depth
    let tree = elementor::get_page_elements(&wp, id).await.unwrap();
    let flat = elementor::flatten_tree(&tree, "");
    assert!(flat.len() >= 15, "Landing page should have 15+ elements, got {}", flat.len());

    cleanup(&wp, id).await;
}

#[tokio::test]
async fn page_all_widgets() {
    let wp = require_wp!();
    let el = vec![container(vec![
        heading("All Widgets Test", "h1"),
        text("<p>Text editor content</p>"),
        image("https://via.placeholder.com/600x300"),
        button("Click", "#"),
        icon_list(vec![("Item 1", "fas fa-check"), ("Item 2", "fas fa-star")]),
        divider(),
        spacer(20),
        html_widget("<div>Custom HTML</div>"),
        video("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
        icon_box("fas fa-heart", "Love", "Description"),
        image_box("https://via.placeholder.com/100", "Title", "Desc"),
        counter(999, "Count"),
        progress_bar("Progress", 75),
        testimonial("Quote", "Author", "Role"),
        social_icons(vec!["facebook", "twitter"]),
        alert("Alert", "Message", "warning"),
        star_rating(4.0, "Rating"),
        toggle(vec![("Q1", "A1")]),
        accordion(vec![("S1", "C1")]),
        tabs(vec![("T1", "C1")]),
    ])];
    let id = create_and_verify(&wp, "E2E: all-widgets", el, 1).await;
    let tree = elementor::get_page_elements(&wp, id).await.unwrap();
    assert_eq!(tree[0].elements.len(), 20, "Should have 20 widgets");
    cleanup(&wp, id).await;
}

// ═══════════════════════════════════════════════════════════════════════════════
// HTML TO ELEMENTOR CONVERSION TESTS
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn convert_html_headings() {
    let wp = require_wp!();
    let html = "<h1>Main Title</h1><h2>Subtitle</h2><h3>Section</h3>";
    let elements = html_to_elements(html);
    assert_eq!(elements.len(), 3);
    assert_eq!(elements[0].widget_type.as_deref(), Some("heading"));
    assert_eq!(elements[0].settings["title"], "Main Title");
    assert_eq!(elements[0].settings["header_size"], "h1");
    assert_eq!(elements[1].settings["header_size"], "h2");

    // Create page with converted elements
    let el = vec![container(elements)];
    let id = create_and_verify(&wp, "E2E: html-headings", el, 1).await;
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn convert_html_mixed_content() {
    let wp = require_wp!();
    let html = r#"<h1>Welcome</h1><p>This is a paragraph with <strong>bold</strong> text.</p><img src="https://via.placeholder.com/400x200"><p>Another paragraph.</p>"#;
    let elements = html_to_elements(html);
    assert_eq!(elements.len(), 4);
    assert_eq!(elements[0].widget_type.as_deref(), Some("heading"));
    assert_eq!(elements[1].widget_type.as_deref(), Some("text-editor"));
    assert_eq!(elements[2].widget_type.as_deref(), Some("image"));
    assert_eq!(elements[3].widget_type.as_deref(), Some("text-editor"));

    let el = vec![container(elements)];
    let id = create_and_verify(&wp, "E2E: html-mixed", el, 1).await;
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn convert_html_full_page() {
    let wp = require_wp!();
    let html = r#"
        <h1>Company Name</h1>
        <p>We build amazing products for the modern web.</p>
        <h2>Our Services</h2>
        <p>We offer consulting, development, and design services.</p>
        <img src="https://via.placeholder.com/800x400">
        <h2>Contact Us</h2>
        <p>Email us at hello@example.com</p>
    "#;
    let elements = html_to_elements(html);
    assert!(elements.len() >= 6, "Should parse at least 6 elements, got {}", elements.len());

    let el = vec![container(elements)];
    let id = create_and_verify(&wp, "E2E: html-full-page", el, 1).await;
    let tree = elementor::get_page_elements(&wp, id).await.unwrap();
    assert!(tree[0].elements.len() >= 6);
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn convert_html_unknown_tags_fallback() {
    let wp = require_wp!();
    let html = r#"<h1>Title</h1><div class="custom">Custom block</div><p>End</p>"#;
    let elements = html_to_elements(html);
    // div should become html widget (fallback)
    let types: Vec<_> = elements.iter().map(|e| e.widget_type.as_deref().unwrap_or("")).collect();
    assert_eq!(types[0], "heading");
    assert_eq!(types[1], "html"); // fallback for <div>
    assert_eq!(types[2], "text-editor");

    let el = vec![container(elements)];
    let id = create_and_verify(&wp, "E2E: html-fallback", el, 1).await;
    cleanup(&wp, id).await;
}

// ═══════════════════════════════════════════════════════════════════════════════
// ELEMENT OPERATION TESTS — add, remove, move, duplicate, reorder
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn ops_add_widget_to_existing_page() {
    let wp = require_wp!();
    let el = vec![container(vec![heading("Original", "h2")])];
    let id = create_and_verify(&wp, "E2E: ops-add", el, 1).await;

    // Add a text widget
    let mut tree = elementor::get_page_elements(&wp, id).await.unwrap();
    let parent_id = tree[0].id.clone();
    elementor::insert_at(&mut tree, Some(&parent_id), 1, text("<p>Added</p>"));
    elementor::set_page_elements(&wp, id, &tree).await.unwrap();

    let updated = elementor::get_page_elements(&wp, id).await.unwrap();
    assert_eq!(updated[0].elements.len(), 2);
    assert_eq!(updated[0].elements[1].widget_type.as_deref(), Some("text-editor"));
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn ops_remove_widget() {
    let wp = require_wp!();
    let w1 = heading("Keep", "h2");
    let w2 = text("<p>Remove me</p>");
    let w2_id = w2.id.clone();
    let el = vec![container(vec![w1, w2])];
    let id = create_and_verify(&wp, "E2E: ops-remove", el, 1).await;

    let mut tree = elementor::get_page_elements(&wp, id).await.unwrap();
    elementor::remove_by_id(&mut tree, &w2_id);
    elementor::set_page_elements(&wp, id, &tree).await.unwrap();

    let updated = elementor::get_page_elements(&wp, id).await.unwrap();
    assert_eq!(updated[0].elements.len(), 1);
    assert_eq!(updated[0].elements[0].widget_type.as_deref(), Some("heading"));
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn ops_move_between_containers() {
    let wp = require_wp!();
    let w = heading("Movable", "h3");
    let w_id = w.id.clone();
    let c2 = container(vec![text("<p>Target</p>")]);
    let c2_id = c2.id.clone();
    let el = vec![container(vec![w]), c2];
    let id = create_and_verify(&wp, "E2E: ops-move", el, 2).await;

    let mut tree = elementor::get_page_elements(&wp, id).await.unwrap();
    let moved = elementor::remove_by_id(&mut tree, &w_id).unwrap();
    elementor::insert_at(&mut tree, Some(&c2_id), 0, moved);
    elementor::set_page_elements(&wp, id, &tree).await.unwrap();

    let updated = elementor::get_page_elements(&wp, id).await.unwrap();
    assert_eq!(updated[0].elements.len(), 0, "Source container should be empty");
    assert_eq!(updated[1].elements.len(), 2, "Target should have 2 elements");
    assert_eq!(updated[1].elements[0].widget_type.as_deref(), Some("heading"));
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn ops_duplicate_widget() {
    let wp = require_wp!();
    let w = heading("Original", "h2");
    let w_id = w.id.clone();
    let el = vec![container(vec![w])];
    let id = create_and_verify(&wp, "E2E: ops-duplicate", el, 1).await;

    let mut tree = elementor::get_page_elements(&wp, id).await.unwrap();
    let original = elementor::find_by_id(&tree, &w_id).unwrap();
    let mut clone = original.clone();
    elementor::regenerate_ids(&mut clone);
    let parent_id = tree[0].id.clone();
    elementor::insert_at(&mut tree, Some(&parent_id), 1, clone);
    elementor::set_page_elements(&wp, id, &tree).await.unwrap();

    let updated = elementor::get_page_elements(&wp, id).await.unwrap();
    assert_eq!(updated[0].elements.len(), 2);
    assert_eq!(updated[0].elements[0].settings["title"], "Original");
    assert_eq!(updated[0].elements[1].settings["title"], "Original");
    assert_ne!(updated[0].elements[0].id, updated[0].elements[1].id);
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn ops_update_settings_partial() {
    let wp = require_wp!();
    let w = heading_styled("Before", "h2", "#000000", 24);
    let w_id = w.id.clone();
    let el = vec![container(vec![w])];
    let id = create_and_verify(&wp, "E2E: ops-update", el, 1).await;

    let mut tree = elementor::get_page_elements(&wp, id).await.unwrap();
    elementor::mutate_by_id(&mut tree, &w_id, &|el| {
        elementor::merge_settings(&mut el.settings, &json!({"title": "After", "title_color": "#FF0000"}));
    });
    elementor::set_page_elements(&wp, id, &tree).await.unwrap();

    let updated = elementor::get_page_elements(&wp, id).await.unwrap();
    let w = elementor::find_by_id(&updated, &w_id).unwrap();
    assert_eq!(w.settings["title"], "After");
    assert_eq!(w.settings["title_color"], "#FF0000");
    // Original settings preserved
    assert_eq!(w.settings["header_size"], "h2");
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn ops_reorder_widgets() {
    let wp = require_wp!();
    let w1 = heading("First", "h2");
    let w2 = heading("Second", "h2");
    let w3 = heading("Third", "h2");
    let w3_id = w3.id.clone();
    let el = vec![container(vec![w1, w2, w3])];
    let id = create_and_verify(&wp, "E2E: ops-reorder", el, 1).await;

    // Move "Third" to position 0
    let mut tree = elementor::get_page_elements(&wp, id).await.unwrap();
    let parent_id = tree[0].id.clone();
    let moved = elementor::remove_by_id(&mut tree, &w3_id).unwrap();
    elementor::insert_at(&mut tree, Some(&parent_id), 0, moved);
    elementor::set_page_elements(&wp, id, &tree).await.unwrap();

    let updated = elementor::get_page_elements(&wp, id).await.unwrap();
    assert_eq!(updated[0].elements[0].settings["title"], "Third");
    assert_eq!(updated[0].elements[1].settings["title"], "First");
    assert_eq!(updated[0].elements[2].settings["title"], "Second");
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn ops_copy_page_structure() {
    let wp = require_wp!();
    // Create source page
    let el = vec![
        container(vec![heading("Page Title", "h1"), text("<p>Content</p>")]),
        container(vec![button("CTA", "#")]),
    ];
    let src_id = create_and_verify(&wp, "E2E: copy-source", el, 2).await;

    // Read source, create copy with new IDs
    let mut tree = elementor::get_page_elements(&wp, src_id).await.unwrap();
    for el in &mut tree { elementor::regenerate_ids(el); }

    let data = to_elementor_data(&tree);
    let body = json!({
        "title": "E2E: copy-target", "status": "draft",
        "meta": {"_elementor_data": data, "_elementor_edit_mode": "builder"}
    });
    let result = wp.post("wp/v2/pages", &body).await.unwrap();
    let tgt_id = result["id"].as_u64().unwrap();

    // Verify copy has same structure
    let copy = elementor::get_page_elements(&wp, tgt_id).await.unwrap();
    assert_eq!(copy.len(), 2);
    assert_eq!(copy[0].elements.len(), 2);
    assert_eq!(copy[0].elements[0].settings["title"], "Page Title");

    cleanup(&wp, src_id).await;
    cleanup(&wp, tgt_id).await;
}
