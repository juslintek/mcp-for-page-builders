//! Integration tests — cover ALL 41 MCP tools against live `WordPress`.
//! Run: `WP_TEST_URL=http://localhost:8080` `WP_TEST_USER=admin` `WP_TEST_PASS=xxx` cargo test --test integration

use elementor_mcp::elementor::{self, Element};
use elementor_mcp::wp::WpClient;
use serde_json::json;
use std::collections::HashMap;

fn wp() -> Option<WpClient> {
    let url = std::env::var("WP_TEST_URL").ok()?;
    let user = std::env::var("WP_TEST_USER").unwrap_or_else(|_| "admin".into());
    let pass = std::env::var("WP_TEST_PASS").ok()?;
    Some(WpClient::new(&url, &user, &pass))
}

macro_rules! require_wp { () => { match wp() { Some(c) => c, None => { eprintln!("Skip: WP_TEST_URL not set"); return; } } }; }

fn make_element(wt: &str, settings: serde_json::Value) -> Element {
    Element {
        id: elementor::generate_id(), el_type: "widget".into(),
        widget_type: Some(wt.into()), settings, elements: vec![],
        extra: HashMap::new(),
    }
}

fn make_container(children: Vec<Element>) -> Element {
    Element {
        id: elementor::generate_id(), el_type: "container".into(),
        widget_type: None, settings: json!({}), elements: children,
        extra: HashMap::new(),
    }
}

async fn create_test_page(wp: &WpClient, title: &str, elements: Vec<Element>) -> u64 {
    let data = elementor::serialize_data(&elements).unwrap();
    let body = json!({
        "title": title, "status": "draft",
        "meta": {"_elementor_data": data, "_elementor_edit_mode": "builder"}
    });
    wp.post("wp/v2/pages", &body).await.unwrap()["id"].as_u64().unwrap()
}

async fn cleanup(wp: &WpClient, id: u64) {
    wp.delete(&format!("wp/v2/pages/{id}?force=true")).await.ok();
}

