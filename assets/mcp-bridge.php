<?php
/*
Plugin Name: MCP Bridge (mu-plugin)
Description: Authenticated REST endpoints for safe Elementor inspection and writes.
Version: 1.3.0
*/
if (!defined('ABSPATH')) exit;

function mcp_bridge_hash($value) {
    if (is_string($value)) return hash('sha256', $value);
    $normalize = static function ($value) use (&$normalize) {
        if (!is_array($value)) return $value;
        $native_element = isset($value['elType']) && is_string($value['elType']);
        foreach ($value as $key => &$item) {
            if ($native_element && $key === 'styles' && mcp_bridge_native_style_map($item)) {
                foreach ($item as &$style) $style = $normalize($style);
                unset($style);
            } else {
                $item = $normalize($item);
            }
        }
        unset($item);
        if (!array_is_list($value)) ksort($value);
        return $value;
    };
    return hash('sha256', wp_json_encode($normalize($value)));
}

function mcp_bridge_legacy_hash($value) {
    if (is_string($value)) return hash('sha256', $value);
    $normalize = static function ($value) use (&$normalize) {
        if (!is_array($value)) return $value;
        if (!array_is_list($value)) ksort($value);
        foreach ($value as &$item) $item = $normalize($item);
        unset($item);
        return $value;
    };
    return hash('sha256', wp_json_encode($normalize($value)));
}

function mcp_bridge_native_style_map($styles) {
    if (!is_array($styles) || array_is_list($styles)) return false;
    foreach ($styles as $style_id => $style) {
        if (!is_string($style_id) || $style_id === '' || !is_array($style) || ($style['id'] ?? null) !== $style_id) return false;
    }
    return true;
}

function mcp_bridge_has_ordered_native_styles($value) {
    if (!is_array($value)) return false;
    if (isset($value['elType']) && is_string($value['elType']) && mcp_bridge_native_style_map($value['styles'] ?? null)) return true;
    foreach ($value as $item) if (mcp_bridge_has_ordered_native_styles($item)) return true;
    return false;
}

function mcp_bridge_ordered_fingerprint_matches($stored, $current, $legacy, $payload) {
    if (str_starts_with((string) $stored, 'native-style-order-v1:')) return hash_equals((string) $stored, (string) $current);
    if (mcp_bridge_has_ordered_native_styles($payload)) return false;
    return hash_equals((string) $stored, (string) $legacy);
}

function mcp_bridge_lock($post_id) {
    $key = '_mcp_elementor_lock_' . (int) $post_id;
    $token = wp_generate_uuid4();
    if (!add_option($key, ['token' => $token, 'created' => time()], '', false)) {
        return new WP_Error('document_busy', 'Another checked Elementor write is in progress.', ['status' => 409]);
    }
    return ['key' => $key, 'token' => $token];
}

function mcp_bridge_unlock($lock) {
    $current = get_option($lock['key']);
    if (is_array($current) && hash_equals($lock['token'], (string) ($current['token'] ?? ''))) delete_option($lock['key']);
}

function mcp_bridge_begin_resource_transaction($resources) {
    global $wpdb;
    foreach ([$wpdb->posts, $wpdb->postmeta, $wpdb->options] as $table) {
        $engine = $wpdb->get_var($wpdb->prepare('SELECT ENGINE FROM information_schema.TABLES WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = %s', $table));
        if (strtoupper((string) $engine) !== 'INNODB') return new WP_Error('resource_lock_unavailable', 'Elementor resource writes require InnoDB row locks.', ['status' => 503]);
    }
    if (false === $wpdb->query('START TRANSACTION')) return new WP_Error('resource_lock_unavailable', 'Could not start the native resource transaction.', ['status' => 503]);
    $kit = mcp_bridge_active_kit();
    if (is_wp_error($kit) || !is_callable([$kit, 'get_id'])) {
        $wpdb->query('ROLLBACK');
        return new WP_Error('resource_lock_unavailable', 'The active Elementor kit cannot be locked.', ['status' => 503]);
    }
    $post_ids = [(int) $kit->get_id()];
    foreach ($resources as $resource) {
        if (in_array($resource['kind'], ['page_settings', 'template', 'post_metadata', 'trash'], true) && ctype_digit((string) $resource['id'])) $post_ids[] = (int) $resource['id'];
    }
    $post_ids = array_values(array_unique($post_ids));
    sort($post_ids);
    foreach ($post_ids as $post_id) {
        $found = $wpdb->get_var($wpdb->prepare("SELECT ID FROM {$wpdb->posts} WHERE ID = %d FOR UPDATE", $post_id));
        if (!$found) {
            $wpdb->query('ROLLBACK');
            return new WP_Error('resource_lock_unavailable', 'An Elementor resource post could not be locked.', ['status' => 503, 'post_id' => $post_id]);
        }
        $wpdb->get_results($wpdb->prepare("SELECT meta_id FROM {$wpdb->postmeta} WHERE post_id = %d FOR UPDATE", $post_id));
        clean_post_cache($post_id);
    }
    $class_ids = $wpdb->get_col($wpdb->prepare("SELECT ID FROM {$wpdb->posts} WHERE post_type = %s FOR UPDATE", 'e_global_class'));
    foreach ($class_ids as $class_id) {
        $wpdb->get_results($wpdb->prepare("SELECT meta_id FROM {$wpdb->postmeta} WHERE post_id = %d FOR UPDATE", (int) $class_id));
        clean_post_cache((int) $class_id);
    }
    return true;
}

function mcp_bridge_cas_document($post_id, $expected, $replacement) {
    global $wpdb;
    $rows = $wpdb->get_results($wpdb->prepare(
        "SELECT meta_id, meta_value FROM {$wpdb->postmeta} WHERE post_id = %d AND meta_key = %s",
        $post_id,
        '_elementor_data'
    ), ARRAY_A);
    if (!is_array($rows) || count($rows) !== 1) return new WP_Error('metadata_conflict', 'Expected exactly one _elementor_data row.', ['status' => 409]);
    $row = $rows[0];
    if ((string) $row['meta_value'] !== (string) $expected) return new WP_Error('stale_revision', 'Document changed during the compare-and-set write.', ['status' => 409]);
    if ((string) $expected === (string) $replacement) return (int) $row['meta_id'];
    $changed = $wpdb->query($wpdb->prepare(
        "UPDATE {$wpdb->postmeta} SET meta_value = %s WHERE meta_id = %d AND post_id = %d AND meta_key = %s AND meta_value = %s",
        $replacement,
        (int) $row['meta_id'],
        $post_id,
        '_elementor_data',
        $expected
    ));
    if ($changed !== 1) return new WP_Error('stale_revision', 'Document changed before the compare-and-set write completed.', ['status' => 409]);
    wp_cache_delete($post_id, 'post_meta');
    clean_post_cache($post_id);
    do_action('updated_post_meta', (int) $row['meta_id'], $post_id, '_elementor_data', $replacement);
    do_action('updated_postmeta', (int) $row['meta_id'], $post_id, '_elementor_data', $replacement);
    return (int) $row['meta_id'];
}

function mcp_bridge_editor_guard_supported() {
    if (!defined('ELEMENTOR_VERSION') || ELEMENTOR_VERSION !== '4.2.3' || !class_exists('Elementor\\Plugin')) return false;
    $plugin = Elementor\Plugin::instance();
    return isset($plugin->documents) && is_object($plugin->documents) && is_callable([$plugin->documents, 'ajax_save']);
}

function mcp_bridge_editor_guard_fail($code, $message, $status = 409) {
    throw new RuntimeException($message, $status);
}

function mcp_bridge_editor_guard_fence($post_id, $acquire = true) {
    global $wpdb;
    $name = 'mcp_editor_' . substr(hash('sha256', DB_NAME . ':' . (int) $post_id), 0, 48);
    $result = $wpdb->get_var($wpdb->prepare($acquire ? 'SELECT GET_LOCK(%s, 0)' : 'SELECT RELEASE_LOCK(%s)', $name));
    return $result === null ? null : (string) $result === '1';
}

function mcp_bridge_editor_guard_full_state($post_id) {
    global $wpdb;
    clean_post_cache($post_id);
    wp_cache_delete($post_id, 'post_meta');
    $post = $wpdb->get_row($wpdb->prepare("SELECT * FROM {$wpdb->posts} WHERE ID = %d", $post_id), ARRAY_A);
    if ($wpdb->last_error !== '') return new WP_Error('native_snapshot_unavailable', 'Could not read the native post row.', ['status' => 503]);
    if (!is_array($post)) return new WP_Error('document_not_found', 'The native editor post row is missing.', ['status' => 404]);
    $metadata = $wpdb->get_results($wpdb->prepare("SELECT meta_id, post_id, meta_key, meta_value FROM {$wpdb->postmeta} WHERE post_id = %d ORDER BY meta_id", $post_id), ARRAY_A);
    if ($wpdb->last_error !== '' || !is_array($metadata)) return new WP_Error('native_snapshot_unavailable', 'Could not read complete native post metadata.', ['status' => 503]);
    $revision_ids = $wpdb->get_col($wpdb->prepare("SELECT ID FROM {$wpdb->posts} WHERE post_type = 'revision' AND post_parent = %d ORDER BY ID", $post_id));
    if ($wpdb->last_error !== '' || !is_array($revision_ids)) return new WP_Error('native_snapshot_unavailable', 'Could not read the native revision family.', ['status' => 503]);
    $revisions = [];
    foreach ($revision_ids as $revision_id) {
        $revision = $wpdb->get_row($wpdb->prepare("SELECT * FROM {$wpdb->posts} WHERE ID = %d", (int) $revision_id), ARRAY_A);
        if ($wpdb->last_error !== '' || !is_array($revision) || (int) $revision['ID'] !== (int) $revision_id) return new WP_Error('native_snapshot_unavailable', 'Could not read a complete native revision row.', ['status' => 503]);
        $revision_meta = $wpdb->get_results($wpdb->prepare("SELECT meta_id, post_id, meta_key, meta_value FROM {$wpdb->postmeta} WHERE post_id = %d ORDER BY meta_id", (int) $revision_id), ARRAY_A);
        if ($wpdb->last_error !== '' || !is_array($revision_meta)) return new WP_Error('native_snapshot_unavailable', 'Could not read complete native revision metadata.', ['status' => 503]);
        $revisions[] = ['post' => $revision, 'meta' => $revision_meta];
    }
            $css = null;
    if (!class_exists('Elementor\\Core\\Files\\CSS\\Post')) return new WP_Error('native_snapshot_unavailable', 'Elementor post CSS API is unavailable.', ['status' => 503]);
    if (class_exists('Elementor\\Core\\Files\\CSS\\Post')) {
        try {
            $css_object = new Elementor\Core\Files\CSS\Post($post_id);
            if (!is_callable([$css_object, 'get_meta']) || !is_callable([$css_object, 'get_path'])) return new WP_Error('native_snapshot_unavailable', 'Elementor CSS inspection API is unavailable.', ['status' => 503]);
            $css_status = $css_object->get_meta('status');
            $css = ['status' => $css_status, 'inline' => $css_object->get_meta('css'), 'path' => null, 'bytes' => null];
            if ($css_status === 'file') {
                $css_path = $css_object->get_path();
                if (!is_file($css_path) || !is_readable($css_path)) return new WP_Error('native_snapshot_unavailable', 'Generated Elementor CSS file cannot be snapshotted.', ['status' => 503]);
                $css_bytes = file_get_contents($css_path);
                if (!is_string($css_bytes)) return new WP_Error('native_snapshot_unavailable', 'Generated Elementor CSS file could not be read.', ['status' => 503]);
                $css['path'] = $css_path;
                $css['bytes'] = base64_encode($css_bytes);
            } elseif (in_array($css_status, [null, ''], true) && $css['inline'] === null) {
                $css['path'] = $css_object->get_path();
                if (is_file($css['path'])) {
                    if (!is_readable($css['path'])) return new WP_Error('native_snapshot_unavailable', 'Generated Elementor CSS file with missing status is unreadable.', ['status' => 503]);
                    $css_bytes = file_get_contents($css['path']);
                    if (!is_string($css_bytes)) return new WP_Error('native_snapshot_unavailable', 'Generated Elementor CSS file with missing status could not be read.', ['status' => 503]);
                    $css['bytes'] = base64_encode($css_bytes);
                }
            } elseif (!in_array($css_status, ['inline', 'empty'], true) || !is_string($css['inline'])) {
                return new WP_Error('native_snapshot_unavailable', 'Elementor CSS state cannot be snapshotted.', ['status' => 503]);
            }
        } catch (Throwable $error) {
            return new WP_Error('native_snapshot_unavailable', $error->getMessage(), ['status' => 503]);
        }
    }
    $atomic_css = mcp_bridge_atomic_css_artifact_state($post_id);
    if (is_wp_error($atomic_css)) return $atomic_css;
    return ['post' => $post, 'meta' => $metadata, 'revisions' => $revisions, 'revision_ids' => array_map('intval', $revision_ids), 'css' => $css, 'atomic_css' => $atomic_css];
}

function mcp_bridge_editor_guard_state_fingerprint($state) {
    return mcp_bridge_hash($state);
}

function mcp_bridge_editor_guard_yoast_integration_available() {
    $class = 'Yoast\\WP\\SEO\\Integrations\\Third_Party\\Elementor';
    if (!defined('WPSEO_VERSION') || !class_exists($class)) return false;
    foreach (['register_hooks', 'save_postdata'] as $method) {
        if (!method_exists($class, $method) || !(new ReflectionMethod($class, $method))->isPublic()) return false;
    }
    return true;
}

function mcp_bridge_editor_guard_yoast_ajax_handler_registered() {
    global $wp_filter;
    $hook = $wp_filter['wp_ajax_wpseo_elementor_save'] ?? null;
    if (!$hook instanceof WP_Hook) return false;
    $matches = [];
    foreach ($hook->callbacks[10] ?? [] as $entry) {
        $callback = $entry['function'] ?? null;
        if (is_array($callback) && is_object($callback[0] ?? null) && get_class($callback[0]) === 'Yoast\\WP\\SEO\\Integrations\\Third_Party\\Elementor' && ($callback[1] ?? '') === 'save_postdata') $matches[] = $callback;
    }
    return count($matches) === 1;
}

function mcp_bridge_editor_guard_native_post_save_mode($mode) {
    if ($mode === null) return null;
    if ($mode !== 'yoast' || !mcp_bridge_editor_guard_yoast_integration_available()) return new WP_Error('native_post_save_unavailable', 'The native Yoast Elementor integration is unavailable.', ['status' => 501]);
    return 'yoast';
}

function mcp_bridge_editor_guard_operation_fingerprint($body, $native_post_save) {
    if ($native_post_save === 'yoast') $body[] = 'yoast';
    return 'native-style-order-v1:' . mcp_bridge_hash($body);
}

function mcp_bridge_editor_guard_legacy_operation_fingerprint($body, $native_post_save) {
    if ($native_post_save === 'yoast') $body[] = 'yoast';
    return mcp_bridge_legacy_hash($body);
}

function mcp_bridge_editor_guard_yoast_response_verified($completed, $body, $status) {
    $response = is_string($body) ? json_decode($body, true) : null;
    return $completed && is_array($response) && ($response['success'] ?? false) === true && $status === 200;
}

function mcp_bridge_editor_guard_yoast_die_handler($handler) {
    return is_array($GLOBALS['mcp_bridge_yoast_post_save'] ?? null) ? 'mcp_bridge_editor_guard_yoast_die' : $handler;
}

function mcp_bridge_editor_guard_yoast_partial($context, $error) {
    global $wpdb;
    if (!is_array($context)) return;
    if (!empty($context['transaction_open'])) $wpdb->query('ROLLBACK');
    $lease = get_option($context['lease_key']);
    if (!is_array($lease) || !hash_equals((string) ($context['token_hash'] ?? ''), (string) ($lease['token_hash'] ?? ''))) return;
    $state = mcp_bridge_editor_guard_full_state((int) $context['post_id']);
    $receipt = get_option($context['operation_key']);
    if (!is_array($receipt)) $receipt = $context['receipt'];
    $receipt['after'] = is_wp_error($state) ? null : $state;
    $receipt['after_fingerprint'] = is_wp_error($state) ? null : mcp_bridge_editor_guard_state_fingerprint($state);
    $receipt['status'] = 'partial_failure';
    $receipt['readback_verified'] = false;
    $receipt['error'] = $error;
    update_option($context['operation_key'], $receipt, false);
    $lease['state'] = 'partial_failure';
    $lease['expires'] = time() + 120;
    update_option($context['lease_key'], $lease, false);
    mcp_bridge_editor_guard_fence((int) $context['post_id'], false);
}

function mcp_bridge_editor_guard_yoast_die($message, $title = '', $args = []) {
    global $wpdb;
    $context = $GLOBALS['mcp_bridge_yoast_post_save'] ?? null;
    if (!is_array($context)) return _ajax_wp_die_handler($message, $title, $args);
    $body = ob_get_contents();
    ob_end_clean();
    $status = http_response_code();
    $success = mcp_bridge_editor_guard_yoast_response_verified(!empty($context['completed']), $body, $status);
    if ($success) {
        $current = mcp_bridge_editor_guard_full_state((int) $context['post_id']);
        $success = !is_wp_error($current) && hash_equals((string) $context['after_fingerprint'], mcp_bridge_editor_guard_state_fingerprint($current));
    }
    if ($success) {
        if ($wpdb->query('COMMIT') === false) {
            mcp_bridge_editor_guard_yoast_partial($context, 'Native Yoast save transaction could not be committed.');
            $success = false;
        } else {
            $receipt = $context['receipt'];
            $receipt['after'] = $context['after'];
            $receipt['after_fingerprint'] = $context['after_fingerprint'];
            $receipt['document_revision'] = $context['document_revision'];
            $receipt['status'] = 'saved';
            $receipt['readback_verified'] = true;
            $receipt['error'] = null;
            update_option($context['operation_key'], $receipt, false);
            delete_option($context['lease_key']);
            mcp_bridge_unlock($context['lease']['lock'] ?? []);
            mcp_bridge_editor_guard_fence((int) $context['post_id'], false);
        }
    } else {
        mcp_bridge_editor_guard_yoast_partial($context, 'Native Yoast AJAX response did not confirm a completed successful save.');
    }
    unset($GLOBALS['mcp_bridge_yoast_post_save']);
    if (!$success) {
        if (!headers_sent()) status_header(500);
        _ajax_wp_die_handler(wp_json_encode(['success' => false, 'data' => 'Native Yoast save was not verified.']), '', ['response' => null]);
        return;
    }
    echo $body;
    _ajax_wp_die_handler($message, $title, $args);
}

function mcp_bridge_editor_guard_yoast_before() {
    global $wpdb;
    if (($_REQUEST['action'] ?? '') !== 'wpseo_elementor_save') return;
    $post_id = isset($_POST['post_id']) && is_string($_POST['post_id']) ? absint(wp_unslash($_POST['post_id'])) : 0;
    if (!$post_id) {
        if (isset($_POST['mcp_guard_token'])) wp_send_json_error('The guarded Yoast save has no post ID.', 409);
        return;
    }
    $key = '_mcp_elementor_editor_lease_' . $post_id;
    $lease = get_option($key);
    if (!is_array($lease) || ($lease['native_post_save'] ?? null) !== 'yoast') {
        if (isset($_POST['mcp_guard_token'])) wp_send_json_error('No guarded Yoast save matches this request.', 409);
        return;
    }
    $token = isset($_POST['mcp_guard_token']) && is_string($_POST['mcp_guard_token']) ? wp_unslash($_POST['mcp_guard_token']) : '';
    if (($_POST['action'] ?? '') !== 'wpseo_elementor_save' || !is_user_logged_in() || !current_user_can('edit_post', $post_id) || ($lease['state'] ?? '') !== 'awaiting_post_save' || (int) ($lease['expires'] ?? 0) <= time() || (int) ($lease['user_id'] ?? 0) !== get_current_user_id() || (int) ($lease['post_id'] ?? 0) !== $post_id || $token === '' || !hash_equals((string) ($lease['token_hash'] ?? ''), hash('sha256', $token)) || !mcp_bridge_editor_guard_yoast_integration_available() || !mcp_bridge_editor_guard_yoast_ajax_handler_registered() || ($lease['yoast_runtime_version'] ?? '') !== WPSEO_VERSION) wp_send_json_error('The guarded Yoast save is not authorized.', 409);
    $operation_key = '_mcp_elementor_editor_operation_' . hash('sha256', (string) $lease['operation_id']);
    $receipt = get_option($operation_key);
    if (!is_array($receipt) || ($receipt['status'] ?? '') !== 'awaiting_post_save' || (int) ($receipt['post_id'] ?? 0) !== $post_id || (int) ($receipt['user_id'] ?? 0) !== get_current_user_id()) wp_send_json_error('The guarded Yoast operation is unavailable.', 409);
    if (!mcp_bridge_editor_guard_fence($post_id)) wp_send_json_error('The guarded document is busy.', 409);
    $locked = mcp_bridge_begin_resource_transaction([['kind' => 'post_metadata', 'id' => (string) $post_id]]);
    if (is_wp_error($locked)) { mcp_bridge_editor_guard_fence($post_id, false); wp_send_json_error('The guarded document could not be locked.', 503); }
    $wpdb->get_results($wpdb->prepare("SELECT ID FROM {$wpdb->posts} WHERE post_type = 'revision' AND post_parent = %d ORDER BY ID FOR UPDATE", $post_id));
    $current = mcp_bridge_editor_guard_full_state($post_id);
    if (is_wp_error($current) || !hash_equals((string) ($lease['intermediate_fingerprint'] ?? ''), mcp_bridge_editor_guard_state_fingerprint($current))) {
        $wpdb->query('ROLLBACK');
        mcp_bridge_editor_guard_fence($post_id, false);
        wp_send_json_error('The guarded document changed before the Yoast save.', 409);
    }
    $raw = $wpdb->get_var($wpdb->prepare("SELECT option_value FROM {$wpdb->options} WHERE option_name = %s FOR UPDATE", $key));
    $stored = maybe_unserialize($raw);
    if (!is_array($stored) || ($stored['state'] ?? '') !== 'awaiting_post_save' || !hash_equals((string) ($stored['token_hash'] ?? ''), hash('sha256', $token))) {
        $wpdb->query('ROLLBACK');
        mcp_bridge_editor_guard_fence($post_id, false);
        wp_send_json_error('The guarded Yoast token was already consumed.', 409);
    }
    $stored['state'] = 'running_post_save';
    if ($wpdb->query($wpdb->prepare('UPDATE ' . $wpdb->options . ' SET option_value = %s WHERE option_name = %s AND option_value = %s', maybe_serialize($stored), $key, $raw)) !== 1) {
        $wpdb->query('ROLLBACK');
        mcp_bridge_editor_guard_fence($post_id, false);
        wp_send_json_error('The guarded Yoast token was already consumed.', 409);
    }
    wp_cache_delete($key, 'options');
    $GLOBALS['mcp_bridge_yoast_post_save'] = ['post_id' => $post_id, 'lease_key' => $key, 'operation_key' => $operation_key, 'token_hash' => hash('sha256', $token), 'lease' => $stored, 'receipt' => $receipt, 'intermediate' => $current, 'transaction_open' => true, 'completed' => false];
    ob_start();
    add_action('wpseo_saved_postdata', 'mcp_bridge_editor_guard_yoast_completed', PHP_INT_MAX);
    add_filter('wp_die_ajax_handler', 'mcp_bridge_editor_guard_yoast_die_handler', PHP_INT_MAX);
    register_shutdown_function(static function() {
        $context = $GLOBALS['mcp_bridge_yoast_post_save'] ?? null;
        if (is_array($context)) mcp_bridge_editor_guard_yoast_partial($context, 'The native Yoast save request ended before its response was verified.');
    });
}

function mcp_bridge_editor_guard_yoast_completed() {
    $context = $GLOBALS['mcp_bridge_yoast_post_save'] ?? null;
    if (!is_array($context) || $context['completed']) return;
    $post_id = (int) $context['post_id'];
    $after = mcp_bridge_editor_guard_full_state($post_id);
    $document = get_post_meta($post_id, '_elementor_data', true);
    $intermediate = $context['intermediate'];
    $valid = !is_wp_error($after) && is_string($document) && hash_equals((string) ($context['receipt']['document_revision'] ?? ''), mcp_bridge_hash($document)) && $after['revision_ids'] === $intermediate['revision_ids'] && mcp_bridge_hash($after['css']) === mcp_bridge_hash($intermediate['css']);
    if (!$valid) {
        mcp_bridge_editor_guard_yoast_partial($context, 'Native Yoast save changed the Elementor document, revisions, or CSS contract.');
        $context['transaction_open'] = false;
        $GLOBALS['mcp_bridge_yoast_post_save'] = $context;
        return;
    }
    $context['after'] = $after;
    $context['after_fingerprint'] = mcp_bridge_editor_guard_state_fingerprint($after);
    $context['document_revision'] = mcp_bridge_hash($document);
    $context['completed'] = true;
    $GLOBALS['mcp_bridge_yoast_post_save'] = $context;
}

add_action('wp_ajax_wpseo_elementor_save', 'mcp_bridge_editor_guard_yoast_before', 0);

function mcp_bridge_editor_guard_settings_expectation($post_id, $settings, $readback = false) {
    global $wpdb;
    if (!mcp_bridge_editor_guard_supported() || !class_exists('Elementor\\Core\\Settings\\Manager')) return new WP_Error('editor_guard_unavailable', 'The active Elementor page settings API is unavailable.', ['status' => 501]);
    try {
        $manager = Elementor\Core\Settings\Manager::get_settings_managers('page');
        $method = new ReflectionMethod($manager, 'get_special_settings_names');
        if (!$method->isProtected() || $method->getDeclaringClass()->getName() !== get_class($manager)) return new WP_Error('editor_guard_unavailable', 'Elementor page special settings API changed.', ['status' => 501]);
        $method->setAccessible(true);
        $special_names = $method->invoke($manager);
    } catch (Throwable $error) {
        return new WP_Error('editor_guard_unavailable', 'Elementor page special settings API could not be inspected.', ['status' => 501]);
    }
    $expected_special_names = ['id', 'post_title', 'post_status', 'template', 'post_excerpt', 'post_featured_image', 'menu_order', 'comment_status'];
    if ($special_names !== $expected_special_names) return new WP_Error('editor_guard_unavailable', 'Elementor page special settings changed for the verified runtime.', ['status' => 501]);
    $special = array_fill_keys($special_names, true);
    $ordinary = array_diff_key($settings, $special);
    $raw_meta = $wpdb->get_col($wpdb->prepare("SELECT meta_value FROM {$wpdb->postmeta} WHERE post_id = %d AND meta_key = %s ORDER BY meta_id", $post_id, '_elementor_page_settings'));
    if ($wpdb->last_error !== '' || !is_array($raw_meta)) return new WP_Error('native_snapshot_unavailable', 'Could not read native Elementor page settings.', ['status' => 503]);
    $saved_ordinary = count($raw_meta) === 0 ? [] : (count($raw_meta) === 1 ? maybe_unserialize($raw_meta[0]) : null);
    if (!is_array($saved_ordinary)) $saved_ordinary = null;
    $ordinary_paths = $readback ? ($saved_ordinary === null ? ['$'] : mcp_bridge_editor_guard_diff_paths($ordinary, $saved_ordinary)) : [];
    $post = get_post($post_id);
    if (!$post) return new WP_Error('document_not_found', 'The native editor post row is missing.', ['status' => 404]);
    $actual = [
        'id' => (string) $post->ID,
        'post_title' => $post->post_title,
        'post_status' => $post->post_status,
        'template' => get_post_meta($post_id, '_wp_page_template', true) ?: 'default',
        'post_excerpt' => $post->post_excerpt,
        'post_featured_image' => (string) get_post_thumbnail_id($post_id),
        'menu_order' => (string) $post->menu_order,
        'comment_status' => $post->comment_status,
    ];
    $supported = [
        'id' => true,
        'post_title' => true,
        'post_status' => true,
        'template' => Elementor\Utils::is_cpt_custom_templates_supported(),
        'post_excerpt' => post_type_supports($post->post_type, 'excerpt'),
        'post_featured_image' => post_type_supports($post->post_type, 'thumbnail') && current_user_can('publish_posts'),
        'menu_order' => is_post_type_hierarchical($post->post_type),
        'comment_status' => post_type_supports($post->post_type, 'comments'),
    ];
    $special_paths = [];
    foreach ($special_names as $name) {
        if (!array_key_exists($name, $settings)) continue;
        if (!$supported[$name]) { $special_paths[] = '$.' . $name; continue; }
        if ($name === 'post_title' && $settings[$name] === '' && $actual[$name] !== '') { $special_paths[] = '$.' . $name; continue; }
        if ($readback && (string) $settings[$name] !== $actual[$name]) $special_paths[] = '$.' . $name;
    }
    return ['ordinary_matches' => $ordinary_paths === [], 'ordinary_paths' => $ordinary_paths, 'special_matches' => $special_paths === [], 'special_paths' => $special_paths];
}

