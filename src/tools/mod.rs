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

pub use crate::types::Tool;

pub fn all_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(page::CreatePage),
        Box::new(page::GetPage),
        Box::new(page::UpdatePage),
        Box::new(page::DeletePage),
        Box::new(page::GetPageBySlug),
        Box::new(page::ListPages),
        Box::new(post::CreatePost),
        Box::new(post::GetPost),
        Box::new(post::ListPosts),
        Box::new(post::UpdatePost),
        Box::new(post::DeletePost),
        Box::new(cache::ClearCache),
        Box::new(file_io::DownloadPage),
        Box::new(file_io::UploadPage),
        Box::new(file_io::BackupPage),
        Box::new(element::GetElement),
        Box::new(element::AddElement),
        Box::new(element::UpdateElement),
        Box::new(element::RemoveElement),
        Box::new(element::MoveElement),
        Box::new(element::DuplicateElement),
        Box::new(element::FindElements),
        Box::new(element::GetElementTree),
        Box::new(global::GetGlobalColors),
        Box::new(global::SetGlobalColor),
        Box::new(global::DeleteGlobalColor),
        Box::new(global::GetGlobalTypography),
        Box::new(global::SetGlobalTypography),
        Box::new(global::DeleteGlobalTypography),
        Box::new(settings::GetKitSchema),
        Box::new(settings::GetKitDefaults),
        Box::new(settings::GetExperiments),
        Box::new(settings::SetExperiment),
        Box::new(visual::Screenshot),
        Box::new(visual::ScreenshotPage),
        Box::new(visual::VisualCompare),
        Box::new(visual::ExtractStyles),
        Box::new(visual::VisualDiff),
        Box::new(inspect::InspectPage),
        Box::new(css_map::CssToElementor),
        Box::new(editor::ElementorEditor),
        Box::new(clone::CloneElement),
        Box::new(bridge::InstallBridge),
        Box::new(bridge::CreateWidget),
        Box::new(schema::ListWidgets),
        Box::new(schema::GetWidgetSchema),
        Box::new(schema::ValidateElement),
        Box::new(seed::SeedContent),
        Box::new(auth::Authenticate),
        Box::new(template::CreateTemplate),
        Box::new(template::UpdateTemplate),
        Box::new(template::ListTemplates),
        Box::new(template::GetTemplate),
        Box::new(template::DeleteTemplate),
        Box::new(option::GetWpOption),
        Box::new(option::SetWpOption),
    ]
}
