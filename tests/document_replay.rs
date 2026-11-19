use mcp_for_page_builders::wp::WpClient;
use serde_json::{Value, json};
use std::{process::Command, sync::OnceLock};

#[path = "native_test_config.rs"]
mod native_test_config;

fn config() -> &'static native_test_config::NativeTestConfig {
    static CONFIG: OnceLock<native_test_config::NativeTestConfig> = OnceLock::new();
    CONFIG.get_or_init(|| {
        native_test_config::NativeTestConfig::load(false)
            .expect("valid isolated native test configuration")
    })
}

struct Cleanup {
    base_url: String,
    user: String,
    password: String,
    id: u64,
    done: bool,
}

impl Cleanup {
    fn wp_cli(&self, args: &[&str]) -> std::process::Output {
        Command::new("docker")
            .args([
                "exec",
                "-u",
                config().docker_user.as_str(),
                config().container.as_str(),
                "wp",
                config().wp_path_arg.as_str(),
            ])
            .args(args)
            .output()
            .expect("run clone WP-CLI")
    }

    fn remove_and_verify(&self) -> anyhow::Result<()> {
        let id = self.id.to_string();
        let existing = self.wp_cli(&["post", "get", &id, "--field=ID"]);
        if existing.status.success() {
            let removed = self.wp_cli(&["post", "delete", &id, "--force", "--quiet"]);
            anyhow::ensure!(
                removed.status.success(),
                "WP-CLI exact page delete failed: {}",
                String::from_utf8_lossy(&removed.stderr)
            );
        }
        let absent = self.wp_cli(&["post", "get", &id, "--field=ID"]);
        anyhow::ensure!(
            !absent.status.success()
                && String::from_utf8_lossy(&absent.stderr)
                    .contains(&format!("Could not find the post with ID {id}")),
            "WP-CLI did not confirm exact replay fixture {id} absence: {}",
            String::from_utf8_lossy(&absent.stderr)
        );
        Ok(())
    }

    async fn finish(&mut self) {
        let wp = WpClient::new(&self.base_url, &self.user, &self.password);
        let deleted = wp
            .delete(&format!("wp/v2/pages/{}?force=true", self.id))
            .await;
        if let Err(error) = &deleted {
            assert!(
                error.to_string().contains("404"),
                "delete failed without actual REST 404: {error:#}"
            );
        }
        assert!(
            self.remove_and_verify().is_ok(),
            "native WP-CLI must confirm exact fixture absence"
        );
        self.done = true;
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        let cleanup = self.remove_and_verify();
        if let Err(error) = cleanup {
            if std::thread::panicking() {
                eprintln!(
                    "panic cleanup failed for exact document replay fixture {}: {error:#}",
                    self.id
                );
            } else {
                panic!(
                    "cleanup failed for exact document replay fixture {}: {error:#}",
                    self.id
                );
            }
        }
    }
}

fn wp() -> WpClient {
    let config = config();
    WpClient::new(&config.url, &config.user, &config.password)
}

fn heading(title: &str) -> Value {
    json!([{"id":"replay001","elType":"widget","widgetType":"heading","settings":{"title":title},"elements":[]}])
}

async fn read(wp: &WpClient, id: u64) -> Value {
    wp.get(&format!("mcp-for-page-builders/v1/document/{id}"))
        .await
        .expect("read native bridge document")
}

async fn apply(
    wp: &WpClient,
    id: u64,
    operation_id: &str,
    revision: &Value,
    document: Value,
) -> anyhow::Result<Value> {
    wp.request(
        "POST",
        &format!("mcp-for-page-builders/v1/document/{id}/apply"),
        Some(
            &json!({"operation_id":operation_id,"expected_revision":revision,"document":document}),
        ),
        None,
    )
    .await
}

fn assert_conflict(error: anyhow::Error, expected_code: &str) {
    let message = format!("{error:#}");
    assert!(message.contains("409"), "expected HTTP 409, got {message}");
    assert!(
        message.contains(expected_code),
        "expected {expected_code}, got {message}"
    );
}