function mcp_bridge_editor_guard_delegate($native, $request, $post_id) {
    if (!$post_id) return call_user_func($native, $request);
    if (!mcp_bridge_editor_guard_fence($post_id)) mcp_bridge_editor_guard_fail('document_busy', 'Another native editor callback owns the post fence.');
    global $wpdb;
    $transaction = false;
    try {
        $locked = mcp_bridge_begin_resource_transaction([['kind' => 'post_metadata', 'id' => (string) $post_id]]);
        if (is_wp_error($locked)) mcp_bridge_editor_guard_fail('native_fence_unavailable', $locked->get_error_message(), 503);
        $transaction = true;
        $result = call_user_func($native, $request);
        if (is_wp_error($result)) mcp_bridge_editor_guard_fail('native_save_failed', $result->get_error_message(), 500);
        if ($wpdb->query('COMMIT') === false) mcp_bridge_editor_guard_fail('native_save_commit_failed', 'Native editor transaction could not be committed.', 500);
        $transaction = false;
        return $result;
    } catch (Throwable $error) {
        if ($transaction) $wpdb->query('ROLLBACK');
        throw $error;
    } finally { mcp_bridge_editor_guard_fence($post_id, false); }
}

function mcp_bridge_editor_guard_canonical($value) {
    if (!is_array($value)) return $value;
    foreach ($value as &$element) {
        if (!is_array($element) || !in_array($element['elType'] ?? null, ['widget', 'section', 'column', 'container', 'wrapper', 'e-flexbox'], true)) continue;
        $element_type = $element['elType'];
        if (($element_type === 'widget' && in_array($element['isInner'] ?? null, [false, null], true)) || (in_array($element_type, ['section', 'column'], true) && ($element['isInner'] ?? null) === false)) unset($element['isInner']);
        foreach (['interactions', 'editor_settings'] as $field) if (($element[$field] ?? null) === []) unset($element[$field]);
        if (mcp_bridge_native_style_map($element['styles'] ?? null)) {
            foreach ($element['styles'] as &$style) {
                if (($style['type'] ?? null) !== 'class' || !is_array($style['variants'] ?? null)) continue;
                foreach ($style['variants'] as &$variant) {
                    $meta = $variant['meta'] ?? null;
                    $valid = is_array($meta) && in_array($meta['breakpoint'] ?? null, ['desktop', 'tablet', 'mobile'], true) && (!isset($meta['state']) || $meta['state'] === null || is_string($meta['state'])) && is_array($variant['props'] ?? null);
                    if ($valid && array_key_exists('custom_css', $variant) && $variant['custom_css'] === null) unset($variant['custom_css']);
                }
                unset($variant);
            }
            unset($style);
        }
        if (isset($element['elements']) && is_array($element['elements'])) $element['elements'] = mcp_bridge_editor_guard_canonical($element['elements']);
    }
    unset($element);
    return $value;
}

function mcp_bridge_editor_guard_native_tree($elements) {
    $manager = Elementor\Plugin::instance()->elements_manager;
    $serialize = static function($element_data) use ($manager) {
        if (!is_array($element_data) || !isset($element_data['elType'])) return new WP_Error('native_serializer_unavailable', 'Elementor element data cannot be serialized by the registered native element manager.', ['status' => 501]);
        $element = $manager->create_element_instance($element_data);
        if (!$element || !is_callable([$element, 'get_data_for_save'])) return new WP_Error('native_serializer_unavailable', 'Elementor has no registered native save serializer for an element in this document.', ['status' => 501]);
        $serialized = $element->get_data_for_save();
        if (!is_array($serialized)) return new WP_Error('native_serializer_unavailable', 'Elementor native element serialization returned an invalid result.', ['status' => 501]);
        return $serialized;
    };
    $serialized = [];
    foreach ($elements as $element_data) {
        $element = $serialize($element_data);
        if (is_wp_error($element)) return $element;
        $serialized[] = $element;
    }
    return $serialized;
}

function mcp_bridge_editor_guard_check_native_tree($elements) {
    $serialized = mcp_bridge_editor_guard_native_tree($elements);
    if (is_wp_error($serialized)) return $serialized;
    $paths = mcp_bridge_editor_guard_diff_paths(mcp_bridge_editor_guard_canonical($elements), mcp_bridge_editor_guard_canonical($serialized));
    if (!$paths && mcp_bridge_hash(mcp_bridge_editor_guard_canonical($elements)) !== mcp_bridge_hash(mcp_bridge_editor_guard_canonical($serialized))) $paths[] = '$.styles (native declaration order)';
    if ($paths) return new WP_Error('native_serializer_loss', 'The active Elementor native serializer would drop unsupported document fields at ' . implode(', ', $paths) . '.', ['status' => 501, 'paths' => $paths]);
    return true;
}

function mcp_bridge_editor_guard_diff_paths($expected, $actual, $path = '$', &$paths = []) {
    if (count($paths) >= 20 || $expected === $actual) return $paths;
    if (!is_array($expected) || !is_array($actual)) { $paths[] = $path; return $paths; }
    foreach (array_unique(array_merge(array_keys($expected), array_keys($actual))) as $key) {
        if (!array_key_exists($key, $expected) || !array_key_exists($key, $actual)) $paths[] = $path . '.' . (string) $key;
        else mcp_bridge_editor_guard_diff_paths($expected[$key], $actual[$key], $path . '.' . (string) $key, $paths);
        if (count($paths) >= 20) break;
    }
    return $paths;
}

function mcp_bridge_editor_guard_restore_state($post_id, $snapshot, $owned_revision_ids) {
    global $wpdb;
    if (!isset($snapshot['post'], $snapshot['meta'], $snapshot['revisions'], $snapshot['css']) || !is_array($snapshot['post']) || !is_array($snapshot['meta']) || !is_array($snapshot['revisions'])) return new WP_Error('restore_snapshot_invalid', 'The native restore snapshot is incomplete.', ['status' => 500]);
    try {
        $current_post = $wpdb->get_var($wpdb->prepare("SELECT ID FROM {$wpdb->posts} WHERE ID = %d FOR UPDATE", $post_id));
        if (!$current_post) throw new RuntimeException('Native post disappeared during restore.');
        $post_data = $snapshot['post'];
        unset($post_data['ID']);
        if ($wpdb->update($wpdb->posts, $post_data, ['ID' => $post_id]) === false) throw new RuntimeException('Could not restore the native post row.');
        if ($wpdb->delete($wpdb->postmeta, ['post_id' => $post_id], ['%d']) === false) throw new RuntimeException('Could not clear current native post metadata.');
        foreach ($snapshot['meta'] as $meta) {
            if ($wpdb->insert($wpdb->postmeta, $meta) === false) throw new RuntimeException('Could not restore complete native post metadata.');
        }
        foreach ($snapshot['revisions'] as $revision) {
            if (!is_array($revision['post'] ?? null) || (int) ($revision['post']['post_parent'] ?? 0) !== $post_id || ($revision['post']['post_type'] ?? '') !== 'revision' || !is_array($revision['meta'] ?? null)) throw new RuntimeException('The revision snapshot is incomplete or belongs to another post.');
            $revision_id = (int) $revision['post']['ID'];
            if ($wpdb->delete($wpdb->postmeta, ['post_id' => $revision_id], ['%d']) === false) throw new RuntimeException('Could not clear a current native revision metadata row.');
            if ($wpdb->delete($wpdb->posts, ['ID' => $revision_id], ['%d']) === false) throw new RuntimeException('Could not clear a current native revision row.');
            if ($wpdb->insert($wpdb->posts, $revision['post']) === false) throw new RuntimeException('Could not restore a pre-existing revision row.');
            foreach ($revision['meta'] as $meta) {
                if ($wpdb->insert($wpdb->postmeta, $meta) === false) throw new RuntimeException('Could not restore complete pre-existing revision metadata.');
            }
            clean_post_cache($revision_id);
        }
        foreach ($owned_revision_ids as $revision_id) {
            if (in_array((int) $revision_id, $snapshot['revision_ids'], true)) continue;
            if ($wpdb->delete($wpdb->postmeta, ['post_id' => (int) $revision_id], ['%d']) === false) throw new RuntimeException('Could not remove native-owned revision metadata.');
            if ($wpdb->delete($wpdb->posts, ['ID' => (int) $revision_id, 'post_type' => 'revision', 'post_parent' => $post_id], ['%d', '%s', '%d']) === false) throw new RuntimeException('Could not remove a native-owned revision row.');
            clean_post_cache((int) $revision_id);
        }
    } catch (Throwable $error) {
        return new WP_Error('restore_failed', $error->getMessage(), ['status' => 500]);
    }
    clean_post_cache($post_id);
    wp_cache_delete($post_id, 'post_meta');
    return true;
}

function mcp_bridge_editor_guard_restore_css($post_id, $snapshot) {
    if (is_array($snapshot['css'])) {
        try {
            $css = new Elementor\Core\Files\CSS\Post($post_id);
            $path = is_callable([$css, 'get_path']) ? $css->get_path() : null;
            if (($snapshot['css']['status'] ?? '') === 'file' || is_string($snapshot['css']['bytes'] ?? null)) {
                $bytes = base64_decode((string) ($snapshot['css']['bytes'] ?? ''), true);
                if (!is_string($path) || !is_string($bytes) || file_put_contents($path, $bytes, LOCK_EX) !== strlen($bytes)) return new WP_Error('restore_partial', 'Database state was restored, but the original generated CSS file could not be restored.', ['status' => 500]);
            } elseif (is_string($path) && is_file($path) && !unlink($path)) {
                return new WP_Error('restore_partial', 'Database state was restored, but the generated CSS file could not be removed.', ['status' => 500]);
            }
        } catch (Throwable $error) {
            return new WP_Error('restore_partial', 'Database state was restored, but CSS artifact restoration failed: ' . $error->getMessage(), ['status' => 500]);
        }
    }
    return mcp_bridge_editor_guard_restore_atomic_css($post_id, $snapshot['atomic_css'] ?? null);
}

function mcp_bridge_editor_guard_restore_atomic_css($post_id, $snapshot) {
    if (!is_array($snapshot) || !array_key_exists('cache_validity', $snapshot) || !is_array($snapshot['files'] ?? null)) return new WP_Error('restore_partial', 'The original Atomic CSS cache artifacts were not fully captured.', ['status' => 500]);
    $current = mcp_bridge_atomic_css_artifact_state($post_id);
    if (is_wp_error($current)) return $current;
    $upload = wp_upload_dir();
    if (!empty($upload['error']) || empty($upload['basedir'])) return new WP_Error('restore_partial', 'Atomic CSS artifact directory is unavailable during restore.', ['status' => 500]);
    foreach ($current['files'] as $name => $_hash) {
        if (isset($snapshot['files'][$name])) continue;
        $path = trailingslashit($upload['basedir']) . 'elementor/css/' . basename($name);
        if (is_file($path) && !unlink($path)) return new WP_Error('restore_partial', 'A generated Atomic CSS artifact could not be removed during restore.', ['status' => 500]);
    }
    foreach ($snapshot['files'] as $name => $file) {
        $bytes = base64_decode((string) ($file['bytes'] ?? ''), true);
        if (!is_string($bytes) || !hash_equals((string) ($file['sha256'] ?? ''), hash('sha256', $bytes))) return new WP_Error('restore_partial', 'A saved Atomic CSS artifact failed its integrity check.', ['status' => 500]);
        $path = trailingslashit($upload['basedir']) . 'elementor/css/' . basename($name);
        if (!is_dir(dirname($path)) && !wp_mkdir_p(dirname($path))) return new WP_Error('restore_partial', 'The Atomic CSS artifact directory could not be restored.', ['status' => 500]);
        if (file_put_contents($path, $bytes, LOCK_EX) !== strlen($bytes)) return new WP_Error('restore_partial', 'An original Atomic CSS artifact could not be restored.', ['status' => 500]);
    }
    $key = 'elementor_atomic_cache_validity__local';
    $tree = get_option($key, ['state' => false]);
    if (!is_array($tree)) return new WP_Error('restore_partial', 'Atomic CSS cache validity metadata is unreadable during restore.', ['status' => 500]);
    if ($snapshot['cache_validity'] === null) unset($tree['children'][(string) (int) $post_id]);
    else $tree['children'][(string) (int) $post_id] = $snapshot['cache_validity'];
    if (!update_option($key, $tree, false) && mcp_bridge_hash(get_option($key)) !== mcp_bridge_hash($tree)) return new WP_Error('restore_partial', 'Original Atomic CSS cache validity metadata could not be restored.', ['status' => 500]);
    return true;
}

function mcp_bridge_editor_guard_restore_matches($before, $after) {
    if (!is_array($before) || !is_array($after)) return false;
    return hash_equals(mcp_bridge_hash($before), mcp_bridge_hash($after));
}

function mcp_bridge_editor_guard_capture_ready() {
    if (!mcp_bridge_editor_guard_supported()) return new WP_Error('editor_guard_unavailable', 'This Elementor runtime has no verified native save adapter.', ['status' => 501]);
    $plugin = Elementor\Plugin::instance();
    if (!$plugin->common && is_callable([$plugin, 'init_common'])) $plugin->init_common();
    if (!isset($plugin->common) || !is_callable([$plugin->common, 'get_component'])) return new WP_Error('editor_guard_unavailable', 'Elementor AJAX manager is unavailable.', ['status' => 501]);
    $ajax = $plugin->common->get_component('ajax');
    if (!is_object($ajax)) return new WP_Error('editor_guard_unavailable', 'Elementor AJAX manager is unavailable.', ['status' => 501]);
    $GLOBALS['mcp_bridge_editor_guard_callback_id'] = null;
    try {
        do_action('elementor/ajax/register_actions', $ajax);
        $property = new ReflectionProperty($ajax, 'ajax_actions');
        if (!$property->isPrivate()) return new WP_Error('editor_guard_unavailable', 'Elementor AJAX callback registry visibility changed.', ['status' => 501]);
        $property->setAccessible(true);
        $callback = $property->getValue($ajax)['save_builder']['callback'] ?? null;
    } catch (Throwable $error) {
        return new WP_Error('editor_guard_unavailable', 'Elementor native save callback could not be inspected: ' . $error->getMessage(), ['status' => 501]);
    }
    if (!($callback instanceof Closure) || empty($GLOBALS['mcp_bridge_editor_guard_callback_id']) || spl_object_id($callback) !== $GLOBALS['mcp_bridge_editor_guard_callback_id']) return new WP_Error('editor_guard_unavailable', 'The verified native save callback was not replaced by the MCP guard.', ['status' => 501]);
    return true;
}

function mcp_bridge_editor_guard_register($ajax_manager) {
    if (!mcp_bridge_editor_guard_supported() || !is_object($ajax_manager) || !is_callable([$ajax_manager, 'register_ajax_action'])) return;
    try {
        $property = new ReflectionProperty($ajax_manager, 'ajax_actions');
        if (!$property->isPrivate()) return;
        $property->setAccessible(true);
        $actions = $property->getValue($ajax_manager);
    } catch (Throwable $error) {
        $post_id = absint($_REQUEST['editor_post_id'] ?? 0);
        if ($post_id && is_array(get_option('_mcp_elementor_editor_lease_' . $post_id))) $ajax_manager->register_ajax_action('save_builder', static function() use ($error) { throw new RuntimeException('Guarded native save is unavailable: ' . $error->getMessage(), 503); });
        return;
    }
    $native = $actions['save_builder']['callback'] ?? null;
    if (!is_callable($native) || !is_array($native) || $native[0] !== Elementor\Plugin::instance()->documents || $native[1] !== 'ajax_save') {
        $post_id = absint($_REQUEST['editor_post_id'] ?? 0);
        if ($post_id && is_array(get_option('_mcp_elementor_editor_lease_' . $post_id))) $ajax_manager->register_ajax_action('save_builder', static function() { throw new RuntimeException('Guarded native save callback changed and is unavailable.', 503); });
        return;
    }
    $guard = static function($request, $manager) use ($native) {
        $post_id = absint($request['editor_post_id'] ?? 0);
        $token = $request['mcp_guard_token'] ?? '';
        if (!$post_id) {
            if ($token !== '') mcp_bridge_editor_guard_fail('editor_guard_rejected', 'No guarded document matches this one-use save token.');
            return mcp_bridge_editor_guard_delegate($native, $request, $post_id);
        }
        $key = '_mcp_elementor_editor_lease_' . $post_id;
        $lease = get_option($key);
        if (!is_array($lease)) {
            if ($token !== '') mcp_bridge_editor_guard_fail('editor_guard_rejected', 'No guarded document matches this one-use save token.');
            return mcp_bridge_editor_guard_delegate($native, $request, $post_id);
        }
        if (($lease['state'] ?? '') === 'armed' && (int) ($lease['expires'] ?? 0) <= time()) {
            delete_option($key);
            if (isset($lease['lock']) && is_array($lease['lock'])) mcp_bridge_unlock($lease['lock']);
            if (!empty($request['mcp_guard_token'])) mcp_bridge_editor_guard_fail('editor_guard_expired', 'The guarded editor save lease expired before Elementor received the save.');
            return mcp_bridge_editor_guard_delegate($native, $request, $post_id);
        }
        if (($lease['state'] ?? '') === 'running') mcp_bridge_editor_guard_fail('document_busy', 'A guarded native save is already running for this document.');
        if ((int) ($lease['expires'] ?? 0) <= time()) mcp_bridge_editor_guard_fail('editor_guard_expired', 'The guarded native save is still marked running past its lease expiry; refusing competing writes.');
        if (absint($_REQUEST['editor_post_id'] ?? 0) !== $post_id) mcp_bridge_editor_guard_fail('editor_guard_post_mismatch', 'The guarded save must use the leased outer Elementor editor post ID.');
        $status = $request['status'] ?? '';
        $elements = $request['elements'] ?? null;
        $settings = $request['settings'] ?? null;
        $expected = $lease['expected_revision'] ?? '';
        $operation_id = $lease['operation_id'] ?? '';
        if ((int) ($lease['user_id'] ?? 0) !== get_current_user_id() || !is_string($token) || !hash_equals((string) ($lease['token_hash'] ?? ''), hash('sha256', $token)) || !in_array($status, ['draft', 'publish'], true) || !is_array($elements) || !array_is_list($elements) || !is_array($settings)) {
            mcp_bridge_editor_guard_fail('editor_guard_rejected', 'A guarded save requires its one-use token, full elements and settings, and an explicit draft or publish status.');
        }
        $current = get_post_meta($post_id, '_elementor_data', true);
        if (!is_string($current) || !hash_equals((string) $expected, mcp_bridge_hash($current))) mcp_bridge_editor_guard_fail('stale_revision', 'The document changed after the guarded save was prepared.');
        $before_state = mcp_bridge_editor_guard_full_state($post_id);
        if (is_wp_error($before_state) || !hash_equals((string) ($lease['before_fingerprint'] ?? ''), mcp_bridge_editor_guard_state_fingerprint($before_state))) mcp_bridge_editor_guard_fail('stale_native_state', 'Native post metadata, revision, or CSS state changed after the guarded save was prepared.');
        $tree = json_decode($current, true);
        $previous = [];
        if (is_array($tree)) mcp_bridge_element_index($tree, $previous);
        $ids = [];
        $valid = mcp_bridge_validate_tree($elements, $ids, $previous);
        if (is_wp_error($valid)) mcp_bridge_editor_guard_fail($valid->get_error_code(), $valid->get_error_message(), (int) ($valid->get_error_data()['status'] ?? 409));
        $native_tree = mcp_bridge_editor_guard_check_native_tree($elements);
        if (is_wp_error($native_tree)) mcp_bridge_editor_guard_fail($native_tree->get_error_code(), $native_tree->get_error_message(), (int) ($native_tree->get_error_data()['status'] ?? 501));
        $settings_expectation = mcp_bridge_editor_guard_settings_expectation($post_id, $settings);
        if (is_wp_error($settings_expectation)) mcp_bridge_editor_guard_fail($settings_expectation->get_error_code(), $settings_expectation->get_error_message(), (int) ($settings_expectation->get_error_data()['status'] ?? 501));
        if ($settings_expectation['special_paths']) mcp_bridge_editor_guard_fail('editor_guard_unsupported_setting', 'Elementor cannot natively save requested page settings at ' . implode(', ', $settings_expectation['special_paths']) . '.', 501);
        $fingerprint_body = [$post_id, get_current_user_id(), $expected, $operation_id, ELEMENTOR_VERSION, $status, $elements, $settings];
        $fingerprint = mcp_bridge_editor_guard_operation_fingerprint($fingerprint_body, $lease['native_post_save'] ?? null);
        if (!mcp_bridge_ordered_fingerprint_matches($lease['fingerprint'] ?? '', $fingerprint, mcp_bridge_editor_guard_legacy_operation_fingerprint($fingerprint_body, $lease['native_post_save'] ?? null), $fingerprint_body)) mcp_bridge_editor_guard_fail('editor_guard_body_mismatch', 'The native save body differs from the leased request.');
        if (($lease['state'] ?? '') !== 'armed') mcp_bridge_editor_guard_fail('document_busy', 'The guarded editor save is already running or has been consumed.');
        global $wpdb;
        $raw = $wpdb->get_var($wpdb->prepare("SELECT option_value FROM {$wpdb->options} WHERE option_name = %s", $key));
        $stored = maybe_unserialize($raw);
        if (!is_array($stored) || ($stored['state'] ?? '') !== 'armed' || !hash_equals((string) ($stored['token_hash'] ?? ''), (string) ($lease['token_hash'] ?? ''))) mcp_bridge_editor_guard_fail('document_busy', 'The guarded editor save is already running or has been consumed.');
        if (!mcp_bridge_editor_guard_fence($post_id)) mcp_bridge_editor_guard_fail('document_busy', 'A native editor callback still owns the post fence.');
        $locked = mcp_bridge_begin_resource_transaction([['kind' => 'post_metadata', 'id' => (string) $post_id]]);
        if (is_wp_error($locked)) {
            mcp_bridge_editor_guard_fence($post_id, false);
            mcp_bridge_editor_guard_fail('native_fence_unavailable', $locked->get_error_message(), 503);
        }
        $locked_document = get_post_meta($post_id, '_elementor_data', true);
        $locked_state = mcp_bridge_editor_guard_full_state($post_id);
        if (!is_string($locked_document) || !hash_equals((string) $expected, mcp_bridge_hash($locked_document)) || is_wp_error($locked_state) || !hash_equals((string) ($lease['before_fingerprint'] ?? ''), mcp_bridge_editor_guard_state_fingerprint($locked_state))) {
            $wpdb->query('ROLLBACK');
            mcp_bridge_editor_guard_fence($post_id, false);
            mcp_bridge_editor_guard_fail('stale_native_state', 'Native post state changed before the guarded save acquired its database row locks.');
        }
        $before_state = $locked_state;
        $lease['state'] = 'running';
        $claimed = $wpdb->query($wpdb->prepare("UPDATE {$wpdb->options} SET option_value = %s WHERE option_name = %s AND option_value = %s", maybe_serialize($lease), $key, $raw));
        if ($claimed !== 1) {
            $wpdb->query('ROLLBACK');
            mcp_bridge_editor_guard_fence($post_id, false);
            mcp_bridge_editor_guard_fail('document_busy', 'The guarded editor save is already running or has been consumed.');
        }
        wp_cache_delete($key, 'options');
        $editor_transaction_open = true;
        $preserve_post_save_lease = false;
        $native_revision_ids = [];
        $revision_capture = static function($revision_id, $revision, $update, $post_before) use ($post_id, $before_state, &$native_revision_ids) {
            if ($update === false && $revision instanceof WP_Post && $revision->post_type === 'revision' && (int) $revision->post_parent === $post_id && !in_array((int) $revision_id, $before_state['revision_ids'], true)) $native_revision_ids[] = (int) $revision_id;
        };
        add_action('wp_after_insert_post', $revision_capture, PHP_INT_MAX, 4);
        try {
            $native_result = call_user_func($native, $request);
            $css_generated = mcp_bridge_regenerate_css($post_id);
            $after_state = mcp_bridge_editor_guard_full_state($post_id);
            $saved_document = get_post_meta($post_id, '_elementor_data', true);
            $saved_tree = is_string($saved_document) ? json_decode($saved_document, true) : null;
            $saved_settings = mcp_bridge_editor_guard_settings_expectation($post_id, $settings, true);
            $css_state = mcp_bridge_css_artifact_state($post_id);
            $saved_post = get_post($post_id);
            $verified = !is_wp_error($native_result) && !is_wp_error($css_generated) && !is_wp_error($after_state) && !is_wp_error($css_state) && $css_state !== null && $saved_post && $saved_post->post_status === $status && is_array($saved_tree) && hash_equals(mcp_bridge_hash(mcp_bridge_editor_guard_canonical($elements)), mcp_bridge_hash(mcp_bridge_editor_guard_canonical($saved_tree))) && is_array($saved_settings) && $saved_settings['ordinary_matches'] && $saved_settings['special_matches'];
            $verification_errors = [];
            if (is_wp_error($native_result)) $verification_errors[] = 'native save: ' . $native_result->get_error_message();
            if (is_wp_error($css_generated)) $verification_errors[] = 'CSS generation: ' . $css_generated->get_error_message();
            if (is_wp_error($after_state)) $verification_errors[] = 'full-state readback: ' . $after_state->get_error_message();
            if (is_wp_error($css_state)) $verification_errors[] = 'CSS readback: ' . $css_state->get_error_message();
            if ($css_state === null) $verification_errors[] = 'CSS artifact missing';
            if (!$saved_post || $saved_post->post_status !== $status) $verification_errors[] = 'post status mismatch';
            if (!is_array($saved_tree) || hash_equals(mcp_bridge_hash(mcp_bridge_editor_guard_canonical($elements)), mcp_bridge_hash(mcp_bridge_editor_guard_canonical($saved_tree))) === false) $verification_errors[] = 'document tree mismatch at ' . implode(', ', mcp_bridge_editor_guard_diff_paths(mcp_bridge_editor_guard_canonical($elements), mcp_bridge_editor_guard_canonical($saved_tree)));
            if (is_wp_error($saved_settings)) $verification_errors[] = 'page settings readback: ' . $saved_settings->get_error_message();
            elseif (!$saved_settings['ordinary_matches']) $verification_errors[] = 'ordinary page settings mismatch at ' . implode(', ', $saved_settings['ordinary_paths']);
            if (is_array($saved_settings) && !$saved_settings['special_matches']) $verification_errors[] = 'native page settings mismatch at ' . implode(', ', $saved_settings['special_paths']);
            if ($verified) {
                if ($wpdb->query('COMMIT') === false) mcp_bridge_editor_guard_fail('native_save_commit_failed', 'Native editor transaction could not be committed.', 500);
                $editor_transaction_open = false;
            } else {
                $wpdb->query('ROLLBACK');
                $editor_transaction_open = false;
                $after_state = mcp_bridge_editor_guard_full_state($post_id);
            }
            $operation_key = '_mcp_elementor_editor_operation_' . hash('sha256', $operation_id);
            $receipt = ['post_id' => $post_id, 'user_id' => get_current_user_id(), 'operation_id' => $operation_id, 'body_fingerprint' => $fingerprint, 'before' => $before_state, 'before_fingerprint' => mcp_bridge_editor_guard_state_fingerprint($before_state), 'after' => is_wp_error($after_state) ? null : $after_state, 'after_fingerprint' => is_wp_error($after_state) ? null : mcp_bridge_editor_guard_state_fingerprint($after_state), 'document_revision' => is_string($saved_document) ? mcp_bridge_hash($saved_document) : null, 'owned_revision_ids' => array_values(array_unique($native_revision_ids)), 'status' => $verified ? 'saved' : 'partial_failure', 'readback_verified' => $verified, 'error' => $verification_errors ? implode('; ', $verification_errors) : null, 'restore_operation_id' => $operation_id, 'filesystem_effects_reversible' => false];
            if ($verified && ($lease['native_post_save'] ?? null) === 'yoast') {
                $lease['state'] = 'awaiting_post_save';
                $lease['expires'] = time() + 120;
                $lease['intermediate_fingerprint'] = mcp_bridge_editor_guard_state_fingerprint($after_state);
                $lease['owned_revision_ids'] = array_values(array_unique($native_revision_ids));
                $receipt['status'] = 'awaiting_post_save';
                $receipt['readback_verified'] = false;
                $receipt['error'] = null;
                $receipt['intermediate_fingerprint'] = $lease['intermediate_fingerprint'];
                update_option($operation_key, $receipt, false);
                update_option($key, $lease, false);
                $preserve_post_save_lease = true;
                return $native_result;
            }
            update_option($operation_key, $receipt, false);
            if (!$verified) mcp_bridge_editor_guard_fail('native_save_partial', 'Elementor native save did not pass full-state verification: ' . implode('; ', $verification_errors), 500);
            return $native_result;
        } catch (Throwable $error) {
            if ($editor_transaction_open) $wpdb->query('ROLLBACK');
            $after_state = mcp_bridge_editor_guard_full_state($post_id);
            update_option('_mcp_elementor_editor_operation_' . hash('sha256', $operation_id), ['post_id' => $post_id, 'user_id' => get_current_user_id(), 'operation_id' => $operation_id, 'body_fingerprint' => $fingerprint, 'before' => $before_state, 'before_fingerprint' => mcp_bridge_editor_guard_state_fingerprint($before_state), 'after' => is_wp_error($after_state) ? null : $after_state, 'after_fingerprint' => is_wp_error($after_state) ? null : mcp_bridge_editor_guard_state_fingerprint($after_state), 'document_revision' => is_string(get_post_meta($post_id, '_elementor_data', true)) ? mcp_bridge_hash(get_post_meta($post_id, '_elementor_data', true)) : null, 'owned_revision_ids' => array_values(array_unique($native_revision_ids)), 'status' => 'partial_failure', 'error' => $error->getMessage(), 'restore_operation_id' => $operation_id, 'filesystem_effects_reversible' => false], false);
            if (($lease['native_post_save'] ?? null) === 'yoast') {
                $lease['state'] = 'partial_failure';
                $lease['expires'] = time() + 120;
                update_option($key, $lease, false);
                $preserve_post_save_lease = true;
            }
            throw $error;
        } finally {
            remove_action('wp_after_insert_post', $revision_capture, PHP_INT_MAX);
            mcp_bridge_editor_guard_fence($post_id, false);
            if (!$preserve_post_save_lease) {
                $current_lease = get_option($key);
                if (is_array($current_lease) && hash_equals((string) ($lease['token_hash'] ?? ''), (string) ($current_lease['token_hash'] ?? ''))) delete_option($key);
                mcp_bridge_unlock($lease['lock']);
            }
        }
    };
    $ajax_manager->register_ajax_action('save_builder', $guard);
    $GLOBALS['mcp_bridge_editor_guard_callback_id'] = spl_object_id($guard);
}

