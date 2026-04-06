use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::{str_arg, u64_arg, usize_arg};
use crate::elementor::{Element, ElementorService};
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use super::Tool;

// ── GetElement ────────────────────────────────────────────────────────────────

pub struct GetElement;

#[async_trait]
impl Tool for GetElement {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_element",
            description: "Get a single Elementor element by ID from a page's element tree.",
            input_schema: json!({
                "type": "object",
                "required": ["page_id", "element_id"],
                "properties": {
                    "page_id": { "type": "integer" },
                    "element_id": { "type": "string", "description": "8-char hex element ID" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let page_id = u64_arg(&args, "page_id").ok_or_else(|| anyhow::anyhow!("page_id required"))?;
        let eid = str_arg(&args, "element_id").ok_or_else(|| anyhow::anyhow!("element_id required"))?;

        let tree = ElementorService::new(wp).get_tree(page_id).await?;
        match tree.find(&eid) {
            Some(el) => Ok(ToolResult::text(serde_json::to_string_pretty(&el)?)),
            None => Ok(ToolResult::error(format!("Element {eid} not found on page {page_id}"))),
        }
    }
}

// ── AddElement ────────────────────────────────────────────────────────────────

pub struct AddElement;

#[async_trait]
impl Tool for AddElement {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "add_element",
            description: "Insert a new element (widget/container) into a page at a specific position. Provide the element as JSON.",
            input_schema: json!({
                "type": "object",
                "required": ["page_id", "element"],
                "properties": {
                    "page_id": { "type": "integer" },
                    "parent_id": { "type": "string", "description": "Parent element ID. Omit for root level." },
                    "position": { "type": "integer", "description": "Index to insert at (0-based). Defaults to end." },
                    "element": { "type": "object", "description": "Full element JSON with elType, settings, etc. ID auto-generated if missing." }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let page_id = u64_arg(&args, "page_id").ok_or_else(|| anyhow::anyhow!("page_id required"))?;
        let parent_id = str_arg(&args, "parent_id");
        let position = usize_arg(&args, "position").unwrap_or(usize::MAX);

        let el_json = args.get("element").ok_or_else(|| anyhow::anyhow!("element required"))?;
        let mut new_el: Element = serde_json::from_value(el_json.clone())?;
        if new_el.id.is_empty() {
            new_el.id = crate::elementor::generate_id();
        }
        let new_id = new_el.id.clone();

        let svc = ElementorService::new(wp);
        let mut tree = svc.get_tree(page_id).await?;
        if !tree.insert(parent_id.as_deref(), position, new_el) {
            return Ok(ToolResult::error(format!("Parent {} not found", parent_id.unwrap_or_default())));
        }
        svc.save_tree(page_id, &tree).await?;

        Ok(ToolResult::text(format!("Added element {new_id} to page {page_id}")))
    }
}

// ── UpdateElement ─────────────────────────────────────────────────────────────

pub struct UpdateElement;

#[async_trait]
impl Tool for UpdateElement {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "update_element",
            description: "Update an element's settings by ID. Merges provided settings with existing ones (partial update).",
            input_schema: json!({
                "type": "object",
                "required": ["page_id", "element_id", "settings"],
                "properties": {
                    "page_id": { "type": "integer" },
                    "element_id": { "type": "string" },
                    "settings": { "type": "object", "description": "Settings to merge into the element" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let page_id = u64_arg(&args, "page_id").ok_or_else(|| anyhow::anyhow!("page_id required"))?;
        let eid = str_arg(&args, "element_id").ok_or_else(|| anyhow::anyhow!("element_id required"))?;
        let new_settings = args.get("settings").ok_or_else(|| anyhow::anyhow!("settings required"))?.clone();

        let svc = ElementorService::new(wp);
        let mut tree = svc.get_tree(page_id).await?;
        let found = tree.mutate(&eid, |el| el.merge_settings(&new_settings));

        if !found {
            return Ok(ToolResult::error(format!("Element {eid} not found on page {page_id}")));
        }

        svc.save_tree(page_id, &tree).await?;
        Ok(ToolResult::text(format!("Updated settings for element {eid} on page {page_id}")))
    }
}

// ── RemoveElement ─────────────────────────────────────────────────────────────

pub struct RemoveElement;

#[async_trait]
impl Tool for RemoveElement {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "remove_element",
            description: "Remove an element by ID from a page's element tree.",
            input_schema: json!({
                "type": "object",
                "required": ["page_id", "element_id"],
                "properties": {
                    "page_id": { "type": "integer" },
                    "element_id": { "type": "string" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let page_id = u64_arg(&args, "page_id").ok_or_else(|| anyhow::anyhow!("page_id required"))?;
        let eid = str_arg(&args, "element_id").ok_or_else(|| anyhow::anyhow!("element_id required"))?;

        let svc = ElementorService::new(wp);
        let mut tree = svc.get_tree(page_id).await?;
        match tree.remove(&eid) {
            Some(_) => {
                svc.save_tree(page_id, &tree).await?;
                Ok(ToolResult::text(format!("Removed element {eid} from page {page_id}")))
            }
            None => Ok(ToolResult::error(format!("Element {eid} not found on page {page_id}"))),
        }
    }
}

// ── MoveElement ───────────────────────────────────────────────────────────────

pub struct MoveElement;

#[async_trait]
impl Tool for MoveElement {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "move_element",
            description: "Move an element to a different parent and/or position within the page.",
            input_schema: json!({
                "type": "object",
                "required": ["page_id", "element_id"],
                "properties": {
                    "page_id": { "type": "integer" },
                    "element_id": { "type": "string", "description": "Element to move" },
                    "target_parent_id": { "type": "string", "description": "New parent ID. Omit for root level." },
                    "position": { "type": "integer", "description": "Index in new parent. Defaults to end." }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let page_id = u64_arg(&args, "page_id").ok_or_else(|| anyhow::anyhow!("page_id required"))?;
        let eid = str_arg(&args, "element_id").ok_or_else(|| anyhow::anyhow!("element_id required"))?;
        let target = str_arg(&args, "target_parent_id");
        let position = usize_arg(&args, "position").unwrap_or(usize::MAX);

        let svc = ElementorService::new(wp);
        let mut tree = svc.get_tree(page_id).await?;

        let el = tree.remove(&eid)
            .ok_or_else(|| anyhow::anyhow!("Element {eid} not found"))?;

        if !tree.insert(target.as_deref(), position, el) {
            return Ok(ToolResult::error(format!("Target parent {} not found", target.unwrap_or_default())));
        }

        svc.save_tree(page_id, &tree).await?;
        Ok(ToolResult::text(format!("Moved element {eid} on page {page_id}")))
    }
}

// ── DuplicateElement ──────────────────────────────────────────────────────────

pub struct DuplicateElement;

#[async_trait]
impl Tool for DuplicateElement {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "duplicate_element",
            description: "Clone an element (and all children) with new IDs, inserted right after the original.",
            input_schema: json!({
                "type": "object",
                "required": ["page_id", "element_id"],
                "properties": {
                    "page_id": { "type": "integer" },
                    "element_id": { "type": "string" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let page_id = u64_arg(&args, "page_id").ok_or_else(|| anyhow::anyhow!("page_id required"))?;
        let eid = str_arg(&args, "element_id").ok_or_else(|| anyhow::anyhow!("element_id required"))?;

        let svc = ElementorService::new(wp);
        let mut tree = svc.get_tree(page_id).await?;

        let mut clone = tree.find(&eid)
            .ok_or_else(|| anyhow::anyhow!("Element {eid} not found"))?;
        clone.regenerate_ids();
        let clone_id = clone.id.clone();

        if !tree.insert_after(&eid, clone) {
            return Ok(ToolResult::error(format!("Could not insert clone after {eid}")));
        }

        svc.save_tree(page_id, &tree).await?;
        Ok(ToolResult::text(format!("Duplicated {eid} → {clone_id} on page {page_id}")))
    }
}

// ── FindElements ──────────────────────────────────────────────────────────────

pub struct FindElements;

#[async_trait]
impl Tool for FindElements {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "find_elements",
            description: "Search for elements by widget type and/or setting key/value.",
            input_schema: json!({
                "type": "object",
                "required": ["page_id"],
                "properties": {
                    "page_id": { "type": "integer" },
                    "widget_type": { "type": "string", "description": "Filter by widget type (e.g. 'heading', 'text-editor')" },
                    "setting_key": { "type": "string", "description": "Filter by setting key existence or value" },
                    "setting_value": { "type": "string", "description": "Required value for setting_key" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let page_id = u64_arg(&args, "page_id").ok_or_else(|| anyhow::anyhow!("page_id required"))?;
        let wt = str_arg(&args, "widget_type");
        let sk = str_arg(&args, "setting_key");
        let sv = str_arg(&args, "setting_value");

        let tree = ElementorService::new(wp).get_tree(page_id).await?;
        let results = tree.search(wt.as_deref(), sk.as_deref(), sv.as_deref());

        if results.is_empty() {
            return Ok(ToolResult::text("No matching elements found."));
        }

        let mut lines = vec![format!("Found {} elements:", results.len())];
        for el in &results {
            let wt = el.widget_type.as_deref().unwrap_or("-");
            lines.push(format!("  {} ({}) id={}", el.el_type, wt, el.id));
        }
        Ok(ToolResult::text(lines.join("\n")))
    }
}

// ── GetElementTree ────────────────────────────────────────────────────────────

pub struct GetElementTree;

#[async_trait]
impl Tool for GetElementTree {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_element_tree",
            description: "Get a flattened view of a page's element tree showing paths, types, and IDs.",
            input_schema: json!({
                "type": "object",
                "required": ["page_id"],
                "properties": {
                    "page_id": { "type": "integer" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let page_id = u64_arg(&args, "page_id").ok_or_else(|| anyhow::anyhow!("page_id required"))?;

        let tree = ElementorService::new(wp).get_tree(page_id).await?;
        let flat = tree.flatten();

        if flat.is_empty() {
            return Ok(ToolResult::text("Page has no Elementor elements."));
        }

        let lines: Vec<String> = flat.iter().map(|(path, label)| format!("{path}  {label}")).collect();
        Ok(ToolResult::text(lines.join("\n")))
    }
}
