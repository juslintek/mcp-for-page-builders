use mcp_for_page_builders::{
    elementor::get_page_document,
    tools::{
        Tool,
        elementor_plan_tools::{ApplyElementorPlan, RestoreElementorSnapshot},
        elementor_planning::{self, CssMigrationInput, PlanContext},
    },
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

fn fixture() -> (
    String,
    std::net::SocketAddr,
    Arc<AtomicBool>,
    thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local CSS fixture");
    let address = listener.local_addr().expect("fixture address");
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let worker = thread::spawn(move || {
        loop {
            let (mut stream, _) = listener.accept().expect("fixture accept");
            if thread_stop.load(Ordering::Relaxed) {
                break;
            }
            let mut request = [0; 2048];
            let count = stream.read(&mut request).expect("read fixture request");
            let path = String::from_utf8_lossy(&request[..count]);
            let (status, content_type, body) = if path.starts_with("GET / ") {
                ("200 OK", "text/html", "<link rel=stylesheet href=/site.css><div id=hero><span class=child>Hi</span></div>".to_owned())
            } else if path.starts_with("GET /site.css ") {
                ("200 OK", "text/css", "@import url('/loaded.css'); @import url('/missing.css'); #hero { color: rgb(1, 2, 3); filter: blur(2px); } #hero:hover { color: red; } #hero .child { color: blue; } @media (max-width: 767px) { #hero { color: green; } } @supports (display: grid) { #hero { outline-color: purple; } } @layer theme { #hero { background-color: navy; } }".to_owned())
            } else if path.starts_with("GET /loaded.css ") {
                ("200 OK", "text/css", "body { margin: 0; }".to_owned())
            } else {
                ("404 Not Found", "text/plain", "missing".to_owned())
            };
            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).expect("write fixture response");
        }
    });
    (format!("http://{address}/"), address, stop, worker)
}

fn wp() -> mcp_for_page_builders::wp::WpClient {
    mcp_for_page_builders::wp::WpClient::new(
        &std::env::var("WP_TEST_URL").expect("clone WP_TEST_URL required"),
        &std::env::var("WP_TEST_USER").expect("clone WP_TEST_USER required"),
        &std::env::var("WP_TEST_PASS").expect("clone WP_TEST_PASS required"),
    )
}

#[tokio::test]
async fn browser_css_provenance_plans_executable_and_retained_rules_and_restores_clone_draft() {
    let wp = wp();
    let (url, address, stop, worker) = fixture();
    let source = elementor_planning::parse_css_for_migration(None, Some(&url))
        .await
        .expect("real browser CSSOM parse");
    assert!(source.rules.iter().any(|rule| {
        rule.selector == "#hero"
            && rule
                .source_stylesheet
                .as_deref()
                .is_some_and(|source| source.ends_with("/site.css"))
    }));
    assert!(source.rules.iter().any(|rule| {
        rule.contexts
            .iter()
            .any(|context| context.starts_with("@media:"))
    }));
    assert!(source.rules.iter().any(|rule| {
        rule.contexts
            .iter()
            .any(|context| context.starts_with("@supports:"))
    }));
    assert!(source.rules.iter().any(|rule| {
        rule.contexts
            .iter()
            .any(|context| context.starts_with("@layer:"))
    }));
    assert!(
        source
            .imports
            .iter()
            .any(|href| href.ends_with("/missing.css")),
        "failed import must be reported unresolved"
    );
    assert!(
        !source
            .imports
            .iter()
            .any(|href| href.ends_with("/loaded.css")),
        "loaded imports must not be reported unresolved"
    );

    let created = wp.post("wp/v2/pages", &json!({"title":"CSS migration isolated draft","status":"draft","meta":{"_elementor_data":"[{\"id\":\"csshero1\",\"elType\":\"widget\",\"widgetType\":\"heading\",\"settings\":{\"title\":\"CSS check\"},\"elements\":[]}]","_elementor_edit_mode":"builder"}})).await.expect("create isolated clone draft");
    let page_id = created["id"].as_u64().expect("draft page id");
    let _result = async {
        let before = get_page_document(&wp, page_id).await.expect("read draft document");
        let tree: Vec<Value> = serde_json::from_str(&before.raw).expect("draft tree JSON");
        let schema = wp.get("mcp-for-page-builders/v1/element-schema/heading").await.expect("registered heading schema");
        let allowed = schema["controls"].as_object().expect("heading controls").keys().cloned().collect();
        let operation = format!("css_migration_{}", std::process::id());
        let planned = elementor_planning::plan_css_migration(CssMigrationInput {
            context: PlanContext { site_url: wp.base_url().to_owned(), page_id, expected_revision: before.revision, operation_id: operation.clone() },
            document_tree: tree, css_text: None, source_url: Some(url.clone()), selector: Some("#hero".into()), target_element_id: Some("csshero1".into()), matched_styles: None,
            target: "classic".into(), widget_type: Some("heading".into()), classic_allowed_keys: Some(allowed), atomic_conversion: None, page_settings: None, custom_css_capability: false, breakpoints: Default::default(),
        }).await.expect("checked CSS migration plan");
        let patch = planned.plan.document_patch.as_ref().unwrap_or_else(|| panic!("exact matched selector lacks executable patch; computed color is {}", planned.matched_styles.as_ref().unwrap()["computedStyle"].as_array().unwrap().iter().find(|style| style["name"] == "color").unwrap()["value"]));
        assert!(patch[0]["settings"]["title_color"].is_string());
        let expected_color = patch[0]["settings"]["title_color"].clone();
        assert!(!planned.plan.retained.is_empty(), "pseudo, descendant and contextual rules stay retained");
        assert_eq!(planned.unresolved_imports.len(), 1);
        assert!(planned.plan.source.as_ref().unwrap()["unresolved_rules"].as_array().unwrap().iter().any(|rule| rule["declarations"]["filter"] == "blur(2px)"), "unsupported declaration must remain visible in the proposal");
        let applied = ApplyElementorPlan.run(json!({"plan":planned.plan}), &wp).await.expect("apply checked clone draft plan");
        assert_ne!(applied.is_error, Some(true), "draft plan apply rejected: {}", serde_json::to_string(&applied).unwrap());
        let persisted = get_page_document(&wp, page_id).await.expect("read persisted draft");
        let persisted_tree: Value = serde_json::from_str(&persisted.raw).expect("persisted tree JSON");
        assert_eq!(persisted_tree[0]["settings"]["title_color"], expected_color);
        let restored = RestoreElementorSnapshot.run(json!({"page_id":page_id,"operation_id":operation,"expected_revision":persisted.revision}), &wp).await.expect("restore checked draft snapshot");
        assert_ne!(restored.is_error, Some(true), "draft restore rejected: {}", serde_json::to_string(&restored).unwrap());
        let after = get_page_document(&wp, page_id).await.expect("read restored draft");
        assert_eq!(after.raw, before.raw, "restore must reproduce original draft document");
    }.await;
    let deleted = wp
        .delete(&format!("wp/v2/pages/{page_id}?force=true"))
        .await
        .expect("delete isolated clone draft");
    assert_eq!(deleted["deleted"], true);
    stop.store(true, Ordering::Relaxed);
    std::net::TcpStream::connect(address).expect("wake fixture server");
    worker.join().expect("CSS fixture server thread");
}

