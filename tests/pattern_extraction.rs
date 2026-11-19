use anyhow::{Context, Result, ensure};
use mcp_for_page_builders::{
    elementor::{ElementorService, get_page_document},
    tools::{
        Tool,
        elementor_plan_tools::ApplyElementorPlan,
        elementor_planning_tools::ExtractElementorPatterns,
        template::{CreateTemplate, GetTemplate},
    },
    wp::WpClient,
};
use serde_json::{Value, json};
use std::{fs, process::Command, sync::OnceLock};

#[path = "native_test_config.rs"]
mod native_test_config;

fn config() -> &'static native_test_config::NativeTestConfig {
    static CONFIG: OnceLock<native_test_config::NativeTestConfig> = OnceLock::new();
    CONFIG.get_or_init(|| {
        native_test_config::NativeTestConfig::load(false)
            .expect("valid isolated native test configuration")
    })
}

fn wp() -> WpClient {
    let config = config();
    WpClient::new(&config.url, &config.user, &config.password)
}

fn result_json(result: &mcp_for_page_builders::mcp::ToolResult) -> Result<Value> {
    ensure!(
        result.is_error != Some(true),
        "tool returned an error: {}",
        serde_json::to_string(result)?
    );
    let serialized = serde_json::to_value(result)?;
    let text = serialized["content"][0]["text"]
        .as_str()
        .context("tool response has no text content")?;
    Ok(serde_json::from_str(text)?)
}

fn pattern_fixture() -> Value {
    let repeated = |container_id: &str,
                    heading_id: &str,
                    image_id: &str,
                    button_id: &str,
                    suffix: &str| {
        json!({
            "id":container_id,"elType":"container","settings":{"_css_classes":"karjera-card","background_background":"classic","background_color":"#f4f4f4","padding":{"unit":"px","top":"22","right":"24","bottom":"22","left":"24","isLinked":true}},
            "elements":[
                {"id":heading_id,"elType":"widget","widgetType":"heading","settings":{"title":format!("Benefit {suffix}"),"header_size":"h2","_css_classes":"card-title","attributes":{"data-label":"benefit-title"}},"elements":[]},
                {"id":image_id,"elType":"widget","widgetType":"image","settings":{"image":{"id":0,"url":format!("https://example.test/{suffix}.jpg"),"alt":format!("Photo {suffix}")},"image_size":"full","link_to":"custom","link":{"url":format!("https://example.test/{suffix}/"),"is_external":true},"_css_classes":"card-image"},"elements":[]},
                {"id":button_id,"elType":"widget","widgetType":"button","settings":{"text":format!("Open {suffix}"),"link":{"url":format!("https://example.test/{suffix}/apply"),"is_external":true,"nofollow":"yes"},"_css_classes":"card-action","attributes":{"aria-label":format!("Apply for {suffix}")}},"elements":[]},
                {"id":format!("{container_id}5"),"elType":"container","settings":{"_css_classes":"nested-card"},"elements":[{"id":format!("{container_id}6"),"elType":"widget","widgetType":"heading","settings":{"title":"Nested heading","attributes":{"data-scope":"nested"}},"elements":[]}]}
            ]
        })
    };

    json!([
        repeated("1000001","1000002","1000003","1000004","first"),
        repeated("2000001","2000002","2000003","2000004","second"),
        {"id":"7000001","elType":"widget","widgetType":"heading","settings":{"title":"Right first","align":"right"},"elements":[]},
        {"id":"7000002","elType":"widget","widgetType":"heading","settings":{"title":"Right second","align":"right"},"elements":[]},
        {"id":"3000001","elType":"container","settings":{"_css_classes":"unsafe-wrapper"},"elements":[{"id":"3000002","elType":"widget","widgetType":"unsupported-karjera-widget","settings":{"title":"Keep this subtree"},"elements":[]}]},
        {"id":"4000001","elType":"container","settings":{"_css_classes":"unsafe-wrapper"},"elements":[{"id":"4000002","elType":"widget","widgetType":"unsupported-karjera-widget","settings":{"title":"Keep this subtree"},"elements":[]}]},
        {"id":"5000001","elType":"widget","widgetType":"e-heading","settings":{},"styles":[{"meta":{"breakpoint":"desktop","state":"hover"},"props":{"color":{"$$type":"color","value":"#111111"}}}],"elements":[]},
        {"id":"6000001","elType":"widget","widgetType":"e-heading","settings":{},"styles":[{"meta":{"breakpoint":"desktop","state":"active"},"props":{"color":{"$$type":"color","value":"#222222"}}}],"elements":[]}
    ])
}