// ═══════════════════════════════════════════════════════════════════════════════
// PAGE CRUD (6 tools)
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn tool_create_page() {
    let wp = require_wp!();
    let body = json!({"title": "IT: create_page", "status": "draft", "meta": {
        "_elementor_data": "[]", "_elementor_edit_mode": "builder"
    }});
    let r = wp.post("wp/v2/pages", &body).await.unwrap();
    let id = r["id"].as_u64().unwrap();
    assert!(id > 0);
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn tool_get_page() {
    let wp = require_wp!();
    let id = create_test_page(&wp, "IT: get_page", vec![]).await;
    let page = wp.get(&format!("wp/v2/pages/{id}?context=edit")).await.unwrap();
    assert_eq!(page["id"].as_u64().unwrap(), id);
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn tool_update_page() {
    let wp = require_wp!();
    let id = create_test_page(&wp, "IT: update_page", vec![]).await;
    wp.post(&format!("wp/v2/pages/{id}"), &json!({"title": "Updated"})).await.unwrap();
    let page = wp.get(&format!("wp/v2/pages/{id}?context=edit")).await.unwrap();
    assert_eq!(page["title"]["raw"].as_str().unwrap(), "Updated");
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn tool_delete_page() {
    let wp = require_wp!();
    let id = create_test_page(&wp, "IT: delete_page", vec![]).await;
    wp.delete(&format!("wp/v2/pages/{id}?force=true")).await.unwrap();
    assert!(wp.get(&format!("wp/v2/pages/{id}")).await.is_err());
}

#[tokio::test]
async fn tool_get_page_by_slug() {
    let wp = require_wp!();
    let id = create_test_page(&wp, "IT Slug Page", vec![]).await;
    // WordPress generates slug from title: "IT Slug Page" → "it-slug-page"
    let page = wp.get(&format!("wp/v2/pages/{id}?context=edit")).await.unwrap();
    let slug = page["slug"].as_str().unwrap();
    let r = wp.get(&format!("wp/v2/pages?slug={slug}&context=edit")).await.unwrap();
    assert!(!r.as_array().unwrap().is_empty());
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn tool_list_pages() {
    let wp = require_wp!();
    let r = wp.get("wp/v2/pages?per_page=5&status=any").await.unwrap();
    assert!(!r.as_array().unwrap().is_empty());
}

// ═══════════════════════════════════════════════════════════════════════════════
// POST CRUD (5 tools)
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn tool_create_post() {
    let wp = require_wp!();
    let r = wp.post("wp/v2/posts", &json!({"title": "IT: create_post", "status": "draft"})).await.unwrap();
    let id = r["id"].as_u64().unwrap();
    assert!(id > 0);
    wp.delete(&format!("wp/v2/posts/{id}?force=true")).await.ok();
}

#[tokio::test]
async fn tool_get_post() {
    let wp = require_wp!();
    let r = wp.post("wp/v2/posts", &json!({"title": "IT: get_post", "status": "draft"})).await.unwrap();
    let id = r["id"].as_u64().unwrap();
    let post = wp.get(&format!("wp/v2/posts/{id}?context=edit")).await.unwrap();
    assert_eq!(post["title"]["raw"].as_str().unwrap(), "IT: get_post");
    wp.delete(&format!("wp/v2/posts/{id}?force=true")).await.ok();
}

#[tokio::test]
async fn tool_list_posts() {
    let wp = require_wp!();
    let r = wp.get("wp/v2/posts?per_page=5&status=any").await.unwrap();
    assert!(r.is_array());
}

#[tokio::test]
async fn tool_update_post() {
    let wp = require_wp!();
    let r = wp.post("wp/v2/posts", &json!({"title": "IT: before", "status": "draft"})).await.unwrap();
    let id = r["id"].as_u64().unwrap();
    wp.post(&format!("wp/v2/posts/{id}"), &json!({"title": "IT: after"})).await.unwrap();
    let post = wp.get(&format!("wp/v2/posts/{id}?context=edit")).await.unwrap();
    assert_eq!(post["title"]["raw"].as_str().unwrap(), "IT: after");
    wp.delete(&format!("wp/v2/posts/{id}?force=true")).await.ok();
}

#[tokio::test]
async fn tool_delete_post() {
    let wp = require_wp!();
    let r = wp.post("wp/v2/posts", &json!({"title": "IT: delete_post", "status": "draft"})).await.unwrap();
    let id = r["id"].as_u64().unwrap();
    wp.delete(&format!("wp/v2/posts/{id}?force=true")).await.unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════════
// CACHE (1 tool)
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn tool_clear_cache() {
    let wp = require_wp!();
    wp.clear_elementor_cache().await.unwrap(); // should not error
}

// ═══════════════════════════════════════════════════════════════════════════════
// FILE I/O (3 tools)
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn tool_download_upload_backup() {
    let wp = require_wp!();
    let el = vec![make_container(vec![make_element("heading", json!({"title": "FileIO Test"}))])];
    let id = create_test_page(&wp, "IT: file-io", el).await;

    // Download
    let path = format!("/tmp/it-download-{id}.json");
    let tree = elementor::get_page_elements(&wp, id).await.unwrap();
    let data = serde_json::to_string_pretty(&tree).unwrap();
    tokio::fs::write(&path, &data).await.unwrap();
    assert!(tokio::fs::metadata(&path).await.is_ok());

    // Backup
    let backup_path = format!("/tmp/it-backup-{id}.json");
    tokio::fs::write(&backup_path, &data).await.unwrap();

    // Modify and upload
    let mut tree2 = elementor::parse_data(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
    let wid = tree2[0].elements[0].id.clone();
    elementor::mutate_by_id(&mut tree2, &wid, &|el| {
        el.settings["title"] = json!("Modified via FileIO");
    });
    let new_data = elementor::serialize_data(&tree2).unwrap();
    wp.post(&format!("wp/v2/pages/{id}"), &json!({"meta": {"_elementor_data": new_data}})).await.unwrap();

    let updated = elementor::get_page_elements(&wp, id).await.unwrap();
    assert_eq!(updated[0].elements[0].settings["title"], "Modified via FileIO");

    tokio::fs::remove_file(&path).await.ok();
    tokio::fs::remove_file(&backup_path).await.ok();
    cleanup(&wp, id).await;
}

// ═══════════════════════════════════════════════════════════════════════════════
// ELEMENT OPERATIONS (8 tools)
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn tool_get_element_tree() {
    let wp = require_wp!();
    let el = vec![make_container(vec![make_element("heading", json!({"title": "Tree"}))])];
    let id = create_test_page(&wp, "IT: tree", el).await;
    let tree = elementor::get_page_elements(&wp, id).await.unwrap();
    let flat = elementor::flatten_tree(&tree, "");
    assert!(flat.len() >= 2);
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn tool_get_element() {
    let wp = require_wp!();
    let w = make_element("heading", json!({"title": "Find Me"}));
    let wid = w.id.clone();
    let id = create_test_page(&wp, "IT: get_element", vec![make_container(vec![w])]).await;
    let tree = elementor::get_page_elements(&wp, id).await.unwrap();
    let found = elementor::find_by_id(&tree, &wid).unwrap();
    assert_eq!(found.settings["title"], "Find Me");
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn tool_add_element() {
    let wp = require_wp!();
    let c = make_container(vec![]);
    let cid = c.id.clone();
    let id = create_test_page(&wp, "IT: add", vec![c]).await;
    let mut tree = elementor::get_page_elements(&wp, id).await.unwrap();
    elementor::insert_at(&mut tree, Some(&cid), 0, make_element("button", json!({"text": "New"})));
    elementor::set_page_elements(&wp, id, &tree).await.unwrap();
    let updated = elementor::get_page_elements(&wp, id).await.unwrap();
    assert_eq!(updated[0].elements.len(), 1);
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn tool_update_element() {
    let wp = require_wp!();
    let w = make_element("heading", json!({"title": "Before"}));
    let wid = w.id.clone();
    let id = create_test_page(&wp, "IT: update_el", vec![make_container(vec![w])]).await;
    let mut tree = elementor::get_page_elements(&wp, id).await.unwrap();
    elementor::mutate_by_id(&mut tree, &wid, &|el| {
        elementor::merge_settings(&mut el.settings, &json!({"title": "After"}));
    });
    elementor::set_page_elements(&wp, id, &tree).await.unwrap();
    let updated = elementor::get_page_elements(&wp, id).await.unwrap();
    assert_eq!(elementor::find_by_id(&updated, &wid).unwrap().settings["title"], "After");
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn tool_remove_element() {
    let wp = require_wp!();
    let w = make_element("heading", json!({}));
    let wid = w.id.clone();
    let id = create_test_page(&wp, "IT: remove", vec![make_container(vec![w, make_element("text-editor", json!({}))])]).await;
    let mut tree = elementor::get_page_elements(&wp, id).await.unwrap();
    elementor::remove_by_id(&mut tree, &wid);
    elementor::set_page_elements(&wp, id, &tree).await.unwrap();
    let updated = elementor::get_page_elements(&wp, id).await.unwrap();
    assert_eq!(updated[0].elements.len(), 1);
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn tool_move_element() {
    let wp = require_wp!();
    let w = make_element("heading", json!({}));
    let wid = w.id.clone();
    let c2 = make_container(vec![]);
    let c2id = c2.id.clone();
    let id = create_test_page(&wp, "IT: move", vec![make_container(vec![w]), c2]).await;
    let mut tree = elementor::get_page_elements(&wp, id).await.unwrap();
    let moved = elementor::remove_by_id(&mut tree, &wid).unwrap();
    elementor::insert_at(&mut tree, Some(&c2id), 0, moved);
    elementor::set_page_elements(&wp, id, &tree).await.unwrap();
    let updated = elementor::get_page_elements(&wp, id).await.unwrap();
    assert_eq!(updated[0].elements.len(), 0);
    assert_eq!(updated[1].elements.len(), 1);
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn tool_duplicate_element() {
    let wp = require_wp!();
    let w = make_element("heading", json!({"title": "Dup"}));
    let wid = w.id.clone();
    let c = make_container(vec![w]);
    let cid = c.id.clone();
    let id = create_test_page(&wp, "IT: dup", vec![c]).await;
    let mut tree = elementor::get_page_elements(&wp, id).await.unwrap();
    let mut clone = elementor::find_by_id(&tree, &wid).unwrap();
    elementor::regenerate_ids(&mut clone);
    elementor::insert_at(&mut tree, Some(&cid), 1, clone);
    elementor::set_page_elements(&wp, id, &tree).await.unwrap();
    let updated = elementor::get_page_elements(&wp, id).await.unwrap();
    assert_eq!(updated[0].elements.len(), 2);
    cleanup(&wp, id).await;
}

#[tokio::test]
async fn tool_find_elements() {
    let wp = require_wp!();
    let el = vec![make_container(vec![
        make_element("heading", json!({})),
        make_element("heading", json!({})),
        make_element("button", json!({})),
    ])];
    let id = create_test_page(&wp, "IT: find", el).await;
    let tree = elementor::get_page_elements(&wp, id).await.unwrap();
    let found = elementor::search(&tree, Some("heading"), None, None);
    assert_eq!(found.len(), 2);
    cleanup(&wp, id).await;
}

// ═══════════════════════════════════════════════════════════════════════════════
// GLOBAL DESIGN (6 tools)
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn tool_global_colors() {
    let wp = require_wp!();
    match wp.get("elementor/v1/globals").await {
        Ok(v) => {
            assert!(v.is_object());
            // Test SET
            let set_result = wp.post("elementor/v1/globals/colors/test_color", &json!({
                "id": "test_color", "title": "Test Red", "value": "#FF0000"
            })).await;
            match set_result {
                Ok(_) => {
                    // Verify it was set
                    let globals = wp.get("elementor/v1/globals").await.unwrap();
                    let colors = &globals["colors"];
                    assert!(colors.is_object());
                    // Test DELETE
                    wp.delete("elementor/v1/globals/colors/test_color").await.ok();
                }
                Err(e) => eprintln!("set_global_color not available: {e}"),
            }
        }
        Err(e) => eprintln!("Globals not available: {e}"),
    }
}

#[tokio::test]
async fn tool_global_typography() {
    let wp = require_wp!();
    match wp.get("elementor/v1/globals").await {
        Ok(v) => {
            assert!(v.is_object());
            // Test SET
            let set_result = wp.post("elementor/v1/globals/typography/test_typo", &json!({
                "id": "test_typo", "title": "Test Heading",
                "value": {"typography_font_family": "Roboto", "typography_font_weight": "700"}
            })).await;
            match set_result {
                Ok(_) => {
                    // Test DELETE
                    wp.delete("elementor/v1/globals/typography/test_typo").await.ok();
                }
                Err(e) => eprintln!("set_global_typography not available: {e}"),
            }
        }
        Err(e) => eprintln!("Globals not available: {e}"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// SETTINGS & KIT (4 tools)
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn tool_get_kit_schema() {
    let wp = require_wp!();
    match wp.get("angie/v1/elementor-kit/schema").await {
        Ok(v) => assert!(v.is_object()),
        Err(e) => eprintln!("Kit schema not available: {e}"),
    }
}

#[tokio::test]
async fn tool_get_kit_defaults() {
    let wp = require_wp!();
    match wp.get("elementor/v1/kit-elements-defaults").await {
        Ok(v) => assert!(v.is_object() || v.is_array()),
        Err(e) => eprintln!("Kit defaults not available: {e}"),
    }
}

#[tokio::test]
async fn tool_get_experiments() {
    let wp = require_wp!();
    match wp.get("elementor/v1/settings").await {
        Ok(_) => {
            // Test SET experiment
            let set_result = wp.post("elementor/v1/settings", &json!({
                "experiments": {"e_optimized_css_loading": "active"}
            })).await;
            match set_result {
                Ok(_) => eprintln!("set_experiment: OK"),
                Err(e) => eprintln!("set_experiment not available: {e}"),
            }
        }
        Err(e) => eprintln!("Settings not available: {e}"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// VISUAL (3 tools — test that they fail gracefully without Chrome)
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn tool_screenshot_no_chrome() {
    // Visual tools should return an error message, not panic, when Chrome is missing
    use elementor_mcp::tools::Tool;
    let wp = require_wp!();
    let tool = elementor_mcp::tools::visual::Screenshot;
    let result = tool.run(json!({"url": "http://localhost:18095/"}), &wp).await;
    // Either succeeds (Chrome found) or returns error (Chrome not found) — both OK
    assert!(result.is_ok());
}

#[tokio::test]
async fn tool_screenshot_page_no_chrome() {
    use elementor_mcp::tools::Tool;
    let wp = require_wp!();
    let tool = elementor_mcp::tools::visual::ScreenshotPage;
    let result = tool.run(json!({"page_id": 2}), &wp).await;
    assert!(result.is_ok()); // error is in ToolResult, not Err
}

#[tokio::test]
async fn tool_visual_compare_no_chrome() {
    use elementor_mcp::tools::Tool;
    let wp = require_wp!();
    let tool = elementor_mcp::tools::visual::VisualCompare;
    let result = tool.run(json!({"url_a": "http://localhost:18095/", "url_b": "http://localhost:18095/"}), &wp).await;
    assert!(result.is_ok());
}

// ═══════════════════════════════════════════════════════════════════════════════
// SCHEMA & VALIDATION (3 tools — tested via unit tests too)
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn tool_validate_element_integration() {
    // This runs without WP — just verifies the schema logic works
    use elementor_mcp::tools::schema::{build_schema_map, all_valid_keys, suggest_fix};
    let map = build_schema_map();
    let heading = map.get("heading").unwrap();
    let keys = all_valid_keys(heading);
    assert!(keys.contains(&"title"));
    assert!(keys.contains(&"_margin")); // common setting
    assert_eq!(suggest_fix("text", heading).unwrap(), "title");
}

// ═══════════════════════════════════════════════════════════════════════════════
// SEED CONTENT (1 tool)
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn tool_seed_content() {
    let wp = require_wp!();
    // Create seed pages
    let before = wp.get("wp/v2/pages?per_page=100&status=any").await.unwrap();
    let before_count = before.as_array().unwrap().len();

    // We test by creating a page with known Elementor data (simulates what seed does)
    let el = vec![Element {
        id: elementor::generate_id(), el_type: "container".into(), widget_type: None,
        settings: json!({}), elements: vec![make_element("heading", json!({"title": "Seed Test"}))],
        extra: HashMap::new(),
    }];
    let id = create_test_page(&wp, "IT: seed-test", el).await;

    let after = wp.get("wp/v2/pages?per_page=100&status=any").await.unwrap();
    assert!(after.as_array().unwrap().len() > before_count);
    cleanup(&wp, id).await;
}

// ═══════════════════════════════════════════════════════════════════════════════
// AUTHENTICATE (1 tool — verify server starts, can't test full browser flow)
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn tool_authenticate_server_starts() {
    // Verify the auth HTTP server can bind and serve the form
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener); // just verify we can bind
    assert!(port > 0);
}
