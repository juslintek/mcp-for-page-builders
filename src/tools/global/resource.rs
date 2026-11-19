use serde_json::{Value, json};

use crate::wp::WpClient;

pub async fn kit_settings(wp: &WpClient) -> anyhow::Result<Value> {
    let state = wp
        .get("mcp-for-page-builders/v1/resources/kit/settings")
        .await?;
    let mut settings = state["value"]
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Active Elementor kit settings are unavailable"))?;
    settings.insert("revision".into(), state["revision"].clone());
    Ok(Value::Object(settings))
}

pub async fn save_kit_settings(
    wp: &WpClient,
    mut settings: Value,
    expected_revision: &str,
) -> anyhow::Result<()> {
    if settings["revision"].as_str() != Some(expected_revision) {
        anyhow::bail!("Stale kit revision; read global colors or typography again");
    }
    settings
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("Kit settings response is invalid"))?
        .remove("revision");
    let response = wp
        .post(
            "mcp-for-page-builders/v1/resources/apply",
            &json!({
                "operation_id": format!("mcp-{}", crate::elementor::generate_id()),
                "resource_patches": [{
                    "kind": "kit", "id": "settings", "expected_revision": expected_revision,
                    "after": settings,
                }]
            }),
        )
        .await?;
    if response["applied"] != true {
        anyhow::bail!(
            "Elementor kit settings save was not verified: {}",
            serde_json::to_string(&response)?
        );
    }
    Ok(())
}

pub fn upsert(
    settings: &mut Value,
    collection: &str,
    id: &str,
    mut preset: Value,
) -> anyhow::Result<()> {
    let presets = settings[collection]
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("Kit {collection} presets are unavailable"))?;
    preset["_id"] = Value::String(id.to_owned());
    if let Some(existing) = presets.iter_mut().find(|item| item["_id"] == id) {
        *existing = preset;
    } else {
        presets.push(preset);
    }
    Ok(())
}

pub fn remove(settings: &mut Value, collection: &str, id: &str) -> anyhow::Result<()> {
    let presets = settings[collection]
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("Kit {collection} presets are unavailable"))?;
    let index = presets
        .iter()
        .position(|item| item["_id"] == id)
        .ok_or_else(|| anyhow::anyhow!("Global preset {id} not found"))?;
    presets.remove(index);
    Ok(())
}

pub fn collection(settings: &Value, system: &str, custom: &str, id: &str) -> String {
    if settings[system]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["_id"] == id))
        || matches!(id, "primary" | "secondary" | "text" | "accent")
    {
        system.to_owned()
    } else {
        custom.to_owned()
    }
}
