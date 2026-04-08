use crate::types::Element;
use super::ops::{parse_data, serialize_data, find_by_id, mutate_by_id, remove_by_id, insert_at, insert_after_id, search, flatten_tree};

/// Owned tree of Elementor elements with traversal and mutation methods.
pub struct ElementTree(pub(crate) Vec<Element>);

impl ElementTree {
    #[allow(dead_code)]
    pub fn parse(raw: &str) -> anyhow::Result<Self> {
        Ok(Self(parse_data(raw)?))
    }

    #[allow(dead_code)]
    pub fn serialize(&self) -> anyhow::Result<String> {
        serialize_data(&self.0)
    }

    #[allow(dead_code)]
    pub fn as_slice(&self) -> &[Element] {
        &self.0
    }

    pub fn find(&self, id: &str) -> Option<Element> {
        find_by_id(&self.0, id)
    }

    pub fn mutate(&mut self, id: &str, f: impl Fn(&mut Element)) -> bool {
        mutate_by_id(&mut self.0, id, &f)
    }

    pub fn remove(&mut self, id: &str) -> Option<Element> {
        remove_by_id(&mut self.0, id)
    }

    pub fn insert(&mut self, parent_id: Option<&str>, position: usize, el: Element) -> bool {
        insert_at(&mut self.0, parent_id, position, el)
    }

    pub fn insert_after(&mut self, after_id: &str, el: Element) -> bool {
        insert_after_id(&mut self.0, after_id, el)
    }

    pub fn search(
        &self,
        widget_type: Option<&str>,
        setting_key: Option<&str>,
        setting_value: Option<&str>,
    ) -> Vec<Element> {
        search(&self.0, widget_type, setting_key, setting_value)
    }

    pub fn flatten(&self) -> Vec<(String, String)> {
        flatten_tree(&self.0, "")
    }
}
