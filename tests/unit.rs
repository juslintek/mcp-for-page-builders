use elementor_mcp::elementor::*;
use serde_json::json;

fn make_widget(id: &str, widget_type: &str) -> Element {
    Element {
        id: id.to_string(),
        el_type: "widget".to_string(),
        widget_type: Some(widget_type.to_string()),
        settings: json!({}),
        elements: vec![],
        extra: Default::default(),
    }
}

fn make_container(id: &str, children: Vec<Element>) -> Element {
    Element {
        id: id.to_string(),
        el_type: "container".to_string(),
        widget_type: None,
        settings: json!({}),
        elements: children,
        extra: Default::default(),
    }
}

fn sample_tree() -> Vec<Element> {
    vec![
        make_container("root1", vec![
            make_widget("w1", "heading"),
            make_widget("w2", "text-editor"),
        ]),
        make_container("root2", vec![
            make_container("inner1", vec![
                make_widget("w3", "image"),
            ]),
        ]),
    ]
}

// ── Tree operations ───────────────────────────────────────────────────────────

#[test]
fn find_root_element() {
    let tree = sample_tree();
    let found = find_by_id(&tree, "root1").unwrap();
    assert_eq!(found.id, "root1");
    assert_eq!(found.elements.len(), 2);
}

#[test]
fn find_nested_widget() {
    let tree = sample_tree();
    let found = find_by_id(&tree, "w3").unwrap();
    assert_eq!(found.widget_type.as_deref(), Some("image"));
}

#[test]
fn find_missing_returns_none() {
    assert!(find_by_id(&sample_tree(), "nonexistent").is_none());
}

#[test]
fn remove_root_element() {
    let mut tree = sample_tree();
    let removed = remove_by_id(&mut tree, "root1").unwrap();
    assert_eq!(removed.id, "root1");
    assert_eq!(tree.len(), 1);
}

#[test]
fn remove_nested_element() {
    let mut tree = sample_tree();
    remove_by_id(&mut tree, "w2").unwrap();
    let root1 = find_by_id(&tree, "root1").unwrap();
    assert_eq!(root1.elements.len(), 1);
}

#[test]
fn insert_at_root_position_zero() {
    let mut tree = sample_tree();
    insert_at(&mut tree, None, 0, make_widget("new1", "button"));
    assert_eq!(tree[0].id, "new1");
    assert_eq!(tree.len(), 3);
}

#[test]
fn insert_into_parent_at_start() {
    let mut tree = sample_tree();
    insert_at(&mut tree, Some("root1"), 0, make_widget("new3", "divider"));
    let root1 = find_by_id(&tree, "root1").unwrap();
    assert_eq!(root1.elements[0].id, "new3");
    assert_eq!(root1.elements.len(), 3);
}

#[test]
fn mutate_settings_by_id() {
    let mut tree = sample_tree();
    mutate_by_id(&mut tree, "w1", &|el| {
        el.settings = json!({"title": "Updated"});
    });
    let w1 = find_by_id(&tree, "w1").unwrap();
    assert_eq!(w1.settings["title"], "Updated");
}

#[test]
fn merge_settings_partial_update() {
    let mut base = json!({"title": "Original", "align": "left"});
    merge_settings(&mut base, &json!({"title": "Updated", "header_size": "h3"}));
    assert_eq!(base["title"], "Updated");
    assert_eq!(base["align"], "left");
    assert_eq!(base["header_size"], "h3");
}

#[test]
fn regenerate_ids_all_unique() {
    let mut el = make_container("p", vec![make_widget("c1", "heading"), make_widget("c2", "button")]);
    let old = vec![el.id.clone(), el.elements[0].id.clone(), el.elements[1].id.clone()];
    regenerate_ids(&mut el);
    assert_ne!(el.id, old[0]);
    assert_ne!(el.elements[0].id, old[1]);
    assert_ne!(el.elements[1].id, old[2]);
}

#[test]
fn generate_id_format() {
    let id = generate_id();
    assert_eq!(id.len(), 7);
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn flatten_tree_paths() {
    let flat = flatten_tree(&sample_tree(), "");
    assert_eq!(flat[0].0, "[0]");
    assert_eq!(flat[1].0, "[0][0]");
    assert_eq!(flat[5].0, "[1][0][0]");
    assert_eq!(flat.len(), 6);
}

#[test]
fn search_by_widget_type() {
    let results = search(&sample_tree(), Some("heading"), None, None);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "w1");
}

#[test]
fn parse_serialize_roundtrip() {
    let json = r#"[{"id":"abc1234","elType":"widget","widgetType":"heading","settings":{"title":"Hello"},"elements":[]}]"#;
    let elements = parse_data(json).unwrap();
    let serialized = serialize_data(&elements).unwrap();
    let reparsed = parse_data(&serialized).unwrap();
    assert_eq!(reparsed[0].settings["title"], "Hello");
}
