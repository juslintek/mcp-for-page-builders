use anyhow::{Context, Result, ensure};
use mcp_for_page_builders::{
    tools::{
        Tool,
        option::SetWpOption,
        page::CreatePage,
        post::{GetPost, UpdatePost},
        template::{CreateTemplate, GetTemplate},
        wp_api::WpApi,
    },
    wp::WpClient,
};
use serde_json::{Value, json};
use std::process::{Command, Output};

const CLONE_URL: &str = "https://karjera-recovery.ddev.site";
const CLONE_CONTAINER: &str = "ddev-karjera-recovery-web";

fn wp() -> WpClient {
    let url =
        std::env::var("WP_TEST_URL").expect("WP_TEST_URL is required for mutation boundary checks");
    assert_eq!(
        url.trim_end_matches('/'),
        CLONE_URL,
        "mutation boundary fixtures may only touch the recovery clone"
    );
    WpClient::new(
        &url,
        &std::env::var("WP_TEST_USER").expect("WP_TEST_USER is required"),
        &std::env::var("WP_TEST_PASS").expect("WP_TEST_PASS is required"),
    )
}

fn wp_cli(args: &[String]) -> Result<Output> {
    Command::new("docker")
        .args([
            "exec",
            "-u",
            "linasjusys",
            CLONE_CONTAINER,
            "wp",
            "--path=/var/www/html/web/wp",
        ])
        .args(args)
        .output()
        .context("run recovery clone WP-CLI")
}

fn verify_absent(id: u64) -> Result<()> {
    let output = wp_cli(&[
        "eval".into(),
        format!("echo get_post({id}) ? 'present' : 'absent';"),
    ])?;
    ensure!(
        output.status.success(),
        "WP-CLI absence check failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    ensure!(
        String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .ends_with("absent"),
        "fixture post {id} still exists"
    );
    Ok(())
}

