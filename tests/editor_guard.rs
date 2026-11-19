use std::{fs, process::Command, sync::OnceLock};
use std::{io::Write, os::unix::fs::OpenOptionsExt};

use mcp_for_page_builders::tools::{Tool, elementor_plan_tools::RestoreElementorSnapshot};
use mcp_for_page_builders::wp::WpClient;
use serde_json::{Value, json};

#[path = "native_test_config.rs"]
mod native_test_config;

fn config() -> &'static native_test_config::NativeTestConfig {
    static CONFIG: OnceLock<native_test_config::NativeTestConfig> = OnceLock::new();
    CONFIG.get_or_init(|| {
        native_test_config::NativeTestConfig::load(false)
            .expect("valid isolated native test configuration")
    })
}

fn write_private_fixture(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents.as_bytes())
}

#[test]
fn native_editor_guard_rejects_competitors_and_preserves_ordinary_saves() {
    let config = config();
    let script = r##"<?php
$user = get_user_by('login', getenv('WP_TEST_USER'));
if (!$user || !user_can($user, 'manage_options')) throw new RuntimeException('configured test user must have manage_options');
wp_set_current_user($user->ID);
$ids = [];
$lease_ids = [];
$operation_ids = [];
$css_print_method_filter = null;
function guard_assert($ok, $message) { if (!$ok) throw new RuntimeException($message); }
function guard_reject($callback, $request, $manager, $label, $expected = 409) {
    try { call_user_func($callback, $request, $manager); }
    catch (RuntimeException $error) { guard_assert((int) $error->getCode() === $expected, $label . ' returned error code ' . $error->getCode() . ': ' . $error->getMessage()); return; }
    throw new RuntimeException($label . ' was not rejected before native save');
}

try {
    guard_assert(mcp_bridge_editor_guard_yoast_integration_available(), 'active Yoast version or native Elementor methods were not recognized');
    guard_assert(!mcp_bridge_editor_guard_yoast_ajax_handler_registered(), 'REST context unexpectedly registered the admin AJAX callback');
    guard_assert(mcp_bridge_editor_guard_native_post_save_mode('yoast') === 'yoast', 'active integration was rejected before admin AJAX initialization');
    $selector = static function($handler) { return mcp_bridge_editor_guard_yoast_die_handler($handler); };
    add_filter('wp_die_ajax_handler', $selector, PHP_INT_MAX);
    guard_assert(apply_filters('wp_die_ajax_handler', '_ajax_wp_die_handler') === '_ajax_wp_die_handler', 'Yoast die-handler filter changed the default outside a guarded request');
    $GLOBALS['mcp_bridge_yoast_post_save'] = ['post_id' => 1];
    guard_assert(apply_filters('wp_die_ajax_handler', '_ajax_wp_die_handler') === 'mcp_bridge_editor_guard_yoast_die', 'Yoast die-handler filter did not return the guarded callable');
    unset($GLOBALS['mcp_bridge_yoast_post_save']);
    remove_filter('wp_die_ajax_handler', $selector, PHP_INT_MAX);
    foreach (['leased', 'ordinary'] as $suffix) {
        $id = wp_insert_post(['post_title' => 'MCP guard smoke ' . wp_generate_uuid4(), 'post_type' => 'page', 'post_status' => 'draft'], true);
        if (is_wp_error($id)) throw new RuntimeException($id->get_error_message());
        $ids[$suffix] = $id;
        update_post_meta($id, '_elementor_data', '[]');
        update_post_meta($id, '_elementor_edit_mode', 'builder');
    }
    $id = $ids['leased'];
    $elements = [['id' => 'containerprobe', 'elType' => 'container', 'settings' => ['background_background' => 'classic', 'background_color' => '#123456'], 'elements' => [['id' => 'headingprobe', 'elType' => 'widget', 'widgetType' => 'heading', 'settings' => ['title' => 'Native snapshot probe', 'header_size' => 'h2', 'align' => 'center'], 'elements' => []]]]];
    $unsupported_elements = $elements;
    $unsupported_elements[0]['vendor_unknown'] = ['nested' => ['keep' => true]];
    $settings = ['hide_title' => 'yes', 'vendor_page_setting' => ['keep' => true]];
    $canonical_input = [['elType' => 'widget', 'isInner' => false, 'isLocked' => false, 'interactions' => [], 'settings' => ['isInner' => false, 'isLocked' => false, 'interactions' => [], 'custom' => ['elType' => 'container', 'isInner' => false, 'isLocked' => false, 'interactions' => []]]]];
    $canonical_expected = [['elType' => 'widget', 'settings' => ['isInner' => false, 'isLocked' => false, 'interactions' => [], 'custom' => ['elType' => 'container', 'isInner' => false, 'isLocked' => false, 'interactions' => []]]]];
    guard_assert(mcp_bridge_editor_guard_canonical($canonical_input) === $canonical_expected, 'serializer normalization changed meaningful nested settings or custom data');
    update_post_meta($id, '_elementor_data', wp_json_encode($elements));
    update_post_meta($id, '_elementor_page_settings', $settings);
    add_post_meta($id, '_mcp_snapshot_probe', 'first', false);
    add_post_meta($id, '_mcp_snapshot_probe', 'second', false);
    $autosave_id = wp_insert_post(['post_title' => 'Pre-existing autosave fixture', 'post_name' => $id . '-autosave-' . $user->ID, 'post_type' => 'revision', 'post_status' => 'inherit', 'post_parent' => $id], true);
    if (is_wp_error($autosave_id)) throw new RuntimeException($autosave_id->get_error_message());
    $revision_meta_ids = [add_metadata('post', $autosave_id, '_mcp_revision_probe', 'first', false), add_metadata('post', $autosave_id, '_mcp_revision_probe', 'second', false), add_metadata('post', $autosave_id, '_mcp_revision_unknown', ['keep' => true], true)];
    guard_assert($revision_meta_ids[0] && $revision_meta_ids[1] && $revision_meta_ids[2] && get_post_meta($autosave_id, '_mcp_revision_probe', false) === ['first', 'second'] && get_post_meta($autosave_id, '_mcp_revision_unknown', true) === ['keep' => true], 'could not establish repeated and unknown pre-existing autosave metadata');
    $fixture_css = mcp_bridge_regenerate_css($id);
    guard_assert(!is_wp_error($fixture_css), 'could not generate Elementor CSS for the native snapshot fixture');
    $make_lease = static function($post_id, $elements, $settings) {
        static $attempt = 0;
        $attempt++;
        $request = new WP_REST_Request('POST', '/mcp-for-page-builders/v1/document/' . $post_id . '/editor-lease');
        $request->set_header('content-type', 'application/json');
        $document = get_post_meta($post_id, '_elementor_data', true);
        $request->set_body(wp_json_encode(['expected_revision' => hash('sha256', $document), 'operation_id' => 'guard-smoke-' . wp_generate_uuid4(), 'status' => 'draft', 'elements' => $elements, 'settings' => $settings]));
        $response = rest_do_request($request);
        guard_assert($response->get_status() === 200, 'lease acquisition attempt ' . $attempt . ' failed: ' . wp_json_encode($response->get_data()));
        return $response->get_data();
    };
    $before_unsupported = mcp_bridge_editor_guard_full_state($id);
    $unsupported_request = new WP_REST_Request('POST', '/mcp-for-page-builders/v1/document/' . $id . '/editor-lease');
    $unsupported_request->set_header('content-type', 'application/json');
    $unsupported_request->set_body(wp_json_encode(['expected_revision' => hash('sha256', get_post_meta($id, '_elementor_data', true)), 'operation_id' => 'guard-unsupported-' . wp_generate_uuid4(), 'status' => 'draft', 'elements' => $unsupported_elements, 'settings' => $settings]));
    $unsupported_response = rest_do_request($unsupported_request);
    $after_unsupported = mcp_bridge_editor_guard_full_state($id);
    guard_assert($unsupported_response->get_status() === 501 && strpos($unsupported_response->get_data()['message'] ?? '', '$.0.vendor_unknown') !== false, 'native serializer loss was not rejected with a path-only diagnostic');
    guard_assert(!is_wp_error($before_unsupported) && !is_wp_error($after_unsupported) && mcp_bridge_editor_guard_state_fingerprint($before_unsupported) === mcp_bridge_editor_guard_state_fingerprint($after_unsupported), 'unsupported native field changed the complete post, revision, metadata, or CSS snapshot');
    guard_assert(get_option('_mcp_elementor_editor_lease_' . $id) === false, 'unsupported native field created a save lease');
    $lease = $make_lease($id, $elements, $settings);
    $lease_ids[] = $id;
    $operation_ids[] = $lease['operation_id'];
    $plugin = Elementor\Plugin::instance();
    if (!$plugin->common) $plugin->init_common();
    $ajax = $plugin->common->get_component('ajax');
    do_action('elementor/ajax/register_actions', $ajax);
    $property = new ReflectionProperty($ajax, 'ajax_actions');
    $property->setAccessible(true);
    $callback = $property->getValue($ajax)['save_builder']['callback'];
    guard_assert($callback instanceof Closure, 'verified guarded callback was not installed');
    $_REQUEST['editor_post_id'] = $id;
    $before = get_post_meta($id, '_elementor_data', true);
    $before_revisions = count(wp_get_post_revisions($id));
    guard_reject($callback, ['editor_post_id' => $id, 'status' => 'autosave', 'elements' => [], 'settings' => []], $ajax, 'un-tokened autosave');
    guard_assert(get_post_meta($id, '_elementor_data', true) === $before && count(wp_get_post_revisions($id)) === $before_revisions, 'rejected autosave changed document or revision state');
    $_REQUEST['editor_post_id'] = $ids['ordinary'];
    guard_reject($callback, ['editor_post_id' => $ids['ordinary'], 'mcp_guard_token' => $lease['guard_token'], 'status' => 'draft', 'elements' => [], 'settings' => []], $ajax, 'wrong-post token');
    $_REQUEST['editor_post_id'] = $id;
    guard_reject($callback, ['editor_post_id' => $id, 'mcp_guard_token' => $lease['guard_token'], 'status' => 'draft', 'elements' => []], $ajax, 'incomplete native body');
    guard_reject($callback, ['editor_post_id' => $id, 'mcp_guard_token' => $lease['guard_token'], 'status' => 'draft', 'elements' => [['id' => 'broken']], 'settings' => []], $ajax, 'invalid element', 400);
    update_post_meta($id, '_elementor_data', '[ ]');
    $stale = get_post_meta($id, '_elementor_data', true);
    guard_reject($callback, ['editor_post_id' => $id, 'mcp_guard_token' => $lease['guard_token'], 'status' => 'draft', 'elements' => [], 'settings' => []], $ajax, 'stale revision');
    guard_assert(get_post_meta($id, '_elementor_data', true) === $stale, 'stale rejection changed the document');
    update_post_meta($id, '_elementor_data', wp_json_encode($elements));
    $before_state = mcp_bridge_editor_guard_full_state($id);
    guard_assert(!is_wp_error($before_state), 'full native pre-save snapshot unavailable');
    guard_assert(in_array((int) $autosave_id, $before_state['revision_ids'], true), 'pre-existing autosave fixture was absent from full snapshot');
    try { $saved = call_user_func($callback, ['editor_post_id' => $id, 'mcp_guard_token' => $lease['guard_token'], 'status' => 'draft', 'elements' => $elements, 'settings' => $settings], $ajax); }
    catch (Throwable $error) {
        $failed_receipt = get_option('_mcp_elementor_editor_operation_' . hash('sha256', $lease['operation_id']));
        throw new RuntimeException('native callback failed: ' . wp_json_encode(['message' => $error->getMessage(), 'receipt_error' => $failed_receipt['error'] ?? null]));
    }
    guard_assert(is_array($saved), 'valid guarded request did not invoke native Documents_Manager::ajax_save');
    $operation_key = '_mcp_elementor_editor_operation_' . hash('sha256', $lease['operation_id']);
    $receipt = get_option($operation_key);
    guard_assert(is_array($receipt) && ($receipt['status'] ?? '') === 'saved' && !empty($receipt['readback_verified']), 'native full-state save receipt was not verified: ' . wp_json_encode(['status' => $receipt['status'] ?? null, 'error' => $receipt['error'] ?? null, 'readback_verified' => $receipt['readback_verified'] ?? false]));
    $saved_tree = json_decode((string) get_post_meta($id, '_elementor_data', true), true);
    guard_assert(!isset($saved_tree[0]['vendor_unknown']) && ($saved_tree[0]['settings']['background_color'] ?? '') === '#123456' && ($saved_tree[0]['elements'][0]['settings']['title'] ?? '') === 'Native snapshot probe', 'registered container, heading, or style settings did not survive native save');
    guard_assert(get_post_meta($id, '_elementor_page_settings', true) === $settings, 'native save changed complete page settings');
    guard_assert(get_post_meta($id, '_mcp_snapshot_probe', false) === ['first', 'second'], 'native save changed repeated unknown metadata');
    $status_request = new WP_REST_Request('GET', '/mcp-for-page-builders/v1/document/' . $id . '/editor-operation/' . $lease['operation_id']);
    $status_response = rest_do_request($status_request);
    guard_assert($status_response->get_status() === 200 && !isset($status_response->get_data()['before']) && !isset($status_response->get_data()['after']), 'operation status exposed private snapshot contents');
    $replay_request = new WP_REST_Request('POST', '/mcp-for-page-builders/v1/document/' . $id . '/editor-lease');
    $replay_request->set_header('content-type', 'application/json');
    $replay_request->set_body(wp_json_encode(['expected_revision' => hash('sha256', $before), 'operation_id' => $lease['operation_id'], 'status' => 'draft', 'elements' => $elements, 'settings' => $settings]));
    $replay = rest_do_request($replay_request);
    guard_assert($replay->get_status() === 200 && !isset($replay->get_data()['guard_token']) && !empty($replay->get_data()['idempotent_replay']), 'matching applied operation did not return a read-only replay receipt');
    $conflict_request = new WP_REST_Request('POST', '/mcp-for-page-builders/v1/document/' . $id . '/editor-lease');
    $conflict_request->set_header('content-type', 'application/json');
    $conflict_request->set_body(wp_json_encode(['expected_revision' => hash('sha256', $before), 'operation_id' => $lease['operation_id'], 'status' => 'draft', 'elements' => $elements, 'settings' => ['hide_title' => 'no']]));
    guard_assert(rest_do_request($conflict_request)->get_status() === 409, 'same operation ID accepted a different body');
    guard_assert(get_option('_mcp_elementor_editor_lease_' . $id) === false && get_option('_mcp_elementor_lock_' . $id) === false, 'native callback did not release its lease and checked-write lock');
    $_REQUEST['editor_post_id'] = $ids['ordinary'];
    $ordinary = call_user_func($callback, ['editor_post_id' => $ids['ordinary'], 'status' => 'draft', 'elements' => [], 'settings' => []], $ajax);
    guard_assert(is_array($ordinary), 'ordinary exact-other-post save did not delegate to native callback');
    $restore_path = '/mcp-for-page-builders/v1/document/' . $id . '/editor-operation/' . $lease['operation_id'] . '/restore';
    $foreign_revision_id = wp_insert_post(['post_title' => 'Unrelated revision after native save', 'post_name' => $id . '-revision-foreign-' . wp_generate_password(6, false), 'post_type' => 'revision', 'post_status' => 'inherit', 'post_parent' => $id], true);
    if (is_wp_error($foreign_revision_id)) throw new RuntimeException($foreign_revision_id->get_error_message());
    $foreign_state = mcp_bridge_editor_guard_full_state($id);
    $restore_request = new WP_REST_Request('POST', $restore_path);
    $restore_request->set_param('expected_state_fingerprint', $receipt['after_fingerprint']);
    guard_assert(rest_do_request($restore_request)->get_status() === 409, 'restore overwrote an unrelated revision created after save');
    $foreign_state_after = mcp_bridge_editor_guard_full_state($id);
    guard_assert(!is_wp_error($foreign_state) && !is_wp_error($foreign_state_after) && mcp_bridge_editor_guard_state_fingerprint($foreign_state) === mcp_bridge_editor_guard_state_fingerprint($foreign_state_after), 'revision conflict changed the full native state');
    wp_delete_post($foreign_revision_id, true);
    add_post_meta($id, '_mcp_intervening_edit', 'changed', true);
    $restore_request = new WP_REST_Request('POST', $restore_path);
    $restore_request->set_param('expected_state_fingerprint', $receipt['after_fingerprint']);
    guard_assert(rest_do_request($restore_request)->get_status() === 409, 'restore overwrote an intervening metadata edit');
    delete_post_meta($id, '_mcp_intervening_edit');
    $restore_request = new WP_REST_Request('POST', $restore_path);
    $restore_request->set_param('expected_state_fingerprint', $receipt['after_fingerprint']);
    $restore_response = rest_do_request($restore_request);
    guard_assert($restore_response->get_status() === 200 && !empty($restore_response->get_data()['restored']), 'full native restore failed: ' . wp_json_encode($restore_response->get_data()));
    $restored_state = mcp_bridge_editor_guard_full_state($id);
    guard_assert(!is_wp_error($restored_state) && mcp_bridge_editor_guard_state_fingerprint($restored_state) === mcp_bridge_editor_guard_state_fingerprint($before_state), 'restore did not recover the full original native state');
    $restored_revision_meta = ['repeated' => get_post_meta($autosave_id, '_mcp_revision_probe', false), 'unknown' => get_post_meta($autosave_id, '_mcp_revision_unknown', true)];
    guard_assert($restored_revision_meta === ['repeated' => ['first', 'second'], 'unknown' => ['keep' => true]], 'restore changed repeated or unknown metadata on a pre-existing autosave: ' . wp_json_encode($restored_revision_meta));
    $restore_replay = new WP_REST_Request('POST', $restore_path);
    $restore_replay->set_param('expected_state_fingerprint', $restore_response->get_data()['restored_fingerprint']);
    guard_assert(rest_do_request($restore_replay)->get_status() === 200, 'restore replay failed against its fresh restored state');
    $restored_status = rest_do_request(new WP_REST_Request('GET', '/mcp-for-page-builders/v1/document/' . $id . '/editor-operation/' . $lease['operation_id']));
    guard_assert($restored_status->get_status() === 200 && !empty($restored_status->get_data()['readback_verified']) && hash_equals($restore_response->get_data()['restored_fingerprint'], $restored_status->get_data()['current_state_fingerprint']), 'restored operation GET did not verify current full native state');
    $css_print_method_filter = static function($method) { return 'internal'; };
    add_filter('option_elementor_css_print_method', $css_print_method_filter);
    guard_assert(get_option('elementor_css_print_method', 'external') === 'internal', 'fixture-scoped inline CSS setting was not active');
    guard_assert(!is_wp_error(mcp_bridge_regenerate_css($id)), 'could not prepare inline CSS for the after-save failure fixture');
    $before_failure = mcp_bridge_editor_guard_full_state($id);
    guard_assert(!is_wp_error($before_failure), 'could not snapshot before the native after-save failure fixture');
    $failed_lease = $make_lease($id, $elements, $settings);
    $lease_ids[] = $id;
    $operation_ids[] = $failed_lease['operation_id'];
    $fail_after_native_save = static function() { throw new RuntimeException('editor guard after-save failure fixture'); };
    add_action('elementor/document/after_save', $fail_after_native_save, PHP_INT_MAX);
    $_REQUEST['editor_post_id'] = $id;
    $failure_thrown = false;
    try {
        call_user_func($callback, ['editor_post_id' => $id, 'mcp_guard_token' => $failed_lease['guard_token'], 'status' => 'draft', 'elements' => $elements, 'settings' => $settings], $ajax);
    } catch (Throwable $error) {
        $failure_thrown = true;
        guard_assert(strpos($error->getMessage(), 'editor guard after-save failure fixture') !== false, 'native callback failed before the after-save fixture ran');
    } finally { remove_action('elementor/document/after_save', $fail_after_native_save, PHP_INT_MAX); }
    guard_assert($failure_thrown, 'native after-save failure fixture did not fail the callback');
    $failed_receipt = get_option('_mcp_elementor_editor_operation_' . hash('sha256', $failed_lease['operation_id']));
    guard_assert(is_array($failed_receipt) && ($failed_receipt['status'] ?? '') === 'partial_failure' && !empty($failed_receipt['after_fingerprint']), 'native after-save failure did not produce a recoverable partial receipt');
    $failure_status = rest_do_request(new WP_REST_Request('GET', '/mcp-for-page-builders/v1/document/' . $id . '/editor-operation/' . $failed_lease['operation_id']));
    guard_assert($failure_status->get_status() === 200 && !isset($failure_status->get_data()['before']) && !isset($failure_status->get_data()['after']), 'partial failure status exposed private snapshots');
    guard_assert(empty($failure_status->get_data()['readback_verified']) && is_string($failure_status->get_data()['current_state_fingerprint']), 'partial receipt must remain unverified while exposing its safe current-state CAS fingerprint');
    $failure_restore_path = '/mcp-for-page-builders/v1/document/' . $id . '/editor-operation/' . $failed_lease['operation_id'] . '/restore';
    $failure_restore = new WP_REST_Request('POST', $failure_restore_path);
    $failure_restore->set_param('expected_state_fingerprint', $failed_receipt['after_fingerprint']);
    $failure_restore_response = rest_do_request($failure_restore);
    guard_assert($failure_restore_response->get_status() === 200 && !empty($failure_restore_response->get_data()['restored']), 'native partial failure could not be restored: ' . wp_json_encode($failure_restore_response->get_data()));
    $failure_restored_state = mcp_bridge_editor_guard_full_state($id);
    guard_assert(!is_wp_error($failure_restored_state) && mcp_bridge_editor_guard_state_fingerprint($failure_restored_state) === mcp_bridge_editor_guard_state_fingerprint($before_failure), 'partial failure restore did not recover the full pre-save state');
    remove_filter('option_elementor_css_print_method', $css_print_method_filter);
    $css_print_method_filter = null;
    guard_assert(!is_wp_error(mcp_bridge_regenerate_css($id)), 'could not regenerate CSS before the expired-lease fixture');
    $expired = $make_lease($id, $elements, $settings);
    $lease_ids[] = $id;
    $operation_ids[] = $expired['operation_id'];
    $expired_option = get_option('_mcp_elementor_editor_lease_' . $id);
    $expired_option['expires'] = time() - 1;
    update_option('_mcp_elementor_editor_lease_' . $id, $expired_option, false);
    $_REQUEST['editor_post_id'] = $id;
    guard_reject($callback, ['editor_post_id' => $id, 'mcp_guard_token' => $expired['guard_token'], 'status' => 'draft', 'elements' => [], 'settings' => []], $ajax, 'expired token');
    guard_assert(get_option('_mcp_elementor_editor_lease_' . $id) === false && get_option('_mcp_elementor_lock_' . $id) === false, 'expired pending lease left a lock sentinel');
    echo 'PASS native AJAX guard matrix' . PHP_EOL;
} finally {
    if ($css_print_method_filter !== null) remove_filter('option_elementor_css_print_method', $css_print_method_filter);
    foreach (array_unique($lease_ids) as $id) {
        $lease = get_option('_mcp_elementor_editor_lease_' . $id);
        if (is_array($lease) && isset($lease['lock']) && is_array($lease['lock'])) mcp_bridge_unlock($lease['lock']);
        delete_option('_mcp_elementor_editor_lease_' . $id);
        delete_option('_mcp_elementor_lock_' . $id);
    }
    foreach (array_unique($operation_ids) as $operation_id) delete_option('_mcp_elementor_editor_operation_' . hash('sha256', $operation_id));
    foreach ($ids as $id) {
        wp_delete_post($id, true);
        if (get_post($id)) throw new RuntimeException('exact draft cleanup failed for ' . $id);
    }
}
"##;
    let path = std::env::temp_dir().join(format!("mcp-editor-guard-{}.php", std::process::id()));
    write_private_fixture(&path, script).expect("write private temporary native guard check");
    let container = config.container.as_str();
    let bridge = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mcp-bridge.php");
    let wp_path = config
        .wp_path_arg
        .strip_prefix("--path=")
        .expect("validated WordPress path argument");
    let bridge_target = std::path::Path::new(wp_path)
        .parent()
        .expect("WordPress root has a content directory")
        .join("app/mu-plugins/mcp-for-page-builders/mcp-bridge.php");
    let bridge_copy = Command::new("docker")
        .args(["cp"])
        .arg(bridge)
        .arg(format!("{container}:{}", bridge_target.display()))
        .output()
        .expect("docker is required to install the bridge in the isolated clone");
    assert!(
        bridge_copy.status.success(),
        "copy bridge to isolated clone failed: {}",
        String::from_utf8_lossy(&bridge_copy.stderr)
    );
    let copied = Command::new("docker")
        .args(["cp"])
        .arg(&path)
        .arg(format!(
            "{container}:/tmp/mcp-editor-guard-{}.php",
            std::process::id()
        ))
        .output()
        .expect("docker is required for native guard integration");
    assert!(
        copied.status.success(),
        "copy native guard check failed: {}",
        String::from_utf8_lossy(&copied.stderr)
    );
    let protected = Command::new("docker")
        .args(["exec", container, "chmod", "600"])
        .arg(format!("/tmp/mcp-editor-guard-{}.php", std::process::id()))
        .output()
        .expect("protect temporary native guard check");
    assert!(
        protected.status.success(),
        "could not protect temporary native guard check"
    );
    let output = Command::new("docker")
        .args(["exec", "-u", config.docker_user.as_str(), "-e"])
        .arg(format!("WP_TEST_USER={}", config.user))
        .args([
            container,
            "wp",
            config.wp_path_arg.as_str(),
            "--quiet",
            "eval-file",
        ])
        .arg(format!("/tmp/mcp-editor-guard-{}.php", std::process::id()))
        .output()
        .expect("run clone WP-CLI native guard check");
    let cleanup = Command::new("docker")
        .args(["exec", container, "rm", "-f"])
        .arg(format!("/tmp/mcp-editor-guard-{}.php", std::process::id()))
        .output()
        .expect("remove temporary clone script");
    let local_cleanup = fs::remove_file(&path);
    assert!(
        cleanup.status.success(),
        "remove temporary clone script failed"
    );
    local_cleanup.expect("remove private temporary native guard check");
    assert!(
        output.status.success(),
        "native guard integration failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("PASS native AJAX guard matrix"),
        "native guard check did not report its assertions"
    );
}

