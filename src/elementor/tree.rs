use super::ops::{
    find_by_id, flatten_tree, insert_after_id, insert_at, mutate_by_id, parse_data, remove_by_id,
    search, serialize_data,
};
use crate::types::Element;

/// Owned tree of Elementor elements with traversal and mutation methods.
pub struct ElementTree {
    pub(crate) elements: Vec<Element>,
    pub(crate) revision: Option<String>,
}

impl ElementTree {
    #[allow(dead_code)]
    pub fn parse(raw: &str) -> anyhow::Result<Self> {
        Ok(Self {
            elements: parse_data(raw)?,
            revision: None,
        })
    }

    #[allow(dead_code)]
    pub fn serialize(&self) -> anyhow::Result<String> {
        serialize_data(&self.elements)
    }

    #[allow(dead_code)]
    pub fn as_slice(&self) -> &[Element] {
        &self.elements
    }

    pub fn find(&self, id: &str) -> Option<Element> {
        find_by_id(&self.elements, id)
    }

    pub fn mutate(&mut self, id: &str, f: impl Fn(&mut Element)) -> bool {
        mutate_by_id(&mut self.elements, id, &f)
    }

    pub fn remove(&mut self, id: &str) -> Option<Element> {
        remove_by_id(&mut self.elements, id)
    }

    pub fn insert(&mut self, parent_id: Option<&str>, position: usize, el: Element) -> bool {
        insert_at(&mut self.elements, parent_id, position, el)
    }

    pub fn insert_after(&mut self, after_id: &str, el: Element) -> bool {
        insert_after_id(&mut self.elements, after_id, el)
    }

    pub fn search(
        &self,
        widget_type: Option<&str>,
        setting_key: Option<&str>,
        setting_value: Option<&str>,
    ) -> Vec<Element> {
        search(&self.elements, widget_type, setting_key, setting_value)
    }

    pub fn flatten(&self) -> Vec<(String, String)> {
        flatten_tree(&self.elements, "")
    }
}