#[tokio::test]
async fn known_atomic_breakpoint_builds_native_variant_and_revision_bound_page_css_patch() {
    let wp = wp();
    let created = wp.post("wp/v2/pages", &json!({"title":"Atomic CSS variant planning draft","status":"draft","meta":{"_elementor_data":"[{\"id\":\"csshero1\",\"elType\":\"widget\",\"widgetType\":\"e-heading\",\"settings\":{},\"styles\":[],\"elements\":[]}]","_elementor_edit_mode":"builder"}})).await.expect("create isolated draft");
    let page_id = created["id"].as_u64().expect("isolated draft id");
    let document_before = get_page_document(&wp, page_id)
        .await
        .expect("read isolated draft document");
    let tree: Vec<Value> = serde_json::from_str(&document_before.raw).expect("draft tree JSON");
    let (url, address, stop, worker) = fixture();
    let parsed = elementor_planning::parse_css_for_migration(None, Some(&url))
        .await
        .expect("CSSOM fixture parse");
    let responsive_index = parsed
        .rules
        .iter()
        .position(|rule| {
            rule.selector == "#hero"
                && !rule.contexts.is_empty()
                && rule.contexts[0].starts_with("@media:")
        })
        .expect("fixture media rule");
    let rule = &parsed.rules[responsive_index];
    let converted = wp
        .post(
            "elementor/v1/css-to-atomic",
            &json!({"blocks":{"responsive":rule.declarations}}),
        )
        .await
        .expect("native Atomic CSS conversion");
    let props = converted
        .pointer("/data/responsive/props")
        .filter(|props| props.is_object() && !props.as_object().unwrap().is_empty())
        .expect("native responsive style props")
        .clone();
    let resource = wp
        .get(&format!(
            "mcp-for-page-builders/v1/resources/page_settings/{page_id}"
        ))
        .await
        .expect("read page settings resource revision");
    let page_settings = (
        resource["value"].clone(),
        resource["revision"]
            .as_str()
            .expect("resource revision")
            .to_owned(),
    );
    let computed = elementor_planning::css_matched_styles(&url, "#hero")
        .await
        .expect("matched selector evidence");
    let mut breakpoints = std::collections::HashMap::new();
    breakpoints.insert("@media (max-width: 767px)".to_owned(), "mobile".to_owned());
    let planned = elementor_planning::plan_css_migration(CssMigrationInput {
        context: PlanContext {
            site_url: wp.base_url().to_owned(),
            page_id,
            expected_revision: document_before.revision.clone(),
            operation_id: format!("atomic_css_{}", std::process::id()),
        },
        document_tree: tree,
        css_text: None,
        source_url: Some(url.clone()),
        selector: Some("#hero".into()),
        target_element_id: Some("csshero1".into()),
        matched_styles: Some(computed),
        target: "atomic".into(),
        widget_type: None,
        classic_allowed_keys: None,
        atomic_conversion: Some(
            parsed
                .rules
                .iter()
                .enumerate()
                .map(|(index, _)| {
                    (
                        index,
                        mcp_for_page_builders::tools::css_map::AtomicCssConversion {
                            settings: if index == responsive_index {
                                props.clone()
                            } else {
                                json!({})
                            },
                            custom_css: None,
                            rejected: vec![],
                            capability: "elementor-native-css-converter".into(),
                        },
                    )
                })
                .collect::<HashMap<_, _>>(),
        ),
        page_settings: Some(page_settings.clone()),
        custom_css_capability: true,
        breakpoints,
    })
    .await
    .expect("responsive Atomic migration plan");
    let variants = &planned
        .plan
        .document_patch
        .as_ref()
        .expect("Atomic element patch")[0]["styles"][0]["variants"];
    assert!(
        variants
            .as_array()
            .unwrap()
            .iter()
            .any(|variant| variant["meta"]["breakpoint"] == "mobile" && variant["props"] == props),
        "winning declarations must be written as the matching native breakpoint variant"
    );
    let page_patch = planned
        .plan
        .resource_patches
        .iter()
        .find(|patch| {
            matches!(
                patch.kind,
                mcp_for_page_builders::types::elementor_plan::ResourceKind::PageSettings
            )
        })
        .expect("scoped CSS page settings patch");
    assert_eq!(
        page_patch.expected_revision.as_deref(),
        Some(page_settings.1.as_str())
    );
    assert_eq!(page_patch.before.as_ref(), Some(&page_settings.0));
    assert!(
        page_patch.after["custom_css"]
            .as_str()
            .is_some_and(|css| css.contains("#hero:hover") && css.contains("#hero .child")),
        "unmapped contexts and descendants stay in scoped CSS"
    );
    let operation_id = planned.plan.operation_id.clone();
    let applied = ApplyElementorPlan
        .run(json!({"plan":planned.plan}), &wp)
        .await
        .expect("apply isolated draft CSS plan");
    assert_ne!(
        applied.is_error,
        Some(true),
        "CSS plan apply rejected: {}",
        serde_json::to_string(&applied).unwrap()
    );
    let persisted = get_page_document(&wp, page_id)
        .await
        .expect("read applied draft");
    let persisted_tree: Value = serde_json::from_str(&persisted.raw).expect("persisted draft tree");
    assert_eq!(
        persisted_tree[0]["styles"][0]["variants"][0]["meta"]["breakpoint"],
        "mobile"
    );
    let persisted_settings = wp
        .get(&format!(
            "mcp-for-page-builders/v1/resources/page_settings/{page_id}"
        ))
        .await
        .expect("read persisted page CSS");
    assert!(
        persisted_settings["value"]["custom_css"]
            .as_str()
            .is_some_and(|css| css.contains("#hero:hover"))
    );
    let applied_value = serde_json::to_value(&applied).expect("serialize apply response");
    let reported_revision = applied_value["content"][0]["text"]
        .as_str()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .and_then(|response| {
            response
                .pointer("/resources/resources/0/after_revision")
                .cloned()
        });
    assert_eq!(
        persisted_settings["revision"],
        reported_revision.expect("resource apply after revision")
    );
    let restored = RestoreElementorSnapshot.run(json!({"page_id":page_id,"operation_id":operation_id,"expected_revision":persisted.revision}), &wp).await.expect("restore isolated CSS plan");
    assert_ne!(
        restored.is_error,
        Some(true),
        "CSS restore rejected: {}",
        serde_json::to_string(&restored).unwrap()
    );
    let restored_document = get_page_document(&wp, page_id)
        .await
        .expect("read restored draft");
    assert_eq!(restored_document.raw, document_before.raw);
    let restored_settings = wp
        .get(&format!(
            "mcp-for-page-builders/v1/resources/page_settings/{page_id}"
        ))
        .await
        .expect("read restored page settings");
    assert_eq!(restored_settings["value"], page_settings.0);
    let deleted = wp
        .delete(&format!("wp/v2/pages/{page_id}?force=true"))
        .await
        .expect("delete isolated draft");
    assert_eq!(deleted["deleted"], true);
    stop.store(true, Ordering::Relaxed);
    std::net::TcpStream::connect(address).expect("wake fixture server");
    worker.join().expect("CSS fixture server thread");
}