add_action('elementor/ajax/register_actions', 'mcp_bridge_editor_guard_register', PHP_INT_MAX, 1);

function mcp_bridge_element_index($elements, &$index) {
    foreach ($elements as $element) {
        if (isset($element['id'])) $index[$element['id']] = $element;
        if (isset($element['elements']) && is_array($element['elements'])) mcp_bridge_element_index($element['elements'], $index);
    }
}

function mcp_bridge_validate_atomic_changes($element, $previous) {
    if (!class_exists('Elementor\Plugin')) return new WP_Error('elementor_unavailable', 'Elementor runtime is unavailable.', ['status' => 503]);
    $instance = Elementor\Plugin::instance();
    if (($element['elType'] ?? '') === 'widget') {
        if (empty($element['widgetType'])) return new WP_Error('invalid_widget_type', 'Widgets need a widgetType.', ['status' => 400]);
        $registered = $instance->widgets_manager->get_widget_types();
        $type = $registered[$element['widgetType']] ?? null;
    } else {
        $registered = $instance->elements_manager->get_element_types();
        $type = $registered[$element['elType']] ?? null;
    }
    if (!$type) return new WP_Error('invalid_atomic_type', 'Element type is not registered in this Elementor runtime.', ['status' => 400]);
    $initial = is_callable([$type, 'get_initial_config']) ? $type->get_initial_config() : [];
    if (!is_array($initial) && is_callable([$type, 'get_config'])) $initial = $type->get_config();
    $atomic_schema = is_array($initial) ? ($initial['atomic_props_schema'] ?? null) : null;
    $schema = is_callable([$type, 'get_props_schema']) ? call_user_func([get_class($type), 'get_props_schema']) : $atomic_schema;
    $is_atomic = is_array($schema) && !empty($schema);
    if (!$is_atomic) return true;
    $settings = is_array($element['settings'] ?? null) ? $element['settings'] : [];
    $old_settings = is_array($previous['settings'] ?? null) ? $previous['settings'] : [];
    foreach ($settings as $key => $value) {
        if (array_key_exists($key, $old_settings) && $old_settings[$key] === $value) continue;
        if (!isset($schema[$key])) return new WP_Error('unknown_atomic_prop', 'Changed Atomic property is not registered in the native schema.', ['status' => 400, 'property' => $key]);
        if (!$schema[$key]->validate($value)) {
            return new WP_Error('invalid_atomic_prop', 'Atomic property failed the registered Elementor Prop_Type validator.', ['status' => 400, 'widget_type' => $element['widgetType'], 'property' => $key]);
        }
    }
    if (isset($element['settings']) && ($previous['settings'] ?? []) !== $settings && empty($schema) && ($element['settings'] ?? []) !== ($previous['settings'] ?? [])) return new WP_Error('atomic_prop_validator_unavailable', 'No native Atomic property schema is registered for this element.', ['status' => 503]);
    if (isset($element['styles']) && ($previous['styles'] ?? null) !== $element['styles']) {
        if (!class_exists('Elementor\\Modules\\AtomicWidgets\\Parsers\\Style_Parser') || !class_exists('Elementor\\Modules\\AtomicWidgets\\Styles\\Style_Schema')) return new WP_Error('atomic_style_validator_unavailable', 'Elementor native Atomic style validator is unavailable.', ['status' => 503]);
        if (!is_array($element['styles']) || (!array_is_list($element['styles']) && !mcp_bridge_native_style_map($element['styles']))) return new WP_Error('invalid_atomic_style', 'Atomic style maps must bind each native style ID to its matching definition.', ['status' => 400]);
        $parser = Elementor\Modules\AtomicWidgets\Parsers\Style_Parser::make(Elementor\Modules\AtomicWidgets\Styles\Style_Schema::get());
        foreach ($element['styles'] as $style) {
            if (!is_array($style) || !is_string($style['id'] ?? null) || $style['id'] === '') return new WP_Error('invalid_atomic_style', 'Atomic styles must contain native style objects with non-empty IDs.', ['status' => 400]);
            $parsed = $parser->parse($style);
            if (!$parsed->is_valid()) return new WP_Error('invalid_atomic_style', 'Atomic style failed the registered Elementor style schema.', ['status' => 400]);
        }
    }
    return true;
}

function mcp_bridge_validate_tree($elements, &$ids, $previous = []) {
    if (!is_array($elements)) return new WP_Error('invalid_document', 'Elementor document must be an array.', ['status' => 400]);
    foreach ($elements as $element) {
        if (!is_array($element) || !isset($element['id'], $element['elType']) || !is_string($element['id']) || $element['id'] === '') {
            return new WP_Error('invalid_element', 'Each Elementor element needs a non-empty id and elType.', ['status' => 400]);
        }
        if (isset($ids[$element['id']])) return new WP_Error('duplicate_element_id', 'Element IDs must be unique.', ['status' => 400]);
        $ids[$element['id']] = true;
        $known_type = in_array($element['elType'], ['section', 'column', 'container', 'widget'], true);
        if (!$known_type && class_exists('Elementor\Plugin')) {
            try {
                $types = Elementor\Plugin::instance()->elements_manager->get_element_types();
                $known_type = isset($types[$element['elType']]);
            } catch (Throwable $error) {
                $known_type = false;
            }
        }
        if (!$known_type) {
            return new WP_Error('invalid_element_type', 'Unsupported Elementor elType.', ['status' => 400]);
        }
        if ($element['elType'] === 'widget' && (!isset($element['widgetType']) || !is_string($element['widgetType']))) {
            return new WP_Error('invalid_widget_type', 'Widgets need a string widgetType.', ['status' => 400]);
        }
        if ($element['elType'] === 'widget' && class_exists('Elementor\Plugin')) {
            $widgets = Elementor\Plugin::instance()->widgets_manager->get_widget_types();
            if (!isset($widgets[$element['widgetType']])) return new WP_Error('invalid_widget_type', 'Widget type is not registered in this Elementor runtime.', ['status' => 400]);
        }
        if (isset($element['settings']) && !is_array($element['settings'])) return new WP_Error('invalid_settings', 'Element settings must be an object.', ['status' => 400]);
        if (isset($element['styles']) && !is_array($element['styles'])) return new WP_Error('invalid_styles', 'Atomic styles must be an object.', ['status' => 400]);
        $atomic_validation = mcp_bridge_validate_atomic_changes($element, $previous[$element['id']] ?? []);
        if (is_wp_error($atomic_validation)) return $atomic_validation;
        $children = $element['elements'] ?? [];
        if (!is_array($children)) return new WP_Error('invalid_children', 'Element children must be an array.', ['status' => 400]);
        if ($element['elType'] !== 'widget' && class_exists('Elementor\Plugin')) {
            $type = Elementor\Plugin::instance()->elements_manager->get_element_types()[$element['elType']] ?? null;
            if ($type && is_callable([$type, 'get_child_types'])) {
                $allowed = $type->get_child_types();
                if (is_array($allowed) && $allowed) foreach ($children as $child) {
                    if (!is_array($child) || !in_array($child['elType'] ?? '', $allowed, true)) return new WP_Error('invalid_child_type', 'Child element type is not allowed by the native Elementor element.', ['status' => 400, 'parent_type' => $element['elType'], 'child_type' => is_array($child) ? ($child['elType'] ?? null) : null]);
                }
            }
        }
        $error = mcp_bridge_validate_tree($children, $ids, $previous);
        if (is_wp_error($error)) return $error;
    }
    return true;
}

function mcp_bridge_regenerate_css($post_id, $warm_html_cache = false) {
    if (!class_exists('Elementor\\Core\\Files\\CSS\\Post')) return new WP_Error('css_unavailable', 'Elementor post CSS regeneration API is unavailable.');
    $atomic_ready = mcp_bridge_atomic_css_ready();
    if (is_wp_error($atomic_ready)) return $atomic_ready;
    try {
        $css = new Elementor\Core\Files\CSS\Post($post_id);
        $css->update();
        if (!is_callable([$css, 'get_meta']) || !is_callable([$css, 'get_path'])) return new WP_Error('css_regeneration_failed', 'Elementor CSS status or file API is unavailable.');
        $status = $css->get_meta('status');
        if (!in_array($status, ['file', 'inline', 'empty'], true)) return new WP_Error('css_regeneration_failed', 'Elementor did not report a completed CSS generation status.');
        if ($status === 'file') {
            $path = $css->get_path();
            clearstatcache(true, $path);
            if (!is_file($path) || !is_readable($path) || !filesize($path)) return new WP_Error('css_regeneration_failed', 'Elementor CSS file was not generated or is empty.');
        }
        $atomic_clear = mcp_bridge_invalidate_atomic_css($post_id);
        if (is_wp_error($atomic_clear)) return $atomic_clear;
        if ($warm_html_cache) {
            $html_cache = mcp_bridge_invalidate_document_html_cache($post_id);
            if (is_wp_error($html_cache)) return $html_cache;
        }
        do_action('elementor/post/render', (int) $post_id);
        do_action('elementor/frontend/after_enqueue_post_styles');
        if ($warm_html_cache) {
            $html = Elementor\Plugin::instance()->frontend->get_builder_content_for_display((int) $post_id, false);
            $html_cache = json_decode((string) get_post_meta($post_id, Elementor\Core\Base\Document::CACHE_META_KEY, true), true);
            $document_data = json_decode((string) get_post_meta($post_id, '_elementor_data', true), true);
            $empty_document = $document_data === [] && is_string($html) && trim($html) === '';
            if (!is_string($html) || (!$empty_document && (!is_array($html_cache) || !is_array($html_cache['value'] ?? null) || !$html_cache['value']))) return new WP_Error('html_cache_regeneration_failed', 'Elementor did not store the affected document HTML cache.');
        }
        $atomic_state = mcp_bridge_atomic_css_artifact_state($post_id);
        if (is_wp_error($atomic_state)) return $atomic_state;
        if (in_array($atomic_state['active_context_status'], ['cold', 'stale'], true)) return new WP_Error('atomic_css_regeneration_failed', 'Elementor native Atomic CSS cache remains cold or stale after generation.', ['context' => $atomic_state['active_context'], 'status' => $atomic_state['active_context_status']]);
        return true;
    } catch (Throwable $error) {
        return new WP_Error('css_regeneration_failed', $error->getMessage());
    }
}

function mcp_bridge_invalidate_document_html_cache($post_id) {
    if (!class_exists('Elementor\\Core\\Base\\Document') || !defined('Elementor\\Core\\Base\\Document::CACHE_META_KEY')) return new WP_Error('html_cache_unavailable', 'Elementor document HTML cache API is unavailable.');
    $documents = Elementor\Plugin::instance()->documents;
    $document = is_object($documents) && is_callable([$documents, 'get']) ? $documents->get((int) $post_id, false) : null;
    if (!is_object($document) || !is_callable([$document, 'delete_meta'])) return new WP_Error('html_cache_unavailable', 'The affected Elementor document cache API is unavailable.');
    $document->delete_meta(Elementor\Core\Base\Document::CACHE_META_KEY);
    return true;
}

function mcp_bridge_atomic_css_ready() {
    if (!defined('ELEMENTOR_VERSION') || ELEMENTOR_VERSION !== '4.2.3') return new WP_Error('atomic_css_unsupported', 'Atomic CSS operations are only verified for Elementor 4.2.3.', ['status' => 501]);
    if (!class_exists('Elementor\\Modules\\AtomicWidgets\\Styles\\Atomic_Widget_Styles') || !class_exists('Elementor\\Modules\\AtomicWidgets\\Styles\\Atomic_Styles_Manager')) return new WP_Error('atomic_css_unavailable', 'Elementor native Atomic CSS cache APIs are unavailable.', ['status' => 503]);
    global $wp_filter;
    $manager = Elementor\Modules\AtomicWidgets\Styles\Atomic_Styles_Manager::instance();
    foreach (['elementor/atomic-widgets/styles/clear', 'elementor/frontend/after_enqueue_post_styles', 'elementor/post/render'] as $hook) {
        $registered = false;
        foreach (($wp_filter[$hook]->callbacks ?? []) as $callbacks) foreach ($callbacks as $entry) {
            $callback = $entry['function'] ?? null;
            if ($callback instanceof Closure && (new ReflectionFunction($callback))->getClosureThis() === $manager) $registered = true;
        }
        if (!$registered) return new WP_Error('atomic_css_unavailable', 'The native Atomic CSS manager has not registered its expected hook.', ['status' => 503, 'hook' => $hook]);
    }
    $provider_registered = false;
    foreach (($wp_filter['elementor/atomic-widgets/styles/register']->callbacks ?? []) as $callbacks) foreach ($callbacks as $entry) {
        $callback = $entry['function'] ?? null;
        if ($callback instanceof Closure && (new ReflectionFunction($callback))->getClosureThis() instanceof Elementor\Modules\AtomicWidgets\Styles\Atomic_Widget_Styles) $provider_registered = true;
    }
    if (!$provider_registered) return new WP_Error('atomic_css_unavailable', 'The native Atomic widget style provider has not registered its style hook.', ['status' => 503, 'hook' => 'elementor/atomic-widgets/styles/register']);
    return true;
}

function mcp_bridge_invalidate_atomic_css($post_id) {
    $ready = mcp_bridge_atomic_css_ready();
    if (is_wp_error($ready)) return $ready;
    try {
        do_action('elementor/atomic-widgets/styles/clear', ['local', (int) $post_id]);
    } catch (Throwable $error) {
        return new WP_Error('atomic_css_invalidation_failed', $error->getMessage(), ['status' => 500]);
    }
    return true;
}

function mcp_bridge_atomic_css_artifact_state($post_id) {
    $ready = mcp_bridge_atomic_css_ready();
    if (is_wp_error($ready)) return $ready;
    $option_key = 'elementor_atomic_cache_validity__local';
    $tree = get_option($option_key, ['state' => false]);
    if (!is_array($tree)) return new WP_Error('atomic_css_metadata_unreadable', 'Elementor Atomic CSS validity metadata is unreadable.', ['status' => 503]);
    $node = $tree;
    $node = $node['children'][(string) (int) $post_id] ?? null;
    $upload = wp_upload_dir();
    if (!empty($upload['error']) || empty($upload['basedir'])) return new WP_Error('atomic_css_artifact_unavailable', 'WordPress upload directory is unavailable for Atomic CSS inspection.', ['status' => 503]);
    $files = [];
    $has_valid_leaf = false;
    $has_invalid_leaf = false;
    $context_states = [];
    $active_context = Elementor\Plugin::instance()->preview->is_editor_or_preview() ? 'preview' : 'frontend';
    $expected_css = mcp_bridge_atomic_css_expected_css($post_id);
    if (is_wp_error($expected_css)) return $expected_css;
    $required_active_breakpoints = array_fill_keys(array_keys(array_filter($expected_css, static fn($css) => $css !== '')), false);
    $collect = function($current, $parts) use (&$collect, &$files, &$has_valid_leaf, &$has_invalid_leaf, &$context_states, &$required_active_breakpoints, $upload, $active_context, $expected_css) {
        $is_leaf = is_bool($current) || (is_array($current) && empty($current['children']));
        $is_valid = true === $current || (is_array($current) && !empty($current['state']));
        if ($is_leaf && $is_valid && count($parts) === 4 && $parts[0] === 'local' && $parts[2] === $active_context && array_key_exists($parts[3], $expected_css)) {
            $path = trailingslashit($upload['basedir']) . 'elementor/css/' . implode('-', $parts) . '.css';
            clearstatcache(true, $path);
            if ($expected_css[$parts[3]] === '') {
                if (is_file($path)) $is_valid = false;
            } elseif (!is_file($path) || !is_readable($path) || filesize($path) === 0) {
                $is_valid = false;
            } else {
                $bytes = file_get_contents($path);
                if (!is_string($bytes)) return new WP_Error('atomic_css_artifact_unreadable', 'A generated Elementor Atomic CSS file could not be read.', ['status' => 503]);
                if ($bytes !== $expected_css[$parts[3]]) $is_valid = false;
            }
        }
        if ($is_leaf && count($parts) === 4 && $parts[0] === 'local' && $parts[2] === $active_context && array_key_exists($parts[3], $required_active_breakpoints) && $is_valid) $required_active_breakpoints[$parts[3]] = true;
        if (is_bool($current)) {
            if ($is_valid) $has_valid_leaf = true;
            else $has_invalid_leaf = true;
            if (count($parts) === 4 && $parts[0] === 'local') $context_states[$parts[2]][$is_valid ? 'valid' : 'invalid'] = true;
        } elseif (is_array($current) && empty($current['children'])) {
            if ($is_valid) $has_valid_leaf = true;
            else $has_invalid_leaf = true;
            if (count($parts) === 4 && $parts[0] === 'local') $context_states[$parts[2]][$is_valid ? 'valid' : 'invalid'] = true;
        }
        if ((true === $current || (is_array($current) && !empty($current['state']))) && $parts) {
            $path = trailingslashit($upload['basedir']) . 'elementor/css/' . implode('-', $parts) . '.css';
            if (is_file($path)) {
                if (!is_readable($path)) return new WP_Error('atomic_css_artifact_unreadable', 'A generated Elementor Atomic CSS file is unreadable.', ['status' => 503]);
                $sha256 = hash_file('sha256', $path);
                if (!is_string($sha256)) return new WP_Error('atomic_css_artifact_unreadable', 'A generated Elementor Atomic CSS file could not be hashed.', ['status' => 503]);
                $bytes = file_get_contents($path);
                if (!is_string($bytes)) return new WP_Error('atomic_css_artifact_unreadable', 'A generated Elementor Atomic CSS file could not be snapshotted.', ['status' => 503]);
                $files[basename($path)] = ['sha256' => $sha256, 'bytes' => base64_encode($bytes)];
            }
        }
        if (!is_array($current)) return true;
        foreach (($current['children'] ?? []) as $key => $child) {
            $error = $collect($child, array_merge($parts, [(string) $key]));
            if (is_wp_error($error)) return $error;
        }
        return true;
    };
    if (is_array($node)) {
        $error = $collect($node, ['local', (string) (int) $post_id]);
        if (is_wp_error($error)) return $error;
    }
    foreach ($required_active_breakpoints as $complete) if (!$complete) $context_states[$active_context]['invalid'] = true;
    $active_node = is_array($node) ? ($node['children'][$active_context] ?? null) : null;
    $active_context_status = $active_node === null ? 'cold' : (!empty($context_states[$active_context]['invalid']) ? 'stale' : (!empty($context_states[$active_context]['valid']) ? 'complete' : (is_array($active_node) && !empty($active_node['state']) ? 'empty' : 'stale')));
    ksort($files);
    $status = $node === null ? 'cold' : ($has_invalid_leaf ? 'stale' : ($has_valid_leaf ? 'complete' : 'empty'));
    return ['status' => $status, 'active_context' => $active_context, 'active_context_status' => $active_context_status, 'cache_validity' => $node, 'files' => $files];
}

function mcp_bridge_atomic_css_expected_css($post_id) {
    $styles = [];
    try {
        Elementor\Modules\AtomicWidgets\Utils\Utils::traverse_post_elements((string) (int) $post_id, function($element_data) use (&$styles) {
            $type = Elementor\Modules\GlobalClasses\Utils\Atomic_Elements_Utils::get_element_type($element_data);
            $instance = Elementor\Modules\GlobalClasses\Utils\Atomic_Elements_Utils::get_element_instance($type);
            if (Elementor\Modules\AtomicWidgets\Utils\Utils::is_atomic($instance)) $styles = array_merge($styles, $element_data['styles'] ?? []);
        });
        $styles = Elementor\Modules\AtomicWidgets\Styles\Atomic_Widget_Styles::get_license_based_filtered_styles($styles);
        $breakpoints = Elementor\Plugin::instance()->breakpoints;
        $breakpoints_config = $breakpoints->get_breakpoints_config();
        $breakpoint_names = array_map(static function($breakpoint) { return $breakpoint->get_name(); }, $breakpoints->get_breakpoints());
        $breakpoint_names = array_merge(['desktop'], array_reverse($breakpoint_names));
        $grouped = [];
        foreach ($styles as $style) foreach (($style['variants'] ?? []) as $variant) {
            $breakpoint = $variant['meta']['breakpoint'] ?? 'desktop';
            $grouped[$breakpoint][$style['id']] ??= ['id' => $style['id'], 'type' => $style['type'], 'variants' => []];
            $grouped[$breakpoint][$style['id']]['variants'][] = $variant;
        }
        $expected = [];
        foreach (array_unique($breakpoint_names) as $breakpoint) {
            $media = isset($breakpoints_config[$breakpoint])
                ? Elementor\Modules\AtomicWidgets\Styles\Styles_Renderer::get_media_query($breakpoints_config[$breakpoint])
                : 'all';
            if (!$media) continue;
            $expected[$breakpoint] = Elementor\Modules\AtomicWidgets\Styles\Styles_Renderer::make($breakpoints_config)->render(array_values($grouped[$breakpoint] ?? []));
        }
        return $expected;
    } catch (Throwable $error) {
        return new WP_Error('atomic_css_artifact_unavailable', $error->getMessage(), ['status' => 503]);
    }
}

function mcp_bridge_css_artifact_state($post_id) {
    $atomic_css = mcp_bridge_atomic_css_artifact_state($post_id);
    if (is_wp_error($atomic_css)) return $atomic_css;
    $raw = get_post_meta($post_id, '_elementor_data', true);
    $settings = get_post_meta($post_id, '_elementor_page_settings', true);
    if ((!is_string($raw) || $raw === '') && (!is_array($settings) || !$settings)) return ['status' => 'uninitialized', 'sha256' => null, 'atomic_css' => $atomic_css];
    if (!metadata_exists('post', $post_id, '_elementor_css')) return ['status' => 'uninitialized', 'sha256' => null, 'atomic_css' => $atomic_css];
    if (!class_exists('Elementor\\Core\\Files\\CSS\\Post')) return new WP_Error('css_unavailable', 'Elementor post CSS inspection API is unavailable.');
    try {
        $css = new Elementor\Core\Files\CSS\Post($post_id);
        if (!is_callable([$css, 'get_meta']) || !is_callable([$css, 'get_path'])) return new WP_Error('css_unavailable', 'Elementor post CSS status or file API is unavailable.');
        $status = $css->get_meta('status');
        if (!in_array($status, ['file', 'inline', 'empty'], true)) return new WP_Error('css_unavailable', 'Elementor post CSS status is unavailable.');
        if ($status === 'file') {
            $path = $css->get_path();
            clearstatcache(true, $path);
            if (!is_file($path) || !is_readable($path)) return new WP_Error('css_artifact_missing', 'Generated Elementor CSS file is missing or unreadable.');
            $sha256 = hash_file('sha256', $path);
            if (!is_string($sha256)) return new WP_Error('css_artifact_unreadable', 'Generated Elementor CSS file could not be hashed.');
            return ['status' => $status, 'sha256' => $sha256, 'atomic_css' => $atomic_css];
        }
        $generated_css = $css->get_meta('css');
        if (!is_string($generated_css)) return new WP_Error('css_artifact_unavailable', 'Generated Elementor inline CSS is unavailable.');
        return ['status' => $status, 'sha256' => hash('sha256', $generated_css), 'atomic_css' => $atomic_css];
    } catch (Throwable $error) {
        return new WP_Error('css_artifact_unavailable', $error->getMessage());
    }
}

function mcp_bridge_active_kit() {
    try { return Elementor\Plugin::instance()->kits_manager->get_active_kit(); }
    catch (Throwable $error) { return new WP_Error('kit_unavailable', 'Active Elementor kit is unavailable.'); }
}

function mcp_bridge_resource_state($kind, $id) {
    if ($kind === 'trash' && ctype_digit($id)) {
        $post = get_post((int) $id);
        if (!$post || !in_array($post->post_type, ['page', 'post', 'elementor_library'], true)) return new WP_Error('resource_not_found', 'Supported post was not found.', ['status' => 404]);
        $meta = get_post_meta($post->ID);
        foreach (array_keys($meta) as $key) if (strpos($key, '_wp_trash_meta_') === 0) unset($meta[$key]);
        $elementor_css = mcp_bridge_css_artifact_state($post->ID);
        if (is_wp_error($elementor_css)) return $elementor_css;
        $value = ['post_type' => $post->post_type, 'post_status' => $post->post_status, 'post_title' => $post->post_title, 'post_name' => $post->post_name, 'post_content' => $post->post_content, 'post_excerpt' => $post->post_excerpt, 'post_parent' => (int) $post->post_parent, 'post_author' => (int) $post->post_author, 'post_date' => $post->post_date, 'post_date_gmt' => $post->post_date_gmt, 'post_password' => $post->post_password, 'post_mime_type' => $post->post_mime_type, 'comment_status' => $post->comment_status, 'ping_status' => $post->ping_status, 'guid' => $post->guid, 'menu_order' => (int) $post->menu_order, 'meta' => $meta, 'elementor_css' => $elementor_css];
        return ['value' => $value, 'revision' => mcp_bridge_hash($value)];
    }
    if ($kind === 'page' && ctype_digit($id)) {
        $post = get_post((int) $id);
        if (!$post || $post->post_type !== 'page') return new WP_Error('resource_not_found', 'Page was not found.', ['status' => 404]);
        $raw = get_post_meta($post->ID, '_elementor_data', true);
        $protected = (is_string($raw) && $raw !== '') || get_post_meta($post->ID, '_elementor_page_settings', true) !== '' || get_post_meta($post->ID, '_elementor_conditions', true) !== '';
        $value = ['title' => $post->post_title, 'status' => $post->post_status, 'slug' => $post->post_name, 'document' => is_string($raw) ? json_decode($raw, true) : null, 'protected_elementor' => $protected];
        return ['value' => $value, 'revision' => mcp_bridge_hash($value)];
    }
    if ($kind === 'post_metadata' && ctype_digit($id)) {
        $post = get_post((int) $id);
        if (!$post || !in_array($post->post_type, ['page', 'post', 'elementor_library'], true)) return new WP_Error('resource_not_found', 'Supported post was not found.', ['status' => 404]);
        $value = ['title' => $post->post_title, 'status' => $post->post_status, 'post_type' => $post->post_type, 'protected_elementor' => get_post_meta($post->ID, '_elementor_data', true) !== '' || get_post_meta($post->ID, '_elementor_page_settings', true) !== '' || get_post_meta($post->ID, '_elementor_conditions', true) !== ''];
        return ['value' => $value, 'revision' => mcp_bridge_hash($value)];
    }
    if (!class_exists('Elementor\Plugin')) return new WP_Error('elementor_unavailable', 'Elementor runtime is unavailable.', ['status' => 503]);
    $kit = mcp_bridge_active_kit();
    if (is_wp_error($kit)) return $kit;
    try {
        if ($kind === 'class' && $id === 'collection' && class_exists('Elementor\\Modules\\GlobalClasses\\Global_Classes_Repository')) {
            $value = Elementor\Modules\GlobalClasses\Global_Classes_Repository::make($kit)->all(true)->get();
        } elseif ($kind === 'variable' && ($id === 'collection' || preg_match('/^[A-Za-z0-9_-]+$/', $id)) && class_exists('Elementor\\Modules\\Variables\\Storage\\Variables_Repository') && class_exists('Elementor\\Modules\\Variables\\Storage\\Variables_Collection')) {
            $collection = (new Elementor\Modules\Variables\Storage\Variables_Repository($kit))->load();
            $value = $collection->serialize(true);
            if ($id !== 'collection') $value = $value['data'][$id] ?? null;
        } elseif ($kind === 'kit' && $id === 'settings') {
            $value = $kit->get_settings();
        } elseif ($kind === 'page_settings' && ctype_digit($id) && class_exists('Elementor\Plugin') && is_callable([Elementor\Plugin::instance()->documents, 'get'])) {
            $document = Elementor\Plugin::instance()->documents->get((int) $id, false);
            if (!$document || !is_callable([$document, 'get_settings'])) return new WP_Error('resource_unavailable', 'Native Elementor page settings API is unavailable.', ['status' => 501]);
            $value = $document->get_settings();
            $controls = is_callable([$document, 'get_controls']) ? $document->get_controls() : [];
            $custom_css_available = is_array($controls) && isset($controls['custom_css']);
        } elseif ($kind === 'template' && preg_match('/^\d+$/', $id) && post_type_exists('elementor_library')) {
            $post = get_post((int) $id);
            if (!$post || $post->post_type !== 'elementor_library') return new WP_Error('resource_not_found', 'Elementor template was not found.', ['status' => 404]);
            $value = ['title' => $post->post_title, 'status' => $post->post_status, 'type' => get_post_meta($post->ID, '_elementor_template_type', true), 'document' => json_decode((string) get_post_meta($post->ID, '_elementor_data', true), true)];
        } else return new WP_Error('resource_unavailable', 'This Elementor resource API is unavailable on the active runtime.', ['status' => 501, 'kind' => $kind]);
        return ['value' => $value, 'revision' => mcp_bridge_hash($value), 'capabilities' => $kind === 'page_settings' ? ['custom_css' => $custom_css_available] : null];
    } catch (Throwable $error) { return new WP_Error('resource_read_failed', $error->getMessage(), ['status' => 500]); }
}