fn run_native_restore_php(script: &str, config: &native_test_config::NativeTestConfig) -> String {
    let container = config.container.as_str();
    let name = format!("mcp-native-restore-{}.php", std::process::id());
    let path = std::env::temp_dir().join(&name);
    write_private_fixture(&path, script).expect("write private native restore fixture");
    let container_path = format!("/tmp/{name}");
    let copied = Command::new("docker")
        .args(["cp"])
        .arg(&path)
        .arg(format!("{container}:{container_path}"))
        .output()
        .expect("copy native fixture to clone");
    assert!(
        copied.status.success(),
        "could not copy native restore fixture"
    );
    let protected = Command::new("docker")
        .args(["exec", container, "chmod", "600", &container_path])
        .output()
        .expect("protect clone fixture");
    assert!(
        protected.status.success(),
        "could not protect native restore fixture"
    );
    let output = Command::new("docker")
        .args(["exec", "-u", config.docker_user.as_str(), "-e"])
        .arg(format!("WP_TEST_USER={}", config.user))
        .args([
            container,
            "wp",
            config.wp_path_arg.as_str(),
            "--quiet",
            "eval-file",
        ])
        .arg(&container_path)
        .output()
        .expect("run clone WP-CLI fixture");
    let cleanup = Command::new("docker")
        .args(["exec", container, "rm", "-f", &container_path])
        .output()
        .expect("remove clone fixture");
    fs::remove_file(&path).expect("remove private local native restore fixture");
    assert!(cleanup.status.success(), "could not remove clone fixture");
    assert!(
        output.status.success(),
        "native restore fixture failed; stdout: {}; stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("native restore fixture output is UTF-8")
}

#[tokio::test]
async fn rust_native_snapshot_restore_uses_real_guarded_save_and_full_state_readback() {
    let config = config();
    let wp = WpClient::new(&config.url, &config.user, &config.password);
    let fixture = run_native_restore_php(
        r#"<?php
$user=get_user_by('login',getenv('WP_TEST_USER')); if(!$user||!user_can($user,'manage_options')) throw new RuntimeException('configured test user must have manage_options'); wp_set_current_user($user->ID);
$id=wp_insert_post(['post_title'=>'MCP native restore fixture '.wp_generate_uuid4(),'post_type'=>'page','post_status'=>'draft'],true); if(is_wp_error($id)) throw new RuntimeException('fixture page creation failed'); echo "NATIVE_RESTORE_FIXTURE_PAGE_ID=$id\n";
 $operation='rust-restore-'.wp_generate_uuid4(); $partial_operation='';
try {
update_post_meta($id,'_elementor_data','[]'); update_post_meta($id,'_elementor_edit_mode','builder'); $css=mcp_bridge_regenerate_css($id); if(is_wp_error($css)) throw new RuntimeException('original CSS snapshot preparation failed'); $before=mcp_bridge_editor_guard_full_state($id); if(is_wp_error($before)) throw new RuntimeException('original snapshot unavailable: '.$before->get_error_code());
$elements=[['id'=>'restoreprobe','elType'=>'widget','widgetType'=>'heading','settings'=>['title'=>'Changed fixture'],'elements'=>[]]]; $request=new WP_REST_Request('POST','/mcp-for-page-builders/v1/document/'.$id.'/editor-lease'); $request->set_header('content-type','application/json'); $request->set_body(wp_json_encode(['expected_revision'=>hash('sha256',get_post_meta($id,'_elementor_data',true)),'operation_id'=>$operation,'status'=>'draft','elements'=>$elements,'settings'=>[]])); $lease=rest_do_request($request); if($lease->get_status()!==200) throw new RuntimeException('native fixture lease failed');
$plugin=Elementor\Plugin::instance(); if(!$plugin->common) $plugin->init_common(); $ajax=$plugin->common->get_component('ajax'); do_action('elementor/ajax/register_actions',$ajax); $property=new ReflectionProperty($ajax,'ajax_actions'); $property->setAccessible(true); $callback=$property->getValue($ajax)['save_builder']['callback']; $_REQUEST['editor_post_id']=$id; call_user_func($callback,['editor_post_id'=>$id,'mcp_guard_token'=>$lease->get_data()['guard_token'],'status'=>'draft','elements'=>$elements,'settings'=>[]],$ajax);
$receipt=get_option('_mcp_elementor_editor_operation_'.hash('sha256',$operation)); if(!is_array($receipt)||empty($receipt['after_fingerprint'])) throw new RuntimeException('native save receipt missing');
$partial_operation='rust-partial-'.wp_generate_uuid4(); $partial_elements=[['id'=>'restoreprobe','elType'=>'widget','widgetType'=>'heading','settings'=>['title'=>'Partial failure fixture'],'elements'=>[]]]; $partial_request=new WP_REST_Request('POST','/mcp-for-page-builders/v1/document/'.$id.'/editor-lease'); $partial_request->set_header('content-type','application/json'); $partial_request->set_body(wp_json_encode(['expected_revision'=>hash('sha256',get_post_meta($id,'_elementor_data',true)),'operation_id'=>$partial_operation,'status'=>'draft','elements'=>$partial_elements,'settings'=>[]])); $partial_lease=rest_do_request($partial_request); if($partial_lease->get_status()!==200) throw new RuntimeException('partial fixture lease failed');
$fail_after_save=static function(){throw new RuntimeException('native restore partial fixture');}; add_action('elementor/document/after_save',$fail_after_save,PHP_INT_MAX); try { call_user_func($callback,['editor_post_id'=>$id,'mcp_guard_token'=>$partial_lease->get_data()['guard_token'],'status'=>'draft','elements'=>$partial_elements,'settings'=>[]],$ajax); throw new RuntimeException('after-save failure did not fire'); } catch(Throwable $error) { if($error->getMessage()==='after-save failure did not fire') throw $error; } finally { remove_action('elementor/document/after_save',$fail_after_save,PHP_INT_MAX); }
$partial_receipt=get_option('_mcp_elementor_editor_operation_'.hash('sha256',$partial_operation)); if(!is_array($partial_receipt)||($partial_receipt['status']??'')!=='partial_failure'||empty($partial_receipt['after_fingerprint'])) throw new RuntimeException('native after-save failure receipt missing'); echo wp_json_encode(['page_id'=>$id,'operation_id'=>$operation,'expected_state_fingerprint'=>$receipt['after_fingerprint'],'before_fingerprint'=>mcp_bridge_editor_guard_state_fingerprint($before),'partial_operation_id'=>$partial_operation,'partial_state_fingerprint'=>$partial_receipt['after_fingerprint']]);
} catch (Throwable $error) {
    foreach ([$operation, $partial_operation] as $operation_id) if ($operation_id !== '') delete_option('_mcp_elementor_editor_operation_' . hash('sha256', $operation_id));
    delete_option('_mcp_elementor_lock_' . $id);
    wp_delete_post($id, true);
    if (get_post($id)) throw new RuntimeException('failed native restore fixture cleanup for exact page ' . $id, 0, $error);
    throw $error;
}
"#,
        config,
    );
    let fixture: Value = fixture
        .lines()
        .find_map(|line| serde_json::from_str(line).ok())
        .expect("fixture returned sanitized identifiers");
    let page_id = fixture["page_id"].as_u64().expect("fixture page id");
    let operation_id = fixture["operation_id"]
        .as_str()
        .expect("fixture operation id");
    let partial_operation_id = fixture["partial_operation_id"]
        .as_str()
        .expect("partial fixture operation id");
    let tool = RestoreElementorSnapshot;
    let stale = tool.run(json!({"snapshot_kind":"native_editor","page_id":page_id,"operation_id":operation_id,"expected_state_fingerprint":"0000000000000000000000000000000000000000000000000000000000000000"}), &wp).await.expect("stale restore response");
    assert_eq!(
        stale.is_error,
        Some(true),
        "stale fingerprint must be rejected"
    );
    let after_stale = wp
        .request(
            "GET",
            &format!("mcp-for-page-builders/v1/document/{page_id}/editor-operation/{partial_operation_id}"),
            None,
            None,
        )
        .await
        .expect("read native state after stale restore rejection");
    assert_eq!(
        after_stale["current_state_fingerprint"], fixture["partial_state_fingerprint"],
        "stale restore changed native full state"
    );
    let partial_restored = tool.run(json!({"snapshot_kind":"native_editor","page_id":page_id,"operation_id":partial_operation_id,"expected_state_fingerprint":fixture["partial_state_fingerprint"]}), &wp).await.expect("Rust partial native restore tool");
    assert_ne!(
        partial_restored.is_error,
        Some(true),
        "partial receipt restore did not verify"
    );
    let partial_replay = tool.run(json!({"snapshot_kind":"native_editor","page_id":page_id,"operation_id":partial_operation_id,"expected_state_fingerprint":fixture["expected_state_fingerprint"]}), &wp).await.expect("Rust partial restore replay");
    assert_ne!(
        partial_replay.is_error,
        Some(true),
        "partial restore replay did not verify"
    );
    let restored = tool.run(json!({"snapshot_kind":"native_editor","page_id":page_id,"operation_id":operation_id,"expected_state_fingerprint":fixture["expected_state_fingerprint"]}), &wp).await.expect("Rust native restore tool");
    assert_ne!(
        restored.is_error,
        Some(true),
        "native restore did not verify"
    );
    let replay = tool.run(json!({"snapshot_kind":"native_editor","page_id":page_id,"operation_id":operation_id,"expected_state_fingerprint":fixture["before_fingerprint"]}), &wp).await.expect("Rust restore replay");
    assert_ne!(
        replay.is_error,
        Some(true),
        "native restore replay did not verify"
    );
    let verify = format!(
        r#"<?php
$id={page_id}; $operation={operation:?}; $partial_operation={partial_operation:?}; $before={before:?}; $user=get_user_by('login',getenv('WP_TEST_USER')); if(!$user||!user_can($user,'manage_options')) throw new RuntimeException('configured test user must have manage_options'); wp_set_current_user($user->ID);
try {{ $state=mcp_bridge_editor_guard_full_state($id); if(is_wp_error($state)) throw new RuntimeException('full state unavailable after Rust restore'); $current=mcp_bridge_editor_guard_state_fingerprint($state); if(!hash_equals($before,$current)) throw new RuntimeException('full state differs after Rust restore'); $original=rest_do_request(new WP_REST_Request('GET','/mcp-for-page-builders/v1/document/'.$id.'/editor-operation/'.$operation)); if($original->get_status()!==200||empty($original->get_data()['readback_verified'])||!hash_equals($current,(string)($original->get_data()['current_state_fingerprint']??''))||!hash_equals($current,(string)($original->get_data()['restored_fingerprint']??''))) throw new RuntimeException('original receipt readback did not verify current full state'); $partial=rest_do_request(new WP_REST_Request('GET','/mcp-for-page-builders/v1/document/'.$id.'/editor-operation/'.$partial_operation)); $partial_data=$partial->get_data(); if($partial->get_status()!==200||!empty($partial_data['readback_verified'])||!is_string($partial_data['restored_fingerprint']??null)||hash_equals($current,$partial_data['restored_fingerprint'])) throw new RuntimeException('superseded partial receipt was unexpectedly verified against current full state'); echo 'PASS Rust native restore'; }} finally {{ delete_option('_mcp_elementor_editor_operation_'.hash('sha256',$operation)); delete_option('_mcp_elementor_editor_operation_'.hash('sha256',$partial_operation)); wp_delete_post($id,true); if(get_post($id)) throw new RuntimeException('exact fixture cleanup failed'); }}
"#,
        page_id = page_id,
        operation = operation_id,
        partial_operation = partial_operation_id,
        before = fixture["before_fingerprint"]
            .as_str()
            .expect("baseline fingerprint")
    );
    let verified = run_native_restore_php(&verify, config);
    assert!(
        verified.contains("PASS Rust native restore"),
        "native restore readback did not pass"
    );
}
