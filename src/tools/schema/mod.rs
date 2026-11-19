mod get_widget_schema;
mod list_widgets;
mod registry;
mod validate_element;
mod widget_schema;

pub use get_widget_schema::GetWidgetSchema;
pub use list_widgets::ListWidgets;
pub use registry::SchemaRegistry;
pub use validate_element::ValidateElement;
pub use widget_schema::WidgetSchema;

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