function mcp_bridge_is_logical_create_id($kind, $id, $expected) {
    return in_array($kind, ['page', 'template'], true)
        && $expected === 'absent'
        && ($id === 'new' || preg_match('/^new_pattern_(?:[2-9]|[1-9][0-9]+)$/', $id) === 1);
}

function mcp_bridge_apply_resource($kind, $id, $after) {
    if ($kind === 'trash' && ctype_digit($id) && is_array($after) && !$after) {
        $post = get_post((int) $id);
        if (!$post || !in_array($post->post_type, ['page', 'post', 'elementor_library'], true)) return new WP_Error('resource_not_found', 'Supported post was not found.', ['status' => 404]);
        if ($post->post_status === 'trash') return new WP_Error('already_trashed', 'Post is already in the WordPress trash.', ['status' => 409]);
        if (!current_user_can('delete_post', (int) $id)) return new WP_Error('rest_forbidden', 'Current user cannot trash this post.', ['status' => 403]);
        if (defined('EMPTY_TRASH_DAYS') && EMPTY_TRASH_DAYS <= 0) return new WP_Error('trash_disabled', 'WordPress trash is disabled; refusing a permanent deletion.', ['status' => 409]);
        if (!wp_trash_post((int) $id)) return new WP_Error('trash_failed', 'WordPress could not move the post to trash.', ['status' => 500]);
        return [];
    }
    if ($kind === 'page' && mcp_bridge_is_logical_create_id($kind, $id, 'absent')) {
        if (!is_array($after) || !isset($after['title'], $after['status'], $after['document']) || !is_string($after['title']) || !is_string($after['status']) || !is_array($after['document']) || array_diff(array_keys($after), ['title', 'status', 'slug', 'document'])) return new WP_Error('invalid_page', 'New page requires title, status, and document, with optional slug only.', ['status' => 400]);
        $title = sanitize_text_field($after['title']);
        if ($title === '' || $title !== $after['title']) return new WP_Error('invalid_page_title', 'Page title must be non-empty plain text.', ['status' => 400]);
        if (!in_array($after['status'], ['draft', 'private', 'publish'], true)) return new WP_Error('invalid_page_status', 'Page status must be draft, private, or publish.', ['status' => 400]);
        if (!current_user_can('edit_pages') || ($after['status'] === 'publish' && !current_user_can('publish_pages'))) return new WP_Error('rest_forbidden', 'Current user cannot create a page with the requested status.', ['status' => 403]);
        $slug = '';
        if (array_key_exists('slug', $after)) {
            if (!is_string($after['slug']) || sanitize_title($after['slug']) !== $after['slug']) return new WP_Error('invalid_page_slug', 'Slug must be a valid WordPress page slug.', ['status' => 400]);
            $slug = $after['slug'];
        }
        $post_id = wp_insert_post(['post_type' => 'page', 'post_status' => $after['status'], 'post_title' => $title, 'post_name' => $slug], true);
        if (is_wp_error($post_id)) return $post_id;
        if (!class_exists('Elementor\Plugin') || !is_callable([Elementor\Plugin::instance()->documents, 'get'])) return new WP_Error('elementor_document_unavailable', 'Native Elementor document save API is unavailable; created page requires recovery.', ['status' => 501, 'created_id' => $post_id]);
        $document = Elementor\Plugin::instance()->documents->get((int) $post_id, false);
        if (!$document || !is_callable([$document, 'save'])) return new WP_Error('elementor_document_unavailable', 'Native Elementor document save API is unavailable; created page requires recovery.', ['status' => 501, 'created_id' => $post_id]);
        try { $document->save(['elements' => $after['document']]); }
        catch (Throwable $error) { return new WP_Error('resource_save_failed', $error->getMessage(), ['status' => 500, 'created_id' => (string) $post_id]); }
        update_post_meta((int) $post_id, '_elementor_edit_mode', 'builder');
        $saved = json_decode((string) get_post_meta((int) $post_id, '_elementor_data', true), true);
        if (!is_array($saved) || mcp_bridge_hash($saved) !== mcp_bridge_hash($after['document'])) return new WP_Error('readback_failed', 'Created Elementor tree differs from the requested tree.', ['status' => 500, 'created_id' => $post_id]);
        $css = mcp_bridge_regenerate_css((int) $post_id);
        if (is_wp_error($css)) return new WP_Error('css_regeneration_failed', $css->get_error_message(), ['status' => 500, 'created_id' => $post_id]);
        return ['generated_id' => (string) $post_id];
    }
    if ($kind === 'post_metadata' && ctype_digit($id) && is_array($after)) {
        $post = get_post((int) $id);
        if (!$post || !in_array($post->post_type, ['page', 'post', 'elementor_library'], true)) return new WP_Error('resource_not_found', 'Supported post was not found.', ['status' => 404]);
        if (!isset($after['title']) || !is_string($after['title']) || !isset($after['status']) || !is_string($after['status'])) return new WP_Error('invalid_post_metadata', 'Post metadata requires title and status.', ['status' => 400]);
        $allowed = $post->post_type === 'elementor_library' ? ['publish', 'draft', 'private', 'pending'] : ['publish', 'draft', 'private'];
        if (!in_array($after['status'], $allowed, true)) return new WP_Error('invalid_post_metadata', 'Requested status is not allowed for this post type.', ['status' => 400]);
        $updated = wp_update_post(['ID' => (int) $id, 'post_title' => sanitize_text_field($after['title']), 'post_status' => $after['status']], true);
        if (is_wp_error($updated)) return $updated;
        return [];
    }
    $kit = mcp_bridge_active_kit();
    if (is_wp_error($kit)) return $kit;
    $created_id = null;
    try {
        if ($kind === 'class' && $id === 'collection') {
            $parsed = Elementor\Modules\GlobalClasses\Global_Classes_Parser::make()->parse($after);
            if (!$parsed->is_valid()) return new WP_Error('invalid_classes', 'Global classes failed Elementor native validation.', ['status' => 400]);
            $repository = Elementor\Modules\GlobalClasses\Global_Classes_Repository::make($kit);
            $repository->put($parsed->unwrap()['items'], $parsed->unwrap()['order']);
        } elseif ($kind === 'variable' && $id === 'collection' && class_exists('Elementor\\Modules\\Variables\\Storage\\Variables_Repository') && class_exists('Elementor\\Modules\\Variables\\Services\\Batch_Operations\\Batch_Processor')) {
            $current = (new Elementor\Modules\Variables\Storage\Variables_Repository($kit))->load();
            $processor = new Elementor\Modules\Variables\Services\Batch_Operations\Batch_Processor();
            $created = [];
            $generated_ids = [];
            foreach (($after['operations'] ?? []) as $operation) {
                if (isset($operation['id'], $generated_ids[$operation['id']])) $operation['id'] = $generated_ids[$operation['id']];
                $result = $processor->apply_operation($current, $operation);
                $created[] = $result;
                if (($result['type'] ?? '') === 'create' && !empty($result['temp_id'])) $generated_ids[$result['temp_id']] = $result['id'];
            }
            $saved = (new Elementor\Modules\Variables\Storage\Variables_Repository($kit))->save($current);
            if ($saved === false) return new WP_Error('resource_save_failed', 'Elementor variable repository save failed.', ['status' => 500]);
            return ['generated_ids' => array_values(array_filter($created, fn($row) => ($row['type'] ?? '') === 'create'))];
        } elseif ($kind === 'kit' && $id === 'settings' && is_array($after) && is_callable([$kit, 'save'])) {
            $kit->save(['settings' => $after]);
        } elseif ($kind === 'page_settings' && ctype_digit($id) && is_array($after) && class_exists('Elementor\Plugin')) {
            $document = Elementor\Plugin::instance()->documents->get((int) $id, false);
            if (!$document || !is_callable([$document, 'save'])) return new WP_Error('resource_unavailable', 'Native Elementor page settings save API is unavailable.', ['status' => 501]);
            $document->save(['settings' => $after]);
        } elseif ($kind === 'template' && mcp_bridge_is_logical_create_id($kind, $id, 'absent')) {
            if (($after['status'] ?? '') !== 'draft' || !is_array($after['document'] ?? null)) return new WP_Error('invalid_template', 'Portable templates must be explicitly saved as drafts.', ['status' => 400]);
            $type = $after['type'] ?? '';
            if (!in_array($type, ['section', 'page'], true)) return new WP_Error('template_type_unavailable', 'This template type has no verified native document registration in the active runtime.', ['status' => 501, 'type' => $type]);
            if (!isset($after['title']) || !is_string($after['title']) || sanitize_text_field($after['title']) !== $after['title'] || $after['title'] === '') return new WP_Error('invalid_template', 'Template title must be non-empty plain text.', ['status' => 400]);
            $ids = []; $valid = mcp_bridge_validate_tree($after['document'], $ids, []);
            if (is_wp_error($valid)) return $valid;
            $post_id = wp_insert_post(['post_type' => 'elementor_library', 'post_status' => 'draft', 'post_title' => $after['title']], true);
            if (is_wp_error($post_id)) return $post_id;
            $created_id = (string) $post_id;
            update_post_meta($post_id, '_elementor_template_type', $type);
            if (!class_exists('Elementor\\Plugin') || !is_callable([Elementor\Plugin::instance()->documents, 'get'])) return new WP_Error('elementor_document_unavailable', 'Native Elementor template save API is unavailable; created template requires recovery.', ['status' => 501, 'created_id' => $post_id]);
            $document = Elementor\Plugin::instance()->documents->get((int) $post_id, false);
            if (!$document || !is_callable([$document, 'save'])) return new WP_Error('elementor_document_unavailable', 'Native Elementor template save API is unavailable; created template requires recovery.', ['status' => 501, 'created_id' => $post_id]);
            $document->save(['elements' => $after['document']]);
            update_post_meta((int) $post_id, '_elementor_edit_mode', 'builder');
            $saved = json_decode((string) get_post_meta((int) $post_id, '_elementor_data', true), true);
            if (!is_array($saved) || mcp_bridge_hash($saved) !== mcp_bridge_hash($after['document'])) return new WP_Error('readback_failed', 'Created template tree differs from the requested tree.', ['status' => 500, 'created_id' => $post_id]);
            $css = mcp_bridge_regenerate_css((int) $post_id);
            if (is_wp_error($css)) return new WP_Error('css_regeneration_failed', $css->get_error_message(), ['status' => 500, 'created_id' => $post_id]);
            return ['generated_id' => (string) $post_id];
        } else return new WP_Error('resource_unavailable', 'This Elementor resource API is unavailable on the active runtime.', ['status' => 501, 'kind' => $kind]);
        return [];
    } catch (Throwable $error) { return new WP_Error('resource_save_failed', $error->getMessage(), ['status' => 500, 'created_id' => $created_id]); }
}

function mcp_bridge_restore_resource($kind, $id, $before) {
    if ($kind === 'trash' && ctype_digit($id) && is_array($before)) {
        $post = get_post((int) $id);
        if (!$post || $post->post_status !== 'trash') return new WP_Error('resource_restore_conflict', 'Trashed post is missing or no longer in trash.', ['status' => 409]);
        $restore_status = $before['post_status'] ?? 'draft';
        $restore_status_filter = static function($status, $post_id, $previous_status) use ($id, $restore_status) {
            return (int) $post_id === (int) $id ? $restore_status : $status;
        };
        add_filter('wp_untrash_post_status', $restore_status_filter, 10, 3);
        try { $untrashed = wp_untrash_post((int) $id); }
        finally { remove_filter('wp_untrash_post_status', $restore_status_filter, 10); }
        if (!$untrashed) return new WP_Error('resource_restore_failed', 'WordPress could not restore the trashed post.', ['status' => 500]);
        $state = mcp_bridge_resource_state('trash', $id);
        if (is_wp_error($state)) return new WP_Error('resource_restore_conflict', 'Post disappeared during native untrash; refusing to overwrite it.', ['status' => 409]);
        $observed = $state['value'];
        foreach (['post_status', 'post_name', 'post_date', 'post_date_gmt'] as $field) $observed[$field] = $before[$field];
        if (array_key_exists('_wp_desired_post_slug', $before['meta'] ?? [])) $observed['meta']['_wp_desired_post_slug'] = $before['meta']['_wp_desired_post_slug'];
        else unset($observed['meta']['_wp_desired_post_slug']);
        if (mcp_bridge_hash($observed) !== mcp_bridge_hash($before)) return new WP_Error('resource_restore_conflict', 'Post changed during native untrash; refusing to overwrite it.', ['status' => 409]);
        $updated = wp_update_post(['ID' => (int) $id, 'post_name' => $before['post_name'], 'post_date' => $before['post_date'], 'post_date_gmt' => $before['post_date_gmt']], true);
        if (is_wp_error($updated)) return $updated;
        if (array_key_exists('_wp_desired_post_slug', $before['meta'] ?? [])) update_post_meta((int) $id, '_wp_desired_post_slug', $before['meta']['_wp_desired_post_slug'][0] ?? '');
        else delete_post_meta((int) $id, '_wp_desired_post_slug');
        $state = mcp_bridge_resource_state('trash', $id);
        if (is_wp_error($state) || mcp_bridge_hash($state['value']) !== mcp_bridge_hash($before)) return new WP_Error('resource_restore_readback_failed', 'Native untrash did not restore the complete saved post and Elementor snapshot.', ['status' => 500]);
        return true;
    }
    if ($kind === 'post_metadata' && ctype_digit($id) && is_array($before)) {
        $post = get_post((int) $id);
        if (!$post || !in_array($post->post_type, ['page', 'post', 'elementor_library'], true) || ($before['post_type'] ?? null) !== $post->post_type) return new WP_Error('resource_not_found', 'Supported post was not found.', ['status' => 404]);
        if ($before['status'] === 'trash' && $post->post_status !== 'trash' && defined('EMPTY_TRASH_DAYS') && EMPTY_TRASH_DAYS <= 0) return new WP_Error('trash_disabled', 'WordPress trash is disabled; refusing a permanent deletion.', ['status' => 409]);
        $updated = wp_update_post(['ID' => (int) $id, 'post_title' => $before['title']], true);
        if (is_wp_error($updated)) return $updated;
        if ($before['status'] === 'trash' && $post->post_status !== 'trash') {
            if (!wp_trash_post((int) $id)) return new WP_Error('resource_restore_failed', 'WordPress could not move the post back to trash.', ['status' => 500]);
        } elseif ($post->post_status === 'trash' && $before['status'] !== 'trash') {
            $restore_status_filter = static function($status, $post_id) use ($id, $before) { return (int) $post_id === (int) $id ? $before['status'] : $status; };
            add_filter('wp_untrash_post_status', $restore_status_filter, 10, 3);
            try { $untrashed = wp_untrash_post((int) $id); }
            finally { remove_filter('wp_untrash_post_status', $restore_status_filter, 10); }
            if (!$untrashed) return new WP_Error('resource_restore_failed', 'WordPress could not restore the post status.', ['status' => 500]);
        } else {
            $updated = wp_update_post(['ID' => (int) $id, 'post_status' => $before['status']], true);
            if (is_wp_error($updated)) return $updated;
        }
        $state = mcp_bridge_resource_state($kind, $id);
        if (is_wp_error($state) || mcp_bridge_hash($state['value']) !== mcp_bridge_hash($before)) return new WP_Error('resource_restore_readback_failed', 'Post metadata restore differs from the saved snapshot.', ['status' => 500]);
        return true;
    }
    $kit = mcp_bridge_active_kit();
    if (is_wp_error($kit)) return $kit;
    try {
        if ($kind === 'class') {
            $parsed = Elementor\Modules\GlobalClasses\Global_Classes_Parser::make()->parse($before);
            if (!$parsed->is_valid()) return new WP_Error('resource_restore_invalid', 'Saved class collection no longer passes Elementor validation.', ['status' => 500]);
            Elementor\Modules\GlobalClasses\Global_Classes_Repository::make($kit)->put($parsed->unwrap()['items'], $parsed->unwrap()['order']);
        } elseif ($kind === 'variable') {
            $collection = Elementor\Modules\Variables\Storage\Variables_Collection::hydrate($before);
            if ((new Elementor\Modules\Variables\Storage\Variables_Repository($kit))->save($collection) === false) return new WP_Error('resource_restore_failed', 'Elementor variable repository restore failed.', ['status' => 500]);
        } elseif ($kind === 'kit') $kit->save(['settings' => $before]);
        elseif ($kind === 'page_settings') {
            $document = Elementor\Plugin::instance()->documents->get((int) $id, false);
            if (!$document || !is_callable([$document, 'save'])) return new WP_Error('resource_unavailable', 'Native Elementor page settings save API is unavailable.', ['status' => 501]);
            $document->save(['settings' => $before]);
        }
        elseif ($kind === 'template') {
            $post_id = (int) $id;
            wp_update_post(['ID' => $post_id, 'post_title' => $before['title'], 'post_status' => $before['status']]);
            update_post_meta($post_id, '_elementor_template_type', $before['type']);
            update_post_meta($post_id, '_elementor_data', wp_slash(wp_json_encode($before['document'])));
        } else return new WP_Error('resource_unavailable', 'Resource restore is unavailable.', ['status' => 501]);
        $state = mcp_bridge_resource_state($kind, $id);
        if (is_wp_error($state)) return new WP_Error('resource_restore_readback_failed', 'Resource restore could not be read back.', ['status' => 500]);
        $matches = $kind === 'variable'
            ? (($state['value']['data'] ?? null) === ($before['data'] ?? null) && ($state['value']['version'] ?? null) === ($before['version'] ?? null))
            : mcp_bridge_hash($state['value']) === mcp_bridge_hash($before);
        if (!$matches) return new WP_Error('resource_restore_readback_failed', 'Resource restore readback differs from the saved snapshot.', ['status' => 500]);
        return true;
    } catch (Throwable $error) { return new WP_Error('resource_restore_failed', $error->getMessage(), ['status' => 500]); }
}

function mcp_bridge_validate_resource_references($document, $patches, $kit) {
    if (!class_exists('Elementor\\Modules\\GlobalClasses\\Global_Classes_Repository') || !class_exists('Elementor\\Modules\\GlobalClasses\\Utils\\Atomic_Elements_Utils')) return new WP_Error('reference_validator_unavailable', 'Elementor native global class reference validator is unavailable.', ['status' => 503]);
    if (!class_exists('Elementor\\Modules\\Variables\\Storage\\Variables_Repository') || !class_exists('Elementor\\Modules\\Variables\\Storage\\Variables_Collection') || !class_exists('Elementor\\Modules\\Variables\\Services\\Batch_Operations\\Batch_Processor') || !class_exists('Elementor\\Modules\\Variables\\Utils\\Template_Library_Variables_Snapshot_Builder')) return new WP_Error('reference_validator_unavailable', 'Elementor native variable reference validator is unavailable.', ['status' => 503]);
    $existing_classes = Elementor\Modules\GlobalClasses\Global_Classes_Repository::make($kit)->all(true)->get()['items'] ?? [];
    $classes = $existing_classes;
    $variables = (new Elementor\Modules\Variables\Storage\Variables_Repository($kit))->load()->serialize(true)['data'] ?? [];
    foreach ($patches as $patch) {
        if ($patch['kind'] === 'class') $classes = $patch['after']['items'] ?? [];
        if ($patch['kind'] === 'variable') {
            $collection = Elementor\Modules\Variables\Storage\Variables_Collection::hydrate($patch['before']);
            $processor = new Elementor\Modules\Variables\Services\Batch_Operations\Batch_Processor();
            foreach ($patch['after']['operations'] as $operation) $processor->apply_operation($collection, $operation);
            $variables = $collection->serialize(true)['data'] ?? [];
        }
    }
    $walk = function($elements) use (&$walk, &$classes, &$existing_classes) {
        foreach ($elements as $element) {
            if (!class_exists('Elementor\\Modules\\GlobalClasses\\Utils\\Atomic_Elements_Utils')) return new WP_Error('reference_validator_unavailable', 'Elementor native global class reference validator is unavailable.', ['status' => 503]);
            $ids = Elementor\Modules\GlobalClasses\Utils\Atomic_Elements_Utils::collect_class_ids_from_element_data($element);
            foreach ($ids as $id) {
                if (isset($existing_classes[$id]) && !isset($classes[$id])) return new WP_Error('unresolved_class_reference', 'Document refers to a global class missing from the resulting collection.', ['status' => 400, 'class_id' => $id]);
            }
            $error = $walk($element['elements'] ?? []);
            if (is_wp_error($error)) return $error;
        }
        return true;
    };
    $error = $walk($document);
    if (is_wp_error($error)) return $error;
    if (!class_exists('Elementor\\Modules\\Variables\\Utils\\Template_Library_Variables_Snapshot_Builder')) return new WP_Error('reference_validator_unavailable', 'Elementor native variable reference validator is unavailable.', ['status' => 503]);
    $variable_ids = Elementor\Modules\Variables\Utils\Template_Library_Variables_Snapshot_Builder::extract_variable_ids_from_data($document);
    foreach ($variable_ids as $id) if (!isset($variables[$id])) return new WP_Error('unresolved_variable_reference', 'Document refers to a variable missing from the resulting collection.', ['status' => 400, 'variable_id' => $id]);
    return true;
}

