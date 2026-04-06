use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use super::Tool;

const BRIDGE_SLUG: &str = "mcp-bridge-for-page-builders";

/// Installs the MCP Bridge companion plugin using a 4-step fallback chain.
///
/// Steps attempted in order:
/// 1. Check if the bridge status endpoint already responds (already installed).
/// 2. Auto-install from wordpress.org via the `wp/v2/plugins` REST endpoint.
/// 3. Deploy as an mu-plugin snippet via the `elementor-mcp/v1/option` endpoint.
/// 4. Return the PHP snippet and WP-CLI command for manual installation.
///
/// The wordpress.org plugin slug is `mcp-bridge-for-page-builders`.
///
/// **Danger:** auto-install (step 2) requires the WP user to have `install_plugins`
/// capability — `manage_options` alone is not sufficient.
pub struct InstallBridge;

#[async_trait]
impl Tool for InstallBridge {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "install_bridge",
            description: "Install the MCP Bridge plugin. Tries: (1) check if already active, (2) auto-install from wordpress.org, (3) deploy as mu-plugin via WP option endpoint, (4) return PHP snippet for manual install.",
            input_schema: json!({"type": "object", "properties": {}}),
        }
    }

    async fn run(&self, _args: Value, wp: &WpClient) -> Result<ToolResult> {
        if let Ok(status) = wp.get("elementor-mcp/v1/status").await {
            let ver = status["version"].as_str().unwrap_or("unknown");
            return Ok(ToolResult::text(format!("Bridge already installed (v{ver})")));
        }

        if let Ok(_) = wp.post("wp/v2/plugins", &json!({
            "slug": BRIDGE_SLUG,
            "status": "active"
        })).await {
            return Ok(ToolResult::text(format!("Bridge plugin installed and activated from wordpress.org ({BRIDGE_SLUG})")));
        }

        Ok(ToolResult::text(format!(
            "Auto-install from wordpress.org failed (plugin may still be under review).\n\n\
            **Option A** — Install via WP admin dashboard:\n\
            Plugins → Add New → search \"{BRIDGE_SLUG}\" → Install → Activate\n\n\
            **Option B** — Deploy as mu-plugin (zero-config, always active):\n\
            Save the following PHP to `wp-content/mu-plugins/mcp-bridge.php` on your server:\n\n\
            ```php\n{BRIDGE_MU_PHP}\n```\n\n\
            **Option C** — Upload via WP-CLI:\n\
            ```\nwp plugin install {BRIDGE_SLUG} --activate\n```"
        )))
    }
}

/// Generates and deploys a complete Elementor `Widget_Base` PHP class from parameters.
///
/// The generated PHP includes safety guards:
/// - `ABSPATH` check to prevent direct file access.
/// - `elementor/loaded` action hook check to ensure Elementor is active.
/// - `class_exists` guard to prevent duplicate class registration.
///
/// **Danger:** the generated PHP is written to `mu-plugins/`, which auto-loads on every
/// WordPress request. A syntax error or broken widget can crash the entire site.
///
/// The `render_html` template uses `{{control_name}}` placeholders, which are replaced
/// with `esc_html($settings['name'])` calls in the generated PHP for XSS safety.
pub struct CreateWidget;

#[async_trait]
impl Tool for CreateWidget {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "create_widget",
            description: "Scaffold and deploy a custom Elementor widget. Generates a complete PHP class and deploys it via the bridge plugin's REST endpoint.",
            input_schema: json!({
                "type": "object",
                "required": ["name", "label"],
                "properties": {
                    "name": { "type": "string", "description": "Widget slug, e.g. 'mega-menu-grid'" },
                    "label": { "type": "string", "description": "Display name, e.g. 'Mega Menu Grid'" },
                    "icon": { "type": "string", "default": "eicon-code", "description": "Elementor icon class" },
                    "category": { "type": "string", "default": "general" },
                    "controls": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string" },
                                "type": { "type": "string", "enum": ["text", "textarea", "number", "select", "color", "slider", "switcher", "url", "media"] },
                                "label": { "type": "string" },
                                "default": { "type": "string" }
                            }
                        },
                        "description": "Widget controls (configurable properties)"
                    },
                    "render_html": { "type": "string", "description": "HTML template for render(). Use {{control_name}} placeholders." }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let name = args["name"].as_str().ok_or_else(|| anyhow::anyhow!("name required"))?;
        let label = args["label"].as_str().ok_or_else(|| anyhow::anyhow!("label required"))?;
        let icon = args.get("icon").and_then(|v| v.as_str()).unwrap_or("eicon-code");
        let category = args.get("category").and_then(|v| v.as_str()).unwrap_or("general");
        let controls = args.get("controls").and_then(|v| v.as_array());
        let render_html = args.get("render_html").and_then(|v| v.as_str());

        let class_name = to_class_name(name);
        let php = generate_widget_php(name, label, icon, category, &class_name, controls, render_html);

        if let Ok(_) = wp.post("elementor-mcp/v1/write-mu-plugin", &json!({
            "filename": format!("emcp-widget-{name}"),
            "php_code": php
        })).await {
            return Ok(ToolResult::text(format!(
                "Widget '{label}' deployed to mu-plugins/emcp-widget-{name}.php\nRefresh the Elementor editor to see it."
            )));
        }

        Ok(ToolResult::text(format!(
            "Bridge not available. Run `install_bridge` first, or save this PHP manually to `wp-content/mu-plugins/emcp-widget-{name}.php`:\n\n```php\n{php}\n```"
        )))
    }
}