#[tokio::test]
async fn repeated_native_patterns_roundtrip_as_draft_templates_without_changing_source() {
    let wp = wp();
    let fixture = pattern_fixture();
    let source_ids = collect_ids(&fixture);
    let marker = format!(
        "pattern_extraction_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos()
    );
    let title = format!("MCP {marker}");
    let operation_id = format!("pattern_roundtrip_{marker}");
    eprintln!("Creating exact pattern fixture title marker: {title}");
    let created = wp.post("wp/v2/pages", &json!({
        "title":title,
        "status":"draft",
        "meta":{"_elementor_data":serde_json::to_string(&fixture).expect("encode pattern fixture"),"_elementor_edit_mode":"builder"}
    })).await.expect("create isolated pattern fixture draft");
    let page_id = created["id"]
        .as_u64()
        .expect("WordPress returned draft page id");
    eprintln!("Created isolated pattern extraction draft {page_id}");

    let mut restore_operation = false;
    let mut imported_template_ids = Vec::new();
    let mut imported_operation_ids = Vec::new();
    let mut generated_template_ids = Vec::new();
    let run: Result<()> = async {
        let before = get_page_document(&wp, page_id).await.context("read seeded source document")?;
        let before_tree: Value = serde_json::from_str(&before.raw).context("source document is valid JSON")?;
        ensure!(before_tree == fixture, "WordPress did not persist the exact pattern fixture");

        let extracted = ExtractElementorPatterns.run(json!({"page_id":page_id,"operation_id":operation_id}), &wp)
            .await.context("run the registered pattern extraction tool")?;
        let plan = result_json(&extracted)?;
        ensure!(plan["source"]["kind"] == "pattern_extraction", "tool did not return a pattern extraction plan: {plan}");
        ensure!(plan["source"]["synchronization"] == "portable_template_only", "unverified native Components must not be claimed as synchronized");
        ensure!(plan["source"]["synchronized_components"]["available"] == false, "unverified Component ability was treated as usable");

        let groups = plan["source"]["groups"].as_array().context("pattern groups are an array")?;
        ensure!(groups.len() == 2, "expected the repeated cards and repeated headings to form two distinct groups, got {groups:?}");
        ensure!(plan["source"]["multi_template_creation_available"] == true, "native bridge has not proven logical multi-template creation; this roundtrip remains pending");
        ensure!(plan["resource_patches"].as_array().map(Vec::len) == Some(2), "two repeated groups must produce two portable templates");
        ensure!(plan["resource_patches"][0]["id"] == "new" && plan["resource_patches"][1]["id"] == "new_pattern_2", "logical create aliases must be stable and canonical");
        let group = groups
            .iter()
            .find(|group| group["representative"]["settings"]["_css_classes"] == "karjera-card")
            .context("card pattern group is present")?;
        ensure!(group["instances"].as_array().map(Vec::len) == Some(2), "expected both cards in the group");
        ensure!(group["instances"][1]["bindings"]["elements"][0]["settings"]["title"] == "Benefit second", "nested text content was lost from instance bindings");
        ensure!(group["instances"][1]["bindings"]["elements"][1]["settings"]["image"]["url"] == "https://example.test/second.jpg", "media content was lost from instance bindings");
        ensure!(group["instances"][1]["bindings"]["elements"][1]["settings"]["link"]["url"] == "https://example.test/second/", "image link was lost from instance bindings");
        ensure!(group["instances"][1]["bindings"]["elements"][2]["settings"]["attributes"]["aria-label"] == "Apply for second", "widget attributes were lost from instance bindings");
        ensure!(group["representative"]["settings"]["_css_classes"] == "karjera-card", "representative lost its CSS class reference");
        ensure!(group["representative"]["settings"]["background_color"] == "#f4f4f4", "representative lost its style settings");
        ensure!(group["representative"]["elements"][3]["isInner"] == false, "nested native container omitted its factory default");
        ensure!(group["representative"]["elements"][3]["elements"][0]["settings"]["attributes"]["data-scope"] == "nested", "nested template subtree lost its attributes");
        ensure!(group["representative"]["elements"][1]["settings"]["image"]["url"] == "https://example.test/first.jpg", "representative lost its media settings");
        ensure!(group["representative"]["elements"][1]["settings"]["link"]["url"] == "https://example.test/first/", "representative lost its link settings");
        ensure!(group["representative"]["elements"][2]["settings"]["attributes"]["aria-label"] == "Apply for first", "representative lost its attributes");

        for retained_id in ["3000001", "4000001", "5000001", "6000001"] {
            ensure!(plan["retained"].as_array().context("retained elements are an array")?.iter().any(|entry| entry["element_id"] == retained_id), "element {retained_id} must be retained");
        }
        ensure!(plan["retained"].as_array().unwrap().iter().all(|entry| entry["element_id"] != "3000002" && entry["element_id"] != "4000002"), "unsupported descendant should retain its whole parent rather than being extracted independently");

        let representative_ids = collect_ids(&json!([group["representative"].clone()]));
        ensure!(!representative_ids.is_empty(), "portable representative is empty");
        ensure!(representative_ids.iter().all(|id| id.len() == 7 && id.bytes().all(|byte| byte.is_ascii_hexdigit())), "representative contains an invalid Elementor ID: {representative_ids:?}");
        ensure!(representative_ids.len() == representative_ids.iter().collect::<std::collections::HashSet<_>>().len(), "representative IDs are not unique");
        ensure!(representative_ids.iter().all(|id| !source_ids.contains(id)), "template IDs must be remapped away from source IDs");
        let second_group = groups
            .iter()
            .find(|group| group["representative"]["settings"]["align"] == "right")
            .context("right-aligned heading pattern group is present")?;
        ensure!(second_group["instances"].as_array().map(Vec::len) == Some(2), "second distinct pattern group must retain both instances");
        ensure!(plan["resource_patches"][1]["after"]["document"][0] == second_group["representative"], "second portable template does not match its representative");

        let applied = ApplyElementorPlan.run(json!({"plan":plan}), &wp).await.context("apply revision-bound pattern plan")?;
        let applied = result_json(&applied)?;
        ensure!(applied["applied"] == true, "pattern plan apply failed: {applied}");
        restore_operation = true;
        let resources = applied["resources"]["resources"].as_array().context("apply response omitted created resources")?;
        ensure!(resources.len() == 2, "both logical template creates must be journaled: {resources:?}");
        for (index, group) in groups.iter().enumerate() {
            let logical_id = if index == 0 { "new" } else { "new_pattern_2" };
            let resource = resources.iter().find(|resource| resource["logical_id"] == logical_id).context("apply response omitted logical-to-numeric mapping")?;
            let template_id = resource["id"].as_str().context("created template ID is returned as a string")?.parse::<u64>().context("created template ID is numeric")?;
            generated_template_ids.push(template_id);
            let exported = GetTemplate.run(json!({"id":template_id}), &wp).await.context("export created draft template through get_template")?;
            let exported = result_json(&exported)?;
            ensure!(exported["status"] == "draft", "portable extraction must create a draft template");
            let exported_document: Value = serde_json::from_str(exported["meta"]["_elementor_data"].as_str().context("get_template export omitted Elementor document data")?).context("exported Elementor document is valid JSON")?;
            let representative = &group["representative"];
            ensure!(exported_document[0] == *representative, "exported native template differs from its planned representative");

            let import_title = format!("MCP portable import {marker} {index}");
            eprintln!("Creating exact imported template title marker: {import_title}");
            let import_operation = format!("checked_import_{marker}_{index}");
            let imported = CreateTemplate.run(json!({"title":import_title,"template_type":"section","elementor_data":serde_json::to_string(&exported_document)?,"operation_id":import_operation}), &wp).await?;
            let imported = result_json(&imported)?;
            let imported_id = imported["id"].as_str().context("public create_template omitted legacyID")?.parse::<u64>().context("public create_template legacyID is numeric")?;
            imported_template_ids.push(imported_id);
            imported_operation_ids.push(import_operation);
            let imported_export = GetTemplate.run(json!({"id":imported_id}), &wp).await.context("export imported draft template")?;
            let imported_export = result_json(&imported_export)?;
            ensure!(imported_export["status"] == "draft", "portable import must remain a draft");
            ensure!(imported_export["status"] == "draft", "public create_template import must remain a draft");
            let imported_document: Value = serde_json::from_str(imported_export["meta"]["_elementor_data"].as_str().context("imported template export omitted Elementor document data")?).context("imported Elementor document is valid JSON")?;
            ensure!(imported_document == exported_document, "import/export roundtrip changed the portable template document");
            let native_readback = get_page_document(&wp, imported_id).await?;
            ensure!(serde_json::from_str::<Value>(&native_readback.raw)? == exported_document, "public import native Document API readback differs from its full export");
        }

        let after = get_page_document(&wp, page_id).await.context("verify source document after template creation")?;
        ensure!(after.raw == before.raw && after.revision == before.revision, "pattern extraction changed the source page document");
        Ok(())
    }.await;

    let restore = if restore_operation {
        wp.request(
            "POST",
            "mcp-for-page-builders/v1/resources/restore",
            Some(&json!({"operation_id":operation_id})),
            None,
        )
        .await
        .and_then(|restored| {
            ensure!(
                restored["restored"] == true,
                "template resource cleanup did not restore: {restored}"
            );
            Ok(restored)
        })
    } else {
        Ok(Value::Null)
    };
    let imported_deleted: Result<()> = async {
        for import_operation in &imported_operation_ids {
            let restored = wp
                .request(
                    "POST",
                    "mcp-for-page-builders/v1/resources/restore",
                    Some(&json!({"operation_id":import_operation})),
                    None,
                )
                .await?;
            ensure!(
                restored["restored"] == true,
                "imported template journal cleanup was not confirmed: {restored}"
            );
        }
        Ok(())
    }
    .await;
    let deleted = wp
        .delete(&format!("wp/v2/pages/{page_id}?force=true"))
        .await
        .and_then(|deleted| {
            ensure!(
                deleted["deleted"] == true,
                "pattern fixture draft cleanup was not confirmed: {deleted}"
            );
            Ok(deleted)
        });
    if let Err(error) = restore {
        panic!(
            "failed to restore the created draft template after pattern check: {error:#}; fixture draft ID {page_id}; test failure: {}",
            run.as_ref()
                .err()
                .map(|error| format!("{error:#}"))
                .unwrap_or_else(|| "none".into())
        );
    }
    if let Err(error) = imported_deleted {
        panic!(
            "failed to delete imported template drafts {:?}: {error:#}; fixture draft ID {page_id}; test failure: {}",
            imported_template_ids,
            run.as_ref()
                .err()
                .map(|error| format!("{error:#}"))
                .unwrap_or_else(|| "none".into())
        );
    }
    if let Err(error) = deleted {
        panic!(
            "failed to delete pattern fixture draft {page_id}: {error:#}; test failure: {}",
            run.as_ref()
                .err()
                .map(|error| format!("{error:#}"))
                .unwrap_or_else(|| "none".into())
        );
    }
    let verify_absent = |id: u64| {
        let output = std::process::Command::new("docker")
            .args([
                "exec",
                "-u",
                config().docker_user.as_str(),
                config().container.as_str(),
                "wp",
                config().wp_path_arg.as_str(),
                "post",
                "get",
            ])
            .arg(id.to_string())
            .args(["--field=ID"])
            .output()
            .expect("run clone WP-CLI exact-ID absence check");
        ensure!(
            !output.status.success(),
            "pattern fixture ID {id} still exists in clone WP-CLI"
        );
        Ok::<(), anyhow::Error>(())
    };
    eprintln!(
        "Pattern cleanup IDs: source_page={page_id}, generated_templates={generated_template_ids:?}, imported_templates={imported_template_ids:?}"
    );
    verify_absent(page_id)
        .unwrap_or_else(|error| panic!("source page cleanup verification failed: {error:#}"));
    for id in generated_template_ids {
        verify_absent(id).unwrap_or_else(|error| {
            panic!("generated template cleanup verification failed: {error:#}")
        });
    }
    for id in imported_template_ids {
        verify_absent(id).unwrap_or_else(|error| {
            panic!("imported template cleanup verification failed: {error:#}")
        });
    }
    run.unwrap_or_else(|error| {
        panic!("pattern extraction roundtrip failed for draft {page_id}: {error:#}")
    });
}