add_action('rest_api_init', function() {
    $ns = 'mcp-for-page-builders/v1';
    $admin = function() { return current_user_can('manage_options'); };
    $edit_post = function($request) {
        $post_id = (int) $request['post_id'];
        return $post_id > 0 && current_user_can('edit_post', $post_id);
    };
    $rest_data = function($route) {
        $response = rest_do_request(new WP_REST_Request('GET', $route));
        if (is_wp_error($response)) return ['error' => $response->get_error_message()];
        if ($response->get_status() >= 400) return ['status' => $response->get_status(), 'error' => $response->get_data()];
        return $response->get_data();
    };

    register_rest_route($ns, '/status', [
        'methods' => 'GET',
        'callback' => function() {
            $theme = wp_get_theme();
            return ['version' => '1.3.0', 'mu_plugins_writable' => wp_is_writable(WPMU_PLUGIN_DIR), 'theme' => $theme->get_stylesheet(), 'parent_theme' => $theme->get_template(), 'theme_dir' => $theme->get_stylesheet_directory()];
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/capabilities', [
        'methods' => 'GET',
        'callback' => function() {
            $routes = array_keys(rest_get_server()->get_routes());
            $has_route = function($suffix) use ($routes) {
                foreach ($routes as $route) if (strpos($route, $suffix) !== false) return true;
                return false;
            };
            $atomic_route = '/elementor/v1/css-to-atomic';
            $abilities = [];
            $atomic_module = null;
            if (class_exists('Elementor\Plugin')) {
                try {
                    $manager = Elementor\Plugin::instance()->modules_manager;
                    $atomic_module = method_exists($manager, 'get_modules') ? $manager->get_modules('mcp') : null;
                    $registry = is_object($atomic_module) && method_exists($atomic_module, 'registry') ? $atomic_module->registry() : null;
                    if (!$registry || !method_exists($registry, 'all')) throw new RuntimeException('The active MCP module registry is unavailable.');
                    foreach ($registry->all() as $ability) {
                        if (!is_object($ability) || !method_exists($ability, 'get_id')) continue;
                        $available = method_exists($ability, 'is_available') ? $ability->is_available() : null;
                        $permission = method_exists($ability, 'check_permission') ? $ability->check_permission() : null;
                        $registered = function_exists('wp_get_ability') ? wp_get_ability($ability->get_id()) : null;
                        $schemas = [];
                        $schema_reasons = [];
                        foreach (['input_schema' => 'get_input_schema', 'output_schema' => 'get_output_schema'] as $schema => $getter) {
                            if (!$registered) $schema_reasons[$schema] = function_exists('wp_get_ability') ? 'The ability is not registered in the WordPress abilities API.' : 'wp_get_ability() is unavailable.';
                            elseif (!method_exists($registered, $getter)) $schema_reasons[$schema] = 'The registered ability does not expose the public ' . $getter . '() getter.';
                            else {
                                try {
                                    $schemas[$schema] = $registered->$getter();
                                    if (!is_array($schemas[$schema]) || !$schemas[$schema]) $schema_reasons[$schema] = 'The registered schema is empty or unavailable.';
                                } catch (Throwable $schema_error) { $schema_reasons[$schema] = $schema_error->getMessage(); }
                            }
                        }
                        $meta = null;
                        if ($registered && method_exists($registered, 'get_meta')) {
                            try { $meta = $registered->get_meta(); } catch (Throwable $meta_error) { $meta = null; }
                        }
                        $abilities[] = [
                            'id' => $ability->get_id(),
                            'source' => 'elementor_mcp_registry',
                            'schema_source' => $registered ? 'wordpress_abilities_api' : null,
                            'kind' => method_exists($ability, 'get_kind') ? $ability->get_kind() : null,
                            'uri' => method_exists($ability, 'get_uri') ? $ability->get_uri() : null,
                            'proxy_slug' => method_exists($ability, 'get_proxy_slug') ? $ability->get_proxy_slug() : null,
                            'label' => $registered && method_exists($registered, 'get_label') ? $registered->get_label() : null,
                            'description' => $registered && method_exists($registered, 'get_description') ? $registered->get_description() : null,
                            'input_schema' => $schemas['input_schema'] ?? null,
                            'output_schema' => $schemas['output_schema'] ?? null,
                            'annotations' => is_array($meta) ? ($meta['annotations'] ?? null) : null,
                            'meta' => $meta,
                            'schema_unavailability' => $schema_reasons,
                            'available' => !is_wp_error($available) && $available === true,
                            'availability_reason' => is_wp_error($available) ? $available->get_error_message() : ($available === true ? null : 'Ability availability was not explicitly confirmed.'),
                            'permission' => !is_wp_error($permission) && $permission === true,
                            'permission_error' => is_wp_error($permission) ? $permission->get_error_message() : ($permission === true ? null : 'Ability permission was not explicitly confirmed.'),
                            'proxy_exposed' => method_exists($ability, 'is_exposed_via_proxy') ? (bool) $ability->is_exposed_via_proxy() : false,
                        ];
                    }
                } catch (Throwable $error) { $abilities = ['discovery_error' => $error->getMessage()]; }
            }
            $class_api = class_exists('Elementor\\Modules\\GlobalClasses\\Global_Classes_Repository') && class_exists('Elementor\\Modules\\GlobalClasses\\Global_Classes_Parser');
            $variable_api = class_exists('Elementor\\Modules\\Variables\\Storage\\Variables_Repository') && class_exists('Elementor\\Modules\\Variables\\Services\\Batch_Operations\\Batch_Processor');
            return ['bridge_version' => '1.3.0', 'document_read' => true, 'document_compare_and_set' => true, 'document_restore' => true, 'runtime_discovery' => true, 'element_schema' => true, 'style_schema' => true, 'native_css_to_atomic' => $has_route($atomic_route), 'native_css_to_atomic_endpoint' => $has_route($atomic_route) ? '/elementor/v1/css-to-atomic' : null, 'native_mcp_proxy' => $has_route('/elementor/v1/mcp-proxy'), 'wordpress_abilities_api' => $has_route('/wp-abilities/v1/abilities'), 'abilities' => $abilities, 'resource_transactions' => true, 'logical_template_create_ids' => true, 'resources' => ['classes' => $class_api, 'variables' => $variable_api, 'kit_settings' => class_exists('Elementor\\Core\\Kits\\Documents\\Kit'), 'page_settings' => class_exists('Elementor\Plugin'), 'draft_templates' => post_type_exists('elementor_library'), 'post_metadata' => post_type_exists('page') && post_type_exists('elementor_library')]];
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/runtime', [
        'methods' => 'GET',
        'callback' => function() {
            global $wp_version;
            $plugins = get_plugins();
            $active = (array) get_option('active_plugins', []);
            $versions = [];
            foreach ($active as $plugin) if (isset($plugins[$plugin])) $versions[$plugin] = $plugins[$plugin]['Version'];
            $routes = array_keys(rest_get_server()->get_routes());
            $kit = null;
            if (class_exists('Elementor\Plugin')) {
                try { $kit = Elementor\Plugin::instance()->kits_manager->get_active_kit()->get_settings(); } catch (Throwable $error) { $kit = ['discovery_error' => $error->getMessage()]; }
            }
            $elementor_routes = array_values(array_filter($routes, function($route) { return strpos($route, 'elementor/') !== false || strpos($route, 'atomic') !== false; }));
            $network_active = function_exists('get_site_option') ? (array) get_site_option('active_sitewide_plugins', []) : [];
            $network_versions = [];
            foreach (array_keys($network_active) as $plugin) if (isset($plugins[$plugin])) $network_versions[$plugin] = $plugins[$plugin]['Version'];
            $breakpoints = class_exists('Elementor\Plugin') && isset(Elementor\Plugin::instance()->breakpoints) ? Elementor\Plugin::instance()->breakpoints->get_breakpoints_config() : [];
            return ['wordpress_version' => $wp_version, 'plugin_versions' => $versions, 'network_active_plugin_versions' => $network_versions, 'elementor_core_version' => defined('ELEMENTOR_VERSION') ? ELEMENTOR_VERSION : null, 'elementor_pro_version' => defined('ELEMENTOR_PRO_VERSION') ? ELEMENTOR_PRO_VERSION : null, 'rest_routes' => $elementor_routes, 'active_kit_settings' => $kit, 'admin_url' => admin_url(), 'site_url' => site_url(), 'home_url' => home_url(), 'breakpoints' => $breakpoints];
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/element-schemas', [
        'methods' => 'GET',
        'callback' => function() {
            if (!class_exists('Elementor\Plugin')) return new WP_Error('elementor_unavailable', 'Elementor is not active.', ['status' => 404]);
            $widgets = Elementor\Plugin::instance()->widgets_manager->get_widget_types();
            $schemas = [];
            foreach ($widgets as $name => $widget) $schemas[$name] = ['name' => $name, 'title' => $widget->get_title(), 'controls' => $widget->get_controls()];
            $elements = [];
            foreach (Elementor\Plugin::instance()->elements_manager->get_element_types() as $name => $element) {
                $elements[$name] = ['name' => $name, 'title' => $element->get_title()];
                if (is_callable([$element, 'get_controls'])) $elements[$name]['controls'] = $element->get_controls();
                if (is_callable([$element, 'get_initial_config'])) $elements[$name]['initial_config'] = $element->get_initial_config();
                elseif (is_callable([$element, 'get_config'])) $elements[$name]['initial_config'] = $element->get_config();
            }
            $attribute_rendering = [
                'available' => false,
                'reason' => 'The active settings resolver could not be verified for custom attributes.',
                'transformer_class' => null,
                'elementor_core_version' => defined('ELEMENTOR_VERSION') ? ELEMENTOR_VERSION : null,
                'elementor_pro_version' => defined('ELEMENTOR_PRO_VERSION') ? ELEMENTOR_PRO_VERSION : null,
            ];
            $resolver_class = 'Elementor\\Modules\\AtomicWidgets\\PropsResolver\\Render_Props_Resolver';
            $attributes_prop_class = 'Elementor\\Modules\\AtomicWidgets\\PropTypes\\Attributes_Prop_Type';
            $key_value_prop_class = 'Elementor\\Modules\\AtomicWidgets\\PropTypes\\Key_Value_Prop_Type';
            $string_prop_class = 'Elementor\\Modules\\AtomicWidgets\\PropTypes\\Primitives\\String_Prop_Type';
            $transformer_base_class = 'Elementor\\Modules\\AtomicWidgets\\PropsResolver\\Transformer_Base';
            if (class_exists($resolver_class) && class_exists($attributes_prop_class) && class_exists($key_value_prop_class) && class_exists($string_prop_class)) {
                try {
                    $resolver = call_user_func([$resolver_class, 'for_settings']);
                    $transformer = $resolver->get_transformers_registry()->get('attributes');
                    if ($transformer instanceof $transformer_base_class) {
                        $attribute_rendering['transformer_class'] = get_class($transformer);
                        $probe = call_user_func([$attributes_prop_class, 'generate'], [
                            call_user_func([$key_value_prop_class, 'generate'], [
                                'key' => call_user_func([$string_prop_class, 'generate'], 'data-mcp-attribute-probe'),
                                'value' => call_user_func([$string_prop_class, 'generate'], 'verified'),
                            ]),
                        ]);
                        $resolved = $resolver->resolve(
                            ['attributes' => call_user_func([$attributes_prop_class, 'make'])],
                            ['attributes' => $probe]
                        );
                        $attribute_rendering['available'] = ($resolved['attributes'] ?? null) === 'data-mcp-attribute-probe="verified"';
                        $attribute_rendering['reason'] = $attribute_rendering['available'] ? null : 'The registered attributes transformer did not emit the verified attribute string.';
                    } else {
                        $attribute_rendering['reason'] = 'No registered settings transformer handles Atomic attributes.';
                    }
                } catch (Throwable $error) {
                    $attribute_rendering['reason'] = 'The registered attributes transformer probe failed.';
                }
            }
            return ['widgets' => $schemas, 'elements' => $elements, 'render_capabilities' => ['attributes' => $attribute_rendering], 'source' => 'Elementor native managers'];
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/element-schema/(?P<widget_type>[a-zA-Z0-9_-]+)', [
        'methods' => 'GET',
        'callback' => function($request) {
            if (!class_exists('Elementor\Plugin')) return new WP_Error('elementor_unavailable', 'Elementor is not active.', ['status' => 404]);
            $widgets = Elementor\Plugin::instance()->widgets_manager->get_widget_types();
            $widget = $widgets[$request['widget_type']] ?? null;
            $element = Elementor\Plugin::instance()->elements_manager->get_element_types()[$request['widget_type']] ?? null;
            if (!$widget && !$element) return new WP_Error('element_type_not_found', 'Widget or layout type is not registered.', ['status' => 404]);
            if (!$widget) return ['element_type' => $element->get_name(), 'title' => $element->get_title(), 'source' => 'Elementor elements_manager', 'initial_config' => is_callable([$element, 'get_initial_config']) ? $element->get_initial_config() : (is_callable([$element, 'get_config']) ? $element->get_config() : null)];
            $schema = ['widget_type' => $widget->get_name(), 'title' => $widget->get_title(), 'controls' => $widget->get_controls(), 'source' => 'Elementor widgets_manager'];
            if (method_exists($widget, 'get_props_schema')) $schema['props_schema'] = $widget->get_props_schema();
            if (method_exists($widget, 'get_style_schema')) $schema['style_schema'] = $widget->get_style_schema();
            if (method_exists($widget, 'get_atomic_controls')) $schema['atomic_controls'] = $widget->get_atomic_controls();
            if (is_callable([$widget, 'get_initial_config'])) $schema['initial_config'] = $widget->get_initial_config();
            elseif (is_callable([$widget, 'get_config'])) $schema['initial_config'] = $widget->get_config();
            return $schema;
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/style-schema', [
        'methods' => 'GET',
        'callback' => function() use ($rest_data) {
            $schema = null;
            if (class_exists('Elementor\\Modules\\AtomicWidgets\\Styles\\Style_Schema')) {
                try { $schema = call_user_func(['Elementor\\Modules\\AtomicWidgets\\Styles\\Style_Schema', 'get']); } catch (Throwable $error) { $schema = ['discovery_error' => $error->getMessage()]; }
            }
            return ['atomic_style_schema' => $schema, 'kit_defaults' => $rest_data('/elementor/v1/kit-elements-defaults'), 'globals' => $rest_data('/elementor/v1/globals')];
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/document/(?P<post_id>\d+)', [
        'methods' => 'GET',
        'callback' => function($request) {
            $post_id = (int) $request['post_id'];
            $value = get_post_meta($post_id, '_elementor_data', true);
            if (!is_string($value)) return new WP_Error('document_not_found', 'Elementor document metadata is missing.', ['status' => 404]);
            return ['post_id' => $post_id, 'document' => $value, 'revision' => mcp_bridge_hash($value)];
        },
        'permission_callback' => $edit_post,
        'args' => ['post_id' => ['required' => true, 'validate_callback' => static function($value) { return is_numeric($value); }]],
    ]);

    register_rest_route($ns, '/document/(?P<post_id>\d+)/editor-lease', [
        'methods' => 'POST',
        'callback' => function($request) {
            $post_id = (int) $request['post_id'];
            $ready = mcp_bridge_editor_guard_capture_ready();
            if (is_wp_error($ready)) return $ready;
            $params = $request->get_json_params();
            $expected = $params['expected_revision'] ?? '';
            $operation_id = $params['operation_id'] ?? '';
            $status = $params['status'] ?? '';
            $elements = $params['elements'] ?? null;
            $settings = $params['settings'] ?? null;
            $native_post_save = mcp_bridge_editor_guard_native_post_save_mode($params['native_post_save'] ?? null);
            if (is_wp_error($native_post_save)) return $native_post_save;
            if (!is_string($expected) || !preg_match('/^[a-f0-9]{64}$/', $expected) || !is_string($operation_id) || !preg_match('/^[A-Za-z0-9_-]{8,128}$/', $operation_id) || !in_array($status, ['draft', 'publish'], true) || !is_array($elements) || !array_is_list($elements) || !is_array($settings)) return new WP_Error('invalid_request', 'expected_revision, operation_id, explicit status, full elements, and settings are required.', ['status' => 400]);
            if ($status === 'publish' && !current_user_can('publish_post', $post_id)) return new WP_Error('rest_forbidden', 'Current user cannot publish this document.', ['status' => 403]);
            $fingerprint_body = [$post_id, get_current_user_id(), $expected, $operation_id, ELEMENTOR_VERSION, $status, $elements, $settings];
            $fingerprint = mcp_bridge_editor_guard_operation_fingerprint($fingerprint_body, $native_post_save);
            $legacy_fingerprint = mcp_bridge_editor_guard_legacy_operation_fingerprint($fingerprint_body, $native_post_save);
            $operation_key = '_mcp_elementor_editor_operation_' . hash('sha256', $operation_id);
            $prior = get_option($operation_key);
            if ($prior !== false) {
                if (!is_array($prior) || (int) ($prior['post_id'] ?? 0) !== $post_id || (int) ($prior['user_id'] ?? 0) !== get_current_user_id() || !mcp_bridge_ordered_fingerprint_matches($prior['body_fingerprint'] ?? '', $fingerprint, $legacy_fingerprint, $fingerprint_body)) return new WP_Error('operation_id_conflict', 'Operation ID is already bound to a different native save.', ['status' => 409]);
                $current_state = mcp_bridge_editor_guard_full_state($post_id);
                if (($prior['status'] ?? '') !== 'saved' || is_wp_error($current_state) || !hash_equals((string) ($prior['after_fingerprint'] ?? ''), mcp_bridge_editor_guard_state_fingerprint($current_state))) return new WP_Error('operation_replay_conflict', 'The saved native state changed or the prior operation is incomplete.', ['status' => 409]);
                return ['post_id' => $post_id, 'operation_id' => $operation_id, 'status' => 'saved', 'idempotent_replay' => true, 'readback_verified' => true, 'revision' => $prior['document_revision'], 'current_state_fingerprint' => $prior['after_fingerprint']];
            }
            $current = get_post_meta($post_id, '_elementor_data', true);
            if (!is_string($current) || !hash_equals($expected, mcp_bridge_hash($current))) return new WP_Error('stale_revision', 'Document changed since it was read.', ['status' => 409]);
            $before_state = mcp_bridge_editor_guard_full_state($post_id);
            if (is_wp_error($before_state)) return $before_state;
            if (!mcp_bridge_editor_guard_fence($post_id)) return new WP_Error('native_fence_unavailable', 'MySQL post fencing is unavailable or another native save is active.', ['status' => 503]);
            mcp_bridge_editor_guard_fence($post_id, false);
            $tree = json_decode($current, true);
            $previous = [];
            if (is_array($tree)) mcp_bridge_element_index($tree, $previous);
            $ids = [];
            $valid = mcp_bridge_validate_tree($elements, $ids, $previous);
            if (is_wp_error($valid)) return $valid;
            $native_tree = mcp_bridge_editor_guard_check_native_tree($elements);
            if (is_wp_error($native_tree)) return $native_tree;
            $key = '_mcp_elementor_editor_lease_' . $post_id;
            $stale_lease = get_option($key);
            if (is_array($stale_lease) && in_array(($stale_lease['state'] ?? ''), ['running', 'awaiting_post_save', 'running_post_save'], true)) {
                if ((int) ($stale_lease['expires'] ?? 0) > time()) return new WP_Error('document_busy', 'A guarded native save is awaiting bounded recovery.', ['status' => 409]);
                if (!mcp_bridge_editor_guard_fence($post_id)) return new WP_Error('document_busy', 'A native editor callback still owns the post fence.', ['status' => 409]);
                $recovered_state = mcp_bridge_editor_guard_full_state($post_id);
                if (is_wp_error($recovered_state)) { mcp_bridge_editor_guard_fence($post_id, false); return $recovered_state; }
                $operation_key = '_mcp_elementor_editor_operation_' . hash('sha256', (string) ($stale_lease['operation_id'] ?? ''));
                update_option($operation_key, ['post_id' => $post_id, 'user_id' => (int) ($stale_lease['user_id'] ?? 0), 'operation_id' => (string) ($stale_lease['operation_id'] ?? ''), 'body_fingerprint' => (string) ($stale_lease['fingerprint'] ?? ''), 'before' => $stale_lease['before_state'] ?? null, 'before_fingerprint' => $stale_lease['before_fingerprint'] ?? null, 'after' => $recovered_state, 'after_fingerprint' => mcp_bridge_editor_guard_state_fingerprint($recovered_state), 'document_revision' => mcp_bridge_hash((string) get_post_meta($post_id, '_elementor_data', true)), 'owned_revision_ids' => $stale_lease['owned_revision_ids'] ?? [], 'status' => 'partial_failure', 'readback_verified' => false, 'error' => 'The bounded native save lease expired before its receipt was finalized.', 'restore_operation_id' => $stale_lease['operation_id'] ?? null, 'filesystem_effects_reversible' => false], false);
                if (isset($stale_lease['lock']) && is_array($stale_lease['lock'])) mcp_bridge_unlock($stale_lease['lock']);
                delete_option($key);
                mcp_bridge_editor_guard_fence($post_id, false);
            }
            if (is_array($stale_lease) && in_array(($stale_lease['state'] ?? ''), ['armed', 'partial_failure'], true) && (int) ($stale_lease['expires'] ?? 0) <= time()) {
                if (($stale_lease['native_post_save'] ?? null) === 'yoast') {
                    if (!mcp_bridge_editor_guard_fence($post_id)) return new WP_Error('document_busy', 'A native editor callback still owns the post fence.', ['status' => 409]);
                    $recovered_state = mcp_bridge_editor_guard_full_state($post_id);
                    if (is_wp_error($recovered_state)) { mcp_bridge_editor_guard_fence($post_id, false); return $recovered_state; }
                    $operation_key = '_mcp_elementor_editor_operation_' . hash('sha256', (string) ($stale_lease['operation_id'] ?? ''));
                    update_option($operation_key, ['post_id' => $post_id, 'user_id' => (int) ($stale_lease['user_id'] ?? 0), 'operation_id' => (string) ($stale_lease['operation_id'] ?? ''), 'body_fingerprint' => (string) ($stale_lease['fingerprint'] ?? ''), 'before' => $stale_lease['before_state'] ?? null, 'before_fingerprint' => $stale_lease['before_fingerprint'] ?? null, 'after' => $recovered_state, 'after_fingerprint' => mcp_bridge_editor_guard_state_fingerprint($recovered_state), 'document_revision' => mcp_bridge_hash((string) get_post_meta($post_id, '_elementor_data', true)), 'owned_revision_ids' => $stale_lease['owned_revision_ids'] ?? [], 'status' => 'partial_failure', 'readback_verified' => false, 'error' => 'The guarded native save lease expired before completion.', 'restore_operation_id' => $stale_lease['operation_id'] ?? null, 'filesystem_effects_reversible' => false], false);
                    mcp_bridge_editor_guard_fence($post_id, false);
                }
                delete_option($key);
                if (isset($stale_lease['lock']) && is_array($stale_lease['lock'])) mcp_bridge_unlock($stale_lease['lock']);
            }
            $lock = mcp_bridge_lock($post_id);
            if (is_wp_error($lock)) return $lock;
            $token = wp_generate_password(64, false, false);
            $lease = ['user_id' => get_current_user_id(), 'post_id' => $post_id, 'expected_revision' => $expected, 'operation_id' => $operation_id, 'runtime_version' => ELEMENTOR_VERSION, 'fingerprint' => $fingerprint, 'before_fingerprint' => mcp_bridge_editor_guard_state_fingerprint($before_state), 'before_state' => $before_state, 'token_hash' => hash('sha256', $token), 'expires' => time() + 60, 'state' => 'armed', 'lock' => $lock];
            if ($native_post_save === 'yoast') { $lease['native_post_save'] = 'yoast'; $lease['yoast_runtime_version'] = WPSEO_VERSION; }
            if (!add_option($key, $lease, '', false)) {
                mcp_bridge_unlock($lock);
                return new WP_Error('document_busy', 'A guarded editor save lease already exists for this document.', ['status' => 409]);
            }
            return ['post_id' => $post_id, 'user_id' => get_current_user_id(), 'expected_revision' => $expected, 'operation_id' => $operation_id, 'body_fingerprint' => $fingerprint, 'guard_token' => $token, 'expires_at' => $lease['expires'], 'native_save' => 'elementor_documents_manager_ajax_save', 'native_post_save' => $native_post_save];
        },
        'permission_callback' => $edit_post,
    ]);

    register_rest_route($ns, '/document/(?P<post_id>\d+)/editor-lease', [
        'methods' => 'DELETE',
        'callback' => function($request) {
            $post_id = (int) $request['post_id'];
            $params = $request->get_json_params();
            $token = $params['guard_token'] ?? '';
            $key = '_mcp_elementor_editor_lease_' . $post_id;
            $lease = get_option($key);
            if (!is_array($lease) || (int) ($lease['user_id'] ?? 0) !== get_current_user_id() || !is_string($token) || !hash_equals((string) ($lease['token_hash'] ?? ''), hash('sha256', $token))) return new WP_Error('editor_guard_lease_not_found', 'No matching editor save lease belongs to this user and document.', ['status' => 404]);
            if (($lease['state'] ?? '') !== 'armed') return new WP_Error('document_busy', 'A claimed native save lease must be completed or restored before release.', ['status' => 409]);
            delete_option($key);
            mcp_bridge_unlock($lease['lock'] ?? []);
            return ['released' => true, 'operation_id' => $lease['operation_id']];
        },
        'permission_callback' => $edit_post,
    ]);

    register_rest_route($ns, '/document/(?P<post_id>\d+)/editor-operation/(?P<operation_id>[A-Za-z0-9_-]{8,128})', [
        'methods' => 'GET',
        'callback' => function($request) {
            $post_id = (int) $request['post_id'];
            $operation_id = (string) $request['operation_id'];
            $receipt = get_option('_mcp_elementor_editor_operation_' . hash('sha256', $operation_id));
            if (!is_array($receipt) || (int) ($receipt['post_id'] ?? 0) !== $post_id || (int) ($receipt['user_id'] ?? 0) !== get_current_user_id()) return new WP_Error('operation_not_found', 'No native editor receipt belongs to this user and document.', ['status' => 404]);
            $readback_verified = $receipt['readback_verified'] ?? false;
            $current_state_fingerprint = null;
            $readback_error = null;
            if (in_array(($receipt['status'] ?? ''), ['saved', 'restored', 'partial_failure'], true)) {
                $current = mcp_bridge_editor_guard_full_state($post_id);
                if (!is_wp_error($current)) {
                    $current_state_fingerprint = mcp_bridge_editor_guard_state_fingerprint($current);
                    if (($receipt['status'] ?? '') === 'restored') {
                        $readback_verified = is_string($receipt['restored_fingerprint'] ?? null) && hash_equals($receipt['restored_fingerprint'], $current_state_fingerprint);
                    } elseif (($receipt['status'] ?? '') === 'saved') {
                        $readback_verified = !empty($receipt['readback_verified']) && is_string($receipt['after_fingerprint'] ?? null) && hash_equals($receipt['after_fingerprint'], $current_state_fingerprint);
                    } else {
                        $readback_verified = false;
                    }
                } else {
                    $readback_verified = false;
                    $readback_error = 'Native full-state readback is unavailable.';
                }
            }
            return ['operation_id' => $operation_id, 'page_id' => $post_id, 'post_id' => $post_id, 'status' => $receipt['status'], 'readback_verified' => $readback_verified, 'readback_error' => $readback_error, 'document_revision' => $receipt['document_revision'] ?? null, 'after_fingerprint' => $receipt['after_fingerprint'] ?? null, 'restored_fingerprint' => $receipt['restored_fingerprint'] ?? null, 'current_state_fingerprint' => $current_state_fingerprint, 'restore_operation_id' => $receipt['restore_operation_id'] ?? null, 'filesystem_effects_reversible' => false, 'error' => $receipt['error'] ?? null];
        },
        'permission_callback' => $edit_post,
    ]);

    register_rest_route($ns, '/document/(?P<post_id>\d+)/editor-operation/(?P<operation_id>[A-Za-z0-9_-]{8,128})/restore', [
        'methods' => 'POST',
        'callback' => function($request) {
            $post_id = (int) $request['post_id'];
            $operation_id = (string) $request['operation_id'];
            $expected = $request->get_param('expected_state_fingerprint');
            $key = '_mcp_elementor_editor_operation_' . hash('sha256', $operation_id);
            $receipt = get_option($key);
            if (!is_array($receipt) || (int) ($receipt['post_id'] ?? 0) !== $post_id || (int) ($receipt['user_id'] ?? 0) !== get_current_user_id()) return new WP_Error('operation_not_found', 'No native editor receipt belongs to this user and document.', ['status' => 404]);
            $required_fingerprint = ($receipt['status'] ?? '') === 'restored' ? ($receipt['restored_fingerprint'] ?? '') : ($receipt['after_fingerprint'] ?? '');
            if (!is_string($expected) || !hash_equals((string) $required_fingerprint, $expected)) return new WP_Error('restore_conflict', 'Restore requires the receipt current-state fingerprint.', ['status' => 409]);
            if (!mcp_bridge_editor_guard_fence($post_id)) return new WP_Error('document_busy', 'Another native editor callback owns the post fence.', ['status' => 409]);
            $lease_key = '_mcp_elementor_editor_lease_' . $post_id;
            $owned_lease = get_option($lease_key);
            $reuse_lease_lock = is_array($owned_lease) && ($owned_lease['native_post_save'] ?? null) === 'yoast' && (int) ($owned_lease['user_id'] ?? 0) === get_current_user_id() && ($owned_lease['operation_id'] ?? '') === $operation_id && in_array(($owned_lease['state'] ?? ''), ['awaiting_post_save', 'running_post_save', 'partial_failure'], true) && is_array($owned_lease['lock'] ?? null);
            if ($reuse_lease_lock) {
                $lock = $owned_lease['lock'];
                $lock_record = get_option($lock['key'] ?? '');
                if (!is_array($lock_record) || !hash_equals((string) ($lock['token'] ?? ''), (string) ($lock_record['token'] ?? ''))) {
                    mcp_bridge_editor_guard_fence($post_id, false);
                    return new WP_Error('document_busy', 'The pending native save no longer owns its write lease.', ['status' => 409]);
                }
            } else {
                $lock = mcp_bridge_lock($post_id);
                if (is_wp_error($lock)) {
                    mcp_bridge_editor_guard_fence($post_id, false);
                    return $lock;
                }
            }
            global $wpdb;
            $transaction_open = false;
            $restore_succeeded = false;
            try {
                $locked = mcp_bridge_begin_resource_transaction([['kind' => 'post_metadata', 'id' => (string) $post_id]]);
                if (is_wp_error($locked)) return $locked;
                $transaction_open = true;
                $revision_ids = $wpdb->get_col($wpdb->prepare("SELECT ID FROM {$wpdb->posts} WHERE post_type = 'revision' AND post_parent = %d ORDER BY ID FOR UPDATE", $post_id));
                if ($wpdb->last_error !== '' || !is_array($revision_ids)) return new WP_Error('restore_unavailable', 'Could not lock the native revision family.', ['status' => 503]);
                foreach ($revision_ids as $revision_id) {
                    $wpdb->get_results($wpdb->prepare("SELECT meta_id FROM {$wpdb->postmeta} WHERE post_id = %d FOR UPDATE", (int) $revision_id));
                    if ($wpdb->last_error !== '') return new WP_Error('restore_unavailable', 'Could not lock native revision metadata.', ['status' => 503]);
                }
                $current = mcp_bridge_editor_guard_full_state($post_id);
                if (is_wp_error($current) || !hash_equals($expected, mcp_bridge_editor_guard_state_fingerprint($current))) return new WP_Error('restore_conflict', 'Native post, metadata, revision, or CSS state changed after the save.', ['status' => 409]);
                if (($receipt['status'] ?? '') === 'restored') {
                    if (!hash_equals((string) ($receipt['restored_fingerprint'] ?? ''), mcp_bridge_editor_guard_state_fingerprint($current))) return new WP_Error('restore_conflict', 'Native state changed after restore.', ['status' => 409]);
                    if ($wpdb->query('COMMIT') === false) return new WP_Error('restore_failed', 'Could not finish the native restore read transaction.', ['status' => 500]);
                    $transaction_open = false;
                    return ['restored' => true, 'idempotent_replay' => true, 'restored_fingerprint' => $receipt['restored_fingerprint']];
                }
                $owned_ids = $receipt['owned_revision_ids'] ?? [];
                $restored = mcp_bridge_editor_guard_restore_state($post_id, $receipt['before'], $owned_ids);
                if (is_wp_error($restored)) return $restored;
                if ($wpdb->query('COMMIT') === false) return new WP_Error('restore_failed', 'Could not commit the native database restore.', ['status' => 500]);
                $transaction_open = false;
                $css_restored = mcp_bridge_editor_guard_restore_css($post_id, $receipt['before']);
                if (is_wp_error($css_restored)) return $css_restored;
                $verified = mcp_bridge_editor_guard_full_state($post_id);
                if (is_wp_error($verified) || !mcp_bridge_editor_guard_restore_matches($receipt['before'], $verified)) return new WP_Error('restore_partial', 'Restore ran, but native post, metadata, revision, or legacy CSS readback differs from the snapshot.', ['status' => 500]);
                $receipt['status'] = 'restored';
                $receipt['restored_fingerprint'] = mcp_bridge_editor_guard_state_fingerprint($verified);
                update_option($key, $receipt, false);
                $lease_key = '_mcp_elementor_editor_lease_' . $post_id;
                $owned_lease = get_option($lease_key);
                if (is_array($owned_lease) && ($owned_lease['native_post_save'] ?? null) === 'yoast' && (int) ($owned_lease['user_id'] ?? 0) === get_current_user_id() && ($owned_lease['operation_id'] ?? '') === $operation_id) {
                    delete_option($lease_key);
                    mcp_bridge_unlock($owned_lease['lock'] ?? []);
                }
                $restore_succeeded = true;
                return ['restored' => true, 'idempotent_replay' => false, 'restored_fingerprint' => $receipt['restored_fingerprint'], 'filesystem_effects_reversible' => false];
            } finally {
                if ($transaction_open) $wpdb->query('ROLLBACK');
                if (!$reuse_lease_lock || $restore_succeeded) mcp_bridge_unlock($lock);
                mcp_bridge_editor_guard_fence($post_id, false);
            }
        },
        'permission_callback' => $edit_post,
    ]);

    register_rest_route($ns, '/document/(?P<post_id>\d+)/apply', [
        'methods' => 'POST',
        'callback' => function($request) {
            global $wpdb;
            $post_id = (int) $request['post_id'];
            $params = $request->get_json_params();
            $operation_id = $params['operation_id'] ?? '';
            $expected = $params['expected_revision'] ?? '';
            $document = $params['document'] ?? null;
            if (!is_string($operation_id) || !preg_match('/^[A-Za-z0-9_-]{8,128}$/', $operation_id) || !is_string($expected) || !is_array($document)) return new WP_Error('invalid_request', 'operation_id, expected_revision, and document are required.', ['status' => 400]);
            $fence = mcp_bridge_editor_guard_fence($post_id);
            if ($fence !== true) return new WP_Error('document_busy', 'Another native Elementor save is in progress.', ['status' => 409]);
            $lock = mcp_bridge_lock($post_id);
            if (is_wp_error($lock)) { mcp_bridge_editor_guard_fence($post_id, false); return $lock; }
            $transaction_open = false;
            $save_attempted = false;
            $snapshot = null;
            $key = '_mcp_elementor_operation_' . hash('sha256', $operation_id);
            $failure = null;
            $response = null;
            try {
                $locked = mcp_bridge_begin_resource_transaction([['kind' => 'post_metadata', 'id' => (string) $post_id]]);
                if (is_wp_error($locked)) return $locked;
                $transaction_open = true;
                $existing_document = get_post_meta($post_id, '_elementor_data', true);
                $previous = [];
                if (is_string($existing_document)) {
                    $existing_tree = json_decode($existing_document, true);
                    if (is_array($existing_tree)) mcp_bridge_element_index($existing_tree, $previous);
                }
                $encoded = wp_json_encode($document);
                if (!is_string($encoded)) return new WP_Error('encode_failed', 'Could not encode Elementor document.', ['status' => 400]);
                $fingerprint = hash('sha256', wp_json_encode([$post_id, get_current_user_id(), $expected, $encoded]));
                $prior = get_option($key);
                if ($prior !== false) {
                    if (!is_array($prior) || (int) ($prior['user_id'] ?? 0) !== get_current_user_id() || (int) ($prior['post_id'] ?? 0) !== $post_id || !isset($prior['fingerprint']) || !hash_equals((string) $prior['fingerprint'], $fingerprint)) return new WP_Error('operation_id_conflict', 'Operation ID is already bound to a different request.', ['status' => 409]);
                    if (!empty($prior['restored'])) return new WP_Error('operation_id_conflict', 'Operation snapshot has already been restored.', ['status' => 409]);
                    $current = get_post_meta($post_id, '_elementor_data', true);
                    if (!is_string($current) || !hash_equals(mcp_bridge_hash($prior['applied_document'] ?? ''), mcp_bridge_hash($current))) return new WP_Error('operation_replay_conflict', 'Document changed after this operation.', ['status' => 409]);
                    $saved = $current;
                    $replay = true;
                } else {
                    $replay = false;
                }
                if (!$replay) {
                    $ids = [];
                    $validation = mcp_bridge_validate_tree($document, $ids, $previous);
                    if (is_wp_error($validation)) return $validation;
                    $original = get_post_meta($post_id, '_elementor_data', true);
                    if (!is_string($original)) return new WP_Error('document_not_found', 'Elementor document metadata is missing.', ['status' => 404]);
                    $current_revision = mcp_bridge_hash($original);
                    if (!hash_equals($expected, $current_revision)) return new WP_Error('stale_revision', 'Document changed since it was read.', ['status' => 409, 'current_revision' => $current_revision]);
                    if (!mcp_bridge_editor_guard_supported()) return new WP_Error('native_save_unavailable', 'The active Elementor runtime has no verified native document-save adapter.', ['status' => 501]);
                    $native_elements = mcp_bridge_editor_guard_native_tree($document);
                    if (is_wp_error($native_elements)) return $native_elements;
                    $paths = mcp_bridge_editor_guard_diff_paths(mcp_bridge_editor_guard_canonical($document), mcp_bridge_editor_guard_canonical($native_elements));
                    if (!$paths && mcp_bridge_hash(mcp_bridge_editor_guard_canonical($document)) !== mcp_bridge_hash(mcp_bridge_editor_guard_canonical($native_elements))) $paths[] = '$.styles (native declaration order)';
                    if ($paths) return new WP_Error('native_serializer_loss', 'The active Elementor save serializer would drop unsupported document fields at ' . implode(', ', $paths) . '.', ['status' => 501, 'paths' => $paths]);
                    $native_document = Elementor\Plugin::instance()->documents->get($post_id, false);
                    if (!is_object($native_document) || !is_callable([$native_document, 'save'])) return new WP_Error('native_save_unavailable', 'Elementor Document::save is unavailable for this document.', ['status' => 501]);
                    $before_state = mcp_bridge_editor_guard_full_state($post_id);
                    if (is_wp_error($before_state)) return $before_state;
                    $before_documents = array_filter($before_state['meta'], static fn($meta) => ($meta['meta_key'] ?? '') === '_elementor_data' && (string) ($meta['meta_value'] ?? '') === $original);
                    if (count($before_documents) !== 1) return new WP_Error('native_snapshot_unavailable', 'The full native snapshot does not contain the source document.', ['status' => 503]);
                    $snapshot = ['post_id' => $post_id, 'user_id' => get_current_user_id(), 'fingerprint' => $fingerprint, 'original_document' => $original, 'applied_document' => '', 'before_state' => $before_state, 'before_fingerprint' => mcp_bridge_editor_guard_state_fingerprint($before_state), 'after_state' => null, 'after_fingerprint' => null, 'restored' => false];
                    if (!add_option($key, $snapshot, '', false)) return new WP_Error('operation_id_conflict', 'Operation ID is already in use.', ['status' => 409]);
                    $save_attempted = true;
                    try {
                        if ($native_document->save(['elements' => $native_elements]) !== true) throw new RuntimeException('Elementor Document::save did not confirm a completed save.');
                    } catch (Throwable $error) {
                        throw new RuntimeException('Elementor Document::save failed: ' . $error->getMessage(), 500, $error);
                    }
                    clean_post_cache($post_id);
                    wp_cache_delete($post_id, 'post_meta');
                    $saved = get_post_meta($post_id, '_elementor_data', true);
                    if (!is_string($saved)) throw new RuntimeException('Native document save did not leave a readable _elementor_data value.', 500);
                    $snapshot['applied_document'] = $saved;
                    $saved_tree = json_decode($saved, true);
                    if (!is_array($saved_tree)) throw new RuntimeException('Native document readback is not valid Elementor JSON.', 500);
                    $saved_paths = mcp_bridge_editor_guard_diff_paths(mcp_bridge_editor_guard_canonical($document), mcp_bridge_editor_guard_canonical($saved_tree));
                    if (!$saved_paths && mcp_bridge_hash(mcp_bridge_editor_guard_canonical($document)) !== mcp_bridge_hash(mcp_bridge_editor_guard_canonical($saved_tree))) $saved_paths[] = '$.styles (native declaration order)';
                    $save_error = $saved_paths ? 'Native document readback differs from the requested tree at ' . implode(', ', $saved_paths) . '.' : null;
                }
                $css_error = null;
                if (!$replay) {
                    $css = mcp_bridge_regenerate_css($post_id, true);
                    if (is_wp_error($css)) $css_error = $css->get_error_message();
                }
                $after_state = mcp_bridge_editor_guard_full_state($post_id);
                if (is_wp_error($after_state)) throw new RuntimeException('Could not capture full native after-state: ' . $after_state->get_error_message(), 500);
                if (mcp_bridge_hash((string) get_post_meta($post_id, '_elementor_data', true)) !== mcp_bridge_hash($saved)) return new WP_Error('operation_conflict', 'The document changed before its full after-state snapshot was captured.', ['status' => 409]);
                if ($replay) {
                    if (!is_string($prior['after_fingerprint'] ?? null) || !hash_equals($prior['after_fingerprint'], mcp_bridge_editor_guard_state_fingerprint($after_state))) return new WP_Error('operation_replay_conflict', 'The full native state changed after this operation.', ['status' => 409]);
                } else {
                    $snapshot['applied_document'] = $saved;
                    $snapshot['after_state'] = $after_state;
                    $snapshot['after_fingerprint'] = mcp_bridge_editor_guard_state_fingerprint($after_state);
                    $snapshot['status'] = $css_error === null && $save_error === null ? 'saved' : 'partial_failure';
                    $snapshot['error'] = $save_error ?? $css_error;
                    update_option($key, $snapshot, false);
                }
                if ($wpdb->query('COMMIT') === false) throw new RuntimeException('Native Elementor document transaction could not be committed.', 500);
                $transaction_open = false;
                if (!$replay && ($css_error !== null || $save_error !== null)) {
                    $response = ['saved' => true, 'readback_verified' => $save_error === null, 'cache_regenerated' => $css_error === null, 'revision' => mcp_bridge_hash($saved), 'operation_id' => $operation_id, 'idempotent_replay' => false, 'partial_failure' => true, 'error' => $save_error ?? $css_error, 'restore_operation_id' => $operation_id, 'after_state_fingerprint' => mcp_bridge_editor_guard_state_fingerprint($after_state)];
                } else {
                    $response = ['saved' => true, 'readback_verified' => true, 'cache_regenerated' => true, 'revision' => mcp_bridge_hash($saved), 'operation_id' => $operation_id, 'idempotent_replay' => $replay, 'after_state_fingerprint' => mcp_bridge_editor_guard_state_fingerprint($after_state)];
                }
            } catch (Throwable $error) {
                $failure = $error;
            } finally {
                if ($transaction_open) $wpdb->query('ROLLBACK');
                mcp_bridge_unlock($lock);
                mcp_bridge_editor_guard_fence($post_id, false);
            }
            if ($failure !== null) {
                if ($save_attempted && is_array($snapshot)) {
                    clean_post_cache($post_id);
                    wp_cache_delete($post_id, 'post_meta');
                    $observed_document = get_post_meta($post_id, '_elementor_data', true);
                    $observed_state = mcp_bridge_editor_guard_full_state($post_id);
                    if (is_string($observed_document) && !is_wp_error($observed_state)) {
                        $snapshot['applied_document'] = $observed_document;
                        if (mcp_bridge_editor_guard_restore_matches($snapshot['before_state'], $observed_state)) {
                            delete_option($key);
                            return new WP_Error('native_save_failed', $failure->getMessage(), ['status' => 500, 'saved' => false]);
                        }
                        $snapshot['after_state'] = $observed_state;
                        $snapshot['after_fingerprint'] = mcp_bridge_editor_guard_state_fingerprint($observed_state);
                        $snapshot['status'] = 'partial_failure';
                        $snapshot['error'] = $failure->getMessage();
                        if (get_option($key) === false) add_option($key, $snapshot, '', false);
                        else update_option($key, $snapshot, false);
                        return ['saved' => true, 'readback_verified' => false, 'cache_regenerated' => false, 'revision' => mcp_bridge_hash($observed_document), 'operation_id' => $operation_id, 'idempotent_replay' => false, 'partial_failure' => true, 'error' => $failure->getMessage(), 'restore_operation_id' => $operation_id, 'after_state_fingerprint' => $snapshot['after_fingerprint']];
                    }
                    $snapshot['status'] = 'indeterminate';
                    $snapshot['error'] = $failure->getMessage();
                    if (get_option($key) === false) add_option($key, $snapshot, '', false);
                    else update_option($key, $snapshot, false);
                    return ['saved' => true, 'readback_verified' => false, 'cache_regenerated' => false, 'revision' => is_string($observed_document) ? mcp_bridge_hash($observed_document) : null, 'operation_id' => $operation_id, 'idempotent_replay' => false, 'partial_failure' => true, 'error' => $failure->getMessage(), 'restore_operation_id' => $operation_id];
                }
                return new WP_Error('native_save_failed', $failure->getMessage(), ['status' => 500]);
            }
            return $response;
        },
        'permission_callback' => $edit_post,
    ]);

    register_rest_route($ns, '/document/(?P<post_id>\d+)/restore', [
        'methods' => 'POST',
        'callback' => function($request) {
            $post_id = (int) $request['post_id'];
            $params = $request->get_json_params();
            $operation_id = $params['operation_id'] ?? '';
            $expected = $params['expected_revision'] ?? '';
            if (!is_string($operation_id) || !preg_match('/^[A-Za-z0-9_-]{8,128}$/', $operation_id) || !is_string($expected)) return new WP_Error('invalid_request', 'operation_id and expected_revision are required.', ['status' => 400]);
            $key = '_mcp_elementor_operation_' . hash('sha256', $operation_id);
            $fence = mcp_bridge_editor_guard_fence($post_id);
            if ($fence !== true) return new WP_Error('document_busy', 'Another native Elementor save is in progress.', ['status' => 409]);
            $lock = mcp_bridge_lock($post_id);
            if (is_wp_error($lock)) { mcp_bridge_editor_guard_fence($post_id, false); return $lock; }
            global $wpdb;
            $transaction_open = false;
            try {
                $snapshot = get_option($key);
                if (!is_array($snapshot) || (int) ($snapshot['post_id'] ?? 0) !== $post_id || (int) ($snapshot['user_id'] ?? 0) !== get_current_user_id()) return new WP_Error('snapshot_not_found', 'No restore snapshot belongs to this user and document.', ['status' => 404]);
                if (!is_array($snapshot['before_state'] ?? null) || !is_array($snapshot['after_state'] ?? null) || !is_string($snapshot['after_fingerprint'] ?? null)) return new WP_Error('restore_unavailable', 'This operation has no complete paired native-state snapshot.', ['status' => 409]);
                $locked = mcp_bridge_begin_resource_transaction([['kind' => 'post_metadata', 'id' => (string) $post_id]]);
                if (is_wp_error($locked)) return $locked;
                $transaction_open = true;
                $revision_ids = $wpdb->get_col($wpdb->prepare("SELECT ID FROM {$wpdb->posts} WHERE post_type = 'revision' AND post_parent = %d ORDER BY ID FOR UPDATE", $post_id));
                if ($wpdb->last_error !== '' || !is_array($revision_ids)) return new WP_Error('restore_unavailable', 'Could not lock the native revision family.', ['status' => 503]);
                foreach ($revision_ids as $revision_id) {
                    $wpdb->get_results($wpdb->prepare("SELECT meta_id FROM {$wpdb->postmeta} WHERE post_id = %d FOR UPDATE", (int) $revision_id));
                    if ($wpdb->last_error !== '') return new WP_Error('restore_unavailable', 'Could not lock native revision metadata.', ['status' => 503]);
                }
                $current_state = mcp_bridge_editor_guard_full_state($post_id);
                $current_fingerprint = is_wp_error($current_state) ? '' : mcp_bridge_editor_guard_state_fingerprint($current_state);
                $expected_fingerprint = !empty($snapshot['restored']) ? (string) ($snapshot['restored_fingerprint'] ?? '') : $snapshot['after_fingerprint'];
                if (is_wp_error($current_state) || !is_string($expected_fingerprint) || !hash_equals($expected_fingerprint, $current_fingerprint)) return new WP_Error('restore_conflict', 'Native post, metadata, revisions, or CSS changed after this operation.', ['status' => 409]);
                $current = get_post_meta($post_id, '_elementor_data', true);
                if (!is_string($current) || !hash_equals($expected, mcp_bridge_hash($current))) return new WP_Error('restore_conflict', 'Document changed after this operation; refusing to overwrite it.', ['status' => 409]);
                if (!empty($snapshot['restored'])) {
                    if ($wpdb->query('COMMIT') === false) return new WP_Error('restore_failed', 'Could not finish the native restore read transaction.', ['status' => 500]);
                    $transaction_open = false;
                    $restored = $current;
                    $replay = true;
                } else {
                    if (mcp_bridge_hash($current) !== mcp_bridge_hash($snapshot['applied_document'])) return new WP_Error('restore_conflict', 'Document changed after this operation; refusing to overwrite it.', ['status' => 409]);
                    $before_revision_ids = $snapshot['before_state']['revision_ids'] ?? null;
                    $after_revision_ids = $snapshot['after_state']['revision_ids'] ?? null;
                    if (!is_array($before_revision_ids) || !is_array($after_revision_ids) || array_diff($before_revision_ids, $after_revision_ids)) return new WP_Error('restore_conflict', 'Native save removed a pre-existing revision; refusing an incomplete restore.', ['status' => 409]);
                    $owned_revision_ids = array_values(array_diff($after_revision_ids, $before_revision_ids));
                    foreach ($owned_revision_ids as $revision_id) {
                        if (!in_array((int) $revision_id, $current_state['revision_ids'], true)) return new WP_Error('restore_conflict', 'A native revision owned by the operation is missing.', ['status' => 409]);
                    }
                    $replay = false;
                    $restored = mcp_bridge_editor_guard_restore_state($post_id, $snapshot['before_state'], $owned_revision_ids);
                    if (is_wp_error($restored)) return $restored;
                    if ($wpdb->query('COMMIT') === false) return new WP_Error('restore_failed', 'Could not commit the native document restore.', ['status' => 500]);
                    $transaction_open = false;
                    $css_restored = mcp_bridge_editor_guard_restore_css($post_id, $snapshot['before_state']);
                    if (is_wp_error($css_restored)) return $css_restored;
                    $verified = mcp_bridge_editor_guard_full_state($post_id);
                    if (is_wp_error($verified) || !mcp_bridge_editor_guard_restore_matches($snapshot['before_state'], $verified)) return new WP_Error('restore_partial', 'Restore readback differs from the paired native snapshot.', ['status' => 500]);
                    $restored = get_post_meta($post_id, '_elementor_data', true);
                    if (!is_string($restored) || mcp_bridge_hash($restored) !== mcp_bridge_hash($snapshot['original_document'])) return new WP_Error('restore_readback_failed', 'Restored document differs from its source snapshot.', ['status' => 500]);
                    $snapshot['restored'] = true;
                    $snapshot['restored_fingerprint'] = mcp_bridge_editor_guard_state_fingerprint($verified);
                    update_option($key, $snapshot, false);
                }
            } finally {
                if ($transaction_open) $wpdb->query('ROLLBACK');
                mcp_bridge_unlock($lock);
                mcp_bridge_editor_guard_fence($post_id, false);
            }
            return ['restored' => true, 'readback_verified' => true, 'cache_regenerated' => true, 'idempotent_replay' => $replay, 'revision' => mcp_bridge_hash($restored)];
        },
        'permission_callback' => $edit_post,
    ]);

    register_rest_route($ns, '/resources/apply', [
        'methods' => 'POST',
        'callback' => function($request) {
            $params = $request->get_json_params(); $operation_id = $params['operation_id'] ?? ''; $patches = $params['resource_patches'] ?? [];
            $document_only_snapshot = !empty($params['document_snapshot']) && is_array($params['document_patch'] ?? null);
            if (!is_string($operation_id) || !preg_match('/^[A-Za-z0-9_-]{8,128}$/', $operation_id) || !is_array($patches) || (!$patches && !$document_only_snapshot)) return new WP_Error('invalid_request', 'operation_id and resource_patches are required.', ['status' => 400]);
            $page_id = (int) ($params['page_id'] ?? 0); $expected_source = $params['expected_revision'] ?? null;
            if ($page_id > 0 && (!is_string($expected_source) || $expected_source === '')) return new WP_Error('invalid_request', 'expected_revision is required for an existing source document.', ['status' => 400]);
            if ($page_id === 0 && $expected_source !== null && $expected_source !== 'absent') return new WP_Error('invalid_request', 'page_id=0 requires expected_revision absent.', ['status' => 400]);
            $resource_lock = mcp_bridge_lock(0);
            if (is_wp_error($resource_lock)) return $resource_lock;
            $resource_transaction_open = false;
            try {
            $key = '_mcp_elementor_resources_' . hash('sha256', $operation_id); $prior = get_option($key);
            $fingerprint_payload = [
                'page_id' => $page_id,
                'expected_revision' => $params['original_expected_revision'] ?? $expected_source,
                'document_patch' => $params['document_patch'] ?? null,
                'document_snapshot' => !empty($params['document_snapshot']),
                'resource_patches' => $patches,
            ];
            $fingerprint = 'native-style-order-v1:' . mcp_bridge_hash($fingerprint_payload);
            $legacy_fingerprint = mcp_bridge_legacy_hash($fingerprint_payload);
            if (is_array($prior) && (int) ($prior['user_id'] ?? 0) === get_current_user_id()) {
                $prior_fingerprint = (string) ($prior['fingerprint'] ?? '');
                if (!mcp_bridge_ordered_fingerprint_matches($prior_fingerprint, $fingerprint, $legacy_fingerprint, $fingerprint_payload)) return new WP_Error('operation_id_conflict', 'Operation ID is already bound to a different plan.', ['status' => 409]);
                if (($prior['status'] ?? '') === 'completed') {
                    $replay_resources = $prior['resources'] ?? [];
                    if ($page_id > 0) $replay_resources[] = ['kind' => 'page_settings', 'id' => (string) $page_id];
                    $replay_lock = mcp_bridge_begin_resource_transaction($replay_resources);
                    if (is_wp_error($replay_lock)) return $replay_lock;
                    try {
                        if ($page_id > 0) {
                            $source_document = get_post_meta($page_id, '_elementor_data', true);
                            $source_revision = is_string($source_document) ? mcp_bridge_hash($source_document) : '';
                            if (is_array($params['document_patch'] ?? null)) {
                                $document_receipt = get_option('_mcp_elementor_operation_' . hash('sha256', $operation_id));
                                $original_revision = is_array($document_receipt) && is_string($document_receipt['original_document'] ?? null) ? mcp_bridge_hash($document_receipt['original_document']) : '';
                                $saved_revision = is_array($document_receipt) && is_string($document_receipt['applied_document'] ?? null) ? mcp_bridge_hash($document_receipt['applied_document']) : '';
                                if (!is_array($document_receipt) || (int) ($document_receipt['post_id'] ?? 0) !== $page_id || (int) ($document_receipt['user_id'] ?? 0) !== get_current_user_id() || !hash_equals((string) ($params['original_expected_revision'] ?? ''), $original_revision) || !hash_equals((string) ($prior['document_revision'] ?? ''), $saved_revision)) return new WP_Error('stale_revision', 'Combined plan replay is not bound to its checked document save receipt.', ['status' => 409]);
                                $source_state = mcp_bridge_editor_guard_full_state($page_id);
                                if (is_wp_error($source_state) || !is_string($prior['source_after_fingerprint'] ?? null) || !hash_equals($prior['source_after_fingerprint'], mcp_bridge_editor_guard_state_fingerprint($source_state))) return new WP_Error('operation_replay_conflict', 'Native source state changed after this combined plan.', ['status' => 409]);
                            } elseif (!hash_equals((string) ($params['expected_revision'] ?? ''), $source_revision)) {
                                return new WP_Error('stale_revision', 'Source document changed before operation replay.', ['status' => 409, 'expected_revision' => $params['expected_revision'] ?? null, 'current_revision' => $source_revision]);
                            }
                        }
                        foreach (($prior['resources'] ?? []) as $resource) {
                            $state = mcp_bridge_resource_state($resource['kind'], $resource['id']);
                            if (is_wp_error($state) || !hash_equals((string) ($resource['after_revision'] ?? ''), (string) ($state['revision'] ?? ''))) return new WP_Error('operation_replay_conflict', 'A resource changed or disappeared after this operation.', ['status' => 409, 'kind' => $resource['kind'], 'id' => $resource['id'], 'current_revision' => is_wp_error($state) ? null : $state['revision']]);
                        }
                        return ['applied' => true, 'valid' => true, 'idempotent_replay' => true, 'current_revision' => $source_revision ?? null, 'resources' => $prior['resources']];
                    } finally { global $wpdb; $wpdb->query('ROLLBACK'); }
                }
                return new WP_Error('operation_incomplete', 'This operation is pending or partial and must be restored before reuse.', ['status' => 409, 'operation_status' => $prior['status'] ?? 'unknown', 'completed' => $prior['resources'] ?? []]);
            }
            $prepared = []; $seen = [];
            foreach ($patches as $index => $patch) {
                $kind = $patch['kind'] ?? ''; $id = $patch['id'] ?? ''; $expected = $patch['expected_revision'] ?? null; $after = $patch['after'] ?? null;
                if (!is_string($kind) || !is_string($id) || !is_string($expected) || !is_array($after)) return new WP_Error('invalid_patch', 'Each patch needs kind, id, expected_revision, and object after.', ['status' => 400, 'index' => $index]);
                $resource_key = mcp_bridge_is_logical_create_id($kind, $id, $expected) ? 'logical-create:' . $id : $kind . ':' . $id;
                if (isset($seen[$resource_key])) return new WP_Error('duplicate_resource_patch', 'A resource may only be patched once per plan.', ['status' => 400, 'kind' => $kind, 'id' => $id]);
                $seen[$resource_key] = true;
                if (!in_array($kind, ['class', 'variable', 'kit', 'page_settings', 'template', 'post_metadata', 'page', 'trash'], true)) return new WP_Error('resource_unavailable', 'Unsupported resource kind.', ['status' => 501, 'kind' => $kind]);
                if ($kind === 'variable' && $id !== 'collection') return new WP_Error('resource_unavailable', 'Variable writes require collection scope for native generated IDs.', ['status' => 501]);
                $new_template = mcp_bridge_is_logical_create_id($kind, $id, $expected);
                if ($new_template && array_key_exists('before', $patch) && $patch['before'] !== null) return new WP_Error('invalid_create_snapshot', 'Logical resource creation cannot carry a before snapshot.', ['status' => 400, 'kind' => $kind, 'id' => $id]);
                if (in_array($kind, ['page', 'template'], true) && str_starts_with($id, 'new') && !$new_template) return new WP_Error('invalid_create_alias', 'New resource IDs must be canonical and use expected_revision absent.', ['status' => 400, 'kind' => $kind, 'id' => $id]);
                $state = $new_template ? null : mcp_bridge_resource_state($kind, $id);
                if (is_wp_error($state)) return $state;
                $revision = $new_template ? 'absent' : $state['revision'];
                if (!hash_equals($expected, $revision)) return new WP_Error('stale_resource', 'Resource changed since it was read.', ['status' => 409, 'kind' => $kind, 'id' => $id, 'current_revision' => $revision]);
                if ($kind === 'class') {
                    $parsed = Elementor\Modules\GlobalClasses\Global_Classes_Parser::make()->parse($after);
                    if (!$parsed->is_valid()) return new WP_Error('invalid_classes', 'Global classes failed Elementor native validation.', ['status' => 400]);
                }
                if ($kind === 'variable') {
                    if (empty($after['operations'])) return new WP_Error('invalid_variables', 'Variable collection patch needs native create/update operations.', ['status' => 400]);
                    try {
                        $collection = Elementor\Modules\Variables\Storage\Variables_Collection::hydrate($state['value']);
                        $processor = new Elementor\Modules\Variables\Services\Batch_Operations\Batch_Processor();
                        $generated_ids = [];
                        foreach ($after['operations'] as $operation) {
                            if (isset($operation['id'], $generated_ids[$operation['id']])) $operation['id'] = $generated_ids[$operation['id']];
                            $result = $processor->apply_operation($collection, $operation);
                            if (($result['type'] ?? '') === 'create' && !empty($result['temp_id'])) $generated_ids[$result['temp_id']] = $result['id'];
                        }
                    } catch (Throwable $error) { return new WP_Error('invalid_variables', $error->getMessage(), ['status' => 400]); }
                }
                if ($kind === 'page') {
                    if ($id !== 'new' || $expected !== 'absent' || !isset($after['title'], $after['status'], $after['document']) || !is_string($after['title']) || !is_string($after['status']) || !in_array($after['status'], ['draft', 'private', 'publish'], true) || !is_array($after['document']) || array_diff(array_keys($after), ['title', 'status', 'slug', 'document'])) return new WP_Error('invalid_page', 'New page needs title, supported status, and document.', ['status' => 400]);
                    if ($after['title'] === '' || sanitize_text_field($after['title']) !== $after['title']) return new WP_Error('invalid_page_title', 'Page title must be non-empty plain text.', ['status' => 400]);
                    if (array_key_exists('slug', $after) && (!is_string($after['slug']) || sanitize_title($after['slug']) !== $after['slug'])) return new WP_Error('invalid_page_slug', 'Slug must be a valid WordPress page slug.', ['status' => 400]);
                    if (!current_user_can('edit_pages') || ($after['status'] === 'publish' && !current_user_can('publish_pages'))) return new WP_Error('rest_forbidden', 'Current user cannot create a page with the requested status.', ['status' => 403]);
                    $ids = []; $valid = mcp_bridge_validate_tree($after['document'], $ids, []);
                    if (is_wp_error($valid)) return $valid;
                }
                if ($kind === 'template') {
                    if (($after['status'] ?? '') !== 'draft' || !is_array($after['document'] ?? null)) return new WP_Error('invalid_template', 'Portable templates must explicitly be drafts and include a document.', ['status' => 400]);
                    if (!in_array($after['type'] ?? '', ['section', 'page'], true) || !isset($after['title']) || !is_string($after['title']) || $after['title'] === '' || sanitize_text_field($after['title']) !== $after['title']) return new WP_Error('template_type_unavailable', 'Only page and section templates with a plain text title are supported by the verified native document path.', ['status' => 501]);
                    $ids = []; $valid = mcp_bridge_validate_tree($after['document'], $ids, []);
                    if (is_wp_error($valid)) return $valid;
                }
                if ($kind === 'post_metadata') {
                    if (array_diff(array_keys($after), ['title', 'status']) || !isset($after['title'], $after['status']) || !is_string($after['title']) || !is_string($after['status'])) return new WP_Error('invalid_post_metadata', 'Post metadata supports title and status only.', ['status' => 400]);
                    if ($state['value']['post_type'] === 'page' && !in_array($after['status'], ['publish', 'draft', 'private'], true)) return new WP_Error('invalid_post_metadata', 'Requested page status is not allowed.', ['status' => 400]);
                    if ($state['value']['post_type'] === 'elementor_library' && !in_array($after['status'], ['publish', 'draft', 'private', 'pending'], true)) return new WP_Error('invalid_post_metadata', 'Requested template status is not allowed.', ['status' => 400]);
                }
                if ($kind === 'trash') {
                    if ($id === 'new' || $after !== [] || !in_array($state['value']['post_type'] ?? '', ['page', 'post', 'elementor_library'], true)) return new WP_Error('invalid_trash_request', 'Trash requires an existing supported post and an empty after value.', ['status' => 400]);
                    if (($state['value']['post_status'] ?? '') === 'trash') return new WP_Error('already_trashed', 'Post is already in the WordPress trash.', ['status' => 409]);
                    if (!current_user_can('delete_post', (int) $id)) return new WP_Error('rest_forbidden', 'Current user cannot trash this post.', ['status' => 403]);
                    if (defined('EMPTY_TRASH_DAYS') && EMPTY_TRASH_DAYS <= 0) return new WP_Error('trash_disabled', 'WordPress trash is disabled; refusing a permanent deletion.', ['status' => 409]);
                }
                $prepared[] = ['kind' => $kind, 'id' => $id, 'expected_revision' => $expected, 'before' => $new_template ? null : $state['value'], 'after' => $after];
            }
            $has_creates = count(array_filter($prepared, fn($patch) => $patch['before'] === null && in_array($patch['kind'], ['page', 'template'], true))) > 0;
            if ((int) ($params['page_id'] ?? 0) === 0 && $has_creates) {
                if (count($prepared) !== count(array_filter($prepared, fn($patch) => $patch['before'] === null && in_array($patch['kind'], ['page', 'template'], true))) || isset($params['document_patch']) || !$prepared) return new WP_Error('invalid_create_plan', 'page_id=0 plans may contain only logical page/template creations and no document patch.', ['status' => 400]);
            }
            if (is_array($params['document_patch'] ?? null)) {
                $post_id = $page_id; $previous = [];
                $current = $post_id > 0 ? get_post_meta($post_id, '_elementor_data', true) : null;
                if (is_string($current)) { $tree = json_decode($current, true); if (is_array($tree)) mcp_bridge_element_index($tree, $previous); }
                $ids = []; $valid = mcp_bridge_validate_tree($params['document_patch'], $ids, $previous);
                if (is_wp_error($valid)) return $valid;
                $kit = mcp_bridge_active_kit();
                if (is_wp_error($kit)) return $kit;
                $references = mcp_bridge_validate_resource_references($params['document_patch'], $prepared, $kit);
                if (is_wp_error($references)) return $references;
            }
            foreach ($prepared as $patch) if (in_array($patch['kind'], ['page', 'template'], true)) {
                $kit = mcp_bridge_active_kit(); if (is_wp_error($kit)) return $kit;
                $references = mcp_bridge_validate_resource_references($patch['after']['document'], $prepared, $kit);
                if (is_wp_error($references)) return $references;
            }
            if (!empty($params['validate_only'])) {
                if ($page_id > 0) {
                    $source_document = get_post_meta($page_id, '_elementor_data', true);
                    $source_revision = is_string($source_document) ? mcp_bridge_hash($source_document) : '';
                    if (!hash_equals((string) $expected_source, $source_revision)) return new WP_Error('stale_revision', 'Source document changed since it was read.', ['status' => 409, 'expected_revision' => $expected_source, 'current_revision' => $source_revision]);
                    if (is_array($params['document_patch'] ?? null) && !hash_equals((string) ($params['original_expected_revision'] ?? $expected_source), $source_revision)) return new WP_Error('stale_revision', 'Combined plan source revision changed before document save.', ['status' => 409, 'expected_revision' => $params['original_expected_revision'] ?? null, 'current_revision' => $source_revision]);
                }
                return ['valid' => true, 'resources' => array_map(fn($patch) => ['kind' => $patch['kind'], 'id' => $patch['id'], 'revision' => $patch['expected_revision']], $prepared)];
            }
            $lock_resources = $prepared;
            if ($page_id > 0) $lock_resources[] = ['kind' => 'page_settings', 'id' => (string) $page_id];
            $locked = mcp_bridge_begin_resource_transaction($lock_resources);
            if (is_wp_error($locked)) return $locked;
            $resource_transaction_open = true;
            if ($page_id > 0) {
                $source_document = get_post_meta($page_id, '_elementor_data', true);
                $source_revision = is_string($source_document) ? mcp_bridge_hash($source_document) : '';
                if (!hash_equals((string) ($params['expected_revision'] ?? ''), $source_revision)) return new WP_Error('stale_revision', 'Source document changed before its native resource transaction.', ['status' => 409, 'expected_revision' => $params['expected_revision'] ?? null, 'current_revision' => $source_revision]);
                if (is_array($params['document_patch'] ?? null)) {
                    $document_key = '_mcp_elementor_operation_' . hash('sha256', $operation_id);
                    $document_receipt = get_option($document_key);
                    $original_revision = is_array($document_receipt) && is_string($document_receipt['original_document'] ?? null) ? mcp_bridge_hash($document_receipt['original_document']) : '';
                    $saved_revision = is_array($document_receipt) && is_string($document_receipt['applied_document'] ?? null) ? mcp_bridge_hash($document_receipt['applied_document']) : '';
                    if (!is_array($document_receipt) || (int) ($document_receipt['post_id'] ?? 0) !== $page_id || (int) ($document_receipt['user_id'] ?? 0) !== get_current_user_id() || !hash_equals((string) ($params['original_expected_revision'] ?? ''), $original_revision) || !hash_equals((string) ($params['expected_revision'] ?? ''), $saved_revision)) return new WP_Error('stale_revision', 'Combined plan is not bound to its checked document save receipt.', ['status' => 409]);
                    $source_before_resources = mcp_bridge_editor_guard_full_state($page_id);
                    if (is_wp_error($source_before_resources) || !is_string($document_receipt['after_fingerprint'] ?? null) || !hash_equals($document_receipt['after_fingerprint'], mcp_bridge_editor_guard_state_fingerprint($source_before_resources))) return new WP_Error('source_state_conflict', 'Native source state changed after the checked document save.', ['status' => 409]);
                }
            }
            foreach ($prepared as &$patch) {
                $fresh = in_array($patch['kind'], ['template', 'page'], true) && mcp_bridge_is_logical_create_id($patch['kind'], $patch['id'], $patch['expected_revision']) ? null : mcp_bridge_resource_state($patch['kind'], $patch['id']);
                if (is_wp_error($fresh)) return $fresh;
                $fresh_revision = $fresh === null ? 'absent' : $fresh['revision'];
                if (!hash_equals($patch['expected_revision'], $fresh_revision)) return new WP_Error('stale_resource', 'Resource changed before its native write transaction.', ['status' => 409, 'kind' => $patch['kind'], 'id' => $patch['id'], 'current_revision' => $fresh_revision]);
                $patch['before'] = $fresh === null ? null : $fresh['value'];
                if ($patch['kind'] === 'class') {
                    $parsed = Elementor\Modules\GlobalClasses\Global_Classes_Parser::make()->parse($patch['after']);
                    if (!$parsed->is_valid()) return new WP_Error('invalid_classes', 'Global classes failed Elementor native validation.', ['status' => 400]);
                } elseif ($patch['kind'] === 'variable') {
                    try {
                        $collection = Elementor\Modules\Variables\Storage\Variables_Collection::hydrate($patch['before']);
                        $processor = new Elementor\Modules\Variables\Services\Batch_Operations\Batch_Processor();
                        $generated_ids = [];
                        foreach (($patch['after']['operations'] ?? []) as $operation) {
                            if (isset($operation['id'], $generated_ids[$operation['id']])) $operation['id'] = $generated_ids[$operation['id']];
                            $result = $processor->apply_operation($collection, $operation);
                            if (($result['type'] ?? '') === 'create' && !empty($result['temp_id'])) $generated_ids[$result['temp_id']] = $result['id'];
                        }
                    } catch (Throwable $error) { return new WP_Error('invalid_variables', $error->getMessage(), ['status' => 400]); }
                } elseif ($patch['kind'] === 'template') {
                    $ids = []; $valid = mcp_bridge_validate_tree($patch['after']['document'], $ids, []);
                    if (is_wp_error($valid)) return $valid;
                }
            }
            unset($patch);
            $document_snapshot = !empty($params['document_snapshot']);
            if (!add_option($key, ['user_id' => get_current_user_id(), 'fingerprint' => $fingerprint, 'status' => 'pending', 'document_snapshot' => $document_snapshot, 'source_page_id' => $document_snapshot ? $page_id : null, 'document_revision' => $page_id > 0 ? $source_revision : null, 'source_before_resources' => $source_before_resources ?? null, 'source_before_resources_fingerprint' => isset($source_before_resources) ? mcp_bridge_editor_guard_state_fingerprint($source_before_resources) : null, 'resources' => []], '', false)) return new WP_Error('operation_id_conflict', 'Operation ID is already in use.', ['status' => 409]);
            $completed = [];
            foreach ($prepared as $patch) {
                $extra = mcp_bridge_apply_resource($patch['kind'], $patch['id'], $patch['after']);
                if (is_wp_error($extra)) {
                    $write_data = $extra->get_error_data();
                    $created_id = is_array($write_data) && isset($write_data['created_id']) && ctype_digit((string) $write_data['created_id']) ? (string) $write_data['created_id'] : null;
                    $observed_id = $created_id ?? $patch['id'];
                    if ($new_create = mcp_bridge_is_logical_create_id($patch['kind'], $patch['id'], $patch['expected_revision'])) {
                        if ($created_id === null) {
                            $partial_source = $document_snapshot && $page_id > 0 ? mcp_bridge_editor_guard_full_state($page_id) : null;
                            if (is_wp_error($partial_source)) return $partial_source;
                            update_option($key, ['user_id' => get_current_user_id(), 'fingerprint' => $fingerprint, 'status' => 'partial', 'document_snapshot' => $document_snapshot, 'source_page_id' => $document_snapshot ? $page_id : null, 'document_revision' => $page_id > 0 ? $source_revision : null, 'source_before_resources' => $source_before_resources ?? null, 'source_before_resources_fingerprint' => isset($source_before_resources) ? mcp_bridge_editor_guard_state_fingerprint($source_before_resources) : null, 'source_after_fingerprint' => is_array($partial_source) ? mcp_bridge_editor_guard_state_fingerprint($partial_source) : null, 'resources' => $completed, 'failed_logical_id' => $patch['id']], false);
                            global $wpdb;
                            if (false === $wpdb->query('COMMIT')) return new WP_Error('resource_commit_failed', 'Native resource state could not be committed for recovery.', ['status' => 500, 'restore_operation_id' => $operation_id]);
                            $resource_transaction_open = false;
                            return ['status' => 'partial_failure', 'restorable' => true, 'error' => $extra->get_error_message(), 'failed_resource' => ['kind' => $patch['kind'], 'logical_id' => $patch['id'], 'created_id' => null], 'completed' => $completed, 'restore_operation_id' => $operation_id];
                        }
                    }
                    $observed = mcp_bridge_resource_state($patch['kind'], $observed_id);
                    if (is_wp_error($observed)) {
                        $partial_source = $document_snapshot && $page_id > 0 ? mcp_bridge_editor_guard_full_state($page_id) : null;
                        update_option($key, ['user_id' => get_current_user_id(), 'fingerprint' => $fingerprint, 'status' => 'partial_unrestorable', 'document_snapshot' => $document_snapshot, 'source_page_id' => $document_snapshot ? $page_id : null, 'document_revision' => $page_id > 0 ? $source_revision : null, 'source_before_resources' => $source_before_resources ?? null, 'source_before_resources_fingerprint' => isset($source_before_resources) ? mcp_bridge_editor_guard_state_fingerprint($source_before_resources) : null, 'source_after_fingerprint' => is_array($partial_source) ? mcp_bridge_editor_guard_state_fingerprint($partial_source) : null, 'resources' => $completed, 'unverified_created_id' => $created_id], false);
                        global $wpdb;
                        if (false === $wpdb->query('COMMIT')) return new WP_Error('resource_commit_failed', 'Native resource state could not be committed for recovery.', ['status' => 500, 'created_id' => $created_id, 'restore_operation_id' => $operation_id]);
                        $resource_transaction_open = false;
                        return new WP_Error($extra->get_error_code(), $extra->get_error_message(), ['status' => is_array($write_data) ? ($write_data['status'] ?? 500) : 500, 'kind' => $patch['kind'], 'id' => $observed_id, 'created_id' => $created_id, 'native_write_data' => $write_data, 'readback_error' => $observed->get_error_message(), 'restorable' => false]);
                    }
                    if (mcp_bridge_hash($observed['value']) !== mcp_bridge_hash($patch['before'])) $completed[] = ['kind' => $patch['kind'], 'logical_id' => $patch['id'], 'id' => $observed_id, 'owner' => get_current_user_id(), 'before' => $patch['before'], 'before_revision' => $patch['expected_revision'], 'after_revision' => $observed['revision'], 'generated_ids' => []];
                    $partial_source = $document_snapshot && $page_id > 0 ? mcp_bridge_editor_guard_full_state($page_id) : null;
                    if (is_wp_error($partial_source)) return $partial_source;
                    update_option($key, ['user_id' => get_current_user_id(), 'fingerprint' => $fingerprint, 'status' => 'partial', 'document_snapshot' => $document_snapshot, 'source_page_id' => $document_snapshot ? $page_id : null, 'document_revision' => $page_id > 0 ? $source_revision : null, 'source_before_resources' => $source_before_resources ?? null, 'source_before_resources_fingerprint' => isset($source_before_resources) ? mcp_bridge_editor_guard_state_fingerprint($source_before_resources) : null, 'source_after_fingerprint' => is_array($partial_source) ? mcp_bridge_editor_guard_state_fingerprint($partial_source) : null, 'resources' => $completed], false);
                    global $wpdb;
                    if (false === $wpdb->query('COMMIT')) return new WP_Error('resource_commit_failed', 'Native resource transaction could not be committed.', ['status' => 500, 'completed' => $completed, 'restore_operation_id' => $operation_id]);
                    $resource_transaction_open = false;
                    return ['status' => 'partial_failure', 'error' => $extra->get_error_message(), 'error_code' => $extra->get_error_code(), 'native_write_data' => $write_data, 'created_id' => $created_id, 'completed' => $completed, 'failed_resource' => ['kind' => $patch['kind'], 'id' => $observed_id], 'restore_operation_id' => $operation_id];
                }
                $saved_id = $extra['generated_id'] ?? $patch['id']; $saved = mcp_bridge_resource_state($patch['kind'], $saved_id);
                if (is_wp_error($saved)) {
                    if (in_array($patch['kind'], ['page', 'template'], true) && $patch['before'] === null && ctype_digit((string) $saved_id)) {
                        $partial_source = $document_snapshot && $page_id > 0 ? mcp_bridge_editor_guard_full_state($page_id) : null;
                        update_option($key, ['user_id' => get_current_user_id(), 'fingerprint' => $fingerprint, 'status' => 'partial_unrestorable', 'document_snapshot' => $document_snapshot, 'source_page_id' => $document_snapshot ? $page_id : null, 'document_revision' => $page_id > 0 ? $source_revision : null, 'source_before_resources' => $source_before_resources ?? null, 'source_before_resources_fingerprint' => isset($source_before_resources) ? mcp_bridge_editor_guard_state_fingerprint($source_before_resources) : null, 'source_after_fingerprint' => is_array($partial_source) ? mcp_bridge_editor_guard_state_fingerprint($partial_source) : null, 'resources' => $completed, 'unverified_created_id' => (string) $saved_id], false);
                        global $wpdb;
                        if (false === $wpdb->query('COMMIT')) return new WP_Error('resource_commit_failed', 'Native resource state could not be committed for recovery.', ['status' => 500, 'created_id' => (string) $saved_id, 'restore_operation_id' => $operation_id]);
                        $resource_transaction_open = false;
                        return new WP_Error('resource_readback_failed', 'Created resource exists, but its native state could not be verified; automatic restore is unsafe.', ['status' => 500, 'created_id' => (string) $saved_id, 'restore_operation_id' => $operation_id, 'restorable' => false, 'readback_error' => $saved->get_error_message()]);
                    }
                    return new WP_Error('resource_write_rolled_back', 'Native resource write could not be verified; its transaction will be rolled back.', ['status' => 500, 'kind' => $patch['kind'], 'id' => $saved_id]);
                }
                $generated = $extra['generated_ids'] ?? (isset($extra['generated_id']) ? [['logical_id' => $patch['id'], 'id' => $extra['generated_id']]] : []);
                foreach ($generated as &$mapping) if (!isset($mapping['logical_id']) && isset($mapping['temp_id'])) $mapping['logical_id'] = $mapping['temp_id'];
                unset($mapping);
                $completed[] = ['kind' => $patch['kind'], 'logical_id' => $patch['id'], 'id' => $saved_id, 'owner' => get_current_user_id(), 'before' => $patch['before'], 'before_revision' => $patch['expected_revision'], 'after_revision' => $saved['revision'], 'generated_ids' => $generated];
                $source_progress = $document_snapshot && $page_id > 0 ? mcp_bridge_editor_guard_full_state($page_id) : null;
                if (is_wp_error($source_progress)) return $source_progress;
                update_option($key, ['user_id' => get_current_user_id(), 'fingerprint' => $fingerprint, 'status' => 'pending', 'document_snapshot' => $document_snapshot, 'source_page_id' => $document_snapshot ? $page_id : null, 'document_revision' => $page_id > 0 ? $source_revision : null, 'source_before_resources' => $source_before_resources ?? null, 'source_before_resources_fingerprint' => isset($source_before_resources) ? mcp_bridge_editor_guard_state_fingerprint($source_before_resources) : null, 'source_after_fingerprint' => is_array($source_progress) ? mcp_bridge_editor_guard_state_fingerprint($source_progress) : null, 'resources' => $completed], false);
            }
            $source_after_resources = $document_snapshot && $page_id > 0 ? mcp_bridge_editor_guard_full_state($page_id) : null;
            if (is_wp_error($source_after_resources)) return $source_after_resources;
            update_option($key, ['user_id' => get_current_user_id(), 'fingerprint' => $fingerprint, 'status' => 'completed', 'document_snapshot' => $document_snapshot, 'source_page_id' => $document_snapshot ? $page_id : null, 'document_revision' => $page_id > 0 ? $source_revision : null, 'source_before_resources' => $source_before_resources ?? null, 'source_before_resources_fingerprint' => isset($source_before_resources) ? mcp_bridge_editor_guard_state_fingerprint($source_before_resources) : null, 'source_after_fingerprint' => is_array($source_after_resources) ? mcp_bridge_editor_guard_state_fingerprint($source_after_resources) : null, 'resources' => $completed], false);
            global $wpdb;
            if (false === $wpdb->query('COMMIT')) return new WP_Error('resource_commit_failed', 'Native resource transaction could not be committed.', ['status' => 500, 'completed' => $completed, 'restore_operation_id' => $operation_id]);
            $resource_transaction_open = false;
            return ['applied' => true, 'operation_id' => $operation_id, 'document_snapshot' => $document_snapshot, 'resources' => $completed];
            } finally {
                if ($resource_transaction_open) { global $wpdb; $wpdb->query('ROLLBACK'); }
                mcp_bridge_unlock($resource_lock);
            }
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/resources/(?P<kind>class|variable|kit|page_settings|template|post_metadata|page|trash)/(?P<id>[A-Za-z0-9_-]+)', [
        'methods' => 'GET',
        'callback' => function($request) { return mcp_bridge_resource_state($request['kind'], $request['id']); },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/resources/restore', [
        'methods' => 'POST',
        'callback' => function($request) {
            $operation_id = $request->get_param('operation_id');
            if (!is_string($operation_id)) return new WP_Error('invalid_request', 'operation_id is required.', ['status' => 400]);
            $key = '_mcp_elementor_resources_' . hash('sha256', $operation_id); $snapshot = get_option($key);
            if (!is_array($snapshot) || (int) ($snapshot['user_id'] ?? 0) !== get_current_user_id()) return ['restored' => false, 'document_snapshot' => false, 'reason' => 'resource_snapshot_not_found'];
            if (($snapshot['status'] ?? '') === 'partial_unrestorable') return new WP_Error('resource_restore_unverifiable', 'This partial operation contains a created resource whose state could not be verified; manual recovery is required.', ['status' => 409, 'resources' => $snapshot['resources'] ?? [], 'created_id' => $snapshot['unverified_created_id'] ?? null]);
            if (($snapshot['status'] ?? '') === 'restored') {
                $replay_lock = mcp_bridge_lock(0);
                if (is_wp_error($replay_lock)) return $replay_lock;
                $replay_transaction_open = false;
                try {
                $replay_resources = $snapshot['restored_resources'] ?? [];
                $restored_source_page_id = !empty($snapshot['document_snapshot']) ? (int) ($snapshot['source_page_id'] ?? 0) : 0;
                if ($restored_source_page_id > 0) $replay_resources[] = ['kind' => 'page_settings', 'id' => (string) $restored_source_page_id];
                $locked = mcp_bridge_begin_resource_transaction($replay_resources);
                if (is_wp_error($locked)) return $locked;
                $replay_transaction_open = true;
                if ($restored_source_page_id > 0) {
                    $restored_source = mcp_bridge_editor_guard_full_state($restored_source_page_id);
                    $document_receipt = get_option('_mcp_elementor_operation_' . hash('sha256', $operation_id));
                    $expected_source_fingerprint = is_array($document_receipt) && !empty($document_receipt['restored']) ? ($document_receipt['restored_fingerprint'] ?? null) : ($snapshot['source_before_resources_fingerprint'] ?? null);
                    if (is_wp_error($restored_source) || !is_string($expected_source_fingerprint) || !hash_equals($expected_source_fingerprint, mcp_bridge_editor_guard_state_fingerprint($restored_source))) return new WP_Error('resource_restore_conflict', 'Paired source state changed after the combined restore.', ['status' => 409]);
                }
                foreach (($snapshot['restored_resources'] ?? []) as $resource) {
                    if (($resource['revision'] ?? '') === 'deleted' && get_post((int) $resource['id'])) return new WP_Error('resource_restore_conflict', 'A created resource reappeared after restore.', ['status' => 409, 'kind' => $resource['kind'], 'id' => $resource['id']]);
                    if (($resource['revision'] ?? '') !== 'deleted') {
                        $state = mcp_bridge_resource_state($resource['kind'], $resource['id']);
                        if (is_wp_error($state) || !hash_equals((string) $resource['revision'], (string) ($state['revision'] ?? ''))) return new WP_Error('resource_restore_conflict', 'A restored resource changed after restore.', ['status' => 409, 'kind' => $resource['kind'], 'id' => $resource['id']]);
                    }
                }
                global $wpdb;
                if (false === $wpdb->query('COMMIT')) return new WP_Error('resource_restore_failed', 'Could not finish the restore replay read transaction.', ['status' => 500]);
                $replay_transaction_open = false;
                return ['restored' => true, 'idempotent_replay' => true, 'document_snapshot' => !empty($snapshot['document_snapshot']), 'resources' => $snapshot['restored_resources'] ?? []];
                } finally { if ($replay_transaction_open) { global $wpdb; $wpdb->query('ROLLBACK'); } mcp_bridge_unlock($replay_lock); }
            }
            if (($snapshot['status'] ?? '') === 'pending') return new WP_Error('resource_snapshot_incomplete', 'The operation did not record a complete resource revision; refusing to restore by assumption.', ['status' => 409]);
            $resource_lock = mcp_bridge_lock(0);
            if (is_wp_error($resource_lock)) return $resource_lock;
            $resource_transaction_open = false;
            try {
            $paired_source_page_id = !empty($snapshot['document_snapshot']) ? (int) ($snapshot['source_page_id'] ?? 0) : 0;
            $restore_resources = $snapshot['resources'] ?? [];
            if ($paired_source_page_id > 0) $restore_resources[] = ['kind' => 'page_settings', 'id' => (string) $paired_source_page_id];
            $locked = mcp_bridge_begin_resource_transaction($restore_resources);
            if (is_wp_error($locked)) return $locked;
            $resource_transaction_open = true;
            $paired_source_before = null;
            if ($paired_source_page_id > 0) {
                $paired_source_before = $snapshot['source_before_resources'] ?? null;
                if (!is_array($paired_source_before) || !is_string($snapshot['source_before_resources_fingerprint'] ?? null) || !is_string($snapshot['source_after_fingerprint'] ?? null)) return new WP_Error('resource_restore_conflict', 'Combined source snapshot is incomplete; refusing to restore by assumption.', ['status' => 409]);
                $document_receipt = get_option('_mcp_elementor_operation_' . hash('sha256', $operation_id));
                $saved_revision = is_array($document_receipt) && is_string($document_receipt['applied_document'] ?? null) ? mcp_bridge_hash($document_receipt['applied_document']) : '';
                if (!is_array($document_receipt) || (int) ($document_receipt['post_id'] ?? 0) !== $paired_source_page_id || (int) ($document_receipt['user_id'] ?? 0) !== get_current_user_id() || !hash_equals((string) ($snapshot['document_revision'] ?? ''), $saved_revision) || !hash_equals((string) ($document_receipt['after_fingerprint'] ?? ''), $snapshot['source_before_resources_fingerprint'])) return new WP_Error('resource_restore_conflict', 'Combined source snapshot is not bound to its immutable document receipt.', ['status' => 409]);
                $current_source = mcp_bridge_editor_guard_full_state($paired_source_page_id);
                if (is_wp_error($current_source) || !hash_equals($snapshot['source_after_fingerprint'], mcp_bridge_editor_guard_state_fingerprint($current_source))) return new WP_Error('resource_restore_conflict', 'Native source state changed after this combined operation.', ['status' => 409]);
            }
            foreach ($snapshot['resources'] as $resource) {
                $state = mcp_bridge_resource_state($resource['kind'], $resource['id']);
                if (is_wp_error($state) || !hash_equals($resource['after_revision'], $state['revision'])) return new WP_Error('resource_restore_conflict', 'Resource changed after this operation; refusing to overwrite it.', ['status' => 409, 'kind' => $resource['kind'], 'id' => $resource['id'], 'current_revision' => is_wp_error($state) ? null : $state['revision']]);
            }
            $restored = [];
            foreach (array_reverse($snapshot['resources']) as $resource) {
                $state = mcp_bridge_resource_state($resource['kind'], $resource['id']);
                if (is_wp_error($state) || !hash_equals($resource['after_revision'], $state['revision'])) return new WP_Error('resource_restore_conflict', 'Resource changed after this operation; refusing to overwrite it.', ['status' => 409, 'completed' => $restored, 'kind' => $resource['kind'], 'id' => $resource['id']]);
                if (in_array($resource['kind'], ['template', 'page'], true) && $resource['before'] === null) {
                    $created_post = get_post((int) $resource['id']);
                    $expected_type = $resource['kind'] === 'page' ? 'page' : 'elementor_library';
                    if (!$created_post || $created_post->post_type !== $expected_type) return new WP_Error('resource_restore_conflict', 'Created post no longer matches the journaled resource.', ['status' => 409]);
                    $deleted = wp_delete_post((int) $resource['id'], true);
                    if (!$deleted || get_post((int) $resource['id'])) return new WP_Error('resource_restore_partial_failure', 'Created resource could not be deleted and verified.', ['status' => 500, 'completed' => $restored]);
                }
                else {
                    $result = mcp_bridge_restore_resource($resource['kind'], $resource['id'], $resource['before']);
                    if (is_wp_error($result)) return new WP_Error('resource_restore_partial_failure', $result->get_error_message(), ['status' => 500, 'completed' => $restored]);
                }
                $new_state = in_array($resource['kind'], ['template', 'page'], true) && $resource['before'] === null ? null : mcp_bridge_resource_state($resource['kind'], $resource['id']);
                $restored[] = ['kind' => $resource['kind'], 'id' => $resource['id'], 'revision' => is_array($new_state) ? $new_state['revision'] : 'deleted'];
            }
            if ($paired_source_page_id > 0) {
                $current_source = mcp_bridge_editor_guard_full_state($paired_source_page_id);
                if (is_wp_error($current_source)) return $current_source;
                $owned_revision_ids = array_values(array_diff($current_source['revision_ids'] ?? [], $paired_source_before['revision_ids'] ?? []));
                $source_restore = mcp_bridge_editor_guard_restore_state($paired_source_page_id, $paired_source_before, $owned_revision_ids);
                if (is_wp_error($source_restore)) return $source_restore;
                $restored_css = mcp_bridge_editor_guard_restore_css($paired_source_page_id, $paired_source_before);
                if (is_wp_error($restored_css)) return $restored_css;
                $verified_source = mcp_bridge_editor_guard_full_state($paired_source_page_id);
                if (is_wp_error($verified_source) || !mcp_bridge_editor_guard_restore_matches($paired_source_before, $verified_source)) return new WP_Error('resource_restore_partial_failure', 'Resources were restored, but the paired native source state differs from its saved intermediate snapshot.', ['status' => 500, 'completed' => $restored]);
                foreach ($restored as &$restored_resource) {
                    if ((int) ($restored_resource['id'] ?? 0) !== $paired_source_page_id) continue;
                    $state = mcp_bridge_resource_state($restored_resource['kind'], $restored_resource['id']);
                    if (!is_wp_error($state)) $restored_resource['revision'] = $state['revision'];
                }
                unset($restored_resource);
            }
            $snapshot['status'] = 'restored';
            $snapshot['restored_resources'] = $restored;
            global $wpdb;
            if (false === $wpdb->query('COMMIT')) return new WP_Error('resource_restore_commit_failed', 'Native resource restore could not be committed; database rollback will be attempted while restored filesystem effects may remain.', ['status' => 500, 'completed' => $restored, 'restore_operation_id' => $operation_id, 'database_rollback_attempted' => true, 'filesystem_effects_restored' => $paired_source_page_id > 0]);
            $resource_transaction_open = false;
            update_option($key, $snapshot, false);
            return ['restored' => true, 'document_snapshot' => !empty($snapshot['document_snapshot']), 'resources' => $restored];
            } finally {
                if ($resource_transaction_open) { global $wpdb; $wpdb->query('ROLLBACK'); }
                mcp_bridge_unlock($resource_lock);
            }
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/write-mu-plugin', [
        'methods' => 'POST',
        'callback' => function($req) {
            $name = sanitize_file_name($req['filename']);
            $name = preg_replace('/[^a-zA-Z0-9\-_]/', '', pathinfo($name, PATHINFO_FILENAME)) . '.php';
            $code = $req['php_code'];
            if (strpos($code, '<?php') !== 0) return new WP_Error('invalid', 'PHP must start with <?php');
            if (!wp_mkdir_p(WPMU_PLUGIN_DIR)) return new WP_Error('fs', 'Cannot create mu-plugins dir');
            if (file_put_contents(WPMU_PLUGIN_DIR . '/' . $name, $code) === false) return new WP_Error('fs', 'Could not write mu-plugin.');
            return ['written' => $name];
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/write-theme-file', [
        'methods' => 'POST',
        'callback' => function($req) {
            $file = ltrim($req['path'], '/');
            if (strpos($file, '..') !== false) return new WP_Error('invalid', 'Path traversal not allowed');
            $full = get_stylesheet_directory() . '/' . $file;
            if (!wp_mkdir_p(dirname($full)) || file_put_contents($full, $req['content']) === false) return new WP_Error('fs', 'Could not write theme file.');
            return ['written' => $file, 'theme' => get_stylesheet()];
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/read-theme-file', [
        'methods' => 'GET',
        'callback' => function($req) {
            $file = ltrim($req['path'], '/');
            if (strpos($file, '..') !== false) return new WP_Error('invalid', 'Path traversal not allowed');
            $full = get_stylesheet_directory() . '/' . $file;
            if (!file_exists($full)) return new WP_Error('not_found', 'File not found.', ['status' => 404]);
            return ['path' => $file, 'content' => file_get_contents($full)];
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/option/(?P<name>[a-zA-Z0-9_\-]+)', [
        ['methods' => 'GET', 'callback' => function($req) { return rest_ensure_response(get_option($req['name'])); }, 'permission_callback' => $admin],
        ['methods' => 'POST', 'callback' => function($req) {
            global $wpdb;
            $name = (string) $req['name'];
            $lower_name = strtolower($name);
            foreach (['elementor_', '_elementor_', 'elementor_experiment-', '_mcp_elementor_'] as $prefix) {
                if (strpos($lower_name, $prefix) === 0) return new WP_Error('protected_option', 'Elementor native and bridge options cannot be written through this endpoint.', ['status' => 403]);
            }
            $params = json_decode($req->get_body());
            if (!is_object($params) || !property_exists($params, 'value')) return new WP_Error('invalid_body', 'A JSON object with an explicit value field is required.', ['status' => 400]);
            $before_raw = $wpdb->get_var($wpdb->prepare("SELECT option_value FROM {$wpdb->options} WHERE option_name = %s", $name));
            $before = $before_raw === null ? null : maybe_unserialize($before_raw);
            update_option($name, $params->value);
            $stored_raw = $wpdb->get_var($wpdb->prepare("SELECT option_value FROM {$wpdb->options} WHERE option_name = %s", $name));
            if ($stored_raw === null) return new WP_Error('option_write_failed', 'The option was not persisted.', ['status' => 500]);
            $stored = maybe_unserialize($stored_raw);
            $expected = $params->value;
            if (is_bool($expected)) $expected = $expected ? '1' : '';
            elseif ($expected === null) $expected = '';
            elseif (is_int($expected) || is_float($expected)) $expected = (string) $expected;
            if (maybe_serialize($stored) !== maybe_serialize($expected)) return new WP_Error('option_write_failed', 'The persisted option does not match the requested value.', ['status' => 500]);
            wp_cache_delete($name, 'options');
            return ['updated' => $name, 'changed' => $before_raw === null || maybe_serialize($before) !== maybe_serialize($stored)];
        }, 'permission_callback' => $admin],
    ]);

    // --- Elementor V4 Native Tooling Endpoints ---

    $run_ability = function($id, $input) {
        if (!function_exists('wp_get_ability')) return null;
        $ability = wp_get_ability($id);
        if (!$ability || !method_exists($ability, 'execute')) return null;
        return $ability->execute($input);
    };

    $resolve_class_labels = function($labels) use ($kit) {
        if (!class_exists('Elementor\\Modules\\GlobalClasses\\Global_Classes_Repository')) return [];
        $kit_inst = mcp_bridge_active_kit();
        if (is_wp_error($kit_inst)) return [];
        $all = Elementor\Modules\GlobalClasses\Global_Classes_Repository::make($kit_inst)->all(true)->get();
        $by_label = [];
        foreach (($all['items'] ?? []) as $cid => $item) {
            if (!empty($item['label'])) $by_label[$item['label']] = $cid;
        }
        $ids = [];
        foreach ($labels as $lbl) {
            if (isset($by_label[$lbl])) $ids[] = $by_label[$lbl];
        }
        return $ids;
    };

    register_rest_route($ns, '/v4/variables', [
        'methods' => 'POST',
        'callback' => function($req) use ($run_ability) {
            $ops = $req->get_param('operations');
            if (!is_array($ops)) return new WP_Error('invalid_request', 'operations array required', ['status' => 400]);
            $native = $run_ability('elementor/manage-global-variable', ['operations' => $ops]);
            if ($native !== null) return is_wp_error($native) ? $native : rest_ensure_response($native);
            $kit = mcp_bridge_active_kit();
            if (is_wp_error($kit)) return $kit;
            if (!class_exists('Elementor\\Modules\\Variables\\Storage\\Variables_Repository') || !class_exists('Elementor\\Modules\\Variables\\Services\\Batch_Operations\\Batch_Processor')) {
                return new WP_Error('v4_unavailable', 'Elementor Variables repository is unavailable.', ['status' => 501]);
            }
            $repo = new Elementor\Modules\Variables\Storage\Variables_Repository($kit);
            $collection = $repo->load();
            $processor = new Elementor\Modules\Variables\Services\Batch_Operations\Batch_Processor();
            $results = [];
            foreach ($ops as $op) {
                $results[] = $processor->apply_operation($collection, $op);
            }
            $repo->save($collection);
            return ['results' => $results, 'variables' => $collection->serialize(true)];
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/v4/classes', [
        'methods' => 'POST',
        'callback' => function($req) use ($run_ability) {
            $ops = $req->get_param('operations');
            if (!is_array($ops)) return new WP_Error('invalid_request', 'operations array required', ['status' => 400]);
            $native = $run_ability('elementor/manage-classes', ['operations' => $ops]);
            if ($native !== null) return is_wp_error($native) ? $native : rest_ensure_response($native);
            $kit = mcp_bridge_active_kit();
            if (is_wp_error($kit)) return $kit;
            if (!class_exists('Elementor\\Modules\\GlobalClasses\\Global_Classes_Repository')) {
                return new WP_Error('v4_unavailable', 'Elementor Global Classes repository is unavailable.', ['status' => 501]);
            }
            $repo = Elementor\Modules\GlobalClasses\Global_Classes_Repository::make($kit);
            $current = $repo->all(true)->get();
            $items = is_array($current['items'] ?? null) ? $current['items'] : [];
            $order = is_array($current['order'] ?? null) ? $current['order'] : [];
            $results = [];
            foreach ($ops as $op) {
                $action = $op['action'] ?? '';
                if ($action === 'create') {
                    $id = 'g-' . substr(bin2hex(random_bytes(4)), 0, 7);
                    $label = (string) ($op['label'] ?? $id);
                    foreach ($items as $existing) {
                        if (($existing['label'] ?? '') === $label) { $label = 'DUP_' . $label; break; }
                    }
                    $items[$id] = ['id' => $id, 'label' => $label, 'type' => 'class', 'variants' => $op['variants'] ?? []];
                    $order[] = $id;
                    $results[] = ['action' => 'create', 'id' => $id, 'label' => $label];
                } elseif ($action === 'update') {
                    $target_id = $op['id'] ?? null;
                    if (!$target_id && !empty($op['label'])) {
                        foreach ($items as $cid => $item) {
                            if (($item['label'] ?? '') === $op['label']) { $target_id = $cid; break; }
                        }
                    }
                    if (!$target_id || !isset($items[$target_id])) return new WP_Error('not_found', 'Global class not found.', ['status' => 404]);
                    if (!empty($op['label'])) $items[$target_id]['label'] = (string) $op['label'];
                    if (isset($op['variants']) && is_array($op['variants'])) {
                        $mode = $op['mode'] ?? 'patch';
                        if ($mode === 'replace') {
                            $items[$target_id]['variants'] = $op['variants'];
                        } else {
                            $existing_v = is_array($items[$target_id]['variants'] ?? null) ? $items[$target_id]['variants'] : [];
                            foreach ($op['variants'] as $nv) {
                                $matched = false;
                                foreach ($existing_v as &$ev) {
                                    if (($ev['meta'] ?? []) == ($nv['meta'] ?? [])) {
                                        foreach (($nv['props'] ?? []) as $pk => $pv) {
                                            if ($pv === null) unset($ev['props'][$pk]);
                                            else $ev['props'][$pk] = $pv;
                                        }
                                        $matched = true;
                                        break;
                                    }
                                }
                                unset($ev);
                                if (!$matched) $existing_v[] = $nv;
                            }
                            $items[$target_id]['variants'] = $existing_v;
                        }
                    }
                    $results[] = ['action' => 'update', 'id' => $target_id];
                } elseif ($action === 'delete') {
                    $target_id = $op['id'] ?? null;
                    if (!$target_id && !empty($op['label'])) {
                        foreach ($items as $cid => $item) {
                            if (($item['label'] ?? '') === $op['label']) { $target_id = $cid; break; }
                        }
                    }
                    if (!$target_id || !isset($items[$target_id])) return new WP_Error('not_found', 'Global class not found.', ['status' => 404]);
                    unset($items[$target_id]);
                    $order = array_values(array_filter($order, fn($x) => $x !== $target_id));
                    $results[] = ['action' => 'delete', 'id' => $target_id];
                }
            }
            $repo->put($items, $order);
            return ['results' => $results, 'order' => $order];
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/v4/classes/reorder', [
        'methods' => 'POST',
        'callback' => function($req) use ($run_ability) {
            $body = $req->get_json_params() ?: [];
            $native = $run_ability('elementor/reorder-classes', $body);
            if ($native !== null) return is_wp_error($native) ? $native : rest_ensure_response($native);
            $kit = mcp_bridge_active_kit();
            if (is_wp_error($kit)) return $kit;
            if (!class_exists('Elementor\\Modules\\GlobalClasses\\Global_Classes_Repository')) {
                return new WP_Error('v4_unavailable', 'Elementor Global Classes repository is unavailable.', ['status' => 501]);
            }
            $repo = Elementor\Modules\GlobalClasses\Global_Classes_Repository::make($kit);
            $current = $repo->all(true)->get();
            $items = is_array($current['items'] ?? null) ? $current['items'] : [];
            $order = is_array($current['order'] ?? null) ? $current['order'] : array_keys($items);
            if (isset($body['order']) && is_array($body['order'])) {
                $requested = array_values(array_filter($body['order'], fn($id) => isset($items[$id])));
                $remaining = array_values(array_diff($order, $requested));
                $order = array_merge($requested, $remaining);
            }
            foreach (($body['moves'] ?? []) as $move) {
                $id = $move['id'] ?? '';
                $pos = $move['position'] ?? '';
                $ref = $move['ref'] ?? '';
                $order = array_values(array_filter($order, fn($x) => $x !== $id));
                if ($pos === 'start') array_unshift($order, $id);
                elseif ($pos === 'end') $order[] = $id;
                elseif ($pos === 'before' || $pos === 'after') {
                    $idx = array_search($ref, $order, true);
                    if ($idx === false) $order[] = $id;
                    else array_splice($order, $pos === 'after' ? $idx + 1 : $idx, 0, [$id]);
                }
            }
            $repo->put($items, $order);
            return ['order' => $order];
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/v4/default-styles', [
        ['methods' => 'GET', 'callback' => function($req) use ($run_ability) {
            $tag = (string) $req->get_param('tag');
            $native = $run_ability('elementor/get-default-styles', ['tag' => $tag]);
            if ($native !== null) return is_wp_error($native) ? $native : rest_ensure_response($native);
            $kit = mcp_bridge_active_kit();
            if (is_wp_error($kit)) return $kit;
            $styles = get_post_meta($kit->get_id(), '_elementor_default_styles', true);
            $styles = is_array($styles) ? $styles : [];
            return ['tag' => $tag, 'styles' => $styles[$tag] ?? null];
        }, 'permission_callback' => $admin],
        ['methods' => 'POST', 'callback' => function($req) use ($run_ability) {
            $ops = $req->get_param('operations');
            if (!is_array($ops)) return new WP_Error('invalid_request', 'operations array required', ['status' => 400]);
            $native = $run_ability('elementor/manage-default-styles', ['operations' => $ops]);
            if ($native !== null) return is_wp_error($native) ? $native : rest_ensure_response($native);
            $kit = mcp_bridge_active_kit();
            if (is_wp_error($kit)) return $kit;
            $styles = get_post_meta($kit->get_id(), '_elementor_default_styles', true);
            $styles = is_array($styles) ? $styles : [];
            foreach ($ops as $op) {
                $tag = (string) ($op['tag'] ?? '');
                if (($op['action'] ?? '') === 'delete') unset($styles[$tag]);
                elseif (($op['action'] ?? '') === 'update') $styles[$tag] = ['variants' => $op['variants'] ?? [], 'css' => $op['css'] ?? ''];
            }
            update_post_meta($kit->get_id(), '_elementor_default_styles', $styles);
            return ['updated' => true, 'tags' => array_keys($styles)];
        }, 'permission_callback' => $admin],
    ]);

    register_rest_route($ns, '/v4/components/list', [
        'methods' => 'POST',
        'callback' => function($req) use ($run_ability) {
            $body = $req->get_json_params() ?: [];
            $native = $run_ability('elementor/list-components', $body);
            if ($native !== null) return is_wp_error($native) ? $native : rest_ensure_response($native);
            $ids = $body['component_ids'] ?? null;
            $query_args = [
                'post_type' => ['e-component', 'elementor_library'],
                'post_status' => ['publish', 'draft'],
                'posts_per_page' => 100,
                'meta_query' => [['key' => '_elementor_template_type', 'value' => 'component']],
            ];
            if (is_array($ids) && !empty($ids)) {
                $query_args = ['post_type' => 'any', 'post__in' => array_map('intval', $ids), 'post_status' => 'any', 'posts_per_page' => count($ids)];
            }
            $posts = get_posts($query_args);
            $out = [];
            foreach ($posts as $p) {
                $row = ['id' => (int) $p->ID, 'name' => $p->post_title, 'uid' => get_post_meta($p->ID, '_elementor_component_uid', true) ?: ('comp-' . $p->ID)];
                if (is_array($ids) && !empty($ids)) {
                    $row['is_archived'] = $p->post_status === 'trash';
                    $row['overridable_props'] = get_post_meta($p->ID, '_elementor_overridable_props', true) ?: new stdClass();
                }
                $out[] = $row;
            }
            return ['capabilities' => ['can_create' => current_user_can('edit_posts'), 'can_edit' => current_user_can('edit_posts'), 'can_add_to_page' => current_user_can('edit_pages')], 'components' => $out];
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/v4/components/manage', [
        'methods' => 'POST',
        'callback' => function($req) use ($run_ability, $resolve_class_labels) {
            $body = $req->get_json_params() ?: [];
            $action = $body['action'] ?? '';
            $status = ($body['publish_status'] ?? 'publish') === 'draft' ? 'draft' : 'publish';
            if ($action === 'create') {
                $post_id = wp_insert_post(['post_type' => 'elementor_library', 'post_status' => $status, 'post_title' => sanitize_text_field($body['title'] ?? 'Component')], true);
                if (is_wp_error($post_id)) return $post_id;
                $uid = 'comp-' . substr(bin2hex(random_bytes(4)), 0, 8);
                update_post_meta($post_id, '_elementor_template_type', 'component');
                update_post_meta($post_id, '_elementor_edit_mode', 'builder');
                update_post_meta($post_id, '_elementor_component_uid', $uid);
                $elements = $body['elements'] ?? [];
                if (!empty($body['source_post_id']) && !empty($body['element_id'])) {
                    $src = json_decode((string) get_post_meta((int) $body['source_post_id'], '_elementor_data', true), true) ?: [];
                    $find_el = function($list, $eid) use (&$find_el) {
                        foreach ($list as $el) {
                            if (($el['id'] ?? '') === $eid) return $el;
                            if (!empty($el['elements']) && ($found = $find_el($el['elements'], $eid))) return $found;
                        }
                        return null;
                    };
                    $copied = $find_el($src, $body['element_id']);
                    if (!$copied) return new WP_Error('not_found', 'Source element not found.', ['status' => 404]);
                    $elements = [$copied];
                }
                if (!empty($elements)) update_post_meta($post_id, '_elementor_data', wp_slash(wp_json_encode($elements)));
                if (isset($body['overridable_props'])) update_post_meta($post_id, '_elementor_overridable_props', $body['overridable_props']);
                mcp_bridge_regenerate_css($post_id);
                return ['component_id' => $post_id, 'uid' => $uid, 'edit_url' => admin_url("post.php?post={$post_id}&action=elementor"), 'llm_instructions' => 'Share edit_url with the user to review the component.'];
            } elseif ($action === 'update') {
                $cid = (int) ($body['component_id'] ?? 0);
                if (!$cid || !get_post($cid)) return new WP_Error('not_found', 'Component not found.', ['status' => 404]);
                if (!empty($body['elements'])) update_post_meta($cid, '_elementor_data', wp_slash(wp_json_encode($body['elements'])));
                if (isset($body['overridable_props'])) update_post_meta($cid, '_elementor_overridable_props', $body['overridable_props']);
                mcp_bridge_regenerate_css($cid);
                return ['component_id' => $cid, 'updated' => true, 'edit_url' => admin_url("post.php?post={$cid}&action=elementor"), 'llm_instructions' => 'Share edit_url with the user.'];
            } elseif ($action === 'rename') {
                $cid = (int) ($body['component_id'] ?? 0);
                wp_update_post(['ID' => $cid, 'post_title' => sanitize_text_field($body['title'] ?? '')]);
                return ['component_id' => $cid, 'renamed' => true, 'edit_url' => admin_url("post.php?post={$cid}&action=elementor")];
            } elseif ($action === 'archive') {
                $ok = []; $fail = [];
                foreach (($body['component_ids'] ?? []) as $cid) {
                    if (wp_trash_post((int) $cid)) $ok[] = (int) $cid;
                    else $fail[] = (int) $cid;
                }
                return ['success_ids' => $ok, 'failed_ids' => $fail];
            } elseif ($action === 'publish') {
                $cid = (int) ($body['component_id'] ?? 0);
                wp_update_post(['ID' => $cid, 'post_status' => 'publish']);
                mcp_bridge_regenerate_css($cid);
                return ['component_id' => $cid, 'status' => 'publish', 'edit_url' => admin_url("post.php?post={$cid}&action=elementor")];
            }
            return new WP_Error('invalid_action', 'Unknown component action.', ['status' => 400]);
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/v4/build-composition', [
        'methods' => 'POST',
        'callback' => function($req) use ($resolve_class_labels) {
            $body = $req->get_json_params() ?: [];
            $post_id = (int) ($body['post_id'] ?? 0);
            if (!$post_id || !get_post($post_id)) return new WP_Error('not_found', 'Document not found.', ['status' => 404]);
            $new_els = is_array($body['elements'] ?? null) ? $body['elements'] : [];
            $class_labels = is_array($body['class_labels'] ?? null) ? $body['class_labels'] : [];
            $attach_classes = function(&$list) use (&$attach_classes, $class_labels, $resolve_class_labels) {
                foreach ($list as &$el) {
                    $cfg_id = $el['configuration_id'] ?? null;
                    unset($el['configuration_id']);
                    if ($cfg_id && !empty($class_labels[$cfg_id])) {
                        $resolved = $resolve_class_labels($class_labels[$cfg_id]);
                        if (!empty($resolved)) {
                            $existing = $el['settings']['classes']['value'] ?? [];
                            $el['settings']['classes'] = ['$$type' => 'classes', 'value' => array_values(array_merge($existing, $resolved))];
                        }
                    }
                    if (!empty($el['elements']) && is_array($el['elements'])) $attach_classes($el['elements']);
                }
            };
            $attach_classes($new_els);
            $raw = get_post_meta($post_id, '_elementor_data', true);
            $tree = is_string($raw) && $raw !== '' ? (json_decode($raw, true) ?: []) : [];
            $parent_id = (string) ($body['parent_id'] ?? 'document');
            $mode = (string) ($body['mode'] ?? 'append');
            if ($parent_id === 'document' || $parent_id === '') {
                $tree = $mode === 'replace_children' ? $new_els : array_merge($tree, $new_els);
            } else {
                $inserted = false;
                $walk = function(&$nodes) use (&$walk, $parent_id, $mode, $new_els, &$inserted) {
                    foreach ($nodes as &$n) {
                        if (($n['id'] ?? '') === $parent_id) {
                            $existing = is_array($n['elements'] ?? null) ? $n['elements'] : [];
                            $n['elements'] = $mode === 'replace_children' ? $new_els : array_merge($existing, $new_els);
                            $inserted = true;
                            return;
                        }
                        if (!empty($n['elements']) && is_array($n['elements'])) {
                            $walk($n['elements']);
                            if ($inserted) return;
                        }
                    }
                };
                $walk($tree);
                if (!$inserted) return new WP_Error('parent_not_found', "Parent element {$parent_id} not found.", ['status' => 404]);
            }
            update_post_meta($post_id, '_elementor_data', wp_slash(wp_json_encode($tree)));
            update_post_meta($post_id, '_elementor_edit_mode', 'builder');
            mcp_bridge_regenerate_css($post_id);
            return ['post_id' => $post_id, 'id_map' => $body['id_map'] ?? [], 'revision' => mcp_bridge_hash($tree)];
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/v4/resources', [
        'methods' => 'GET',
        'callback' => function() {
            return ['resources' => [
                ['uri' => 'elementor://global-classes', 'name' => 'Global Classes', 'mimeType' => 'application/json'],
                ['uri' => 'elementor://global-variables', 'name' => 'Global Variables', 'mimeType' => 'application/json'],
                ['uri' => 'elementor://dynamic-tags', 'name' => 'Dynamic Tags', 'mimeType' => 'application/json'],
                ['uri' => 'elementor://interactions/schema', 'name' => 'Interactions Schema', 'mimeType' => 'application/json'],
                ['uri' => 'elementor://breakpoints', 'name' => 'Breakpoints', 'mimeType' => 'application/json'],
                ['uri' => 'elementor://wordpress/best-practices', 'name' => 'WordPress Best Practices', 'mimeType' => 'text/markdown'],
            ]];
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/v4/resource', [
        'methods' => 'GET',
        'callback' => function($req) {
            $uri = (string) $req->get_param('uri');
            $kit = mcp_bridge_active_kit();
            if ($uri === 'elementor://global-classes' && !is_wp_error($kit) && class_exists('Elementor\\Modules\\GlobalClasses\\Global_Classes_Repository')) {
                $data = Elementor\Modules\GlobalClasses\Global_Classes_Repository::make($kit)->all(true)->get();
                return ['uri' => $uri, 'mimeType' => 'application/json', 'text' => wp_json_encode($data)];
            }
            if ($uri === 'elementor://global-variables' && !is_wp_error($kit) && class_exists('Elementor\\Modules\\Variables\\Storage\\Variables_Repository')) {
                $data = (new Elementor\Modules\Variables\Storage\Variables_Repository($kit))->load()->serialize(true);
                return ['uri' => $uri, 'mimeType' => 'application/json', 'text' => wp_json_encode($data)];
            }
            if ($uri === 'elementor://dynamic-tags' && class_exists('Elementor\\Plugin') && isset(Elementor\Plugin::instance()->dynamic_tags)) {
                $tags = Elementor\Plugin::instance()->dynamic_tags->get_tags_config();
                return ['uri' => $uri, 'mimeType' => 'application/json', 'text' => wp_json_encode($tags)];
            }
            if ($uri === 'elementor://breakpoints' && class_exists('Elementor\\Plugin') && isset(Elementor\Plugin::instance()->breakpoints)) {
                $bp = Elementor\Plugin::instance()->breakpoints->get_breakpoints_config();
                return ['uri' => $uri, 'mimeType' => 'application/json', 'text' => wp_json_encode($bp)];
            }
            return new WP_Error('resource_not_found', "Resource {$uri} not found or unavailable.", ['status' => 404]);
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/v4/preview-link', [
        'methods' => 'POST',
        'callback' => function($req) {
            $post_id = (int) $req->get_param('post_id');
            if (!$post_id || !get_post($post_id)) return new WP_Error('not_found', 'Post not found.', ['status' => 404]);
            $token = bin2hex(random_bytes(16));
            $expires = time() + 3600;
            set_transient('mcp_preview_' . $token, ['post_id' => $post_id, 'expires' => $expires], 3600);
            $url = add_query_arg(['p' => $post_id, 'mcp_preview' => $token], home_url('/'));
            return ['post_id' => $post_id, 'url' => $url, 'edit_url' => admin_url("post.php?post={$post_id}&action=elementor"), 'expires_at' => gmdate('c', $expires)];
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/v4/publish-document', [
        'methods' => 'POST',
        'callback' => function($req) {
            $post_id = (int) $req->get_param('post_id');
            $post = get_post($post_id);
            if (!$post) return new WP_Error('not_found', 'Document not found.', ['status' => 404]);
            $autosave = wp_get_post_autosave($post_id);
            if ($autosave && strtotime($autosave->post_modified_gmt) >= strtotime($post->post_modified_gmt)) {
                $auto_data = get_post_meta($autosave->ID, '_elementor_data', true);
                if (is_string($auto_data) && $auto_data !== '') update_post_meta($post_id, '_elementor_data', wp_slash($auto_data));
            }
            if ($post->post_status !== 'publish') {
                $res = wp_update_post(['ID' => $post_id, 'post_status' => 'publish'], true);
                if (is_wp_error($res)) return $res;
            }
            mcp_bridge_regenerate_css($post_id);
            return ['post_id' => $post_id, 'status' => 'publish', 'permalink' => get_permalink($post_id)];
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/v4/site-parts', [
        ['methods' => 'GET', 'callback' => function($req) {
            $active_only = filter_var($req->get_param('active'), FILTER_VALIDATE_BOOLEAN);
            $posts = get_posts(['post_type' => 'elementor_library', 'post_status' => ['publish', 'draft', 'private'], 'posts_per_page' => 200]);
            $rows = [];
            foreach ($posts as $p) {
                $type = get_post_meta($p->ID, '_elementor_template_type', true);
                if (!$type || $type === 'component') continue;
                $conditions = get_post_meta($p->ID, '_elementor_conditions', true);
                $conditions = is_array($conditions) ? $conditions : [];
                $has_conditions = !empty($conditions);
                $is_active = $p->post_status === 'publish' && $has_conditions;
                if ($active_only && !$is_active) continue;
                $rows[] = ['post_id' => (int) $p->ID, 'title' => $p->post_title, 'type' => $type, 'post_status' => $p->post_status, 'conditions' => $conditions, 'has_conditions' => $has_conditions, 'is_active' => $is_active];
            }
            return ['site_parts' => $rows];
        }, 'permission_callback' => $admin],
        ['methods' => 'POST', 'callback' => function($req) {
            $ops = $req->get_param('operations');
            if (!is_array($ops)) return new WP_Error('invalid_request', 'operations array required', ['status' => 400]);
            $results = [];
            foreach ($ops as $op) {
                $action = $op['action'] ?? '';
                if ($action === 'create') {
                    $type = sanitize_key((string) ($op['type'] ?? 'section'));
                    $title = sanitize_text_field((string) ($op['title'] ?? ucfirst($type)));
                    $post_id = wp_insert_post(['post_type' => 'elementor_library', 'post_status' => 'draft', 'post_title' => $title], true);
                    if (is_wp_error($post_id)) return $post_id;
                    update_post_meta($post_id, '_elementor_template_type', $type);
                    update_post_meta($post_id, '_elementor_edit_mode', 'builder');
                    if (array_key_exists('conditions', $op)) {
                        $conds = is_array($op['conditions']) && !empty($op['conditions']) ? $op['conditions'] : ['include/general'];
                        update_post_meta($post_id, '_elementor_conditions', $conds);
                    }
                    $results[] = ['action' => 'create', 'post_id' => $post_id, 'status' => 'draft'];
                } elseif ($action === 'update') {
                    $pid = (int) ($op['post_id'] ?? 0);
                    if (!$pid || !get_post($pid)) return new WP_Error('not_found', 'Site part not found.', ['status' => 404]);
                    if (isset($op['conditions']) && is_array($op['conditions'])) update_post_meta($pid, '_elementor_conditions', $op['conditions']);
                    $results[] = ['action' => 'update', 'post_id' => $pid];
                } elseif ($action === 'delete') {
                    $pid = (int) ($op['post_id'] ?? 0);
                    wp_trash_post($pid);
                    $results[] = ['action' => 'delete', 'post_id' => $pid];
                }
            }
            return ['results' => $results];
        }, 'permission_callback' => $admin],
    ]);
});

add_filter('user_has_cap', function($allcaps, $caps, $args) {
    if (!empty($_GET['mcp_preview'])) {
        $token = sanitize_text_field(wp_unslash($_GET['mcp_preview']));
        $data = get_transient('mcp_preview_' . $token);
        if (is_array($data) && ($data['expires'] ?? 0) >= time()) {
            $post_id = (int) ($data['post_id'] ?? 0);
            if ($post_id > 0 && isset($args[2]) && (int) $args[2] === $post_id) {
                foreach ($caps as $cap) $allcaps[$cap] = true;
            }
        }
    }
    return $allcaps;
}, 10, 3);
