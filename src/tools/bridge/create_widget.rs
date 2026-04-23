use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct CreateWidget;

#[async_trait]
impl Tool for CreateWidget {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "create_widget",
            description: "Scaffold and deploy a custom Elementor widget. Generates a complete PHP class and deploys it via the bridge plugin's REST endpoint.",
            input_schema: json!({
                "type": "object", "required": ["name", "label"],
                "properties": {
                    "name": { "type": "string", "description": "Widget slug, e.g. 'mega-menu-grid'" },
                    "label": { "type": "string", "description": "Display name" },
                    "icon": { "type": "string", "default": "eicon-code" },
                    "category": { "type": "string", "default": "general" },
                    "controls": { "type": "array", "items": { "type": "object", "properties": { "name": {"type":"string"}, "type": {"type":"string"}, "label": {"type":"string"}, "default": {"type":"string"} } } },
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

        if wp.post("mcp-for-page-builders/v1/write-mu-plugin", &json!({"filename": format!("emcp-widget-{name}"), "php_code": php})).await.is_ok() {
            return Ok(ToolResult::text(format!("Widget '{label}' deployed to mu-plugins/emcp-widget-{name}.php\nRefresh the Elementor editor to see it.")));
        }

        Ok(ToolResult::text(format!("Bridge not available. Run `install_bridge` first, or save this PHP manually to `wp-content/mu-plugins/emcp-widget-{name}.php`:\n\n```php\n{php}\n```")))
    }
}

fn to_class_name(slug: &str) -> String {
    slug.split('-').map(|w| { let mut c = w.chars(); match c.next() { None => String::new(), Some(f) => f.to_uppercase().to_string() + c.as_str() } }).collect::<Vec<_>>().join("_")
}

#[allow(clippy::match_same_arms)]
fn generate_widget_php(name: &str, label: &str, icon: &str, category: &str, class_name: &str, controls: Option<&Vec<Value>>, render_html: Option<&str>) -> String {
    let mut ctrl_php = String::new();
    if let Some(ctrls) = controls {
        for ctrl in ctrls {
            let cname = ctrl["name"].as_str().unwrap_or("field");
            let ctype = ctrl["type"].as_str().unwrap_or("text");
            let clabel = ctrl["label"].as_str().unwrap_or(cname);
            let cdefault = ctrl["default"].as_str().unwrap_or("");
            let elementor_type = match ctype {
                "text" => "\\Elementor\\Controls_Manager::TEXT", "textarea" => "\\Elementor\\Controls_Manager::TEXTAREA",
                "number" => "\\Elementor\\Controls_Manager::NUMBER", "select" => "\\Elementor\\Controls_Manager::SELECT",
                "color" => "\\Elementor\\Controls_Manager::COLOR", "slider" => "\\Elementor\\Controls_Manager::SLIDER",
                "switcher" => "\\Elementor\\Controls_Manager::SWITCHER", "url" => "\\Elementor\\Controls_Manager::URL",
                "media" => "\\Elementor\\Controls_Manager::MEDIA", _ => "\\Elementor\\Controls_Manager::TEXT",
            };
            #[allow(clippy::format_push_string)]
            ctrl_php.push_str(&format!("\n\t\t$this->add_control('{cname}', [\n\t\t\t'label' => esc_html__('{clabel}', 'mcp-bridge'),\n\t\t\t'type' => {elementor_type},\n\t\t\t'default' => '{cdefault}',\n\t\t]);\n"));
        }
    }
    let render_php = if let Some(html) = render_html {
        let mut processed = html.to_string();
        if let Some(ctrls) = controls {
            for ctrl in ctrls {
                let cname = ctrl["name"].as_str().unwrap_or("field");
                processed = processed.replace(&format!("{{{{{cname}}}}}"), &format!("<?php echo esc_html($settings['{cname}']); ?>"));
            }
        }
        format!("\n\t\t$settings = $this->get_settings_for_display();\n\t\t?>{processed}<?php\n")
    } else {
        "\n\t\t$settings = $this->get_settings_for_display();\n\t\techo '<div class=\"emcp-widget\">';\n\t\tforeach ($settings as $key => $value) {\n\t\t\tif (strpos($key, '_') === 0) continue;\n\t\t\tif (is_string($value) && $value !== '') {\n\t\t\t\techo '<div>' . esc_html($value) . '</div>';\n\t\t\t}\n\t\t}\n\t\techo '</div>';\n".to_string()
    };
    format!(r"<?php
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
")
}