fn option_state(name: &str) -> Result<String> {
    let output = wp_cli(&[
        "eval".into(),
        format!(
            "global $wpdb; $v=$wpdb->get_var($wpdb->prepare('SELECT option_value FROM %i WHERE option_name = %s', $wpdb->options, '{name}')); echo $v === null ? 'absent' : hash('sha256', $v);"
        ),
    ])?;
    ensure!(
        output.status.success(),
        "WP-CLI option-state check failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn delete_option_fixture(name: &str) -> Result<()> {
    let output = wp_cli(&["eval".into(), format!("delete_option('{name}');")])?;
    ensure!(
        output.status.success(),
        "WP-CLI could not delete exact option fixture {name}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    ensure!(
        option_state(name)? == "absent",
        "option fixture {name} still exists after cleanup"
    );
    Ok(())
}

struct Fixtures {
    ids: Vec<u64>,
    cleaned: bool,
}

impl Fixtures {
    fn new() -> Self {
        Self {
            ids: Vec::new(),
            cleaned: false,
        }
    }

    async fn create(&mut self, wp: &WpClient, endpoint: &str, title: &str) -> Result<u64> {
        let created = wp
            .post(
                endpoint,
                &json!({"title":title,"status":"draft","content":"before"}),
            )
            .await?;
        let id = created["id"]
            .as_u64()
            .context("WordPress returned a fixture ID")?;
        self.ids.push(id);
        Ok(id)
    }

    fn finish(&mut self) -> Result<()> {
        for id in self.ids.iter().rev().copied() {
            let deleted = wp_cli(&[
                "post".into(),
                "delete".into(),
                id.to_string(),
                "--force".into(),
            ])?;
            ensure!(
                deleted.status.success(),
                "WP-CLI failed to delete fixture {id}: {}",
                String::from_utf8_lossy(&deleted.stderr)
            );
            verify_absent(id)?;
        }
        self.cleaned = true;
        Ok(())
    }
}

impl Drop for Fixtures {
    fn drop(&mut self) {
        if self.cleaned {
            return;
        }
        for id in self.ids.iter().rev().copied() {
            let cleanup = wp_cli(&[
                "post".into(),
                "delete".into(),
                id.to_string(),
                "--force".into(),
            ])
            .and_then(|deleted| {
                ensure!(
                    deleted.status.success(),
                    "WP-CLI failed to delete fixture {id}: {}",
                    String::from_utf8_lossy(&deleted.stderr)
                );
                verify_absent(id)
            });
            if let Err(error) = cleanup {
                eprintln!("fixture cleanup failed for exact ID {id}: {error:#}");
            }
        }
    }
}

fn set_elementor_data(id: u64) -> Result<()> {
    for (key, value) in [
        ("_elementor_data", "[]"),
        ("_elementor_edit_mode", "builder"),
    ] {
        let output = wp_cli(&[
            "post".into(),
            "meta".into(),
            "update".into(),
            id.to_string(),
            key.into(),
            value.into(),
        ])?;
        ensure!(
            output.status.success(),
            "WP-CLI failed to set {key} on exact fixture {id}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn tool_text(value: &mcp_for_page_builders::mcp::ToolResult) -> Result<String> {
    let serialized = serde_json::to_value(value)?;
    Ok(serialized["content"][0]["text"]
        .as_str()
        .context("tool response has no text")?
        .to_owned())
}

#[tokio::test]
async fn wp_api_blocks_all_elementor_library_writes() -> Result<()> {
    let wp = wp();
    let mut fixtures = Fixtures::new();
    let marker = format!(
        "Mutation library {}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );

    for (title, protected_meta) in [("without metadata", false), ("with metadata", true)] {
        let output = wp_cli(&[
            "post".into(),
            "create".into(),
            "--post_type=elementor_library".into(),
            "--post_status=draft".into(),
            format!("--post_title={marker} {title}"),
            "--porcelain".into(),
        ])?;
        ensure!(
            output.status.success(),
            "WP-CLI could not create the exact Elementor library fixture: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let id: u64 = String::from_utf8(output.stdout)
            .context("decode Elementor library fixture ID")?
            .trim()
            .parse()
            .context("parse Elementor library fixture ID")?;
        fixtures.ids.push(id);

        if protected_meta {
            let output = wp_cli(&[
                "post".into(),
                "meta".into(),
                "update".into(),
                id.to_string(),
                "_elementor_data".into(),
                "[]".into(),
            ])?;
            ensure!(
                output.status.success(),
                "WP-CLI could not set protected metadata on exact fixture {id}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let metadata = wp
            .get(&format!(
                "mcp-for-page-builders/v1/resources/post_metadata/{id}"
            ))
            .await?;
        ensure!(
            metadata["value"]["protected_elementor"] == protected_meta,
            "library fixture metadata protection did not match its setup"
        );

        for method in ["POST", "PUT", "PATCH"] {
            for body in [
                json!({"title":"blocked library write"}),
                json!({"meta":{"_elementor_custom_setting":"blocked"}}),
            ] {
                let response = WpApi
                    .run(
                        json!({"method":method,"endpoint":format!("wp/v2/elementor_library/{id}"),"body":body}),
                        &wp,
                    )
                    .await;
                let error = match response {
                    Err(error) => error,
                    Ok(_) => anyhow::bail!(
                        "{method} Elementor library write unexpectedly passed the guard"
                    ),
                };
                ensure!(
                    error
                        .to_string()
                        .contains("Direct Elementor template writes are blocked"),
                    "{method} library guard returned an unexpected error instead of its guard: {error:#}"
                );
            }
        }
    }

    fixtures.finish()?;
    Ok(())
}

#[tokio::test]
async fn native_page_and_template_creation_receipts_replay_and_restore() -> Result<()> {
    let wp = wp();
    let marker = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let page_operation = format!("page_create_{marker}");
    let template_operation = format!("template_create_{marker}");
    let document = json!([{"id":"receiptcheck1","elType":"widget","widgetType":"heading","settings":{"title":"receipt check"},"elements":[]}]);
    let mut created_ids = Vec::new();
    let result = async {
        for (tool, args, kind, operation_id) in [
            (&CreatePage as &dyn Tool, json!({"title":format!("Native page {marker}"),"elementor_data":document.to_string(),"operation_id":page_operation}), "page", page_operation.as_str()),
            (&CreateTemplate as &dyn Tool, json!({"title":format!("Native template {marker}"),"template_type":"section","elementor_data":document.to_string(),"operation_id":template_operation}), "template", template_operation.as_str()),
        ] {
            let first = tool.run(args.clone(), &wp).await?;
            ensure!(first.is_error != Some(true), "native {kind} creation returned an error: {}", tool_text(&first)?);
            let receipt: Value = serde_json::from_str(&tool_text(&first)?)?;
            let id = receipt["id"].as_str().context("creation wrapper omitted id")?.to_owned();
            println!("native {kind} creation fixture ID: {id}");
            created_ids.push((operation_id.to_owned(), kind, id.clone()));
            let revision = receipt["resource_revision"].as_str().context("creation wrapper omitted resource_revision")?;
            ensure!(receipt["status"] == "created" && receipt["operation_id"] == operation_id && receipt["idempotent_replay"] == false && revision.len() == 64 && revision.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()), "creation wrapper returned an invalid native receipt");
            let state = wp.get(&format!("mcp-for-page-builders/v1/resources/{kind}/{id}")).await?;
            ensure!(state["revision"] == revision && state["value"]["document"] == document, "native {kind}/{id} readback or document revision differs from the receipt");
            let replay = tool.run(args, &wp).await?;
            let replay: Value = serde_json::from_str(&tool_text(&replay)?)?;
            ensure!(replay["id"] == id && replay["idempotent_replay"] == true, "stable {kind} operation replay changed its ID or created another resource");
            if kind == "page" {
                let tree = mcp_for_page_builders::elementor::get_page_document(&wp, id.parse()?).await?;
                ensure!(serde_json::from_str::<Value>(&tree.raw)? == document && tree.revision.len() == 64, "page document tree was not persisted or has no native document revision");
            } else {
                let template = GetTemplate.run(json!({"id":id.parse::<u64>()?}), &wp).await?;
                ensure!(tool_text(&template)?.contains("receipt check"), "native template document was not persisted");
            }
        }
        Ok::<(), anyhow::Error>(())
    }.await;
    let mut cleanup_errors = Vec::new();
    for operation_id in [&page_operation, &template_operation] {
        let restored = match wp
            .post(
                "mcp-for-page-builders/v1/resources/restore",
                &json!({"operation_id":operation_id}),
            )
            .await
        {
            Ok(receipt) if receipt["restored"] == true => true,
            Ok(receipt) => {
                cleanup_errors.push(format!(
                    "restore {operation_id} was not confirmed: {receipt}"
                ));
                false
            }
            Err(error) => {
                cleanup_errors.push(format!("restore {operation_id}: {error:#}"));
                false
            }
        };
        if !restored {
            continue;
        }
        for (_, kind, id) in created_ids
            .iter()
            .filter(|(operation, _, _)| operation == operation_id)
        {
            if wp
                .get(&format!("mcp-for-page-builders/v1/resources/{kind}/{id}"))
                .await
                .is_ok()
            {
                cleanup_errors.push(format!(
                    "native resource {kind}/{id} still exists after restore"
                ));
            }
            if let Err(error) = verify_absent(id.parse()?) {
                cleanup_errors.push(format!("created post {id} was not removed: {error:#}"));
            }
        }
        let delete_receipt = wp_cli(&[
            "eval".into(),
            format!(
                "delete_option('_mcp_elementor_resources_' . hash('sha256', '{operation_id}')); echo get_option('_mcp_elementor_resources_' . hash('sha256', '{operation_id}')) === false ? 'absent' : 'present';"
            ),
        ]);
        match delete_receipt {
            Ok(output)
                if output.status.success()
                    && String::from_utf8_lossy(&output.stdout)
                        .trim_end()
                        .ends_with("absent") => {}
            Ok(output) => cleanup_errors.push(format!(
                "receipt cleanup {operation_id}: {}",
                String::from_utf8_lossy(&output.stderr)
            )),
            Err(error) => cleanup_errors.push(format!("receipt cleanup {operation_id}: {error:#}")),
        }
    }
    result?;
    ensure!(
        cleanup_errors.is_empty(),
        "native creation cleanup failed: {}",
        cleanup_errors.join("; ")
    );
    Ok(())
}

#[tokio::test]
async fn generic_and_post_mutations_respect_elementor_boundaries() -> Result<()> {
    let wp = wp();
    let marker = format!(
        "Mutation boundary {}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let mut fixtures = Fixtures::new();
    let ordinary_id = fixtures
        .create(&wp, "wp/v2/posts", &format!("{marker} ordinary"))
        .await?;
    let native_id = fixtures
        .create(&wp, "wp/v2/posts", &format!("{marker} native"))
        .await?;
    let page_id = fixtures
        .create(&wp, "wp/v2/pages", &format!("{marker} page"))
        .await?;
    set_elementor_data(native_id)?;

    let ordinary = UpdatePost.run(json!({"id":ordinary_id,"title":"Ordinary title allowed","content":"Ordinary content allowed"}), &wp).await?;
    ensure!(
        ordinary.is_error.is_none(),
        "ordinary post title/content update should be allowed: {}",
        tool_text(&ordinary)?
    );
    let ordinary_after = wp
        .get(&format!("wp/v2/posts/{ordinary_id}?context=edit"))
        .await?;
    ensure!(
        ordinary_after["title"]["raw"] == "Ordinary title allowed",
        "ordinary post title did not update"
    );
    ensure!(
        ordinary_after["content"]["raw"] == "Ordinary content allowed",
        "ordinary post content did not update"
    );

    let native_before = wp
        .get(&format!("wp/v2/posts/{native_id}?context=edit"))
        .await?;
    let content_rejected = UpdatePost
        .run(
            json!({"id":native_id,"content":"must remain unchanged"}),
            &wp,
        )
        .await;
    ensure!(
        content_rejected.is_err_and(|error| error
            .to_string()
            .contains("Content updates are unsupported")),
        "native content update must be rejected before write"
    );
    let content_after = wp
        .get(&format!("wp/v2/posts/{native_id}?context=edit"))
        .await?;
    ensure!(
        content_after["content"]["raw"] == native_before["content"]["raw"],
        "blocked native content update changed the post"
    );

    let metadata = wp
        .get(&format!(
            "mcp-for-page-builders/v1/resources/post_metadata/{native_id}"
        ))
        .await?;
    ensure!(
        metadata["value"]["protected_elementor"] == true,
        "fixture was not detected as Elementor-backed"
    );
    let metadata_revision = metadata["revision"]
        .as_str()
        .context("native fixture metadata revision")?;
    let native_post = GetPost.run(json!({"id":native_id}), &wp).await?;
    let native_post: Value = serde_json::from_str(&tool_text(&native_post)?)
        .context("decode GetPost native metadata flags")?;
    ensure!(
        native_post["metadata_revision"] == metadata_revision
            && native_post["protected_elementor"] == true,
        "GetPost omitted or changed the native metadata revision/protection flags"
    );
    let operation_id = format!("native_title_{}", std::process::id());
    let title_update = UpdatePost.run(json!({"id":native_id,"title":"Elementor title through plan","expected_metadata_revision":metadata_revision,"operation_id":operation_id}), &wp).await?;
    ensure!(
        title_update.is_error.is_none(),
        "revision-checked Elementor title update failed: {}",
        tool_text(&title_update)?
    );
    let after_title = wp
        .get(&format!(
            "mcp-for-page-builders/v1/resources/post_metadata/{native_id}"
        ))
        .await?;
    ensure!(
        after_title["value"]["title"] == "Elementor title through plan",
        "checked native title update was not persisted"
    );
    let stale = UpdatePost.run(json!({"id":native_id,"title":"Stale title must not persist","expected_metadata_revision":metadata_revision,"operation_id":format!("native_stale_{}", std::process::id())}), &wp).await;
    let stale_text = match stale {
        Err(error) => error.to_string(),
        Ok(_) => anyhow::bail!("stale metadata revision unexpectedly succeeded"),
    };
    ensure!(
        stale_text.contains("409") || stale_text.contains("stale_resource"),
        "stale rejection did not report conflict: {stale_text}"
    );
    let after_stale = wp
        .get(&format!(
            "mcp-for-page-builders/v1/resources/post_metadata/{native_id}"
        ))
        .await?;
    ensure!(
        after_stale["revision"] == after_title["revision"]
            && after_stale["value"]["title"] == after_title["value"]["title"],
        "stale metadata write changed the resource"
    );

    let page_before = wp
        .get(&format!("wp/v2/pages/{page_id}?context=edit"))
        .await?;
    let wrong_type = UpdatePost
        .run(json!({"id":page_id,"title":"Must remain a page"}), &wp)
        .await;
    ensure!(
        wrong_type.is_err_and(|error| error
            .to_string()
            .contains("only accepts WordPress post resources")),
        "UpdatePost must reject a page ID before write"
    );
    let page_after = wp
        .get(&format!("wp/v2/pages/{page_id}?context=edit"))
        .await?;
    ensure!(
        page_after["title"]["raw"] == page_before["title"]["raw"],
        "UpdatePost changed a page resource"
    );

    for method in ["POST", "PUT", "PATCH"] {
        let plain = WpApi.run(json!({"method":method,"endpoint":format!("wp/v2/posts/{native_id}"),"body":{"title":"generic write must be blocked"}}), &wp).await;
        ensure!(
            plain.is_err(),
            "wp_api {method} without meta must be blocked for an Elementor-backed post"
        );
        let protected = WpApi.run(json!({"method":method,"endpoint":format!("wp/v2/posts/{native_id}"),"body":{"title":"generic write must be blocked","meta":{"_elementor_custom_setting":"blocked"}}}), &wp).await;
        ensure!(
            protected.is_err(),
            "wp_api {method} with protected metadata must be blocked"
        );
    }
    let generic_create = WpApi.run(json!({"method":"POST","endpoint":"wp/v2/posts","body":{"title":format!("{marker} blocked generic create"),"status":"draft","meta":{"_elementor_custom_setting":"blocked"}}}), &wp).await;
    ensure!(
        generic_create.is_err(),
        "generic post creation with an _elementor_ key must be blocked"
    );
    let blocked_title = format!("{marker} blocked generic create");
    let search = wp_cli(&[
        "post".into(),
        "list".into(),
        "--post_type=post".into(),
        format!("--s={blocked_title}"),
        "--fields=ID,post_title".into(),
        "--format=json".into(),
    ])?;
    ensure!(
        search.status.success(),
        "WP-CLI blocked-create lookup failed: {}",
        String::from_utf8_lossy(&search.stderr)
    );
    let matching_posts: Value =
        serde_json::from_slice(&search.stdout).context("decode WP-CLI blocked-create lookup")?;
    ensure!(
        matching_posts
            .as_array()
            .is_some_and(|posts| posts.iter().all(|post| post["post_title"] != blocked_title)),
        "blocked generic create left a post behind"
    );
    let generic_after = wp
        .get(&format!("wp/v2/posts/{native_id}?context=edit"))
        .await?;
    ensure!(
        generic_after["title"]["raw"] == "Elementor title through plan"
            && generic_after["content"]["raw"] == native_before["content"]["raw"],
        "blocked wp_api write changed the native post"
    );

    let missing_id = 987_654_321_u64;
    verify_absent(missing_id)?;
    ensure!(
        wp.get(&format!(
            "mcp-for-page-builders/v1/resources/post_metadata/{missing_id}"
        ))
        .await
        .is_err(),
        "missing resource inspection unexpectedly succeeded"
    );
    let inspection_failure = WpApi.run(json!({"method":"PATCH","endpoint":format!("wp/v2/posts/{missing_id}"),"body":{"title":"must not be created"}}), &wp).await;
    ensure!(
        inspection_failure.is_err(),
        "failed resource inspection must block wp_api before write"
    );
    ensure!(
        UpdatePost
            .run(json!({"id":missing_id,"title":"must not be created"}), &wp)
            .await
            .is_err(),
        "UpdatePost must fail closed when resource inspection fails"
    );
    verify_absent(missing_id)?;

    fixtures.finish()?;
    Ok(())
}

#[tokio::test]
async fn option_writes_enforce_native_boundary_and_verify_persistence() -> Result<()> {
    let wp = wp();
    let unique = format!(
        "{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let owned_name = format!("mcp_boundary_{unique}");
    let protected_names: Vec<String> = [
        "elementor_",
        "_elementor_",
        "elementor_experiment-",
        "_mcp_elementor_",
    ]
    .into_iter()
    .flat_map(|prefix| [prefix.to_owned(), prefix.to_ascii_uppercase()])
    .map(|prefix| format!("{prefix}boundary_{unique}"))
    .collect();
    ensure!(
        option_state(&owned_name)? == "absent",
        "exact option fixture already exists"
    );

    let outcome = async {
        for name in &protected_names {
            let before = option_state(name)?;
            ensure!(
                before == "absent",
                "exact protected option fixture already exists: {name}"
            );
            let rejected = wp
                .post(
                    &format!("mcp-for-page-builders/v1/option/{name}"),
                    &json!({"value":"must-not-write"}),
                )
                .await;
            ensure!(rejected.is_err(), "protected option {name} was writable");
            ensure!(
                option_state(name)? == "absent",
                "protected option {name} changed after rejection"
            );
        }

        for body in [json!({}), json!(null), json!([])] {
            let before = option_state(&owned_name)?;
            let rejected = wp
                .post(
                    &format!("mcp-for-page-builders/v1/option/{owned_name}"),
                    &body,
                )
                .await;
            let message = rejected.expect_err("malformed body must fail").to_string();
            ensure!(
                message.contains("400"),
                "malformed option body did not return 400: {message}"
            );
            ensure!(
                option_state(&owned_name)? == before,
                "malformed body changed the option"
            );
        }

        let first = wp
            .post(
                &format!("mcp-for-page-builders/v1/option/{owned_name}"),
                &json!({"value":41}),
            )
            .await?;
        ensure!(
            first["changed"] == true,
            "first option write should report changed"
        );
        ensure!(
            wp.get(&format!("mcp-for-page-builders/v1/option/{owned_name}"))
                .await?
                == "41",
            "scalar readback did not reflect WordPress persistence normalization"
        );
        let unchanged = wp
            .post(
                &format!("mcp-for-page-builders/v1/option/{owned_name}"),
                &json!({"value":41}),
            )
            .await?;
        ensure!(
            unchanged["changed"] == false,
            "same persisted value should report unchanged"
        );

        let object_value = json!({"enabled":true,"labels":["a","b"]});
        let object_write = wp
            .post(
                &format!("mcp-for-page-builders/v1/option/{owned_name}"),
                &json!({"value":object_value}),
            )
            .await?;
        ensure!(
            object_write["changed"] == true,
            "object write should report changed"
        );
        ensure!(
            wp.get(&format!("mcp-for-page-builders/v1/option/{owned_name}"))
                .await?
                == object_value,
            "object option did not roundtrip"
        );
        let same_object = wp
            .post(
                &format!("mcp-for-page-builders/v1/option/{owned_name}"),
                &json!({"value":object_value}),
            )
            .await?;
        ensure!(
            same_object["changed"] == false,
            "same persisted object should report unchanged"
        );

        let tool_write = SetWpOption
            .run(json!({"name":owned_name,"value":17}), &wp)
            .await?;
        ensure!(
            tool_write.is_error.is_none(),
            "SetWpOption tool failed with the native scalar payload"
        );
        ensure!(
            wp.get(&format!("mcp-for-page-builders/v1/option/{owned_name}"))
                .await?
                == "17",
            "SetWpOption scalar did not persist"
        );
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let mut cleanup_failures = Vec::new();
    for name in protected_names.iter().chain(std::iter::once(&owned_name)) {
        if let Err(error) = delete_option_fixture(name) {
            cleanup_failures.push(format!("{name}: {error:#}"));
        }
    }
    outcome?;
    ensure!(
        cleanup_failures.is_empty(),
        "option fixture cleanup failed: {}",
        cleanup_failures.join("; ")
    );
    Ok(())
}
