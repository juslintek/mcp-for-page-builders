pub mod auth;
pub mod bridge;
pub mod cache;
pub mod clone;
pub mod css_map;
pub mod editor;
pub mod element;
pub mod elementor_plan_tools;
pub mod elementor_planning;
pub mod elementor_planning_tools;
pub mod elementor_runtime;
pub mod ensure_site;
pub mod file_io;
pub mod global;
pub mod inspect;
pub mod install_config;
pub mod laya_decide;
pub mod media;
pub mod option;
pub mod page;
pub mod post;
pub mod schema;
pub mod seed;
pub mod session_state;
pub mod settings;
pub mod setup_wizard;
pub mod site;
pub mod template;
pub mod v4;
pub mod visual;
pub mod wp7_mcp;
pub mod wp_api;

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
        Box::new(element::PatchElements),
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
        Box::new(visual::MatchStyles),
        Box::new(visual::UiQualityAudit),
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
        Box::new(setup_wizard::SetupWizard),
        Box::new(ensure_site::EnsureSite),
        Box::new(install_config::InstallConfig),
        // Site management
        Box::new(site::ListSites),
        Box::new(site::ConnectSite),
        Box::new(site::DisconnectSite),
        Box::new(site::SwitchSite),
        // WP REST API
        Box::new(wp_api::WpApi),
        Box::new(wp_api::ListUsers),
        Box::new(wp_api::ListComments),
        Box::new(wp_api::ListCategories),
        Box::new(wp_api::ListTags),
        Box::new(wp_api::ListMedia),
        Box::new(wp_api::WpSearch),
        // Media
        Box::new(media::UploadMedia),
        // Session
        Box::new(session_state::GetSessionState),
        // Shadow Realm Multiverse
        Box::new(crate::shadow_realm::ShadowSpawn),
        Box::new(crate::shadow_realm::ShadowFork),
        Box::new(crate::shadow_realm::ShadowList),
        Box::new(crate::shadow_realm::ShadowPromote),
        Box::new(crate::shadow_realm::ShadowPortalInfo),
        // WordPress 7 MCP Features
        Box::new(wp7_mcp::Wp7McpCapabilities),
        Box::new(wp7_mcp::Wp7TemplateParts),
        Box::new(wp7_mcp::Wp7BlockRender),
        Box::new(elementor_runtime::ElementorRuntime),
        Box::new(elementor_runtime::InspectElementorCapabilities),
        Box::new(elementor_plan_tools::ApplyElementorPlan),
        Box::new(elementor_plan_tools::RestoreElementorSnapshot),
        Box::new(elementor_planning_tools::PlanCssMigration),
        Box::new(elementor_planning_tools::ConvertElements),
        Box::new(elementor_planning_tools::ElementorDesignSystem),
        Box::new(elementor_planning_tools::ExtractElementorPatterns),
        Box::new(laya_decide::LayaDecideElementor),
        // Elementor V4 Atomic Design System, Components, Composition, Resources & Publishing
        Box::new(v4::ManageGlobalVariable),
        Box::new(v4::ManageClasses),
        Box::new(v4::ReorderClasses),
        Box::new(v4::GetDefaultStyles),
        Box::new(v4::ManageDefaultStyles),
        Box::new(v4::ListComponents),
        Box::new(v4::ManageComponent),
        Box::new(v4::BuildComposition),
        Box::new(v4::ListResources),
        Box::new(v4::ReadResource),
        Box::new(v4::CreatePreviewLink),
        Box::new(v4::PublishDocument),
        Box::new(v4::ListSiteParts),
        Box::new(v4::ManageSiteParts),
    ]
}