#[tokio::test]
async fn document_apply_and_restore_replays_require_matching_current_state() {
    let wp = wp();
    let marker = format!(
        "MCP document replay {}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    );
    let created = wp.post("wp/v2/pages", &json!({"title":marker,"status":"draft","template":"elementor_canvas","meta":{"_elementor_data":"[]","_elementor_edit_mode":"builder"}})).await.expect("create isolated replay draft");
    let id = created["id"].as_u64().expect("draft ID");
    eprintln!("Created exact document replay fixture page {id}");
    let mut cleanup = Cleanup {
        base_url: wp.base_url().to_owned(),
        user: config().user.clone(),
        password: config().password.clone(),
        id,
        done: false,
    };
    let initial = read(&wp, id).await;
    let op = format!("replay_apply_{}", id);
    let first = apply(&wp, id, &op, &initial["revision"], heading("Applied"))
        .await
        .expect("first document apply");
    assert_eq!(first["saved"], true);
    assert_eq!(first["readback_verified"], true);
    assert_eq!(first["cache_regenerated"], true);
    let replay = apply(&wp, id, &op, &initial["revision"], heading("Applied"))
        .await
        .expect("same-body apply replay");
    assert_eq!(replay["idempotent_replay"], true);
    assert_eq!(replay["readback_verified"], true);
    assert_eq!(replay["cache_regenerated"], true);
    let after_apply = read(&wp, id).await;
    assert_conflict(
        apply(&wp, id, &op, &initial["revision"], heading("Different"))
            .await
            .unwrap_err(),
        "operation_id_conflict",
    );
    assert_eq!(read(&wp, id).await["document"], after_apply["document"]);

    let later_op = format!("replay_later_{}", id);
    apply(
        &wp,
        id,
        &later_op,
        &after_apply["revision"],
        heading("Later edit"),
    )
    .await
    .expect("later checked document edit");
    let after_later = read(&wp, id).await;
    assert_conflict(
        apply(&wp, id, &op, &initial["revision"], heading("Applied"))
            .await
            .unwrap_err(),
        "operation_replay_conflict",
    );
    assert_eq!(read(&wp, id).await["document"], after_later["document"]);

    let restore_op = format!("replay_restore_{}", id);
    let restored = wp
        .request(
            "POST",
            &format!("mcp-for-page-builders/v1/document/{id}/restore"),
            Some(&json!({"operation_id":later_op,"expected_revision":after_later["revision"]})),
            None,
        )
        .await
        .expect("restore later edit snapshot");
    assert_eq!(restored["restored"], true);
    assert_eq!(restored["cache_regenerated"], true);
    let original = read(&wp, id).await;
    let restore_replay = wp
        .request(
            "POST",
            &format!("mcp-for-page-builders/v1/document/{id}/restore"),
            Some(&json!({"operation_id":later_op,"expected_revision":original["revision"]})),
            None,
        )
        .await
        .expect("matching restore replay");
    assert_eq!(restore_replay["idempotent_replay"], true);
    assert_eq!(restore_replay["readback_verified"], true);
    assert_eq!(restore_replay["cache_regenerated"], true);
    let after_replay = read(&wp, id).await;
    apply(
        &wp,
        id,
        &restore_op,
        &after_replay["revision"],
        heading("After restore"),
    )
    .await
    .expect("checked edit after restore");
    let changed = read(&wp, id).await;
    assert_conflict(
        wp.request(
            "POST",
            &format!("mcp-for-page-builders/v1/document/{id}/restore"),
            Some(&json!({"operation_id":later_op,"expected_revision":changed["revision"]})),
            None,
        )
        .await
        .unwrap_err(),
        "restore_conflict",
    );
    assert_eq!(read(&wp, id).await["document"], changed["document"]);
    cleanup.finish().await;
}
