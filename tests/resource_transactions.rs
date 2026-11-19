use anyhow::Context;
use mcp_for_page_builders::tools::Tool;
use mcp_for_page_builders::tools::elementor_plan_tools::ApplyElementorPlan;
use mcp_for_page_builders::tools::elementor_plan_tools::RestoreElementorSnapshot;
use mcp_for_page_builders::tools::page::CreatePage;
use mcp_for_page_builders::tools::page::DeletePage;
use mcp_for_page_builders::tools::page::{GetPage, UpdatePage};
use mcp_for_page_builders::tools::wp_api::WpApi;
use mcp_for_page_builders::wp::WpClient;
use serde_json::{Value, json};
use std::sync::OnceLock;

static TRANSACTION_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[path = "native_test_config.rs"]
mod native_test_config;

fn config() -> &'static native_test_config::NativeTestConfig {
    static CONFIG: OnceLock<native_test_config::NativeTestConfig> = OnceLock::new();
    CONFIG.get_or_init(|| {
        native_test_config::NativeTestConfig::load(false)
            .expect("valid isolated native test configuration")
    })
}

fn set_unknown_trash_fixture_meta(id: u64, value: &str) {
    let _ = config();
    let output = std::process::Command::new("docker")
        .args([
            "exec",
            "-u",
            config().docker_user.as_str(),
            config().container.as_str(),
            "wp",
            config().wp_path_arg.as_str(),
            "post",
            "meta",
            "update",
            &id.to_string(),
            "mcp_trash_fixture_unknown",
            value,
        ])
        .output()
        .expect("run exact-ID WP-CLI metadata fixture setup");
    assert!(
        output.status.success(),
        "WP-CLI must insert ordinary fixture metadata: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

struct ExactPageCleanup {
    base_url: String,
    user: String,
    password: String,
    id: u64,
    cleaned: bool,
}

impl ExactPageCleanup {
    fn new(wp: &WpClient, id: u64) -> Self {
        assert_eq!(
            wp.base_url().trim_end_matches('/'),
            config().url.trim_end_matches('/'),
            "transaction fixture cleanup may only mutate the isolated MCP integration clone"
        );
        Self {
            base_url: wp.base_url().to_owned(),
            user: config().user.clone(),
            password: config().password.clone(),
            id,
            cleaned: false,
        }
    }

    async fn finish(&mut self) {
        let deleted = WpClient::new(&self.base_url, &self.user, &self.password)
            .delete(&format!("wp/v2/pages/{}?force=true", self.id))
            .await
            .expect("delete exact transaction fixture");
        assert_eq!(
            deleted["deleted"], true,
            "exact transaction fixture must be deleted"
        );
        assert_fixture_absent(self.id)
            .expect("exact transaction fixture must be absent after deletion");
        self.cleaned = true;
    }
}

fn assert_fixture_absent(id: u64) -> anyhow::Result<()> {
    let php = format!("echo get_post({id}) ? 'present' : 'absent';");
    let output = std::process::Command::new("docker")
        .args([
            "exec",
            "-u",
            config().docker_user.as_str(),
            config().container.as_str(),
            "wp",
            config().wp_path_arg.as_str(),
            "eval",
            &php,
        ])
        .output()
        .context("verify exact transaction fixture absence")?;
    anyhow::ensure!(
        output.status.success(),
        "WP-CLI absence check failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    anyhow::ensure!(
        String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .ends_with("absent"),
        "exact transaction fixture {id} still exists"
    );
    Ok(())
}

struct ExactCreatedPostCleanup(Vec<u64>);

impl ExactCreatedPostCleanup {
    fn new() -> Self {
        Self(Vec::new())
    }

    fn track(&mut self, id: u64) {
        self.0.push(id);
    }

    fn finish(&mut self) {
        for id in &self.0 {
            assert_fixture_absent(*id).expect("restored native resource must be absent");
        }
        self.0.clear();
    }
}

impl Drop for ExactCreatedPostCleanup {
    fn drop(&mut self) {
        for id in self.0.drain(..) {
            let output = std::process::Command::new("docker")
                .args([
                    "exec",
                    "-u",
                    config().docker_user.as_str(),
                    config().container.as_str(),
                    "wp",
                    config().wp_path_arg.as_str(),
                    "post",
                    "delete",
                    &id.to_string(),
                    "--force",
                ])
                .output();
            let cleanup = output
                .map_err(|error| error.to_string())
                .and_then(|output| {
                    if !output.status.success() {
                        return Err(format!(
                            "exact native resource delete failed: {}",
                            String::from_utf8_lossy(&output.stderr)
                        ));
                    }
                    assert_fixture_absent(id).map_err(|error| format!("{error:#}"))
                });
            if let Err(error) = cleanup {
                if std::thread::panicking() {
                    eprintln!("PANIC CLEANUP FAILED for native resource {id}: {error}");
                } else {
                    panic!("exact native resource cleanup failed for {id}: {error}");
                }
            }
        }
    }
}

impl Drop for ExactPageCleanup {
    fn drop(&mut self) {
        if self.cleaned {
            return;
        }
        if self.base_url.trim_end_matches('/') != config().url.trim_end_matches('/') {
            eprintln!(
                "refusing transaction fixture cleanup outside the isolated MCP integration clone (page {})",
                self.id
            );
            return;
        }
        let output = std::process::Command::new("docker")
            .args([
                "exec",
                "-u",
                config().docker_user.as_str(),
                config().container.as_str(),
                "wp",
                config().wp_path_arg.as_str(),
                "post",
                "delete",
                &self.id.to_string(),
                "--force",
            ])
            .output();
        let cleanup = output
            .map_err(|error| error.to_string())
            .and_then(|output| {
                if !output.status.success() {
                    return Err(format!(
                        "WP-CLI delete failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    ));
                }
                assert_fixture_absent(self.id).map_err(|error| format!("{error:#}"))?;
                Ok(())
            });
        if let Err(error) = cleanup {
            if std::thread::panicking() {
                eprintln!(
                    "PANIC CLEANUP FAILED for exact page fixture {}: {error}",
                    self.id
                );
            } else {
                panic!(
                    "exact transaction fixture cleanup failed for page {}: {error}",
                    self.id
                );
            }
        }
    }
}

struct ExactOperationCleanup(Vec<String>);

impl ExactOperationCleanup {
    fn new() -> Self {
        Self(Vec::new())
    }

    fn track(&mut self, operation_id: &str) {
        assert!(
            operation_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        );
        self.0.push(operation_id.to_owned());
    }
}

impl Drop for ExactOperationCleanup {
    fn drop(&mut self) {
        if self.0.is_empty() {
            return;
        }
        let operations = serde_json::to_string(&self.0)
            .expect("serialize exact operation IDs")
            .replace('\'', "\\'");
        let php = format!(
            "$ids = json_decode('{operations}', true); foreach ($ids as $id) {{ $hash = hash('sha256', $id); delete_option('_mcp_elementor_operation_' . $hash); delete_option('_mcp_elementor_resources_' . $hash); }} foreach ($ids as $id) {{ $hash = hash('sha256', $id); if (get_option('_mcp_elementor_operation_' . $hash, '__absent__') !== '__absent__' || get_option('_mcp_elementor_resources_' . $hash, '__absent__') !== '__absent__') exit(1); }} echo 'clean';"
        );
        let cleanup = std::process::Command::new("docker")
            .args([
                "exec",
                "-u",
                config().docker_user.as_str(),
                config().container.as_str(),
                "wp",
                config().wp_path_arg.as_str(),
                "eval",
                &php,
            ])
            .output()
            .map_err(|error| error.to_string())
            .and_then(|output| {
                if !output.status.success() {
                    return Err(format!(
                        "exact operation cleanup failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    ));
                }
                if !String::from_utf8_lossy(&output.stdout)
                    .trim_end()
                    .ends_with("clean")
                {
                    return Err("exact operation journal remains after cleanup".to_owned());
                }
                Ok(())
            });
        if let Err(error) = cleanup {
            if std::thread::panicking() {
                eprintln!("PANIC CLEANUP FAILED for exact source CAS operations: {error:#}");
            } else {
                panic!("exact source CAS operation cleanup failed: {error:#}");
            }
        }
    }
}

fn tool_text(result: &mcp_for_page_builders::mcp::ToolResult) -> &str {
    match result.content.first().expect("tool response content") {
        mcp_for_page_builders::types::tool_result::ToolContent::Text { text } => text,
        _ => panic!("tool response was not text"),
    }
}

#[tokio::test]
async fn checked_trash_rejects_stale_and_changed_posts_and_restores_natively() {
    let _guard = TRANSACTION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let wp = wp();
    let marker = format!(
        "MCP checked trash {}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    println!("checked trash fixture marker before POST: {marker}");
    let created = wp
        .post("wp/v2/pages", &json!({"title":marker,"status":"draft"}))
        .await
        .expect("create exact trash fixture");
    let id = created["id"].as_u64().expect("trash fixture ID");
    println!("checked trash fixture: marker={marker:?} page_id={id}");
    let mut fixture_cleanup = ExactPageCleanup::new(&wp, id);
    let route = format!("mcp-for-page-builders/v1/resources/trash/{id}");
    let before = wp
        .get(&route)
        .await
        .expect("read complete post deletion revision");
    let operation_id = format!(
        "checked_trash_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );

    let stale = DeletePage.run(json!({"id":id,"expected_revision":"stale","operation_id":format!("{operation_id}_stale")}), &wp).await;
    assert!(
        stale.is_err(),
        "stale trash revision must reject before native trash"
    );
    assert_eq!(
        wp.get(&route).await.expect("read after stale rejection")["revision"],
        before["revision"]
    );
    let direct_delete = WpApi
        .run(
            json!({"method":"DELETE","endpoint":format!("/wp/v2/pages/{id}")}),
            &wp,
        )
        .await;
    assert!(
        direct_delete.is_err(),
        "generic wp_api must not bypass revision-checked deletion"
    );
    assert_eq!(
        wp.get(&route).await.expect("read after blocked raw delete")["revision"],
        before["revision"]
    );

    let trashed = DeletePage.run(json!({"id":id,"expected_revision":before["revision"],"operation_id":operation_id,"force":false}), &wp).await.expect("native WordPress trash");
    let trashed_text = serde_json::to_value(trashed).expect("serialize trash receipt")["content"]
        [0]["text"]
        .as_str()
        .expect("trash response text")
        .to_owned();
    assert!(
        trashed_text.contains("restore_operation_id"),
        "trash receipt must expose the restore operation ID"
    );
    let trashed_state = wp.get(&route).await.expect("read native trashed post");
    assert_eq!(trashed_state["value"]["post_status"], "trash");
    let metadata_route = format!("mcp-for-page-builders/v1/resources/post_metadata/{id}");
    let trashed_metadata = wp
        .get(&metadata_route)
        .await
        .expect("read metadata before exact post-trash change");
    let change_operation = format!("{operation_id}_change");
    let changed = wp.request("POST", "mcp-for-page-builders/v1/resources/apply", Some(&json!({
        "operation_id":change_operation,"expected_revision":"absent","resource_patches":[{"kind":"post_metadata","id":id.to_string(),"expected_revision":trashed_metadata["revision"],"after":{"title":"Changed after trash","status":"draft"}}]
    })), None).await.expect("checked post-trash state change");
    assert_eq!(changed["applied"], true);
    let refused = wp
        .request(
            "POST",
            "mcp-for-page-builders/v1/resources/restore",
            Some(&json!({"operation_id":operation_id})),
            None,
        )
        .await;
    assert!(
        refused.is_err(),
        "restore must reject a post changed after trash"
    );
    let force = DeletePage
        .run(
            json!({"id":id,"expected_revision":trashed_state["revision"],"force":true}),
            &wp,
        )
        .await;
    assert!(
        force.is_err(),
        "force=true must fail before permanent deletion"
    );
    let still_exists = wp
        .get(&route)
        .await
        .expect("fixture remains after force rejection");
    assert_eq!(still_exists["value"]["post_status"], "draft");
    let changed_restored = wp
        .request(
            "POST",
            "mcp-for-page-builders/v1/resources/restore",
            Some(&json!({"operation_id":change_operation})),
            None,
        )
        .await
        .expect("restore the exact post-trash change");
    assert_eq!(changed_restored["restored"], true);
    let deletion_restored = wp
        .request(
            "POST",
            "mcp-for-page-builders/v1/resources/restore",
            Some(&json!({"operation_id":operation_id})),
            None,
        )
        .await
        .expect("restore original trashed fixture");
    assert_eq!(deletion_restored["restored"], true);
    fixture_cleanup.finish().await;
}

#[tokio::test]
async fn checked_trash_restore_preserves_native_status_and_elementor_snapshot() {
    let _guard = TRANSACTION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let wp = wp();
    let marker = format!(
        "MCP checked restore {}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    println!("checked restore fixture marker before POST: {marker}");
    let created = wp.post("wp/v2/pages", &json!({"title":marker,"status":"draft","meta":{"_elementor_data":"[]","_elementor_edit_mode":"builder"}})).await.expect("create exact restore fixture");
    let id = created["id"].as_u64().expect("restore fixture ID");
    println!("checked restore fixture: marker={marker:?} page_id={id}");
    let mut fixture_cleanup = ExactPageCleanup::new(&wp, id);
    let unknown_meta = format!("{marker} ordinary postmeta");
    set_unknown_trash_fixture_meta(id, &unknown_meta);
    let route = format!("mcp-for-page-builders/v1/resources/trash/{id}");
    let document_route = format!("mcp-for-page-builders/v1/document/{id}");
    let document = json!([{
        "id":"trashcont1","elType":"container",
        "settings":{"flex_direction":"column","content_width":"boxed","boxed_width":{"unit":"px","size":820,"sizes":[]},"padding":{"unit":"px","top":"18","right":"22","bottom":"18","left":"22","isLinked":true}},
        "elements":[{"id":"trashhead1","elType":"widget","widgetType":"heading","settings":{"title":"Native trash restoration heading","header_size":"h2","align":"center","title_color":"#123456","typography_typography":"custom","typography_font_size":{"unit":"px","size":27,"sizes":[]},"typography_line_height":{"unit":"em","size":1.25,"sizes":[]}},"elements":[]}]
    }]);
    let document_before_save = wp
        .get(&document_route)
        .await
        .expect("read initial Elementor revision");
    let document_apply_route = format!("mcp-for-page-builders/v1/document/{id}/apply");
    let saved = wp.request("POST", &document_apply_route, Some(&json!({"document":document,"expected_revision":document_before_save["revision"],"operation_id":format!("trash_doc_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos())})), None).await.expect("save native Elementor document");
    assert_eq!(
        saved["saved"], true,
        "native Elementor document save must succeed: {saved}"
    );
    let settings_route = format!("mcp-for-page-builders/v1/resources/page_settings/{id}");
    let settings_before = wp
        .get(&settings_route)
        .await
        .expect("read native page settings revision");
    let settings_source = wp
        .get(&document_route)
        .await
        .expect("read document revision before settings write");
    let settings_operation = format!(
        "trash_settings_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let settings_saved = wp.request("POST", "mcp-for-page-builders/v1/resources/apply", Some(&json!({
        "operation_id":settings_operation,
        "page_id":id,"expected_revision":settings_source["revision"],
        "resource_patches":[{"kind":"page_settings","id":id.to_string(),"expected_revision":settings_before["revision"],"after":{"hide_title":"yes","background_background":"classic","background_color":"#f4f4f4"}}]
    })), None).await.expect("save settings through native Elementor document API");
    assert_eq!(
        settings_saved["applied"], true,
        "native Elementor page settings save must succeed"
    );
    let settings_document_revision = wp
        .get(&document_route)
        .await
        .expect("read document revision after native page settings save");
    let css_generated = wp.request("POST", &document_apply_route, Some(&json!({"document":document,"expected_revision":settings_document_revision["revision"],"operation_id":format!("trash_css_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos())})), None).await.expect("save native document and regenerate Elementor CSS");
    assert_eq!(
        css_generated["cache_regenerated"], true,
        "native save must verify generated Elementor CSS: {css_generated}"
    );
    let before = wp
        .get(&route)
        .await
        .expect("read saved post and metadata snapshot");
    let document_before = wp
        .get(&document_route)
        .await
        .expect("read saved Elementor tree");
    let settings_after = wp
        .get(&settings_route)
        .await
        .expect("read saved native page settings");
    let saved_tree: serde_json::Value = serde_json::from_str(
        document_before["document"]
            .as_str()
            .expect("native document response is serialized Elementor JSON"),
    )
    .expect("saved Elementor tree is JSON");
    assert_eq!(saved_tree, document);
    assert_eq!(
        before["value"]["meta"]["mcp_trash_fixture_unknown"],
        json!([unknown_meta]),
        "Trash snapshot must retain unknown ordinary postmeta"
    );
    assert!(
        before["value"]["meta"].get("_elementor_css").is_some(),
        "trash snapshot must include Elementor CSS metadata after native regeneration"
    );
    assert_eq!(settings_after["value"]["hide_title"], "yes");
    assert_eq!(settings_after["value"]["background_color"], "#f4f4f4");
    assert!(
        before["value"]["meta"].get("_elementor_css").is_some(),
        "complete pre-trash snapshot must include native CSS metadata"
    );
    assert_eq!(
        before["value"]["elementor_css"]["sha256"]
            .as_str()
            .map(str::len),
        Some(64),
        "pre-trash snapshot must hash the native CSS artifact"
    );
    let operation_id = format!(
        "checked_restore_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let trashed = DeletePage
        .run(
            json!({"id":id,"expected_revision":before["revision"],"operation_id":operation_id}),
            &wp,
        )
        .await
        .expect("trash exact restore fixture");
    assert_eq!(trashed.is_error, None);
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let restored = wp
        .request(
            "POST",
            "mcp-for-page-builders/v1/resources/restore",
            Some(&json!({"operation_id":operation_id})),
            None,
        )
        .await
        .expect("native untrash restore");
    assert_eq!(restored["restored"], true);
    let after = wp
        .get(&route)
        .await
        .expect("read restored post and metadata snapshot");
    assert_eq!(
        after["value"], before["value"],
        "native untrash must preserve original status, content, metadata, and Elementor tree/settings/conditions"
    );
    let document_after = wp
        .get(&document_route)
        .await
        .expect("read restored Elementor tree");
    assert_eq!(
        document_after["document"], document_before["document"],
        "native untrash must preserve the Elementor tree"
    );
    let settings_restored = wp
        .get(&settings_route)
        .await
        .expect("read restored native page settings");
    assert_eq!(
        settings_restored["value"], settings_after["value"],
        "native untrash must preserve Elementor page settings"
    );
    assert_eq!(
        after["value"]["post_status"], "draft",
        "native untrash must restore the original status"
    );
    fixture_cleanup.finish().await;
}

#[tokio::test]
async fn checked_trash_restore_keeps_published_and_private_statuses() {
    let _guard = TRANSACTION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let wp = wp();
    assert_eq!(
        wp.base_url().trim_end_matches('/'),
        config().url.trim_end_matches('/'),
        "native trash lifecycle test may only mutate the isolated MCP integration clone"
    );
    for status in ["publish", "private"] {
        let marker = format!(
            "MCP trash status {status} {}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let created = wp.post("wp/v2/pages", &json!({"title":marker,"status":status,"meta":{"_elementor_data":"[]","_elementor_edit_mode":"builder"}})).await.expect("create exact status fixture");
        let id = created["id"].as_u64().expect("status fixture ID");
        let mut fixture_cleanup = ExactPageCleanup::new(&wp, id);
        let route = format!("mcp-for-page-builders/v1/resources/trash/{id}");
        let document_route = format!("mcp-for-page-builders/v1/document/{id}");
        let document_before = wp
            .get(&document_route)
            .await
            .expect("read status fixture document revision");
        let regenerated = wp.request("POST", &format!("{document_route}/apply"), Some(&json!({"document":[],"expected_revision":document_before["revision"],"operation_id":format!("trash_status_css_{}_{}", status, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos())})), None).await.expect("generate CSS before status snapshot");
        assert_eq!(
            regenerated["cache_regenerated"], true,
            "native CSS must be generated before the status snapshot"
        );
        let before = wp.get(&route).await.expect("read original status snapshot");
        assert!(
            before["value"]["meta"].get("_elementor_css").is_some(),
            "status snapshot must include native CSS metadata"
        );
        assert_eq!(
            before["value"]["elementor_css"]["sha256"]
                .as_str()
                .map(str::len),
            Some(64),
            "status snapshot must hash the native CSS artifact"
        );
        assert_eq!(before["value"]["post_status"], status);
        let operation_id = format!(
            "trash_status_{}_{}",
            status,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let trashed = DeletePage
            .run(
                json!({"id":id,"expected_revision":before["revision"],"operation_id":operation_id}),
                &wp,
            )
            .await
            .expect("trash exact status fixture");
        assert_eq!(trashed.is_error, None);
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let restored = wp
            .request(
                "POST",
                "mcp-for-page-builders/v1/resources/restore",
                Some(&json!({"operation_id":operation_id})),
                None,
            )
            .await
            .expect("restore exact status fixture");
        assert_eq!(restored["restored"], true);
        let after = wp.get(&route).await.expect("read restored status snapshot");
        assert_eq!(
            after["value"], before["value"],
            "native untrash must preserve the complete {status} snapshot"
        );
        fixture_cleanup.finish().await;
    }
}

#[test]
fn native_trash_disabled_guard_preserves_exact_elementor_fixture() {
    let user = config().user.clone();
    assert_eq!(
        std::env::var("MCP_NATIVE_TRASH_ZERO_LEASE").as_deref(),
        Ok("granted"),
        "native clone writes require MCP_NATIVE_TRASH_ZERO_LEASE=granted"
    );

    let before_wordpress = r#"if (defined('ABSPATH')) { fwrite(STDERR, "--exec ran after WordPress bootstrap\n"); exit(91); } define('EMPTY_TRASH_DAYS', 0);"#;
    let php = r#"
$failures = [];
$fixture_id = 0;
$operation_id = '';
$check = static function ($condition, $message) use (&$failures) {
    if (!$condition) $failures[] = $message;
};
$css_snapshot = static function ($post_id) {
    if (!class_exists('Elementor\\Core\\Files\\CSS\\Post')) throw new RuntimeException('Elementor native CSS API is unavailable');
    $css = new Elementor\Core\Files\CSS\Post($post_id);
    $status = $css->get_meta('status');
    if ($status === 'file') {
        $path = $css->get_path();
        clearstatcache(true, $path);
        $contents = file_get_contents($path);
        if (!is_string($contents)) throw new RuntimeException('Could not read generated Elementor CSS');
        return [$status, $contents];
    }
    if (in_array($status, ['inline', 'empty'], true)) return [$status, $css->get_meta('css')];
    throw new RuntimeException('Elementor CSS generation did not complete: ' . var_export($status, true));
};
$snapshot = static function ($post_id) use ($css_snapshot) {
    $post = get_post($post_id);
    if (!$post) throw new RuntimeException('Fixture disappeared before snapshot');
    return [$post->to_array(), get_post_meta($post_id, '', false), $css_snapshot($post_id)];
};
try {
    $check(defined('ABSPATH') && defined('EMPTY_TRASH_DAYS') && EMPTY_TRASH_DAYS === 0, 'fresh WP-CLI process did not retain EMPTY_TRASH_DAYS=0');
    $user = get_user_by('login', getenv('WP_TEST_USER'));
    $check($user instanceof WP_User, 'authorized native clone user was not found');
    if ($user instanceof WP_User) {
        wp_set_current_user($user->ID);
        $check(current_user_can('manage_options'), 'native test user lacks the bridge administrator capability');
    }
    if (!$failures) {
        $created_id = wp_insert_post([
            'post_type' => 'page',
            'post_status' => 'draft',
            'post_title' => 'MCP empty trash guard ' . wp_generate_uuid4(),
            'post_author' => $user->ID,
            'meta_input' => [
                '_elementor_data' => wp_json_encode([['id' => 'guardhead', 'elType' => 'widget', 'widgetType' => 'heading', 'settings' => ['title' => 'Trash guard fixture'], 'elements' => []]]),
                '_elementor_edit_mode' => 'builder',
            ],
        ], true);
        if (is_wp_error($created_id)) throw new RuntimeException('Could not create exact draft fixture: ' . $created_id->get_error_message());
        if (!is_int($created_id) || $created_id <= 0) throw new RuntimeException('WordPress returned an invalid fixture ID');
        $fixture_id = $created_id;

        $css = new Elementor\Core\Files\CSS\Post($fixture_id);
        $css->update();
        $before = $snapshot($fixture_id);

        $read = rest_do_request(new WP_REST_Request('GET', '/mcp-for-page-builders/v1/resources/trash/' . $fixture_id));
        $check($read->get_status() === 200, 'checked trash snapshot endpoint did not return HTTP 200');
        $state = $read->get_data();
        $check(is_array($state) && isset($state['revision']), 'checked trash snapshot omitted its revision');
        if (is_array($state) && isset($state['revision'])) {
            $request = new WP_REST_Request('POST', '/mcp-for-page-builders/v1/resources/apply');
            $request->set_header('X-WP-Nonce', wp_create_nonce('wp_rest'));
            $request->set_header('Content-Type', 'application/json');
            $operation_id = 'native_empty_trash_' . wp_generate_uuid4();
            $request->set_body(wp_json_encode([
                'operation_id' => $operation_id,
                'expected_revision' => 'absent',
                'resource_patches' => [[
                    'kind' => 'trash',
                    'id' => (string) $fixture_id,
                    'expected_revision' => $state['revision'],
                    'after' => [],
                ]],
            ]));
            $response = rest_do_request($request);
            $data = $response->get_data();
            $check($response->get_status() === 409, 'checked resource endpoint returned HTTP ' . $response->get_status() . ': ' . wp_json_encode($data));
            $check(is_array($data) && ($data['code'] ?? null) === 'trash_disabled', 'checked resource endpoint omitted trash_disabled guard code: ' . wp_json_encode($data));
            $check(is_array($data) && ($data['data']['status'] ?? null) === 409, 'checked resource endpoint omitted the native HTTP 409 guard status: ' . wp_json_encode($data));
        }

        $after = $snapshot($fixture_id);
        $check($after === $before, 'rejected trash changed the exact post, full metadata, or generated Elementor CSS');
        $check(get_post($fixture_id) instanceof WP_Post, 'rejected trash destroyed the native fixture');
    }
} catch (Throwable $error) {
    $failures[] = 'native guard assertion/process error: ' . $error->getMessage();
} finally {
    if ($operation_id !== '') {
        try {
            delete_option('_mcp_elementor_resources_' . hash('sha256', $operation_id));
        } catch (Throwable $error) {
            $failures[] = 'exact resource operation cleanup failed: ' . $error->getMessage();
        }
    }
    if ($fixture_id > 0) {
        try {
            $deleted = wp_delete_post($fixture_id, true);
            if (!$deleted || get_post($fixture_id) !== null) $failures[] = 'exact fixture permanent deletion or absence check failed for ID ' . $fixture_id;
        } catch (Throwable $error) {
            $failures[] = 'exact fixture cleanup failed for ID ' . $fixture_id . ': ' . $error->getMessage();
        }
    }
}
echo wp_json_encode(['fixture_id' => $fixture_id, 'failures' => $failures]), "\n";
if ($failures) exit(1);
"#;

    let output = std::process::Command::new("docker")
        .args(["exec", "-u", config().docker_user.as_str(), "-e"])
        .arg(format!("WP_TEST_USER={user}"))
        .args([
            config().container.as_str(),
            "wp",
            config().wp_path_arg.as_str(),
            &format!("--exec={before_wordpress}"),
            "eval",
            php,
        ])
        .output()
        .expect("run isolated native EMPTY_TRASH_DAYS=0 guard regression");
    assert!(
        output.status.success(),
        "native EMPTY_TRASH_DAYS=0 guard regression failed (status {}):\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn checked_page_creation_replay_guard_and_restore() {
    let _guard = TRANSACTION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let wp = wp();
    let marker = format!(
        "MCP checked create {}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let invalid = CreatePage.run(json!({"title":marker,"elementor_data":r#"[{"id":"duplicate","elType":"widget","widgetType":"heading","settings":{},"elements":[]},{"id":"duplicate","elType":"widget","widgetType":"heading","settings":{},"elements":[]}]"#}), &wp).await;
    assert!(
        invalid.is_err() || invalid.unwrap().is_error == Some(true),
        "duplicate native element IDs must reject creation"
    );
    let invalid_typed = CreatePage.run(json!({"title":marker,"elementor_data":r#"[{"id":"typed001","elType":"widget","widgetType":"heading","settings":"invalid","elements":[]}]"#}), &wp).await;
    assert!(
        invalid_typed.is_err(),
        "settings with an invalid native tree type must reject before creation"
    );
    let untouched = wp
        .request("GET", "wp/v2/pages", None, Some(&json!({"search":marker})))
        .await
        .expect("read page list after rejected creation");
    assert!(
        !untouched
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| row["title"]["rendered"] == marker)),
        "rejected creation must not insert a page"
    );

    let operation_id = format!(
        "checked_create_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let arguments = json!({"title":marker,"elementor_data":"[{\"id\":\"checked001\",\"elType\":\"widget\",\"widgetType\":\"heading\",\"settings\":{\"title\":\"checked\"},\"elements\":[]}]","operation_id":operation_id});
    let created = CreatePage
        .run(arguments.clone(), &wp)
        .await
        .expect("create through checked page resource");
    assert_eq!(created.is_error, None);
    let replay = CreatePage
        .run(arguments, &wp)
        .await
        .expect("replay checked page operation");
    let decode = |result: &mcp_for_page_builders::mcp::ToolResult| -> serde_json::Value {
        let value = serde_json::to_value(result).expect("serialize tool response");
        serde_json::from_str(value["content"][0]["text"].as_str().expect("tool text"))
            .expect("page creation receipt")
    };
    let first = decode(&created);
    let second = decode(&replay);
    assert_eq!(
        first["id"], second["id"],
        "operation replay must return the same page ID"
    );
    assert_eq!(second["idempotent_replay"], true);
    let id = first["id"].as_str().expect("generated page ID");
    let before = wp
        .get(&format!("mcp-for-page-builders/v1/resources/page/{id}"))
        .await
        .expect("read created resource revision");
    let blocked = WpApi.run(json!({"method":"POST","endpoint":format!("/wp/v2/pages/{id}"),"body":{"title":"bypassed"}}), &wp).await;
    assert!(
        blocked.is_err(),
        "direct page writes to Elementor pages must be blocked"
    );
    let after = wp
        .get(&format!("mcp-for-page-builders/v1/resources/page/{id}"))
        .await
        .expect("read page after blocked write");
    assert_eq!(after["revision"], before["revision"]);
    let unrelated_read = WpApi
        .run(json!({"method":"GET","endpoint":"wp/v2/pages"}), &wp)
        .await;
    assert!(
        unrelated_read.is_ok(),
        "unrelated WordPress reads must remain available"
    );
    let restored = wp
        .request(
            "POST",
            "mcp-for-page-builders/v1/resources/restore",
            Some(&json!({"operation_id":operation_id})),
            None,
        )
        .await
        .expect("restore unchanged created resource");
    assert_eq!(restored["restored"], true);
    assert!(
        wp.get(&format!("wp/v2/pages/{id}")).await.is_err(),
        "restore must delete the exact newly created page"
    );
}

fn wp() -> WpClient {
    let url = config().url.clone();
    WpClient::new(&url, &config().user, &config().password)
}

fn combined_native_snapshot(page_id: u64, operation_id: &str) -> anyhow::Result<Value> {
    let bridge_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mcp-bridge.php");
    let bridge_hash = std::process::Command::new("shasum")
        .args(["-a", "256"])
        .arg(&bridge_path)
        .output()
        .context("hash current MCP bridge source")?;
    anyhow::ensure!(
        bridge_hash.status.success(),
        "could not hash current MCP bridge source"
    );
    let expected_bridge = String::from_utf8(bridge_hash.stdout)?
        .split_whitespace()
        .next()
        .context("shasum omitted the MCP bridge digest")?
        .to_owned();
    let php = r#"$source = WPMU_PLUGIN_DIR . '/mcp-for-page-builders/mcp-bridge.php';
if (!hash_equals(getenv('MCP_EXPECTED_BRIDGE_SHA'), (string) hash_file('sha256', $source))) throw new RuntimeException('installed bridge differs from current source');
$id = (int) getenv('MCP_PAGE_ID');
$operation = (string) getenv('MCP_OPERATION_ID');
$state = mcp_bridge_editor_guard_full_state($id);
if (is_wp_error($state)) throw new RuntimeException($state->get_error_message());
$journal = get_option('_mcp_elementor_resources_' . hash('sha256', $operation));
$document = get_option('_mcp_elementor_operation_' . hash('sha256', $operation));
$style_map = ['z-native-order'=>['id'=>'z-native-order'], 'a-native-order'=>['id'=>'a-native-order']];
$reversed_style_map = array_reverse($style_map, true);
$style_hash_a = mcp_bridge_hash([['elType'=>'e-flexbox', 'styles'=>$style_map]]);
$style_hash_b = mcp_bridge_hash([['elType'=>'e-flexbox', 'styles'=>$reversed_style_map]]);
$native_node_a = ['elType'=>'e-flexbox', 'id'=>'order-check', 'styles'=>$style_map, 'settings'=>[]];
$native_node_b = ['settings'=>[], 'styles'=>$style_map, 'id'=>'order-check', 'elType'=>'e-flexbox'];
$native_node_fields_match = mcp_bridge_hash([$native_node_a]) === mcp_bridge_hash([$native_node_b]);
$legacy_style_fingerprint = mcp_bridge_legacy_hash([$native_node_a]);
$ordered_style_fingerprint = 'native-style-order-v1:' . mcp_bridge_hash([$native_node_a]);
$legacy_styled_receipt_rejected = !mcp_bridge_ordered_fingerprint_matches($legacy_style_fingerprint, $ordered_style_fingerprint, $legacy_style_fingerprint, [$native_node_a]);
$metadata_payload = ['resource_patches'=>[['kind'=>'post_metadata', 'after'=>['title'=>'same']]]];
$legacy_metadata_fingerprint = mcp_bridge_legacy_hash($metadata_payload);
$ordered_metadata_fingerprint = 'native-style-order-v1:' . mcp_bridge_hash($metadata_payload);
$legacy_metadata_receipt_accepted = mcp_bridge_ordered_fingerprint_matches($legacy_metadata_fingerprint, $ordered_metadata_fingerprint, $legacy_metadata_fingerprint, $metadata_payload);
$template_payload = ['resource_patches'=>[['kind'=>'template', 'after'=>['elements'=>[$native_node_a]]]]];
$template_style_receipt_rejected = !mcp_bridge_ordered_fingerprint_matches(mcp_bridge_legacy_hash($template_payload), 'native-style-order-v1:' . mcp_bridge_hash($template_payload), mcp_bridge_legacy_hash($template_payload), $template_payload);
$metadata_hash_a = mcp_bridge_hash(['metadata'=>['left'=>1, 'right'=>2]]);
$metadata_hash_b = mcp_bridge_hash(['metadata'=>['right'=>2, 'left'=>1]]);
$ack_injected = get_option('_mcp_fixture_resource_ack_failure_' . hash('sha256', $operation), false);
echo 'MCP_COMBINED_STATE:' . wp_json_encode(['state' => $state, 'journal' => $journal, 'document_receipt' => $document, 'ack_failure_marker'=>$ack_injected, 'hash_contract'=>['style_order_differs'=>$style_hash_a!==$style_hash_b,'metadata_order_matches'=>$metadata_hash_a===$metadata_hash_b,'native_node_fields_match'=>$native_node_fields_match,'legacy_styled_receipt_rejected'=>$legacy_styled_receipt_rejected,'legacy_metadata_receipt_accepted'=>$legacy_metadata_receipt_accepted,'template_style_receipt_rejected'=>$template_style_receipt_rejected]]);"#;
    let output = std::process::Command::new("docker")
        .args([
            "exec",
            "-u",
            config().docker_user.as_str(),
            "-e",
            &format!("MCP_EXPECTED_BRIDGE_SHA={expected_bridge}"),
            "-e",
            &format!("MCP_PAGE_ID={page_id}"),
            "-e",
            &format!("MCP_OPERATION_ID={operation_id}"),
            config().container.as_str(),
            "wp",
            config().wp_path_arg.as_str(),
            "eval",
            php,
        ])
        .output()
        .context("read combined native source snapshot from the isolated MCP clone")?;
    anyhow::ensure!(
        output.status.success(),
        "combined native snapshot failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout)?;
    let (_, json) = stdout
        .rsplit_once("MCP_COMBINED_STATE:")
        .context("native snapshot omitted its result marker")?;
    serde_json::from_str(json.trim()).context("parse combined native source snapshot")
}

struct ResourceAckFailureCleanup {
    operation_id: String,
    filename: String,
    finished: bool,
}

impl ResourceAckFailureCleanup {
    fn install(operation_id: &str) -> Self {
        let url = config().url.clone();
        assert_eq!(
            url.trim_end_matches('/'),
            config().url.trim_end_matches('/'),
            "acknowledgement fixture may only be installed in the isolated MCP integration clone"
        );
        let filename = format!("mcp-resource-ack-{}.php", operation_id);
        let plugin = format!(
            "<?php\nadd_filter('rest_post_dispatch', static function($response, $server, $request) {{\n    if ($request->get_route() !== '/mcp-for-page-builders/v1/resources/apply') return $response;\n    $params = $request->get_json_params();\n    if (($params['operation_id'] ?? '') !== '{operation_id}') return $response;\n    $data = $response instanceof WP_REST_Response ? $response->get_data() : null;\n    if (!is_array($data) || ($data['applied'] ?? false) !== true || !is_array($data['resources'] ?? null)) return $response;\n    update_option('_mcp_fixture_resource_ack_failure_' . hash('sha256', '{operation_id}'), 'injected', false);\n    return new WP_REST_Response(['code'=>'mcp_fixture_ack_lost','message'=>'Fixture dropped the response after native resource commit.'], 502);\n}}, PHP_INT_MAX, 3);\n"
        );
        let php = format!(
            "$path = WP_CONTENT_DIR . '/mu-plugins/{filename}'; if (file_exists($path)) throw new RuntimeException('ack fixture path already exists'); $source = <<<'MCP_PLUGIN'\n{plugin}MCP_PLUGIN; if (file_put_contents($path, $source, LOCK_EX) !== strlen($source)) {{ if (is_file($path)) unlink($path); throw new RuntimeException('could not write exact acknowledgement fixture'); }} if (!chmod($path, 0644)) {{ unlink($path); throw new RuntimeException('could not set acknowledgement fixture permissions'); }} echo 'ack fixture installed';"
        );
        let output = std::process::Command::new("docker")
            .args([
                "exec",
                "-u",
                config().docker_user.as_str(),
                config().container.as_str(),
                "wp",
                config().wp_path_arg.as_str(),
                "eval",
                &php,
            ])
            .output()
            .expect("install exact native acknowledgement fixture");
        assert!(
            output.status.success(),
            "WP-CLI must install the exact acknowledgement fixture: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Self {
            operation_id: operation_id.to_owned(),
            filename,
            finished: false,
        }
    }

    fn remove(&self) -> Result<(), String> {
        let php = format!(
            "$path = WP_CONTENT_DIR . '/mu-plugins/{}'; if (is_file($path) && !unlink($path)) throw new RuntimeException('could not remove exact acknowledgement fixture'); delete_option('_mcp_fixture_resource_ack_failure_' . hash('sha256', '{}')); if (is_file($path) || get_option('_mcp_fixture_resource_ack_failure_' . hash('sha256', '{}'), '__absent__') !== '__absent__') exit(1); echo 'ack fixture removed';",
            self.filename, self.operation_id, self.operation_id
        );
        let output = std::process::Command::new("docker")
            .args([
                "exec",
                "-u",
                config().docker_user.as_str(),
                config().container.as_str(),
                "wp",
                config().wp_path_arg.as_str(),
                "eval",
                &php,
            ])
            .output()
            .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "WP-CLI must remove the exact acknowledgement fixture: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(())
    }

    fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.remove()
            .unwrap_or_else(|error| panic!("acknowledgement fixture cleanup failed: {error}"));
        self.finished = true;
    }
}

impl Drop for ResourceAckFailureCleanup {
    fn drop(&mut self) {
        if !self.finished {
            if let Err(error) = self.remove() {
                eprintln!(
                    "cleanup failed for acknowledgement fixture {}: {error}",
                    self.filename
                );
            } else {
                self.finished = true;
            }
        }
    }
}

fn fail_second_combined_template_after_save(
    page_id: u64,
    operation_id: &str,
    original_revision: &str,
    document: &Value,
    resource_patches: &Value,
) -> anyhow::Result<Value> {
    let php = r#"$user = get_user_by('login', getenv('WP_TEST_USER'));
if (!$user) throw new RuntimeException('combined failure fixture user is missing');
wp_set_current_user((int) $user->ID);
if (!current_user_can('manage_options')) throw new RuntimeException('combined failure fixture user lacks manage_options');
$page_id = (int) getenv('MCP_PAGE_ID');
$operation_id = (string) getenv('MCP_OPERATION_ID');
$original_revision = (string) getenv('MCP_ORIGINAL_REVISION');
$document = json_decode(getenv('MCP_DOCUMENT'), true, 512, JSON_THROW_ON_ERROR);
$patches = json_decode(getenv('MCP_RESOURCE_PATCHES'), true, 512, JSON_THROW_ON_ERROR);
$preflight = new WP_REST_Request('POST', '/mcp-for-page-builders/v1/resources/apply');
$preflight->set_header('Content-Type', 'application/json');
$preflight->set_body(wp_json_encode(['operation_id'=>$operation_id,'page_id'=>$page_id,'expected_revision'=>$original_revision,'original_expected_revision'=>$original_revision,'document_patch'=>$document,'resource_patches'=>$patches,'document_snapshot'=>true,'validate_only'=>true]));
$preflight_response = rest_do_request($preflight);
if (is_wp_error($preflight_response) || $preflight_response->get_status() >= 400) throw new RuntimeException('combined resource preflight rejected: ' . wp_json_encode(is_wp_error($preflight_response) ? $preflight_response->get_error_data() : $preflight_response->get_data()));
$document_request = new WP_REST_Request('POST', '/mcp-for-page-builders/v1/document/' . $page_id . '/apply');
$document_request->set_header('Content-Type', 'application/json');
$document_request->set_body(wp_json_encode(['operation_id'=>$operation_id,'expected_revision'=>$original_revision,'document'=>$document]));
$document_response = rest_do_request($document_request);
if (is_wp_error($document_response) || $document_response->get_status() >= 400) throw new RuntimeException('combined document save failed: ' . wp_json_encode(is_wp_error($document_response) ? $document_response->get_error_data() : $document_response->get_data()));
$document_data = $document_response->get_data();
$seen = [];
$fail_title = (string) ($patches[1]['after']['title'] ?? '');
$failure_hook = static function($document) use (&$seen, $fail_title) {
  if (!is_object($document) || !is_callable([$document, 'get_main_id'])) return;
  $post = get_post((int) $document->get_main_id());
  if ($post && $post->post_title === $fail_title) {
    $seen[] = (int) $post->ID;
    throw new RuntimeException('combined second-resource after_save fixture');
  }
};
add_action('elementor/document/after_save', $failure_hook, PHP_INT_MAX, 2);
try {
  $resource_request = new WP_REST_Request('POST', '/mcp-for-page-builders/v1/resources/apply');
  $resource_request->set_header('Content-Type', 'application/json');
  $resource_request->set_body(wp_json_encode(['operation_id'=>$operation_id,'page_id'=>$page_id,'expected_revision'=>$document_data['revision'],'original_expected_revision'=>$original_revision,'document_patch'=>$document,'resource_patches'=>$patches,'document_snapshot'=>true]));
  $resource_response = rest_do_request($resource_request);
} finally {
  remove_action('elementor/document/after_save', $failure_hook, PHP_INT_MAX);
}
$resource_data = is_wp_error($resource_response) ? $resource_response->get_error_data() : $resource_response->get_data();
$journal = get_option('_mcp_elementor_resources_' . hash('sha256', $operation_id));
$document_receipt = get_option('_mcp_elementor_operation_' . hash('sha256', $operation_id));
$source = mcp_bridge_editor_guard_full_state($page_id);
if (is_wp_error($source)) throw new RuntimeException($source->get_error_message());
echo 'MCP_COMBINED_FAILURE:' . wp_json_encode(['status'=>$resource_data['status']??null,'error_code'=>$resource_data['error_code']??null,'completed'=>$resource_data['completed']??[],'failed_resource'=>$resource_data['failed_resource']??null,'restore_operation_id'=>$resource_data['restore_operation_id']??null,'hook_seen'=>$seen,'journal'=>$journal,'document_receipt'=>$document_receipt,'source_fingerprint'=>mcp_bridge_editor_guard_state_fingerprint($source)]);"#;
    let output = std::process::Command::new("docker")
        .args([
            "exec",
            "-u",
            config().docker_user.as_str(),
            "-e",
            &format!("WP_TEST_USER={}", config().user),
            "-e",
            &format!("MCP_PAGE_ID={page_id}"),
            "-e",
            &format!("MCP_OPERATION_ID={operation_id}"),
            "-e",
            &format!("MCP_ORIGINAL_REVISION={original_revision}"),
            "-e",
            &format!("MCP_DOCUMENT={}", serde_json::to_string(document)?),
            "-e",
            &format!(
                "MCP_RESOURCE_PATCHES={}",
                serde_json::to_string(resource_patches)?
            ),
            config().container.as_str(),
            "wp",
            config().wp_path_arg.as_str(),
            "eval",
            php,
        ])
        .output()
        .context("run combined second-resource after_save fixture")?;
    anyhow::ensure!(
        output.status.success(),
        "combined after_save fixture failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout)?;
    let (_, json) = stdout
        .rsplit_once("MCP_COMBINED_FAILURE:")
        .context("combined after_save fixture omitted its result marker")?;
    serde_json::from_str(json.trim()).context("parse combined after_save fixture result")
}

#[tokio::test]
async fn resource_plan_binds_the_existing_source_revision() {
    let _guard = TRANSACTION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let wp = wp();
    let marker = format!("MCP source CAS {}", std::process::id());
    let created = wp.post("wp/v2/pages", &json!({"title":marker,"status":"draft","meta":{"_elementor_data":"[]","_elementor_edit_mode":"builder"}})).await.expect("create exact source CAS draft");
    let page_id = created["id"].as_u64().expect("source CAS page ID");
    let mut cleanup = ExactPageCleanup::new(&wp, page_id);
    let mut operation_cleanup = ExactOperationCleanup::new();
    let document_route = format!("mcp-for-page-builders/v1/document/{page_id}");
    let settings_route = format!("mcp-for-page-builders/v1/resources/page_settings/{page_id}");
    let source_before = wp.get(&document_route).await.expect("read source revision");
    let resource_before = wp
        .get(&settings_route)
        .await
        .expect("read resource revision");
    let patch = json!([{"kind":"page_settings","id":page_id.to_string(),"expected_revision":resource_before["revision"],"after":{"hide_title":"yes"}}]);
    let operation_id = format!("source_cas_{}", std::process::id());
    operation_cleanup.track(&operation_id);

    for params in [
        json!({"operation_id":operation_id,"page_id":page_id,"resource_patches":patch,"validate_only":true}),
        json!({"operation_id":operation_id,"page_id":page_id,"expected_revision":false,"resource_patches":patch,"validate_only":true}),
    ] {
        assert!(
            wp.request(
                "POST",
                "mcp-for-page-builders/v1/resources/apply",
                Some(&params),
                None
            )
            .await
            .is_err(),
            "missing or invalid source revision must reject"
        );
    }
    let valid_preflight = json!({"operation_id":operation_id,"page_id":page_id,"expected_revision":source_before["revision"],"resource_patches":patch,"validate_only":true});
    wp.request(
        "POST",
        "mcp-for-page-builders/v1/resources/apply",
        Some(&valid_preflight),
        None,
    )
    .await
    .expect("validate source and resource revisions");
    let same_source_operation = format!("same_source_{}", std::process::id());
    operation_cleanup.track(&same_source_operation);
    let same_source = json!({"operation_id":same_source_operation,"page_id":page_id,"expected_revision":source_before["revision"],"resource_patches":patch});
    let applied = wp
        .request(
            "POST",
            "mcp-for-page-builders/v1/resources/apply",
            Some(&same_source),
            None,
        )
        .await
        .expect("same-source resource update");
    assert_eq!(applied["applied"], true);
    let replay = wp
        .request(
            "POST",
            "mcp-for-page-builders/v1/resources/apply",
            Some(&same_source),
            None,
        )
        .await
        .expect("same-source idempotent replay");
    assert_eq!(replay["idempotent_replay"], true);
    let restored = wp
        .request(
            "POST",
            "mcp-for-page-builders/v1/resources/restore",
            Some(&json!({"operation_id":same_source_operation})),
            None,
        )
        .await
        .expect("restore same-source resource update");
    assert_eq!(restored["restored"], true);
    let planned_document = json!([{"id":"planned1","elType":"widget","widgetType":"heading","settings":{"title":"planned document"},"elements":[]}]);
    let planned_save = wp.request("POST", &format!("mcp-for-page-builders/v1/document/{page_id}/apply"), Some(&json!({"operation_id":operation_id,"expected_revision":source_before["revision"],"document":planned_document})), None).await.expect("apply planned document revision");
    let foreign_document = json!([{"id":"foreign1","elType":"widget","widgetType":"heading","settings":{"title":"foreign document"},"elements":[]}]);
    let foreign_operation = format!("foreign_{}", std::process::id());
    operation_cleanup.track(&foreign_operation);
    let foreign_save = wp.request("POST", &format!("mcp-for-page-builders/v1/document/{page_id}/apply"), Some(&json!({"operation_id":foreign_operation,"expected_revision":planned_save["revision"],"document":foreign_document})), None).await.expect("apply intervening foreign document revision");
    assert_eq!(foreign_save["saved"], true);
    let stale_params = json!({"operation_id":operation_id,"page_id":page_id,"expected_revision":planned_save["revision"],"original_expected_revision":source_before["revision"],"document_patch":planned_document,"document_snapshot":true,"resource_patches":patch});
    assert!(
        wp.request(
            "POST",
            "mcp-for-page-builders/v1/resources/apply",
            Some(&stale_params),
            None
        )
        .await
        .is_err(),
        "late stale source must reject before resource writes"
    );
    assert_eq!(
        wp.get(&document_route)
            .await
            .expect("source after late stale rejection")["revision"],
        foreign_save["revision"]
    );
    assert_eq!(
        wp.get(&settings_route)
            .await
            .expect("resource after stale validation")["revision"],
        resource_before["revision"]
    );

    cleanup.finish().await;
}

#[tokio::test]
async fn page_metadata_revision_write_stale_rejection_and_restore() {
    let _guard = TRANSACTION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let wp = wp();
    let marker = format!(
        "MCP metadata roundtrip {}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    println!("metadata fixture marker before POST: {marker}");
    let created = wp.post("wp/v2/pages", &json!({"title":marker,"status":"draft","meta":{"_elementor_data":"[]","_elementor_edit_mode":"builder"}})).await.expect("create isolated draft page");
    let id = created["id"].as_u64().expect("draft page ID");
    println!("metadata fixture page_id={id}");
    let mut fixture_cleanup = ExactPageCleanup::new(&wp, id);
    let read = GetPage
        .run(json!({"id":id}), &wp)
        .await
        .expect("read page metadata revision");
    let page: serde_json::Value =
        serde_json::from_str(tool_text(&read)).expect("decode get_page WordPress response text");
    let revision = page["metadata_revision"]
        .as_str()
        .expect("metadata revision");
    let operation_id = format!(
        "metadata_roundtrip_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let updated = UpdatePage.run(json!({"id":id,"title":"MCP metadata roundtrip after","expected_metadata_revision":revision,"operation_id":operation_id}), &wp).await.expect("checked metadata update");
    assert_eq!(updated.is_error, None);
    let current_metadata = wp
        .get(&format!(
            "mcp-for-page-builders/v1/resources/post_metadata/{id}"
        ))
        .await
        .expect("read metadata after checked update");
    let stale = UpdatePage.run(json!({"id":id,"title":"MCP stale metadata write","expected_metadata_revision":revision,"operation_id":format!("metadata_stale_{}", std::process::id())}), &wp).await;
    assert!(
        stale.is_err(),
        "stale metadata revision must fail before write: {}",
        stale
            .err()
            .map(|error| error.to_string())
            .unwrap_or_else(|| "unexpected successful response".to_owned())
    );
    let after_stale = wp
        .get(&format!(
            "mcp-for-page-builders/v1/resources/post_metadata/{id}"
        ))
        .await
        .expect("read metadata after stale write");
    assert_eq!(
        after_stale["revision"], current_metadata["revision"],
        "stale write must not change metadata"
    );
    let restored = wp
        .request(
            "POST",
            "mcp-for-page-builders/v1/resources/restore",
            Some(&json!({"operation_id":operation_id})),
            None,
        )
        .await
        .expect("restore metadata snapshot");
    assert_eq!(restored["restored"], true);
    let final_page = wp
        .get(&format!("wp/v2/pages/{id}?context=edit"))
        .await
        .expect("read restored page");
    assert_eq!(final_page["title"]["raw"], marker);
    fixture_cleanup.finish().await;
}

#[tokio::test]
async fn page_tree_and_metadata_restore_together_after_stale_metadata_is_rejected() {
    let _guard = TRANSACTION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let wp = wp();
    let marker = format!(
        "MCP combined before {}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    println!("combined fixture marker before POST: {marker}");
    let created = wp.post("wp/v2/pages", &json!({"title":marker,"status":"draft","meta":{"_elementor_data":"[]","_elementor_edit_mode":"builder"}})).await.expect("create isolated draft page");
    let id = created["id"].as_u64().expect("draft page ID");
    println!("combined fixture page_id={id}");
    let mut fixture_cleanup = ExactPageCleanup::new(&wp, id);
    let before_document = wp
        .get(&format!("mcp-for-page-builders/v1/document/{id}"))
        .await
        .expect("read original document");
    let before_metadata = wp
        .get(&format!(
            "mcp-for-page-builders/v1/resources/post_metadata/{id}"
        ))
        .await
        .expect("read original metadata");
    let operation_id = format!(
        "combined_restore_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let changed_document = json!([{"id":"combined1","elType":"widget","widgetType":"heading","settings":{"title":"changed"},"elements":[]}]);
    let applied = UpdatePage.run(json!({"id":id,"title":"MCP combined after","elementor_data":changed_document.to_string(),"expected_revision":before_document["revision"],"expected_metadata_revision":before_metadata["revision"],"operation_id":operation_id}), &wp).await.expect("apply checked combined update");
    println!("combined update receipt: {}", tool_text(&applied));
    assert_eq!(applied.is_error, None);
    let stale = UpdatePage.run(json!({"id":id,"elementor_data":changed_document.to_string(),"expected_revision":before_document["revision"],"expected_metadata_revision":before_metadata["revision"],"operation_id":format!("stale_combined_{}",std::process::id())}), &wp).await.expect("stale revision rejection");
    assert_eq!(
        stale.is_error,
        Some(true),
        "stale metadata must reject before another tree write"
    );
    let current_document = wp
        .get(&format!("mcp-for-page-builders/v1/document/{id}"))
        .await
        .expect("read updated document");
    let restored = RestoreElementorSnapshot.run(json!({"page_id":id,"operation_id":operation_id,"expected_revision":current_document["revision"]}), &wp).await.expect("restore combined snapshot");
    assert_eq!(restored.is_error, None);
    let after_document = wp
        .get(&format!("mcp-for-page-builders/v1/document/{id}"))
        .await
        .expect("read restored document");
    let after_metadata = wp
        .get(&format!(
            "mcp-for-page-builders/v1/resources/post_metadata/{id}"
        ))
        .await
        .expect("read restored metadata");
    assert_eq!(after_document["document"], before_document["document"]);
    assert_eq!(
        after_metadata["value"]["title"],
        before_metadata["value"]["title"]
    );
    fixture_cleanup.finish().await;
}

#[tokio::test]
async fn checked_class_collection_rejects_stale_revision_and_restores_native_roundtrip() {
    let _guard = TRANSACTION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let wp = wp();
    let route = "mcp-for-page-builders/v1/resources/class/collection";
    let state = wp.get(route).await.expect("read native global classes");
    let revision = state["revision"]
        .as_str()
        .expect("class collection revision");
    let collection = state["value"].clone();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let operation_id = format!("class_roundtrip_{}_{nonce}", std::process::id());
    let later_operation_id = format!("class_later_{}_{nonce}", std::process::id());

    let stale = wp.request("POST", "mcp-for-page-builders/v1/resources/apply", Some(&json!({
        "operation_id": operation_id, "expected_revision":"absent",
        "resource_patches": [{"kind":"class","id":"collection","expected_revision":"stale","after":collection}]
    })), None).await;
    assert!(
        stale.is_err(),
        "stale class revision must be rejected before write"
    );

    let mut after_first = collection.clone();
    if !after_first["items"].is_object() {
        after_first["items"] = json!({});
    }
    after_first["items"]["mcpclass1"] = json!({"id":"mcpclass1","type":"class","label":"mcp-roundtrip","variants":[{"meta":{"state":null,"breakpoint":"desktop"},"props":{}}]});
    after_first["order"]
        .as_array_mut()
        .unwrap()
        .push(json!("mcpclass1"));
    let applied = wp.request("POST", "mcp-for-page-builders/v1/resources/apply", Some(&json!({
        "operation_id": operation_id, "expected_revision":"absent",
        "resource_patches": [{"kind":"class","id":"collection","expected_revision":revision,"after":after_first}]
    })), None).await.expect("native class collection write");
    assert_eq!(applied["applied"], true);
    let after_revision = applied["resources"][0]["after_revision"].as_str().unwrap();
    let reused = wp.request("POST", "mcp-for-page-builders/v1/resources/apply", Some(&json!({
        "operation_id": operation_id, "expected_revision":"absent",
        "resource_patches": [{"kind":"class","id":"collection","expected_revision":revision,"after":{"different":"payload"}}]
    })), None).await;
    assert!(
        reused.is_err(),
        "an operation ID must not replay for a different resource payload"
    );

    let after_first_state = wp
        .get(route)
        .await
        .expect("read first applied class collection");
    let mut after_second = after_first.clone();
    after_second["items"]["mcpclass1"]["label"] = json!("mcp-roundtrip-later");
    let applied_later = wp.request("POST", "mcp-for-page-builders/v1/resources/apply", Some(&json!({
        "operation_id": later_operation_id, "expected_revision":"absent",
        "resource_patches": [{"kind":"class","id":"collection","expected_revision":after_first_state["revision"],"after":after_second}]
    })), None).await.expect("apply later editor-style class update");
    assert_eq!(applied_later["applied"], true);
    let stale_restore = wp
        .request(
            "POST",
            "mcp-for-page-builders/v1/resources/restore",
            Some(&json!({"operation_id":operation_id})),
            None,
        )
        .await;
    assert!(
        stale_restore.is_err(),
        "restore must refuse to overwrite a later resource revision"
    );
    let restored_later = wp
        .request(
            "POST",
            "mcp-for-page-builders/v1/resources/restore",
            Some(&json!({"operation_id":later_operation_id})),
            None,
        )
        .await
        .expect("restore later class update");
    assert_eq!(restored_later["restored"], true);
    let restored = wp
        .request(
            "POST",
            "mcp-for-page-builders/v1/resources/restore",
            Some(&json!({"operation_id":operation_id})),
            None,
        )
        .await
        .expect("restore original native class collection");
    assert_eq!(restored["restored"], true);
    let after = wp.get(route).await.expect("read restored class collection");
    assert_eq!(after["revision"], revision);
    assert!(!after_revision.is_empty());
    let missing = wp
        .get("mcp-for-page-builders/v1/resources/template/999999999")
        .await;
    assert!(
        missing.is_err(),
        "missing templates must not be treated as empty resources"
    );
}

#[tokio::test]
async fn native_variable_batch_and_draft_template_roundtrip_restore_cleanly() {
    let _guard = TRANSACTION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let wp = wp();
    let variables_route = "mcp-for-page-builders/v1/resources/variable/collection";
    let before = wp
        .get(variables_route)
        .await
        .expect("read native variables");
    let variable_operation = format!(
        "variable_roundtrip_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let variable_patch = json!([{
        "kind":"variable", "id":"collection", "expected_revision":before["revision"],
        "after":{"operations":[
            {"type":"create","variable":{"id":"temporary_color","type":"global-color-variable","label":"mcptransactiontest","value":"#123456"}},
            {"type":"update","id":"temporary_color","variable":{"label":"mcptransactiontestupdated","value":"#654321"}}
        ]}
    }]);
    wp.request("POST", "mcp-for-page-builders/v1/resources/apply", Some(&json!({"operation_id":variable_operation,"expected_revision":"absent","resource_patches":variable_patch,"validate_only":true})), None)
        .await.expect("native variable preflight accepts generated-ID update");
    let applied = wp
        .request(
            "POST",
            "mcp-for-page-builders/v1/resources/apply",
            Some(&json!({"operation_id":variable_operation,"expected_revision":"absent","resource_patches":variable_patch})),
            None,
        )
        .await
        .expect("create and update through Elementor's variables repository");
    let generated = applied["resources"][0]["generated_ids"]
        .as_array()
        .expect("generated variable ID mapping");
    let variable_id = generated
        .iter()
        .find(|entry| entry["logical_id"] == "temporary_color")
        .and_then(|entry| entry["id"].as_str())
        .expect("returned variable ID");
    let created = wp
        .get(&format!(
            "mcp-for-page-builders/v1/resources/variable/{variable_id}"
        ))
        .await
        .expect("read created variable");
    assert_eq!(created["value"]["label"], "mcptransactiontestupdated");
    assert_eq!(created["value"]["value"], "#654321");
    let restored = wp
        .request(
            "POST",
            "mcp-for-page-builders/v1/resources/restore",
            Some(&json!({"operation_id":variable_operation})),
            None,
        )
        .await
        .expect("restore original variable collection");
    assert_eq!(restored["restored"], true);
    let after = wp
        .get(variables_route)
        .await
        .expect("read restored variables");
    assert_eq!(after["value"]["data"], before["value"]["data"]);
    assert_eq!(after["value"]["version"], before["value"]["version"]);

    let template_operation = format!(
        "template_roundtrip_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let document = json!([{"id":"mcpdraft1","elType":"widget","widgetType":"heading","settings":{"title":"MCP draft content"},"elements":[]}]);
    let imported = wp.request("POST", "mcp-for-page-builders/v1/resources/apply", Some(&json!({
        "operation_id":template_operation,
        "expected_revision":"absent",
        "resource_patches":[{"kind":"template","id":"new","expected_revision":"absent","after":{"title":"MCP draft roundtrip","status":"draft","type":"section","document":document}}]
    })), None).await.expect("import an isolated draft template");
    let template_id = imported["resources"][0]["id"]
        .as_str()
        .expect("created draft template ID");
    let exported = wp
        .get(&format!(
            "mcp-for-page-builders/v1/resources/template/{template_id}"
        ))
        .await
        .expect("export draft template resource");
    assert_eq!(exported["value"]["status"], "draft");
    assert_eq!(exported["value"]["document"], document);
    let restored = wp
        .request(
            "POST",
            "mcp-for-page-builders/v1/resources/restore",
            Some(&json!({"operation_id":template_operation})),
            None,
        )
        .await
        .expect("delete created draft template during restore");
    assert_eq!(restored["restored"], true);
    assert!(
        wp.get(&format!(
            "mcp-for-page-builders/v1/resources/template/{template_id}"
        ))
        .await
        .is_err(),
        "restored draft template must be deleted"
    );
}

#[tokio::test]
async fn invalid_document_and_resource_batch_leave_both_unchanged() {
    let _guard = TRANSACTION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let wp = wp();
    let classes_route = "mcp-for-page-builders/v1/resources/class/collection";
    let classes_before = wp
        .get(classes_route)
        .await
        .expect("read global classes before plan");
    let page = wp
        .post(
            "wp/v2/pages",
            &json!({"title":"MCP invalid plan draft","status":"draft"}),
        )
        .await
        .expect("create isolated draft page");
    let page_id = page["id"].as_u64().expect("draft page ID");
    let document_before = wp
        .get(&format!("mcp-for-page-builders/v1/document/{page_id}"))
        .await
        .expect("read draft document");
    let schema = wp
        .get("mcp-for-page-builders/v1/element-schema/e-heading")
        .await
        .expect("read native heading schema");
    let property = schema["initial_config"]["atomic_props_schema"]
        .as_object()
        .expect("native Atomic property schema")
        .keys()
        .next()
        .expect("Atomic heading property")
        .clone();
    let operation_id = format!(
        "invalid_plan_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let invalid_document = json!([{"id":"invalidatom1","elType":"widget","widgetType":"e-heading","settings":{(property):{"$$type":"invalid_type","value":"malformed"}},"elements":[]}]);
    let plan = json!({
        "site_url":wp.base_url(), "page_id":page_id, "expected_revision":document_before["revision"], "operation_id":operation_id,
        "document_patch":invalid_document,
        "resource_patches":[{"kind":"class","id":"collection","expected_revision":classes_before["revision"],"after":classes_before["value"]}]
    });
    let result = ApplyElementorPlan.run(json!({"plan":plan}), &wp).await;
    let document_after = wp
        .get(&format!("mcp-for-page-builders/v1/document/{page_id}"))
        .await
        .expect("read draft after rejected plan");
    let classes_after = wp
        .get(classes_route)
        .await
        .expect("read global classes after rejected plan");
    let deleted = wp
        .delete(&format!("wp/v2/pages/{page_id}?force=true"))
        .await
        .expect("delete isolated draft page");
    assert_eq!(deleted["deleted"], true);
    assert!(
        wp.get(&format!("wp/v2/pages/{page_id}")).await.is_err(),
        "isolated draft page must be deleted"
    );
    assert!(
        result.is_err() || result.unwrap().is_error == Some(true),
        "invalid Atomic document must reject the complete plan"
    );
    assert_eq!(
        document_after["document"], document_before["document"],
        "invalid document changed the draft"
    );
    assert_eq!(
        classes_after["revision"], classes_before["revision"],
        "invalid document changed the resource batch"
    );
}

#[tokio::test]
async fn plan_verification_runs_after_save_and_failed_expectations_are_restorable() {
    let _guard = TRANSACTION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let wp = wp();
    let marker = format!(
        "MCP plan verification {}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    println!("plan verification fixture marker before POST: {marker}");
    let page = wp.post("wp/v2/pages", &json!({"title":marker,"status":"publish","template":"elementor_canvas","meta":{"_elementor_data":"[]","_elementor_edit_mode":"builder"}})).await.expect("create temporary published page on the MCP integration clone");
    let page_id = page["id"].as_u64().expect("temporary page ID");
    println!("plan verification fixture page_id={page_id}");
    let mut fixture_cleanup = ExactPageCleanup::new(&wp, page_id);
    let result: anyhow::Result<()> = async {
        anyhow::ensure!(page["template"] == "elementor_canvas", "page must use the native Elementor Canvas template");
        let document_route = format!("mcp-for-page-builders/v1/document/{page_id}");
        let before = wp.get(&document_route).await?;
        let operation_id = format!("plan_verify_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos());
        let document = json!([{"id":"verifycnt1","elType":"container","settings":{"content_width":"boxed","boxed_width":{"unit":"px","size":320,"sizes":[]},"padding":{"unit":"px","top":"0","right":"0","bottom":"0","left":"0","isLinked":true}},"elements":[{"id":"verify01","elType":"widget","widgetType":"heading","settings":{"title":"Plan check","header_size":"h1","title_color":"#123456","typography_typography":"custom","typography_font_size":{"unit":"px","size":16,"sizes":[]},"typography_line_height":{"unit":"em","size":1.25,"sizes":[]}},"elements":[]}]}]);
        let base = json!({"site_url":wp.base_url(),"page_id":page_id,"expected_revision":before["revision"],"operation_id":operation_id,"document_patch":document});
        let mut initial_save = base.clone();
        initial_save["operation_id"] = json!(format!("plan_verify_initial_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()));
        ApplyElementorPlan.run(json!({"plan":initial_save}), &wp).await?;
        let page_link = page["link"].as_str().context("created page link")?;
        let html = reqwest::Client::builder().danger_accept_invalid_certs(std::env::var("WP_TLS_INSECURE").is_ok()).build()?
            .get(page_link).send().await?.text().await?;
        anyhow::ensure!(html.contains("Plan check"), "rendered Canvas HTML must contain the visible heading before verification checks");
        let mut successful_plan = base.clone();
        successful_plan["expected_revision"] = wp.get(&document_route).await?["revision"].clone();
        successful_plan["checks"] = json!([
            {"check":"element_present","element_id":"verify01"},
            {"check":"computed_style","selector":".elementor-element-verify01 .elementor-heading-title","property":"color","expected":"rgb(18, 52, 86)"},
            {"check":"no_horizontal_overflow","viewport":{"width":900,"height":900}},
            {"check":"page_height","expected":900,"max_delta_percent":2,"viewport":{"width":900,"height":900}},
            {"check":"bounding_box","selector":".elementor-element-verify01 .elementor-heading-title","width":320,"height":20,"tolerance_px":1,"viewport":{"width":900,"height":900}}
        ]);
        let applied = ApplyElementorPlan.run(json!({"plan":successful_plan}), &wp).await?;
        println!("successful verification receipt: {}", serde_json::to_value(&applied)?["content"][0]["text"].as_str().context("verification receipt text")?);
        anyhow::ensure!(applied.is_error.is_none(), "successful expectations should pass");
        let mut failure_plan = base.clone();
        failure_plan["expected_revision"] = wp.get(&document_route).await?["revision"].clone();
        failure_plan["operation_id"] = json!(format!("plan_verify_fail_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()));
        failure_plan["checks"] = json!([{"check":"computed_style","selector":".elementor-element-verify01 .elementor-heading-title","property":"color","expected":"rgb(0, 0, 0)"}]);
        let failed = ApplyElementorPlan.run(json!({"plan":failure_plan}), &wp).await?;
        println!("failed verification receipt: {}", serde_json::to_value(&failed)?["content"][0]["text"].as_str().context("failed verification receipt text")?);
        anyhow::ensure!(failed.is_error == Some(true), "deliberate expectation must fail after save");
        let failure_text = serde_json::to_string(&failed.content)?;
        anyhow::ensure!(failure_text.contains("verification_failed") && failure_text.contains("restore_operation_id"), "failure must include restore pointer: {failure_text}");
        let current = wp.get(&document_route).await?;
        let restored = RestoreElementorSnapshot.run(json!({"page_id":page_id,"operation_id":failure_plan["operation_id"],"expected_revision":current["revision"]}), &wp).await?;
        anyhow::ensure!(restored.is_error.is_none(), "restore deliberately failed-verification write");
        let unchanged = wp.get(&document_route).await?;
        let invalid_revision = unchanged["revision"].clone();
        let mut invalid_plan = base;
        invalid_plan["expected_revision"] = invalid_revision.clone();
        invalid_plan["operation_id"] = json!(format!("plan_verify_invalid_{}", std::process::id()));
        invalid_plan["checks"] = json!([{"check":"unknown_check"}]);
        let rejected = ApplyElementorPlan.run(json!({"plan":invalid_plan}), &wp).await;
        anyhow::ensure!(rejected.is_err(), "unknown checks must reject during preflight");
        let after_rejection = wp.get(&document_route).await?;
        anyhow::ensure!(after_rejection["revision"] == invalid_revision, "invalid check changed the document revision");
        Ok(())
    }.await;
    fixture_cleanup.finish().await;
    result.unwrap_or_else(|error| panic!("{error:#}"));
}

#[tokio::test]
async fn resource_revision_failure_after_document_write_is_reported_and_restorable() {
    let _guard = TRANSACTION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let wp = wp();
    let classes_route = "mcp-for-page-builders/v1/resources/class/collection";
    let classes_before = wp
        .get(classes_route)
        .await
        .expect("read classes before partial plan");
    let mut plan_classes = classes_before["value"].clone();
    if !plan_classes["items"].is_object() {
        plan_classes["items"] = json!({});
    }
    plan_classes["items"]["mcpclass2"] = json!({"id":"mcpclass2","type":"class","label":"mcp-plan-update","variants":[{"meta":{"state":null,"breakpoint":"desktop"},"props":{}}]});
    plan_classes["order"]
        .as_array_mut()
        .unwrap()
        .push(json!("mcpclass2"));
    let page = wp
        .post(
            "wp/v2/pages",
            &json!({
                "title":"MCP partial plan draft", "status":"draft",
                "meta":{"_elementor_data":"[]","_elementor_edit_mode":"builder"}
            }),
        )
        .await
        .expect("create isolated draft page");
    let page_id = page["id"].as_u64().expect("draft page ID");
    let document_route = format!("mcp-for-page-builders/v1/document/{page_id}");
    let before = wp.get(&document_route).await.expect("read draft document");
    let plan_operation = format!(
        "partial_plan_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let race_operation = format!(
        "resource_race_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let planned_document = json!([{"id":"planbase1","elType":"widget","widgetType":"heading","settings":{"title":"planned document"},"elements":[]}]);
    let patch = json!([{"kind":"class","id":"collection","expected_revision":classes_before["revision"],"after":plan_classes}]);
    let request = json!({"operation_id":plan_operation,"page_id":page_id,"expected_revision":before["revision"],"document_patch":planned_document,"resource_patches":patch,"document_snapshot":true});
    let mut preflight = request.clone();
    preflight["validate_only"] = json!(true);
    wp.request(
        "POST",
        "mcp-for-page-builders/v1/resources/apply",
        Some(&preflight),
        None,
    )
    .await
    .expect("preflight complete document and resource plan");
    let document_write = wp.request("POST", &format!("mcp-for-page-builders/v1/document/{page_id}/apply"), Some(&json!({
        "operation_id":plan_operation,"expected_revision":before["revision"],"document":planned_document
    })), None).await.expect("write valid draft document before inducing a stale resource revision");
    assert_eq!(document_write["saved"], true);

    let mut race_classes = classes_before["value"].clone();
    if !race_classes["items"].is_object() {
        race_classes["items"] = json!({});
    }
    race_classes["items"]["mcpclass2"] = json!({"id":"mcpclass2","type":"class","label":"mcp-race-update","variants":[{"meta":{"state":null,"breakpoint":"desktop"},"props":{}}]});
    race_classes["order"]
        .as_array_mut()
        .unwrap()
        .push(json!("mcpclass2"));
    let raced = wp.request("POST", "mcp-for-page-builders/v1/resources/apply", Some(&json!({
        "operation_id":race_operation,
        "expected_revision":"absent",
        "resource_patches":[{"kind":"class","id":"collection","expected_revision":classes_before["revision"],"after":race_classes}]
    })), None).await.expect("apply intervening class update");
    assert_eq!(raced["applied"], true);
    let stale_resources = wp.request("POST", "mcp-for-page-builders/v1/resources/apply", Some(&json!({
        "operation_id":plan_operation,"page_id":page_id,"expected_revision":document_write["revision"],"original_expected_revision":before["revision"],"document_patch":planned_document,"document_snapshot":true,"resource_patches":patch
    })), None).await;
    assert!(
        stale_resources.is_err(),
        "a resource revision conflict after the document write must be reported"
    );
    let applied_document = wp
        .get(&document_route)
        .await
        .expect("read persisted document before restore");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(applied_document["document"].as_str().unwrap())
            .unwrap(),
        planned_document
    );
    let applied_classes = wp
        .get(classes_route)
        .await
        .expect("read intervening class update");
    assert_eq!(
        applied_classes["revision"],
        raced["resources"][0]["after_revision"]
    );

    let document_restored = wp
        .request(
            "POST",
            &format!("mcp-for-page-builders/v1/document/{page_id}/restore"),
            Some(&json!({
                "operation_id":plan_operation,"expected_revision":applied_document["revision"]
            })),
            None,
        )
        .await
        .expect("revision-aware document restore");
    assert_eq!(document_restored["restored"], true);
    assert_eq!(
        document_restored["cache_regenerated"], true,
        "restore must verify regenerated Elementor CSS"
    );
    let resource_restored = wp
        .request(
            "POST",
            "mcp-for-page-builders/v1/resources/restore",
            Some(&json!({"operation_id":race_operation})),
            None,
        )
        .await
        .expect("revision-aware resource restore");
    assert_eq!(resource_restored["restored"], true);
    let after_document = wp
        .get(&document_route)
        .await
        .expect("read restored document");
    let after_classes = wp.get(classes_route).await.expect("read restored classes");
    let deleted = wp
        .delete(&format!("wp/v2/pages/{page_id}?force=true"))
        .await
        .expect("delete isolated draft page");
    assert_eq!(deleted["deleted"], true);
    assert!(
        wp.get(&format!("wp/v2/pages/{page_id}")).await.is_err(),
        "isolated draft page must be deleted"
    );
    assert_eq!(after_document["document"], before["document"]);
    assert_eq!(after_classes["revision"], classes_before["revision"]);
}

#[tokio::test]
async fn document_only_plan_records_restore_scope_and_missing_journal_stays_partial() {
    let _guard = TRANSACTION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let wp = wp();
    let marker = format!(
        "MCP document-only restore {}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    println!("document-only fixture marker before POST: {marker}");
    let page = wp.post("wp/v2/pages", &json!({"title":marker,"status":"draft","meta":{"_elementor_data":"[]","_elementor_edit_mode":"builder"}})).await.expect("create isolated draft page");
    let page_id = page["id"].as_u64().expect("draft page ID");
    println!("document-only fixture page_id={page_id}");
    let mut fixture_cleanup = ExactPageCleanup::new(&wp, page_id);
    let route = format!("mcp-for-page-builders/v1/document/{page_id}");
    let before = wp.get(&route).await.expect("read original draft document");
    let operation_id = format!(
        "document_only_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let applied = ApplyElementorPlan.run(json!({"plan":{
        "site_url":wp.base_url(),"page_id":page_id,"expected_revision":before["revision"],"operation_id":operation_id,
        "document_patch":[{"id":"doconly1","elType":"widget","widgetType":"heading","settings":{"title":"document only"},"elements":[]}]
    }}), &wp).await.expect("apply document-only plan");
    println!("document-only plan receipt: {}", tool_text(&applied));
    assert_eq!(
        applied.is_error, None,
        "document-only plan should apply and record an empty resource receipt"
    );
    let current = wp.get(&route).await.expect("read changed draft document");
    let restored = RestoreElementorSnapshot.run(json!({"page_id":page_id,"operation_id":operation_id,"expected_revision":current["revision"]}), &wp).await.expect("restore document-only plan");
    assert_eq!(restored.is_error, None);
    let restored_tree = wp.get(&route).await.expect("read restored draft document");
    assert_eq!(restored_tree["document"], before["document"]);

    let missing = RestoreElementorSnapshot.run(json!({"page_id":page_id,"operation_id":"missing_restore_journal","expected_revision":restored_tree["revision"]}), &wp).await.expect("missing journal must return a partial result");
    let missing_text =
        serde_json::to_string(&missing.content).expect("serialize missing journal result");
    assert!(
        missing_text.contains("partial_failure"),
        "missing resource journal must not report full success: {missing_text}"
    );
    fixture_cleanup.finish().await;
}

#[tokio::test]
async fn combined_document_metadata_plan_replays_and_restores_the_paired_source_state() {
    let _guard = TRANSACTION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let wp = wp();
    let marker = format!("MCP combined replay {}", std::process::id());
    let page = wp.post("wp/v2/pages", &json!({"title":marker,"status":"publish","template":"elementor_canvas","meta":{"_elementor_data":"[]","_elementor_edit_mode":"builder"}})).await.expect("create isolated published canvas page");
    let page_id = page["id"].as_u64().expect("draft page ID");
    let mut fixture_cleanup = ExactPageCleanup::new(&wp, page_id);
    let document_route = format!("mcp-for-page-builders/v1/document/{page_id}");
    let metadata_route = format!("mcp-for-page-builders/v1/resources/post_metadata/{page_id}");
    let before_document = wp.get(&document_route).await.expect("read source document");
    let before_metadata = wp
        .get(&metadata_route)
        .await
        .expect("read source post metadata");
    let mut operation_cleanup = ExactOperationCleanup::new();
    let operation_id = format!(
        "combined_replay_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    operation_cleanup.track(&operation_id);
    let native_before = combined_native_snapshot(page_id, &operation_id)
        .expect("capture native source state before combined plan");
    assert_eq!(
        native_before["hash_contract"]["style_order_differs"], true,
        "native style map ordering must affect request and source fingerprints"
    );
    assert_eq!(
        native_before["hash_contract"]["metadata_order_matches"], true,
        "ordinary metadata object ordering must remain canonical"
    );
    assert_eq!(
        native_before["hash_contract"]["native_node_fields_match"], true,
        "ordinary fields on a styled native node must remain canonical"
    );
    assert_eq!(
        native_before["hash_contract"]["legacy_styled_receipt_rejected"], true,
        "historical ordered-style receipts cannot approve a request whose original order was erased"
    );
    assert_eq!(
        native_before["hash_contract"]["legacy_metadata_receipt_accepted"], true,
        "unchanged historical non-style operations remain replayable"
    );
    assert_eq!(
        native_before["hash_contract"]["template_style_receipt_rejected"], true,
        "ordered styles nested in template resource after data must use the same history guard"
    );
    let document = json!([{"id":"combined1","elType":"e-flexbox","settings":{"classes":{"$$type":"classes","value":["z-native-order","a-native-order"]},"tag":{"$$type":"string","value":"div"}},"styles":{"z-native-order":{"id":"z-native-order","type":"class","label":"z-native-order","variants":[{"meta":{"breakpoint":"desktop","state":null},"props":{"color":{"$$type":"color","value":"#0000ff"}}}]},"a-native-order":{"id":"a-native-order","type":"class","label":"a-native-order","variants":[{"meta":{"breakpoint":"desktop","state":null},"props":{"color":{"$$type":"color","value":"#ff0000"}}}]}},"elements":[]}]);
    let plan = json!({
        "site_url":wp.base_url(), "page_id":page_id, "expected_revision":before_document["revision"], "operation_id":operation_id,
        "document_patch":document,
        "resource_patches":[{"kind":"post_metadata","id":page_id.to_string(),"expected_revision":before_metadata["revision"],"after":{"title":format!("{marker} updated"),"status":"publish"}}],
        "checks":[{"check":"computed_style","selector":".elementor-element-combined1","property":"color","expected":"rgb(255, 0, 0)"}]
    });
    let applied = ApplyElementorPlan
        .run(json!({"plan":plan}), &wp)
        .await
        .expect("apply combined plan");
    assert_eq!(
        applied.is_error,
        None,
        "combined plan should save both document and metadata: {}",
        tool_text(&applied)
    );
    let native_after_apply = combined_native_snapshot(page_id, &operation_id)
        .expect("capture native source state after combined plan");
    let journal = &native_after_apply["journal"];
    let document_receipt = &native_after_apply["document_receipt"];
    assert_eq!(
        journal["status"], "completed",
        "combined resource journal must complete"
    );
    assert_eq!(
        journal["source_before_resources_fingerprint"], document_receipt["after_fingerprint"],
        "resource phase must begin at the immutable document receipt state"
    );
    assert_eq!(
        journal["resources"][0]["after_revision"],
        wp.get(&metadata_route)
            .await
            .expect("read metadata revision after combined plan")["revision"],
        "journal must record the exact post-mutation resource revision"
    );
    let replay = ApplyElementorPlan
        .run(json!({"plan":plan}), &wp)
        .await
        .expect("replay exact combined plan");
    assert_eq!(
        replay.is_error,
        None,
        "exact completed plan should replay: {}",
        tool_text(&replay)
    );
    assert!(
        tool_text(&replay).contains("\"idempotent_replay\":true"),
        "replay must report its checked receipt: {}",
        tool_text(&replay)
    );
    let native_after_replay = combined_native_snapshot(page_id, &operation_id)
        .expect("capture native source state after completed replay");
    assert_eq!(
        native_after_replay["state"], native_after_apply["state"],
        "completed replay must preserve the full native source snapshot"
    );
    assert_eq!(
        native_after_replay["journal"]["source_after_fingerprint"],
        native_after_apply["journal"]["source_after_fingerprint"],
        "replay must retain the completed source fingerprint"
    );
    assert_eq!(
        native_after_replay["journal"]["resources"], native_after_apply["journal"]["resources"],
        "replay must retain every checked resource revision"
    );
    let mut reversed_plan = plan.clone();
    let style_map = reversed_plan["document_patch"][0]["styles"]
        .as_object_mut()
        .expect("native style map must be an object");
    let reversed_styles = style_map
        .iter()
        .rev()
        .map(|(id, style)| (id.clone(), style.clone()))
        .collect();
    *style_map = reversed_styles;
    let rejected_error = match ApplyElementorPlan
        .run(json!({"plan":reversed_plan}), &wp)
        .await
    {
        Ok(response) => panic!(
            "reversing only native style definition order must reject the reused operation ID: {}",
            tool_text(&response)
        ),
        Err(error) => error.to_string(),
    };
    assert!(
        rejected_error.contains("operation_id_conflict"),
        "style-order conflict must identify the operation ID conflict: {rejected_error}"
    );
    let native_after_conflict = combined_native_snapshot(page_id, &operation_id)
        .expect("capture native source state after rejected style-order replay");
    assert_eq!(
        native_after_conflict["state"], native_after_apply["state"],
        "rejected reordered replay must leave native document, revisions, CSS, and artifacts unchanged"
    );
    assert_eq!(
        native_after_conflict["journal"], native_after_apply["journal"],
        "rejected reordered replay must leave its resource journal unchanged"
    );
    let current_document = wp
        .get(&document_route)
        .await
        .expect("read combined document before restore");
    let restored = RestoreElementorSnapshot.run(json!({"page_id":page_id,"operation_id":operation_id,"expected_revision":current_document["revision"]}), &wp).await.expect("restore combined plan");
    assert_eq!(
        restored.is_error,
        None,
        "combined plan restore should succeed: {}",
        tool_text(&restored)
    );
    let after_document = wp
        .get(&document_route)
        .await
        .expect("read restored source document");
    let after_metadata = wp
        .get(&metadata_route)
        .await
        .expect("read restored source metadata");
    let native_after_restore = combined_native_snapshot(page_id, &operation_id)
        .expect("capture native source state after paired restore");
    assert_eq!(
        after_document["document"], before_document["document"],
        "restore must recover the exact original Elementor document"
    );
    assert_eq!(
        after_metadata["value"], before_metadata["value"],
        "restore must recover the exact original post metadata"
    );
    for field in ["revision_ids", "revisions", "css", "atomic_css"] {
        assert_eq!(
            native_after_restore["state"][field], native_before["state"][field],
            "paired restore must recover the exact original native {field} state"
        );
    }
    fixture_cleanup.finish().await;
}

#[tokio::test]
async fn combined_plan_inspects_and_restores_native_resource_after_lost_acknowledgement() {
    let _guard = TRANSACTION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let wp = wp();
    let marker = format!("MCP lost acknowledgement {}", std::process::id());
    let page = wp.post("wp/v2/pages", &json!({"title":marker,"status":"draft","meta":{"_elementor_data":"[]","_elementor_edit_mode":"builder"}})).await.expect("create isolated draft page");
    let page_id = page["id"].as_u64().expect("draft page ID");
    let mut fixture_cleanup = ExactPageCleanup::new(&wp, page_id);
    let document_route = format!("mcp-for-page-builders/v1/document/{page_id}");
    let metadata_route = format!("mcp-for-page-builders/v1/resources/post_metadata/{page_id}");
    let before_document = wp.get(&document_route).await.expect("read source document");
    let before_metadata = wp
        .get(&metadata_route)
        .await
        .expect("read source post metadata");
    let mut operation_cleanup = ExactOperationCleanup::new();
    let operation_id = format!(
        "lost_ack_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    operation_cleanup.track(&operation_id);
    let mut ack_fixture = ResourceAckFailureCleanup::install(&operation_id);
    let native_before = combined_native_snapshot(page_id, &operation_id)
        .expect("capture native source state before lost acknowledgement");
    let document = json!([{"id":"lostack01","elType":"widget","widgetType":"heading","settings":{"title":"Lost acknowledgement fixture"},"elements":[]}]);
    let plan = json!({
        "site_url":wp.base_url(), "page_id":page_id, "expected_revision":before_document["revision"], "operation_id":operation_id,
        "document_patch":document,
        "resource_patches":[{"kind":"post_metadata","id":page_id.to_string(),"expected_revision":before_metadata["revision"],"after":{"title":format!("{marker} updated"),"status":"draft"}}]
    });
    let outcome = ApplyElementorPlan
        .run(json!({"plan":plan}), &wp)
        .await
        .expect("lost acknowledgement is returned as an inspectable tool result");
    assert_eq!(
        outcome.is_error,
        Some(true),
        "lost native resource acknowledgement must report a partial failure: {}",
        tool_text(&outcome)
    );
    let outcome_data: Value =
        serde_json::from_str(&tool_text(&outcome)).expect("parse partial failure receipt");
    assert_eq!(
        outcome_data["status"], "partial_failure",
        "lost acknowledgement must require inspection"
    );
    assert_eq!(
        outcome_data["inspection_required"], true,
        "lost acknowledgement must require state inspection"
    );
    assert_eq!(
        outcome_data["mutations_made"], true,
        "committed native mutations must never be reported as absent"
    );
    assert_eq!(
        outcome_data["resource_restore_operation_id"], operation_id,
        "partial result must retain its resource restore pointer"
    );
    let native_after_failure = combined_native_snapshot(page_id, &operation_id)
        .expect("inspect native resource state after lost acknowledgement");
    assert_eq!(
        native_after_failure["ack_failure_marker"], "injected",
        "fixture must prove the acknowledgement was dropped after commit"
    );
    assert!(
        tool_text(&outcome).contains("mcp_fixture_ack_lost"),
        "native endpoint must return the injected post-commit HTTP error: {}",
        tool_text(&outcome)
    );
    assert_eq!(
        native_after_failure["journal"]["status"], "completed",
        "native resource journal must remain inspectable after the lost response"
    );
    assert_eq!(
        native_after_failure["journal"]["resources"][0]["after_revision"],
        wp.get(&metadata_route)
            .await
            .expect("inspect committed metadata resource")["revision"],
        "completed journal must bind the committed resource revision"
    );
    assert_eq!(
        wp.get(&metadata_route)
            .await
            .expect("read committed metadata")["value"]["title"],
        format!("{marker} updated"),
        "native resource mutation must have committed before the fixture drops its acknowledgement"
    );
    let current_document = wp
        .get(&document_route)
        .await
        .expect("read document before revision-aware restore");
    let restored = RestoreElementorSnapshot
        .run(json!({"page_id":page_id,"operation_id":operation_id,"expected_revision":current_document["revision"]}), &wp)
        .await
        .expect("restore inspected lost-acknowledgement operation");
    assert_eq!(
        restored.is_error,
        None,
        "revision-aware restore must recover the paired native snapshot: {}",
        tool_text(&restored)
    );
    let after_document = wp
        .get(&document_route)
        .await
        .expect("read restored source document");
    let after_metadata = wp
        .get(&metadata_route)
        .await
        .expect("read restored source metadata");
    let native_after_restore = combined_native_snapshot(page_id, &operation_id)
        .expect("capture native state after lost-acknowledgement restore");
    assert_eq!(
        after_document["document"], before_document["document"],
        "restore must recover the original document"
    );
    assert_eq!(
        after_metadata["value"], before_metadata["value"],
        "restore must recover the original metadata"
    );
    assert_eq!(
        native_after_restore["state"], native_before["state"],
        "restore must recover exact revisions, CSS, and Atomic artifacts"
    );
    ack_fixture.finish();
    fixture_cleanup.finish().await;
}

#[tokio::test]
async fn combined_plan_records_and_restores_after_second_resource_after_save_failure() {
    let _guard = TRANSACTION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let wp = wp();
    let marker = format!("MCP combined partial {}", std::process::id());
    let page = wp
        .post(
            "wp/v2/pages",
            &json!({"title":marker,"status":"draft","meta":{"_elementor_data":"[]","_elementor_edit_mode":"builder"}}),
        )
        .await
        .expect("create isolated combined partial draft");
    let page_id = page["id"].as_u64().expect("combined partial page ID");
    let mut fixture_cleanup = ExactPageCleanup::new(&wp, page_id);
    let document_route = format!("mcp-for-page-builders/v1/document/{page_id}");
    let metadata_route = format!("mcp-for-page-builders/v1/resources/post_metadata/{page_id}");
    let before_document = wp
        .get(&document_route)
        .await
        .expect("read combined partial source revision");
    let before_metadata = wp
        .get(&metadata_route)
        .await
        .expect("read combined partial metadata revision");
    let operation_id = format!(
        "combined_partial_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let mut operation_cleanup = ExactOperationCleanup::new();
    operation_cleanup.track(&operation_id);
    let native_before = combined_native_snapshot(page_id, &operation_id)
        .expect("capture source state before combined partial write");
    let document = json!([{"id":"partialdoc1","elType":"widget","widgetType":"heading","settings":{"title":"combined partial document"},"elements":[]}]);
    let fail_title = format!("{marker} second template fails");
    let resource_patches = json!([
        {"kind":"post_metadata","id":page_id.to_string(),"expected_revision":before_metadata["revision"],"after":{"title":format!("{marker} updated"),"status":"draft"}},
        {"kind":"template","id":"new_pattern_2","expected_revision":"absent","after":{"title":fail_title,"status":"draft","type":"section","document":[{"id":"partialtpl1","elType":"widget","widgetType":"heading","settings":{"title":"partial template"},"elements":[]}]}}
    ]);
    let partial = fail_second_combined_template_after_save(
        page_id,
        &operation_id,
        before_document["revision"]
            .as_str()
            .expect("document revision"),
        &document,
        &resource_patches,
    )
    .expect("record combined second-resource after_save failure");
    let mut resource_cleanup = ExactCreatedPostCleanup::new();
    let failed_template_id = partial["failed_resource"]["id"]
        .as_str()
        .and_then(|id| id.parse::<u64>().ok())
        .expect("after_save failure must report the exact created template ID");
    resource_cleanup.track(failed_template_id);
    let native_after_partial = combined_native_snapshot(page_id, &operation_id)
        .expect("capture paired source state after partial combined write");
    let current_document = wp
        .get(&document_route)
        .await
        .expect("read changed source before combined partial restore");
    let restored = RestoreElementorSnapshot
        .run(
            json!({"page_id":page_id,"operation_id":operation_id,"expected_revision":current_document["revision"]}),
            &wp,
        )
        .await
        .expect("restore partial combined plan");
    let native_after_restore = combined_native_snapshot(page_id, &operation_id)
        .expect("capture native source state after partial combined restore");

    assert_eq!(
        partial["status"], "partial_failure",
        "second-resource native failure must produce a partial journal"
    );
    assert_eq!(
        partial["error_code"], "resource_save_failed",
        "journal must preserve the native after_save error"
    );
    assert_eq!(
        partial["hook_seen"].as_array().map(Vec::len),
        Some(1),
        "the after_save fault must run once for the second resource"
    );
    assert_eq!(
        partial["completed"].as_array().map(Vec::len),
        Some(2),
        "journal must include the first resource and the observed second-resource write"
    );
    assert_eq!(partial["journal"]["status"], "partial");
    assert_eq!(partial["journal"]["source_page_id"], page_id);
    assert_eq!(
        partial["journal"]["source_before_resources_fingerprint"],
        partial["document_receipt"]["after_fingerprint"],
        "partial journal must retain its immutable paired document snapshot"
    );
    assert_eq!(
        partial["journal"]["source_after_fingerprint"], partial["source_fingerprint"],
        "partial journal must fingerprint the source after the first resource mutation"
    );
    assert_eq!(
        restored.is_error,
        None,
        "partial combined plan must restore safely: {}",
        tool_text(&restored)
    );
    assert_eq!(
        native_after_partial["journal"]["source_after_fingerprint"], partial["source_fingerprint"],
        "recorded partial fingerprint must match the native source readback"
    );
    assert_eq!(
        native_after_restore["state"], native_before["state"],
        "partial combined restore must recover the exact source post, metadata, revisions, CSS, and Atomic artifacts"
    );
    resource_cleanup.finish();
    fixture_cleanup.finish().await;
}

#[tokio::test]
async fn combined_plan_replay_and_restore_reject_foreign_source_metadata_with_same_tree() {
    let _guard = TRANSACTION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let wp = wp();
    let marker = format!("MCP combined conflict {}", std::process::id());
    let page = wp.post("wp/v2/pages", &json!({"title":marker,"status":"draft","meta":{"_elementor_data":"[]","_elementor_edit_mode":"builder"}})).await.expect("create isolated draft page");
    let page_id = page["id"].as_u64().expect("draft page ID");
    let mut fixture_cleanup = ExactPageCleanup::new(&wp, page_id);
    let document_route = format!("mcp-for-page-builders/v1/document/{page_id}");
    let metadata_route = format!("mcp-for-page-builders/v1/resources/post_metadata/{page_id}");
    let before_document = wp.get(&document_route).await.expect("read source document");
    let before_metadata = wp.get(&metadata_route).await.expect("read source metadata");
    let operation_id = format!(
        "combined_conflict_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let plan = json!({
        "site_url":wp.base_url(), "page_id":page_id, "expected_revision":before_document["revision"], "operation_id":operation_id,
        "document_patch":[{"id":"foreign1","elType":"widget","widgetType":"heading","settings":{"title":"paired state"},"elements":[]}],
        "resource_patches":[{"kind":"post_metadata","id":page_id.to_string(),"expected_revision":before_metadata["revision"],"after":{"title":format!("{marker} updated"),"status":"draft"}}]
    });
    let applied = ApplyElementorPlan
        .run(json!({"plan":plan}), &wp)
        .await
        .expect("apply combined plan");
    assert_eq!(
        applied.is_error, None,
        "combined plan should apply before the foreign write"
    );
    let tree_before_foreign_write = wp
        .get(&document_route)
        .await
        .expect("read tree before foreign metadata write");
    set_unknown_trash_fixture_meta(page_id, "foreign-after-combined-plan");
    let replay = ApplyElementorPlan.run(json!({"plan":plan}), &wp).await;
    assert!(
        replay.is_err()
            || replay
                .as_ref()
                .is_ok_and(|result| result.is_error == Some(true)),
        "same-tree foreign metadata must reject replay"
    );
    let tree_after_foreign_write = wp
        .get(&document_route)
        .await
        .expect("read tree after foreign metadata write");
    assert_eq!(
        tree_after_foreign_write["revision"], tree_before_foreign_write["revision"],
        "foreign write deliberately leaves the Elementor tree unchanged"
    );
    let restore = RestoreElementorSnapshot.run(json!({"page_id":page_id,"operation_id":operation_id,"expected_revision":tree_after_foreign_write["revision"]}), &wp).await;
    assert!(
        restore.is_err()
            || restore
                .as_ref()
                .is_ok_and(|result| result.is_error == Some(true)),
        "restore must reject the foreign source metadata write"
    );
    fixture_cleanup.finish().await;
}
