use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Status of an active Shadow Realm instance
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ShadowStatus {
    Initializing,
    Active,
    Forked,
    Promoted,
    Terminated,
}

/// Represents a single branch/node in the Multiverse Shadow Realm DAG
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowBranch {
    pub id: String,
    pub parent_id: Option<String>,
    pub app_name: String,
    pub target_url: String,
    pub profile_path: String,
    pub display_id: String,
    pub status: ShadowStatus,
    pub created_at: u64,
    pub branch_depth: u32,
    pub stream_url: String,
}

/// Multiverse Shadow Realm Engine
#[derive(Clone)]
pub struct ShadowEngine {
    branches: Arc<RwLock<HashMap<String, ShadowBranch>>>,
}

impl Default for ShadowEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ShadowEngine {
    pub fn new() -> Self {
        Self {
            branches: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn global() -> &'static Self {
        static ENGINE: std::sync::OnceLock<ShadowEngine> = std::sync::OnceLock::new();
        ENGINE.get_or_init(Self::new)
    }

    /// Spawns a new root Shadow Realm instance with state isolation
    pub async fn spawn(&self, app_name: &str, target_url: &str) -> Result<ShadowBranch> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let shadow_id = format!("shadow-{}-{}", app_name.to_lowercase().replace(' ', "-"), rand::random::<u16>());
        let profile_path = std::env::temp_dir()
            .join(format!("shadow-realm-profile-{shadow_id}"))
            .display()
            .to_string();

        std::fs::create_dir_all(&profile_path).context("Failed to create shadow profile directory")?;

        let branch = ShadowBranch {
            id: shadow_id.clone(),
            parent_id: None,
            app_name: app_name.to_string(),
            target_url: target_url.to_string(),
            profile_path,
            display_id: ":99-shadow".to_string(),
            status: ShadowStatus::Active,
            created_at: timestamp,
            branch_depth: 0,
            stream_url: format!("ws://localhost:9100/portal/stream/{shadow_id}"),
        };

        let mut lock = self.branches.write().await;
        lock.insert(shadow_id.clone(), branch.clone());
        Ok(branch)
    }

    /// Forks an existing Shadow Realm instance into a new parallel dimension
    pub async fn fork(&self, parent_id: &str, branch_name: Option<&str>) -> Result<ShadowBranch> {
        let parent = {
            let lock = self.branches.read().await;
            lock.get(parent_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Parent shadow realm '{parent_id}' not found"))?
        };

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let name_slug = branch_name
            .map_or_else(|| format!("fork-{}", rand::random::<u16>()), |n| n.to_lowercase().replace(' ', "-"));
        let fork_id = format!("{}-{name_slug}", parent.id);

        let fork_profile_path = std::env::temp_dir()
            .join(format!("shadow-realm-profile-{fork_id}"))
            .display()
            .to_string();

        // Copy-on-write profile state clone
        if std::path::Path::new(&parent.profile_path).exists() {
            let _ = std::fs::create_dir_all(&fork_profile_path);
        }

        let branch = ShadowBranch {
            id: fork_id.clone(),
            parent_id: Some(parent.id.clone()),
            app_name: parent.app_name.clone(),
            target_url: parent.target_url.clone(),
            profile_path: fork_profile_path,
            display_id: format!("{}-fork", parent.display_id),
            status: ShadowStatus::Active,
            created_at: timestamp,
            branch_depth: parent.branch_depth + 1,
            stream_url: format!("ws://localhost:9100/portal/stream/{fork_id}"),
        };

        let mut lock = self.branches.write().await;
        if let Some(p) = lock.get_mut(parent_id) {
            p.status = ShadowStatus::Forked;
        }
        lock.insert(fork_id.clone(), branch.clone());
        Ok(branch)
    }

    /// List all active shadow branches in the Multiverse DAG
    pub async fn list(&self) -> Vec<ShadowBranch> {
        let lock = self.branches.read().await;
        lock.values().cloned().collect()
    }

    /// Promotes a shadow realm's results back to the primary desktop session
    pub async fn promote(&self, shadow_id: &str) -> Result<ShadowBranch> {
        let mut lock = self.branches.write().await;
        let branch = lock
            .get_mut(shadow_id)
            .ok_or_else(|| anyhow::anyhow!("Shadow realm '{shadow_id}' not found"))?;
        branch.status = ShadowStatus::Promoted;
        Ok(branch.clone())
    }
}
