pub mod page;
pub mod cache;
pub mod file_io;
pub mod element;
pub mod global;
pub mod settings;
pub mod visual;
pub mod schema;
pub mod seed;
pub mod post;
pub mod auth;
pub mod template;
pub mod option;
pub mod inspect;
pub mod css_map;
pub mod editor;
pub mod clone;
pub mod bridge;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;

#[async_trait]
pub trait Tool: Send + Sync {
    fn def(&self) -> ToolDef;
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult>;
}

pub fn all_tools() -> Vec<Box<dyn Tool>> {
    vec![
        // Page CRUD (6)
        Box::new(page::CreatePage),
        Box::new(page::GetPage),
        Box::new(page::UpdatePage),
        Box::new(page::DeletePage),
        Box::new(page::GetPageBySlug),
        Box::new(page::ListPages),
        // Post CRUD (5)
        Box::new(post::CreatePost),
        Box::new(post::GetPost),
        Box::new(post::ListPosts),
        Box::new(post::UpdatePost),
        Box::new(post::DeletePost),
        // Cache (1)
        Box::new(cache::ClearCache),
        // File I/O (3)
        Box::new(file_io::DownloadPage),
        Box::new(file_io::UploadPage),
        Box::new(file_io::BackupPage),
        // Element operations (8)
        Box::new(element::GetElement),
        Box::new(element::AddElement),
        Box::new(element::UpdateElement),
        Box::new(element::RemoveElement),
        Box::new(element::MoveElement),
        Box::new(element::DuplicateElement),
        Box::new(element::FindElements),
        Box::new(element::GetElementTree),
        // Global design tokens (6)
        Box::new(global::GetGlobalColors),
        Box::new(global::SetGlobalColor),
        Box::new(global::DeleteGlobalColor),
        Box::new(global::GetGlobalTypography),
        Box::new(global::SetGlobalTypography),
        Box::new(global::DeleteGlobalTypography),
        // Elementor settings & kit (4)
        Box::new(settings::GetKitSchema),
        Box::new(settings::GetKitDefaults),
        Box::new(settings::GetExperiments),
        Box::new(settings::SetExperiment),
        // Visual + CDP (5)
        Box::new(visual::Screenshot),
        Box::new(visual::ScreenshotPage),
        Box::new(visual::VisualCompare),
        Box::new(visual::ExtractStyles),
        Box::new(visual::VisualDiff),
        // DOM inspection (1)
        Box::new(inspect::InspectPage),
        // CSS → Elementor mapper (1)
        Box::new(css_map::CssToElementor),
        // Live editor control (1)
        Box::new(editor::ElementorEditor),
        // DOM → Elementor JSON (1)
        Box::new(clone::CloneElement),
        // Widget scaffolding + bridge (2)
        Box::new(bridge::InstallBridge),
        Box::new(bridge::CreateWidget),
        // Widget schema & validation (3)
        Box::new(schema::ListWidgets),
        Box::new(schema::GetWidgetSchema),
        Box::new(schema::ValidateElement),
        // Seed content (1)
        Box::new(seed::SeedContent),
        // Authentication (1)
        Box::new(auth::Authenticate),
        // Templates (5)
        Box::new(template::CreateTemplate),
        Box::new(template::UpdateTemplate),
        Box::new(template::ListTemplates),
        Box::new(template::GetTemplate),
        Box::new(template::DeleteTemplate),
        // WordPress Options (2)
        Box::new(option::GetWpOption),
        Box::new(option::SetWpOption),
    ]
}