#[tokio::test]
async fn single_native_pattern_still_roundtrips_as_a_draft_template() {
    let wp = wp();
    let marker = format!(
        "single_pattern_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos()
    );
    let operation_id = format!("single_pattern_{marker}");
    let fixture = json!([
        {"id":"8100001","elType":"widget","widgetType":"heading","settings":{"title":"First","align":"center"},"elements":[]},
        {"id":"8100002","elType":"widget","widgetType":"heading","settings":{"title":"Second","align":"center"},"elements":[]}
    ]);
    let created = wp
        .post(
            "wp/v2/pages",
            &json!({
                "title":format!("MCP {marker}"),
                "status":"draft",
                "meta":{"_elementor_data":serde_json::to_string(&fixture).expect("encode pattern fixture"),"_elementor_edit_mode":"builder"}
            }),
        )
        .await
        .expect("create isolated single-pattern draft");
    let page_id = created["id"]
        .as_u64()
        .expect("WordPress returned draft page id");
    let mut restore_needed = false;
    let mut template_id = None;
    let mut imported_template_id = None;
    let mut import_operation_id = None;
    let run: Result<()> = async {
        let extracted = ExtractElementorPatterns
            .run(json!({"page_id":page_id,"operation_id":operation_id}), &wp)
            .await?;
        let plan = result_json(&extracted)?;
        ensure!(plan["source"]["groups"].as_array().map(Vec::len) == Some(1), "single repeated pattern must remain extractable");
        ensure!(plan["resource_patches"].as_array().map(Vec::len) == Some(1), "single pattern must keep the original one-template path");
        let applied = ApplyElementorPlan.run(json!({"plan":plan}), &wp).await?;
        let applied = result_json(&applied)?;
        ensure!(applied["applied"] == true, "single-pattern apply failed: {applied}");
        restore_needed = true;
        let id = applied.pointer("/resources/resources/0/id").and_then(Value::as_str).context("single-pattern apply omitted template ID")?.parse::<u64>().context("template ID is numeric")?;
        template_id = Some(id);
        let exported = GetTemplate.run(json!({"id":id}), &wp).await?;
        let exported = result_json(&exported)?;
        ensure!(exported["status"] == "draft", "single extracted template must remain a draft");
        let document: Value = serde_json::from_str(exported["meta"]["_elementor_data"].as_str().context("template export omitted document")?)?;
        let import_operation = format!("single_import_{marker}");
        let imported = CreateTemplate.run(json!({"title":format!("MCP import {marker}"),"template_type":"section","elementor_data":serde_json::to_string(&document)?,"operation_id":import_operation}), &wp).await?;
        let imported = result_json(&imported)?;
        let imported_id = imported["id"].as_str().context("public create_template omitted legacyID")?.parse::<u64>().context("public create_template legacyID is numeric")?;
        imported_template_id = Some(imported_id);
        import_operation_id = Some(import_operation);
        let imported_export = GetTemplate.run(json!({"id":imported_id}), &wp).await?;
        let imported_export = result_json(&imported_export)?;
        ensure!(imported_export["status"] == "draft", "public single-pattern import must remain a draft");
        let imported_document: Value = serde_json::from_str(imported_export["meta"]["_elementor_data"].as_str().context("import export omitted document")?)?;
        ensure!(imported_document == document, "single-pattern import/export changed the template");
        let native_readback = get_page_document(&wp, imported_id).await?;
        ensure!(serde_json::from_str::<Value>(&native_readback.raw)? == document, "single-pattern public import native Document API readback differs from its full export");
        Ok(())
    }.await;
    let mut cleanup_errors = Vec::new();
    let mut restored_template = false;
    if restore_needed {
        match wp
            .request(
                "POST",
                "mcp-for-page-builders/v1/resources/restore",
                Some(&json!({"operation_id":operation_id})),
                None,
            )
            .await
        {
            Ok(restored) if restored["restored"] == true => restored_template = true,
            Ok(restored) => cleanup_errors.push(format!("restore was not confirmed: {restored}")),
            Err(error) => cleanup_errors.push(format!("restore failed: {error:#}")),
        }
    }
    if let Some(import_operation) = import_operation_id {
        match wp
            .request(
                "POST",
                "mcp-for-page-builders/v1/resources/restore",
                Some(&json!({"operation_id":import_operation})),
                None,
            )
            .await
        {
            Ok(restored) if restored["restored"] == true => {}
            Ok(restored) => cleanup_errors.push(format!(
                "imported template journal cleanup was not confirmed: {restored}"
            )),
            Err(error) => cleanup_errors.push(format!(
                "imported template journal cleanup failed: {error:#}"
            )),
        }
    }
    for id in (!restored_template).then_some(template_id).flatten() {
        match wp
            .delete(&format!("wp/v2/elementor_library/{id}?force=true"))
            .await
        {
            Ok(deleted) if deleted["deleted"] == true => {}
            Ok(deleted) => cleanup_errors.push(format!(
                "template {id} deletion was not confirmed: {deleted}"
            )),
            Err(error) => cleanup_errors.push(format!("template {id} deletion failed: {error:#}")),
        }
    }
    match wp
        .delete(&format!("wp/v2/pages/{page_id}?force=true"))
        .await
    {
        Ok(deleted) if deleted["deleted"] == true => {}
        Ok(deleted) => {
            cleanup_errors.push(format!("fixture deletion was not confirmed: {deleted}"))
        }
        Err(error) => cleanup_errors.push(format!("fixture deletion failed: {error:#}")),
    }
    if !cleanup_errors.is_empty() {
        panic!(
            "single-pattern cleanup failed for draft {page_id}: {}; test failure: {}",
            cleanup_errors.join("; "),
            run.as_ref()
                .err()
                .map(|error| format!("{error:#}"))
                .unwrap_or_else(|| "none".into())
        );
    }
    run.unwrap_or_else(|error| {
        panic!("single-pattern native roundtrip failed for draft {page_id}: {error:#}")
    });
}

