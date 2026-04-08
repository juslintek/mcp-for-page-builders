mod create_template;
mod update_template;
mod list_templates;
mod get_template;
mod delete_template;

pub use create_template::CreateTemplate;
pub use update_template::UpdateTemplate;
pub use list_templates::ListTemplates;
pub use get_template::GetTemplate;
pub use delete_template::DeleteTemplate;

use serde_json::Value;

pub(crate) fn parse_conditions(args: &Value) -> Option<Vec<String>> {
    args.get("conditions")?.as_array().map(|arr| {
        arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()
    })
}