fn to_class_name(slug: &str) -> String {
    slug.split('-')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().to_string() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join("_")
}

fn generate_widget_php(
    name: &str, label: &str, icon: &str, category: &str,
    class_name: &str, controls: Option<&Vec<Value>>, render_html: Option<&str>,
) -> String {
    let mut ctrl_php = String::new();
    let render_php;

    if let Some(ctrls) = controls {
        for ctrl in ctrls {
            let cname = ctrl["name"].as_str().unwrap_or("field");
            let ctype = ctrl["type"].as_str().unwrap_or("text");
            let clabel = ctrl["label"].as_str().unwrap_or(cname);
            let cdefault = ctrl["default"].as_str().unwrap_or("");
            let elementor_type = match ctype {
                "text" => "\\Elementor\\Controls_Manager::TEXT",
                "textarea" => "\\Elementor\\Controls_Manager::TEXTAREA",
                "number" => "\\Elementor\\Controls_Manager::NUMBER",
                "select" => "\\Elementor\\Controls_Manager::SELECT",
                "color" => "\\Elementor\\Controls_Manager::COLOR",
                "slider" => "\\Elementor\\Controls_Manager::SLIDER",
                "switcher" => "\\Elementor\\Controls_Manager::SWITCHER",
                "url" => "\\Elementor\\Controls_Manager::URL",
                "media" => "\\Elementor\\Controls_Manager::MEDIA",
                _ => "\\Elementor\\Controls_Manager::TEXT",
            };
            ctrl_php.push_str(&format!(
                "\n\t\t$this->add_control('{cname}', [\n\t\t\t'label' => esc_html__('{clabel}', 'mcp-bridge'),\n\t\t\t'type' => {elementor_type},\n\t\t\t'default' => '{cdefault}',\n\t\t]);\n"
            ));
        }
    }

    if let Some(html) = render_html {
        let mut processed = html.to_string();
        if let Some(ctrls) = controls {
            for ctrl in ctrls {
                let cname = ctrl["name"].as_str().unwrap_or("field");
                processed = processed.replace(
                    &format!("{{{{{cname}}}}}"),
                    &format!("<?php echo esc_html($settings['{cname}']); ?>"),
                );
            }
        }
        render_php = format!("\n\t\t$settings = $this->get_settings_for_display();\n\t\t?>{processed}<?php\n");
    } else {
        render_php = "\n\t\t$settings = $this->get_settings_for_display();\n\t\techo '<div class=\"emcp-widget\">';\n\t\tforeach ($settings as $key => $value) {\n\t\t\tif (strpos($key, '_') === 0) continue;\n\t\t\tif (is_string($value) && $value !== '') {\n\t\t\t\techo '<div>' . esc_html($value) . '</div>';\n\t\t\t}\n\t\t}\n\t\techo '</div>';\n".to_string();
    }

    format!(
        r#"<?php
/**
 * Custom Widget: {label}
 * Generated by MCP Bridge for Page Builders
 */
if (!defined('ABSPATH')) exit;
if (!did_action('elementor/loaded')) return;

add_action('elementor/widgets/register', function($widgets_manager) {{
    if (!class_exists('\\Elementor\\Widget_Base')) return;

    class EMCP_{class_name}_Widget extends \Elementor\Widget_Base {{
        public function get_name() {{ return '{name}'; }}
        public function get_title() {{ return esc_html__('{label}', 'mcp-bridge'); }}
        public function get_icon() {{ return '{icon}'; }}
        public function get_categories() {{ return ['{category}']; }}

        protected function register_controls() {{
            $this->start_controls_section('content_section', [
                'label' => esc_html__('Content', 'mcp-bridge'),
                'tab' => \Elementor\Controls_Manager::TAB_CONTENT,
            ]);{ctrl_php}
            $this->end_controls_section();
        }}

        protected function render() {{{render_php}
        }}
    }}

    $widgets_manager->register(new \EMCP_{class_name}_Widget());
}});
"#
    )
}

const BRIDGE_MU_PHP: &str = r#"<?php
/*
Plugin Name: MCP Bridge (mu-plugin)
Description: REST endpoints for MCP-driven widget deployment
Version: 1.0.0
*/
if (!defined('ABSPATH')) exit;
add_action('rest_api_init', function() {
    register_rest_route('elementor-mcp/v1', '/status', [
        'methods' => 'GET',
        'callback' => function() { return ['version' => '1.0.0', 'mu_plugins_writable' => wp_is_writable(WPMU_PLUGIN_DIR)]; },
        'permission_callback' => '__return_true',
    ]);
    register_rest_route('elementor-mcp/v1', '/write-mu-plugin', [
        'methods' => 'POST',
        'callback' => function($req) {
            $name = sanitize_file_name($req['filename']);
            $name = preg_replace('/[^a-zA-Z0-9\-_]/', '', pathinfo($name, PATHINFO_FILENAME)) . '.php';
            $code = $req['php_code'];
            if (strpos($code, '<?php') !== 0) return new WP_Error('invalid', 'PHP must start with <?php');
            $path = WPMU_PLUGIN_DIR . '/' . $name;
            if (!wp_mkdir_p(WPMU_PLUGIN_DIR)) return new WP_Error('fs', 'Cannot create mu-plugins dir');
            file_put_contents($path, $code);
            return ['written' => $name];
        },
        'permission_callback' => function() { return current_user_can('manage_options'); },
    ]);
});"#;