#[tokio::test]
async fn native_multi_template_creation_roundtrips_and_restores_checked_resources() -> Result<()> {
    let wp = wp();
    let service = ElementorService::new(&wp);
    let capabilities = service
        .capabilities()
        .await
        .expect("read native bridge capabilities");
    let runtime = service
        .runtime()
        .await
        .expect("read active Elementor runtime");
    assert!(
        runtime.is_object(),
        "runtime endpoint must return native runtime data"
    );
    assert_eq!(
        capabilities["logical_template_create_ids"], true,
        "native multi-template creation must be advertised after clone proof"
    );
    eprintln!(
        "Native multi-template capability: {}",
        capabilities["logical_template_create_ids"]
    );
    eprintln!(
        "Native runtime versions: WordPress {}, Elementor {}, Pro {}",
        runtime["wordpress_version"],
        runtime["elementor_core_version"],
        runtime["elementor_pro_version"]
    );

    let marker = format!(
        "native_multi_template_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos()
    );
    let operation_id = format!("native_bootstrap_{marker}");
    let source_document = json!([]);
    let created = wp
        .post(
            "wp/v2/pages",
            &json!({
                "title":format!("MCP {marker}"),
                "status":"draft",
                "meta":{"_elementor_data":serde_json::to_string(&source_document).expect("encode bootstrap document"),"_elementor_edit_mode":"builder"}
            }),
        )
        .await
        .expect("create exact owned bootstrap draft");
    let page_id = created["id"]
        .as_u64()
        .expect("WordPress returned fixture page ID");
    ensure!(
        ![8416, 22393].contains(&page_id),
        "WordPress fixture page ID conflicts with a protected page"
    );
    let before = get_page_document(&wp, page_id)
        .await
        .expect("read current native source document");
    let before_value: Value = serde_json::from_str(&before.raw).expect("source document is JSON");
    ensure!(
        before_value == source_document,
        "source page did not persist its fixture document"
    );

    let first_document = json!([{
        "id":"7100001","elType":"container","isInner":false,"settings":{"_css_classes":"bootstrap-container"},
        "elements":[{"id":"7100002","elType":"widget","widgetType":"heading","settings":{"title":"Bootstrap container heading","header_size":"h2"},"elements":[]}]
    }]);
    let second_document = json!([
        {"id":"7200001","elType":"widget","widgetType":"heading","settings":{"title":"Bootstrap heading one","header_size":"h2"},"elements":[]},
        {"id":"7200002","elType":"widget","widgetType":"heading","settings":{"title":"Bootstrap heading two","header_size":"h3"},"elements":[]}
    ]);
    let patches = json!([
        {"kind":"template","id":"new","expected_revision":"absent","after":{"title":format!("{marker} container"),"status":"draft","type":"section","document":first_document}},
        {"kind":"template","id":"new_pattern_2","expected_revision":"absent","after":{"title":format!("{marker} headings"),"status":"draft","type":"section","document":second_document}}
    ]);
    let plan = json!({
        "site_url":wp.base_url(),"page_id":page_id,"expected_revision":before.revision,
        "operation_id":operation_id,"document_patch":source_document,"resource_patches":patches,
        "source":{"purpose":"test_only_native_multi_create_probe"}
    });

    let mut restore_needed = false;
    let mut created_template_ids = Vec::new();
    let mut imported_template_ids = Vec::new();
    let mut imported_operation_ids = Vec::new();
    let run: Result<()> = async {
        let absent_templates = || async {
            let found = wp
                .get(&format!("wp/v2/elementor_library?search={marker}&context=edit&status=draft&per_page=100"))
                .await?;
            ensure!(found.as_array().is_some_and(Vec::is_empty), "bootstrap marker already has template resources: {found}");
            Ok::<(), anyhow::Error>(())
        };

        for (case, mutate) in [
            ("leading_zero", 0usize),
            ("missing_absent", 1usize),
            ("duplicate_alias", 2usize),
            ("incomplete_batch", 3usize),
        ] {
            let mut invalid = plan.clone();
            match mutate {
                0 => invalid["resource_patches"][1]["id"] = json!("new_pattern_02"),
                1 => {
                    invalid["resource_patches"][1]
                        .as_object_mut()
                        .context("second create patch is an object")?
                        .remove("expected_revision");
                }
                2 => invalid["resource_patches"][1]["id"] = json!("new"),
                3 => invalid["resource_patches"][1]["after"] = json!({"title":format!("{marker} incomplete")}),
                _ => unreachable!(),
            }
            let outcome = ApplyElementorPlan.run(json!({"plan":invalid}), &wp).await;
            match outcome {
                Err(error) => ensure!(
                    error.to_string().contains("no mutation was made")
                        || error.to_string().contains("no mutations were made"),
                    "{case} rejection did not report its no-write boundary: {error:#}"
                ),
                Ok(response) => ensure!(response.is_error == Some(true), "{case} create batch unexpectedly succeeded"),
            }
            let unchanged = get_page_document(&wp, page_id).await?;
            ensure!(unchanged.raw == before.raw && unchanged.revision == before.revision, "{case} changed the source document");
            absent_templates().await?;
        }

        let mut stale = plan.clone();
        stale["expected_revision"] = json!("stale-bootstrap-revision");
        let stale_result = ApplyElementorPlan.run(json!({"plan":stale}), &wp).await?;
        ensure!(stale_result.is_error == Some(true), "stale source revision unexpectedly applied");
        let stale_payload = tool_payload(&stale_result)?;
        ensure!(stale_payload["mutations_made"] == false, "stale revision rejection did not prove a no-write result: {stale_payload}");
        let unchanged = get_page_document(&wp, page_id).await?;
        ensure!(unchanged.raw == before.raw && unchanged.revision == before.revision, "stale revision changed the source document");
        absent_templates().await?;

        restore_needed = true;
        let applied = ApplyElementorPlan.run(json!({"plan":plan}), &wp).await?;
        if applied.is_error == Some(true) {
            let failure = tool_payload(&applied)?;
            if let Some(id) = failure["failed_resource"]["id"].as_str() {
                let observed = wp
                    .get(&format!("mcp-for-page-builders/v1/resources/template/{id}"))
                    .await
                    .context("read native template state after a journaled partial failure")?;
                anyhow::bail!("native apply failed with serialized partial state: {failure}; native readback: {observed}");
            }
            anyhow::bail!("native apply failed before a numeric template could be read back: {failure}");
        }
        let applied = result_json(&applied)?;
        ensure!(applied["applied"] == true, "native multi-template plan failed: {applied}");
        let resources = applied["resources"]["resources"].as_array().context("native receipt omitted resource mappings")?;
        ensure!(resources.len() == 2, "native receipt did not map both create aliases: {resources:?}");
        for (logical_id, expected_document) in [("new", &first_document), ("new_pattern_2", &second_document)] {
            let resource = resources.iter().find(|resource| resource["logical_id"] == logical_id).context("native receipt omitted a canonical alias")?;
            let numeric_id = match resource["id"].as_str() {
                Some(id) => id.parse::<u64>().context("native ID is numeric")?,
                None => resource["id"].as_u64().context("native ID is numeric")?,
            };
            ensure!(numeric_id != page_id && ![8416, 22393].contains(&numeric_id), "native template ID conflicts with a protected fixture or source page");
            ensure!(!created_template_ids.contains(&numeric_id), "native create aliases resolved to the same template ID");
            created_template_ids.push(numeric_id);
            let revision = resource["after_revision"].as_str().context("resource mapping omitted readback fingerprint")?;
            ensure!(revision.len() == 64 && revision.bytes().all(|byte| byte.is_ascii_hexdigit()), "native resource fingerprint is invalid: {revision}");
            ensure!(resource["before_revision"] == "absent", "new template did not use absent resource CAS");
            let resource_state = wp
                .get(&format!("mcp-for-page-builders/v1/resources/template/{numeric_id}"))
                .await?;
            ensure!(resource_state["revision"] == revision, "native resource CAS fingerprint differs from the create receipt");

            let exported = GetTemplate.run(json!({"id":numeric_id}), &wp).await?;
            let exported = result_json(&exported)?;
            ensure!(exported["status"] == "draft", "native template is not a draft");
            let document: Value = serde_json::from_str(exported["meta"]["_elementor_data"].as_str().context("native template export omitted document")?)?;
            ensure!(&document == expected_document, "native template readback differs from its checked plan: {document}");
            ensure!(resource_state["value"]["document"] == document, "native resource API and checked template export disagree");
            let native_document = get_page_document(&wp, numeric_id).await?;
            let native_value: Value = serde_json::from_str(&native_document.raw)?;
            ensure!(native_value == document, "native Document API and template-library readback disagree");

            let import_operation = format!("import_{marker}_{logical_id}");
            let imported = CreateTemplate.run(json!({"title":format!("{marker} imported {logical_id}"),"template_type":"section","elementor_data":serde_json::to_string(&document)?,"operation_id":import_operation}), &wp).await?;
            let imported = result_json(&imported)?;
            let imported_id = imported["id"].as_str().context("public create_template omitted legacyID")?.parse::<u64>().context("public create_template legacyID is numeric")?;
            imported_operation_ids.push(import_operation);
            ensure!(
                imported_id != page_id
                    && !created_template_ids.contains(&imported_id)
                    && !imported_template_ids.contains(&imported_id)
                    && ![8416, 22393].contains(&imported_id),
                "imported template ID conflicts with an owned resource or protected fixture"
            );
            imported_template_ids.push(imported_id);
            let imported_export = GetTemplate.run(json!({"id":imported_id}), &wp).await?;
            let imported_export = result_json(&imported_export)?;
            ensure!(imported_export["status"] == "draft", "public template import must remain a draft");
            let imported_document: Value = serde_json::from_str(imported_export["meta"]["_elementor_data"].as_str().context("import readback omitted native document")?)?;
            ensure!(imported_document == document, "public import/export changed native template content");
            let native_readback = get_page_document(&wp, imported_id).await?;
            ensure!(serde_json::from_str::<Value>(&native_readback.raw)? == document, "public import native Document API readback differs from its full export");
        }

        let replayed = ApplyElementorPlan.run(json!({"plan":plan}), &wp).await?;
        let replayed = result_json(&replayed)?;
        ensure!(replayed["resources"]["idempotent_replay"] == true, "operation replay was not recorded as idempotent: {replayed}");
        for resource in resources {
            ensure!(replayed["resources"]["resources"].as_array().context("replay omitted resources")?.iter().any(|replayed_resource| replayed_resource["logical_id"] == resource["logical_id"] && replayed_resource["id"] == resource["id"]), "operation replay changed a native ID mapping");
        }

        eprintln!("Native multi-template roundtrip: source_page={page_id}, source_revision={}, operation={operation_id}, generated={created_template_ids:?}, imported={imported_template_ids:?}", before.revision);
        let templates = wp
            .get("wp/v2/elementor_library?context=edit&status=draft&per_page=100")
            .await?;
        let owned_count = templates
            .as_array()
            .context("native template listing is an array")?
            .iter()
            .filter(|template| template["title"]["raw"].as_str().is_some_and(|title| title.contains(&marker)))
            .count();
        ensure!(owned_count == 4, "idempotent replay must leave exactly two generated and two public-wrapper imported templates; found {owned_count} among the native template listing");
        let after = get_page_document(&wp, page_id).await?;
        ensure!(after.raw == before.raw && after.revision == before.revision, "successful template creation changed the source page");
        let partial = run_native_second_template_failure(&wp, page_id, &marker).await?;
        eprintln!("Native second-template partial-failure receipt: {partial}");
        ensure!(partial["status"] == "partial_failure", "second native save fault did not produce the bridge's recorded partial outcome: {partial}");
        ensure!(partial["error_code"] == "resource_save_failed", "native after_save exception was not reported as the real native save error: {partial}");
        ensure!(partial["completed"].as_array().map(Vec::len) == Some(2), "partial journal did not record both created templates: {partial}");
        ensure!(partial["restore"]["restored"] == true && partial["remaining_owned"] == json!([]), "revision-aware restore left an operation-owned template behind: {partial}");
        ensure!(partial["source_unchanged"] == true, "partial template creation changed the source page: {partial}");
        Ok(())
    }.await;

    let restore = if restore_needed {
        wp.request(
            "POST",
            "mcp-for-page-builders/v1/resources/restore",
            Some(&json!({"operation_id":operation_id})),
            None,
        )
        .await
        .and_then(|restored| {
            ensure!(
                restored["restored"] == true,
                "native resource-aware restore was not confirmed: {restored}"
            );
            Ok(restored)
        })
    } else {
        Ok(Value::Null)
    };
    let mut cleanup_errors = Vec::new();
    if let Err(error) = restore {
        cleanup_errors.push(format!("native resource restore failed: {error:#}"));
    }
    for import_operation in &imported_operation_ids {
        match wp
            .request(
                "POST",
                "mcp-for-page-builders/v1/resources/restore",
                Some(&json!({"operation_id":import_operation})),
                None,
            )
            .await
        {
            Ok(restored) if restored["restored"] == true => {}
            Ok(restored) => cleanup_errors.push(format!(
                "imported template journal cleanup was not confirmed: {restored}"
            )),
            Err(error) => cleanup_errors.push(format!(
                "imported template journal cleanup failed: {error:#}"
            )),
        }
    }
    match wp
        .get(&format!(
            "wp/v2/elementor_library?search={marker}&context=edit&status=draft&per_page=100"
        ))
        .await
    {
        Ok(found) if found.as_array().is_some_and(Vec::is_empty) => {}
        Ok(found) => cleanup_errors.push(format!(
            "owned bootstrap templates remain after cleanup: {found}"
        )),
        Err(error) => cleanup_errors.push(format!(
            "owned bootstrap template absence check failed: {error:#}"
        )),
    }
    let after_cleanup = get_page_document(&wp, page_id).await;
    match after_cleanup {
        Ok(document) if document.raw == before.raw && document.revision == before.revision => {}
        Ok(document) => cleanup_errors.push(format!(
            "source document differs after cleanup (revision {})",
            document.revision
        )),
        Err(error) => cleanup_errors.push(format!(
            "source document verification after cleanup failed: {error:#}"
        )),
    }
    match wp
        .delete(&format!("wp/v2/pages/{page_id}?force=true"))
        .await
    {
        Ok(deleted) if deleted["deleted"] == true => {}
        Ok(deleted) => cleanup_errors.push(format!(
            "source fixture {page_id} cleanup was not confirmed: {deleted}"
        )),
        Err(error) => cleanup_errors.push(format!(
            "source fixture {page_id} cleanup failed: {error:#}"
        )),
    }
    if !cleanup_errors.is_empty() {
        panic!(
            "native multi-template bootstrap cleanup failed for owned source page {page_id}, templates {created_template_ids:?}, imports {imported_template_ids:?}: {}; test result: {}",
            cleanup_errors.join("; "),
            run.as_ref()
                .err()
                .map(|error| format!("{error:#}"))
                .unwrap_or_else(|| "none".into())
        );
    }
    run.unwrap_or_else(|error| {
        panic!("native multi-template bootstrap failed for owned source page {page_id}: {error:#}")
    });
    Ok(())
}

