mod widget_schema;
mod registry;
mod list_widgets;
mod get_widget_schema;
mod validate_element;

pub use widget_schema::WidgetSchema;
pub use registry::SchemaRegistry;
pub use list_widgets::ListWidgets;
pub use get_widget_schema::GetWidgetSchema;
pub use validate_element::ValidateElement;

use std::collections::HashMap;

#[allow(dead_code)]
pub fn build_schema_map() -> HashMap<&'static str, &'static WidgetSchema> {
    let reg = SchemaRegistry::global();
    reg.all().iter().map(|s| (s.widget_type, s)).collect()
}

#[allow(dead_code)]
pub fn all_valid_keys(schema: &WidgetSchema) -> Vec<&str> {
    SchemaRegistry::global().valid_keys(schema)
}

#[allow(dead_code)]
pub fn suggest_fix(key: &str, schema: &WidgetSchema) -> Option<String> {
    SchemaRegistry::global().suggest_fix(key, schema)
}
