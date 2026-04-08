use std::collections::HashMap;
use std::sync::OnceLock;

use super::widget_schema::{WidgetSchema, all_schemas, COMMON_SETTINGS, TYPOGRAPHY_KEYS, COMMON_ALIASES};

static REGISTRY: OnceLock<SchemaRegistry> = OnceLock::new();

/// Singleton registry for widget schemas. Initialised once on first access.
pub struct SchemaRegistry {
    schemas: &'static Vec<WidgetSchema>,
    by_type: HashMap<&'static str, &'static WidgetSchema>,
}

impl SchemaRegistry {
    pub fn global() -> &'static Self {
        REGISTRY.get_or_init(|| {
            let schemas: &'static Vec<WidgetSchema> = Box::leak(Box::new(all_schemas()));
            let by_type = schemas.iter().map(|s| (s.widget_type, s)).collect();
            Self { schemas, by_type }
        })
    }

    pub fn get(&self, widget_type: &str) -> Option<&'static WidgetSchema> {
        self.by_type.get(widget_type).copied()
    }

    pub fn all(&self) -> &[WidgetSchema] {
        self.schemas
    }

    #[allow(clippy::unused_self)]
    pub fn valid_keys(&self, schema: &WidgetSchema) -> Vec<&str> {
        let mut keys: Vec<&str> = Vec::new();
        keys.extend_from_slice(schema.settings);
        keys.extend_from_slice(COMMON_SETTINGS);
        #[allow(clippy::items_after_statements)]
        const TEXT_WIDGETS: &[&str] = &[
            "heading", "text-editor", "button", "icon-box", "image-box",
            "counter", "progress-bar", "testimonial", "alert", "star-rating", "icon-list",
            "animated-headline", "blockquote", "call-to-action", "flip-box",
            "price-table", "table-of-contents", "form",
        ];
        if TEXT_WIDGETS.contains(&schema.widget_type) {
            keys.extend_from_slice(TYPOGRAPHY_KEYS);
        }
        keys
    }

    pub fn suggest_fix(&self, key: &str, schema: &WidgetSchema) -> Option<String> {
        for (wrong, right) in schema.aliases {
            if *wrong == key {
                return Some(right.to_string());
            }
        }
        for (wrong, right) in COMMON_ALIASES {
            if *wrong == key {
                return Some(right.to_string());
            }
        }
        let valid = self.valid_keys(schema);
        valid.iter().find(|v| v.contains(key) || key.contains(*v)).map(std::string::ToString::to_string)
    }
}