async fn run_native_second_template_failure(
    wp: &WpClient,
    source_page_id: u64,
    marker: &str,
) -> Result<Value> {
    use std::os::unix::fs::PermissionsExt;

    let user = wp.get("wp/v2/users/me?context=edit");
    let operation_id = format!("native_partial_{marker}");
    let fail_title = format!("{marker} fail after native save");
    let first_title = format!("{marker} first native resource");
    let first_document = json!([{"id":"7300001","elType":"widget","widgetType":"heading","settings":{"title":"First native resource"},"elements":[]}]);
    let second_document = json!([{"id":"7400001","elType":"widget","widgetType":"heading","settings":{"title":"Second native resource"},"elements":[]}]);
    let user_id = user
        .await?
        .get("id")
        .and_then(Value::as_u64)
        .context("current clone user ID is numeric")?;
    let mut script = String::from(
        r##"<?php
$user_id = __USER_ID__;
$source_page_id = __SOURCE_PAGE_ID__;
$operation_id = __OPERATION_ID__;
$fail_title = __FAIL_TITLE__;
$first_title = __FIRST_TITLE__;
$first_document = json_decode(__FIRST_DOCUMENT__, true, 512, JSON_THROW_ON_ERROR);
$second_document = json_decode(__SECOND_DOCUMENT__, true, 512, JSON_THROW_ON_ERROR);
wp_set_current_user($user_id);
$before_response = rest_do_request(new WP_REST_Request('GET', '/mcp-for-page-builders/v1/document/' . $source_page_id));
if ($before_response->get_status() !== 200) throw new RuntimeException('could not snapshot the source page before the native failure fixture');
$before_document = $before_response->get_data();
$seen = [];
$failure_hook = static function($document) use (&$seen, $fail_title) {
    if (!is_object($document) || !is_callable([$document, 'get_main_id'])) return;
    $post = get_post((int) $document->get_main_id());
    if ($post && $post->post_title === $fail_title) {
        $seen[] = (int) $post->ID;
        throw new RuntimeException('native multi-template second-save fixture');
    }
};
$apply_data = null;
$hook_error = null;
add_action('elementor/document/after_save', $failure_hook, PHP_INT_MAX, 2);
try {
    $request = new WP_REST_Request('POST', '/mcp-for-page-builders/v1/resources/apply');
    $request->set_header('Content-Type', 'application/json');
    $request->set_body(wp_json_encode([
        'operation_id' => $operation_id,
        'page_id' => 0,
        'resource_patches' => [
            ['kind' => 'template', 'id' => 'new', 'expected_revision' => 'absent', 'after' => ['title' => $first_title, 'status' => 'draft', 'type' => 'section', 'document' => $first_document]],
            ['kind' => 'template', 'id' => 'new_pattern_2', 'expected_revision' => 'absent', 'after' => ['title' => $fail_title, 'status' => 'draft', 'type' => 'section', 'document' => $second_document]],
        ],
    ]));
    $apply_response = rest_do_request($request);
    $apply_data = $apply_response instanceof WP_REST_Response ? $apply_response->get_data() : $apply_response->get_error_data();
    $hook_error = $apply_response instanceof WP_Error ? $apply_response->get_error_message() : null;
} finally {
    remove_action('elementor/document/after_save', $failure_hook, PHP_INT_MAX);
}
$completed = is_array($apply_data) ? ($apply_data['completed'] ?? []) : [];
$owned_ids = [];
foreach ($completed as $resource) if (isset($resource['id']) && ctype_digit((string) $resource['id'])) $owned_ids[] = (int) $resource['id'];
$failed_id = is_array($apply_data) && isset($apply_data['failed_resource']['id']) && ctype_digit((string) $apply_data['failed_resource']['id']) ? (int) $apply_data['failed_resource']['id'] : null;
if ($failed_id !== null && !in_array($failed_id, $owned_ids, true)) $owned_ids[] = $failed_id;
$restore_request = new WP_REST_Request('POST', '/mcp-for-page-builders/v1/resources/restore');
$restore_request->set_param('operation_id', $operation_id);
$restore_request->set_body(wp_json_encode(['operation_id' => $operation_id]));
$restore_response = rest_do_request($restore_request);
$restore_data = $restore_response instanceof WP_REST_Response ? $restore_response->get_data() : $restore_response->get_error_data();
$remaining = [];
foreach ($owned_ids as $id) if (get_post($id)) $remaining[] = $id;
$after_response = rest_do_request(new WP_REST_Request('GET', '/mcp-for-page-builders/v1/document/' . $source_page_id));
$after_document = $after_response instanceof WP_REST_Response ? $after_response->get_data() : null;
$source_unchanged = $after_response->get_status() === 200 && ($before_document['revision'] ?? null) === ($after_document['revision'] ?? null) && ($before_document['document'] ?? null) === ($after_document['document'] ?? null);
if (($apply_data['status'] ?? null) !== 'partial_failure') throw new RuntimeException('native failure did not return a recorded partial_failure: ' . wp_json_encode($apply_data));
if (count($seen) !== 1 || $seen[0] !== $failed_id) throw new RuntimeException('after_save fault did not fire only for the exact second template: ' . wp_json_encode($seen));
if (($apply_data['error_code'] ?? null) !== 'resource_save_failed' || count($completed) !== 2 || ($completed[0]['logical_id'] ?? null) !== 'new' || ($completed[1]['logical_id'] ?? null) !== 'new_pattern_2' || count($owned_ids) !== 2 || $failed_id !== $owned_ids[1]) throw new RuntimeException('partial journal did not record the native second-save error and both exact logical template IDs: ' . wp_json_encode($apply_data));
if (empty($restore_data['restored']) || $remaining) throw new RuntimeException('resource-aware restore did not remove exactly the operation-owned partial templates: ' . wp_json_encode(['restore' => $restore_data, 'remaining' => $remaining]));
if (!$source_unchanged) throw new RuntimeException('partial resource creation changed the source page document or revision');
echo 'MCP_NATIVE_PARTIAL_RESULT:' . wp_json_encode(['status' => $apply_data['status'], 'error_code' => $apply_data['error_code'] ?? null, 'hook_seen' => $seen, 'completed' => $completed, 'failed_resource' => $apply_data['failed_resource'] ?? null, 'restore' => $restore_data, 'remaining_owned' => $remaining, 'source_unchanged' => $source_unchanged]) . "\n";
"##,
    );
    for (placeholder, value) in [
        ("__USER_ID__", user_id.to_string()),
        ("__SOURCE_PAGE_ID__", source_page_id.to_string()),
        ("__OPERATION_ID__", serde_json::to_string(&operation_id)?),
        ("__FAIL_TITLE__", serde_json::to_string(&fail_title)?),
        ("__FIRST_TITLE__", serde_json::to_string(&first_title)?),
        (
            "__FIRST_DOCUMENT__",
            serde_json::to_string(&serde_json::to_string(&first_document)?)?,
        ),
        (
            "__SECOND_DOCUMENT__",
            serde_json::to_string(&serde_json::to_string(&second_document)?)?,
        ),
    ] {
        script = script.replace(placeholder, &value);
    }
    let fixture_name = format!("mcp-native-template-failure-{}.php", std::process::id());
    let local_path = std::env::temp_dir().join(&fixture_name);
    fs::write(&local_path, script).context("write temporary native save-failure fixture")?;
    fs::set_permissions(&local_path, fs::Permissions::from_mode(0o600))?;
    let syntax = Command::new("php")
        .arg("-l")
        .arg(&local_path)
        .output()
        .context("lint native save-failure fixture")?;
    if !syntax.status.success() {
        let _ = fs::remove_file(&local_path);
        anyhow::bail!(
            "native save-failure PHP fixture has invalid syntax: {}",
            String::from_utf8_lossy(&syntax.stderr)
        );
    }
    let container = config().container.as_str();
    let container_path = format!("/tmp/{fixture_name}");
    let copied = Command::new("docker")
        .args(["cp"])
        .arg(&local_path)
        .arg(format!("{container}:{container_path}"))
        .output()
        .context("copy native failure fixture into the isolated clone")?;
    if !copied.status.success() {
        let _ = fs::remove_file(&local_path);
        anyhow::bail!(
            "copy native failure fixture failed: {}",
            String::from_utf8_lossy(&copied.stderr)
        );
    }
    let output = Command::new("docker")
        .args([
            "exec",
            "-u",
            config().docker_user.as_str(),
            container,
            "wp",
            config().wp_path_arg.as_str(),
            "--quiet",
            "eval-file",
        ])
        .arg(&container_path)
        .output();
    let cleanup = Command::new("docker")
        .args(["exec", container, "rm", "-f", &container_path])
        .output()
        .context("remove temporary native save-failure fixture from the clone")?;
    let _ = fs::remove_file(local_path);
    ensure!(
        cleanup.status.success(),
        "could not remove native save-failure fixture from clone: {}",
        String::from_utf8_lossy(&cleanup.stderr)
    );
    let output = output.context("run native save-failure fixture through clone WP-CLI")?;
    ensure!(
        output.status.success(),
        "native second-template failure fixture failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let output_text = String::from_utf8_lossy(&output.stdout);
    let result_line = output_text
        .lines()
        .find(|line| line.starts_with("MCP_NATIVE_PARTIAL_RESULT:"))
        .context("native failure fixture returned no structured evidence")?;
    Ok(serde_json::from_str(
        result_line.trim_start_matches("MCP_NATIVE_PARTIAL_RESULT:"),
    )?)
}

fn tool_payload(result: &mcp_for_page_builders::mcp::ToolResult) -> Result<Value> {
    let serialized = serde_json::to_value(result)?;
    let text = serialized["content"][0]["text"]
        .as_str()
        .context("tool response has no text content")?;
    Ok(serde_json::from_str(text)?)
}

fn collect_ids(document: &Value) -> std::collections::HashSet<String> {
    fn visit(elements: &[Value], ids: &mut std::collections::HashSet<String>) {
        for element in elements {
            if let Some(id) = element["id"].as_str() {
                ids.insert(id.to_owned());
            }
            if let Some(children) = element["elements"].as_array() {
                visit(children, ids);
            }
        }
    }
    let mut ids = std::collections::HashSet::new();
    if let Some(elements) = document.as_array() {
        visit(elements, &mut ids);
    }
    ids
}
