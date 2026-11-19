use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    elementor::{ElementorService, get_page_document},
    mcp::{ToolDef, ToolResult},
    tools::{
        Tool,
        css_map::AtomicCssConversion,
        elementor_planning::{
            self, ConvertElementsInput, CssMigrationInput, DesignSystemInput, ExtractPatternsInput,
            PlanContext,
        },
    },
    wp::WpClient,
};

pub struct PlanCssMigration;
pub struct ConvertElements;
pub struct ElementorDesignSystem;
pub struct ExtractElementorPatterns;

fn def(name: &'static str, description: &'static str) -> ToolDef {
    ToolDef {
        name,
        description,
        input_schema: json!({"type":"object","required":["page_id","operation_id"],"properties":{"page_id":{"type":"integer"},"operation_id":{"type":"string"}}}),
    }
}

async fn context(args: &Value, wp: &WpClient) -> Result<(PlanContext, Vec<Value>)> {
    let page_id = args["page_id"].as_u64().context("page_id required")?;
    let operation_id = args["operation_id"]
        .as_str()
        .context("operation_id required")?;
    let document = get_page_document(wp, page_id).await?;
    let elements = serde_json::from_str(document.raw.as_str())
        .context("Elementor document is invalid JSON")?;
    Ok((
        PlanContext {
            site_url: wp.base_url().to_owned(),
            page_id,
            expected_revision: document.revision,
            operation_id: operation_id.to_owned(),
        },
        elements,
    ))
}

fn output(value: impl serde::Serialize) -> Result<ToolResult> {
    Ok(ToolResult::text(serde_json::to_string_pretty(&value)?))
}

fn native_component_ability(capabilities: &Value) -> Option<Value> {
    let mut ability = capabilities["abilities"]
        .as_array()?
        .iter()
        .find(|ability| ability["id"] == "elementor/manage-component")?
        .clone();
    let schema = &ability["input_schema"];
    let has_action = schema["type"] == "object"
        && schema["properties"]["action"]["type"] == "string"
        && schema["required"]
            .as_array()
            .is_some_and(|required| required.iter().any(|field| field == "action"));
    let input_schema_available = schema.as_object().is_some_and(|schema| !schema.is_empty());
    let output_schema = &ability["output_schema"];
    let output_schema_available = output_schema["type"] == "object"
        && output_schema["properties"]
            .as_object()
            .is_some_and(|properties| !properties.is_empty());
    let schema_verified = has_action
        && input_schema_available
        && output_schema_available
        && ability["available"] == true
        && ability["permission"] == true;
    ability["schema_verified"] = json!(schema_verified);
    if !schema_verified {
        let reason = if ability["available"] != true {
            ability["availability_reason"]
                .as_str()
                .unwrap_or("Native ability availability was not explicitly confirmed.")
        } else if ability["permission"] != true {
            ability["permission_error"]
                .as_str()
                .unwrap_or("Native ability permission was not explicitly confirmed.")
        } else if !input_schema_available || !output_schema_available {
            "Registered input and nonempty object output schemas are required."
        } else {
            "Registered mutation schema is missing the required action identifier."
        };
        ability["schema_verification_reason"] = json!(reason);
    }
    Some(ability)
}

fn requested_resource_patches(
    args: &Value,
) -> Result<Vec<(crate::types::elementor_plan::ResourcePatch, &'static str)>> {
    let requested = match args.get("requested_patches") {
        None => return Ok(Vec::new()),
        Some(Value::Array(requested)) => requested,
        Some(_) => anyhow::bail!("requested_patches must be an array"),
    };
    let mut seen = std::collections::HashSet::new();
    let mut patches = Vec::with_capacity(requested.len());
    for value in requested {
        let object = value
            .as_object()
            .context("each requested patch must be an object")?;
        if object.contains_key("expected_revision") || object.contains_key("before") {
            anyhow::bail!("resource revisions and before values are read from current state");
        }
        let patch: crate::types::elementor_plan::ResourcePatch =
            serde_json::from_value(value.clone())?;
        if patch.id.trim().is_empty() {
            anyhow::bail!("resource id must not be empty");
        }
        if !elementor_planning::valid_design_system_resource_id(patch.kind, &patch.id, None) {
            anyhow::bail!("unsupported or invalid resource id for {:?}", patch.kind);
        }
        if !patch.after.is_object() {
            anyhow::bail!("resource after value must be an object");
        }
        let kind = match patch.kind {
            crate::types::elementor_plan::ResourceKind::Class => "class",
            crate::types::elementor_plan::ResourceKind::Variable => "variable",
            crate::types::elementor_plan::ResourceKind::Kit => "kit",
            crate::types::elementor_plan::ResourceKind::PageSettings => "page_settings",
            crate::types::elementor_plan::ResourceKind::Template => "template",
            crate::types::elementor_plan::ResourceKind::PostMetadata
            | crate::types::elementor_plan::ResourceKind::Page => {
                anyhow::bail!("unsupported design-system resource kind")
            }
        };
        if !seen.insert((kind, patch.id.clone())) {
            anyhow::bail!("duplicate design-system resource {kind}/{}", patch.id);
        }
        patches.push((patch, kind));
    }
    Ok(patches)
}

fn synchronized_component_status(ability: Option<&Value>) -> Value {
    let schema_verified = ability.is_some_and(|ability| ability["schema_verified"] == true);
    json!({
        "discovered_ability":ability,
        "input_schema":ability.map(|ability| ability["input_schema"].clone()),
        "output_schema":ability.map(|ability| ability["output_schema"].clone()),
        "schema_verified":schema_verified,
        "available":false,
        "reason":"No checked native Component adapter is implemented; extraction currently emits portable templates."
    })
}

#[async_trait]
impl Tool for ConvertElements {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "convert_elements",
            description: "Plan a revision-bound Elementor conversion using the active page document.",
            input_schema: json!({"type":"object","required":["page_id","operation_id","direction"],"properties":{"page_id":{"type":"integer"},"operation_id":{"type":"string"},"direction":{"type":"string","enum":["classic_to_atomic","atomic_to_classic"]},"element_ids":{"type":"array","items":{"type":"string","minLength":7,"maxLength":8},"minItems":1,"uniqueItems":true}}}),
        }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let (context, elements) = context(&args, wp).await?;
        let service = crate::elementor::ElementorService::new(wp);
        let runtime = wp.get("mcp-for-page-builders/v1/element-schemas").await?;
        let styles = service.style_schema().await?;
        let mut schemas = elementor_planning::AtomicRuntimeSchema::from_runtime(&runtime, &styles);
        for name in [
            "e-heading",
            "e-paragraph",
            "e-image",
            "e-button",
            "e-flexbox",
        ] {
            let schema = service.element_schema(name).await?;
            schemas.add_element_schema(name, &schema);
        }
        if let Some(widgets) = runtime.get("widgets").and_then(Value::as_object) {
            for (name, schema) in widgets {
                schemas.add_classic_schema(name, schema);
            }
        }
        output(elementor_planning::plan_convert_elements_for_ids(
            ConvertElementsInput {
                context,
                direction: args["direction"]
                    .as_str()
                    .context("direction required")?
                    .to_owned(),
                elements,
                schemas: Some(schemas),
            },
            args.get("element_ids")
                .map(|ids| serde_json::from_value(ids.clone()))
                .transpose()
                .context("element_ids must be an array of element ID strings")?,
        )?)
    }
}

#[async_trait]
impl Tool for PlanCssMigration {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "plan_css_migration",
            description: "Parse CSS with CSSOM and return a revision-bound Elementor migration plan.",
            input_schema: json!({"type":"object","required":["page_id","operation_id","target"],"anyOf":[{"required":["css_text"]},{"required":["source_url"]}],"properties":{"page_id":{"type":"integer"},"operation_id":{"type":"string"},"css_text":{"type":"string"},"source_url":{"type":"string"},"selector":{"type":"string"},"target_element_id":{"type":"string"},"target":{"type":"string","enum":["classic","atomic"]},"widget_type":{"type":"string"}}}),
        }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let (context, elements) = context(&args, wp).await?;
        let service = crate::elementor::ElementorService::new(wp);
        let target = args["target"].as_str().context("target required")?;
        let capabilities = service.capabilities().await?;
        let runtime = service.runtime().await?;
        let settings_resource = if capabilities
            .pointer("/resources/page_settings")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            Some(
                wp.get(&format!(
                    "mcp-for-page-builders/v1/resources/page_settings/{}",
                    context.page_id
                ))
                .await?,
            )
        } else {
            None
        };
        let page_settings = settings_resource.as_ref().and_then(|resource| {
            Some((
                resource.get("value")?.clone(),
                resource["revision"].as_str()?.to_owned(),
            ))
        });
        let custom_css_capability = settings_resource
            .as_ref()
            .and_then(|resource| resource.pointer("/capabilities/custom_css"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut breakpoints = std::collections::HashMap::new();
        let mut breakpoint_widths = std::collections::HashMap::new();
        if let Some(points) = runtime.get("breakpoints").and_then(Value::as_object) {
            for (name, value) in points {
                if let Some(query) = value.as_str() {
                    breakpoints.insert(query.to_owned(), name.clone());
                } else if let Some(width) = value.get("value").and_then(Value::as_u64) {
                    let direction = value
                        .get("direction")
                        .and_then(Value::as_str)
                        .unwrap_or("max");
                    let query = format!("({direction}-width: {width}px)");
                    breakpoints.insert(query.clone(), name.clone());
                    breakpoints.insert(format!("@media {query}"), name.clone());
                    breakpoint_widths.insert(name.clone(), width as u32);
                }
            }
        }
        let widget = args["widget_type"].as_str();
        let classic_allowed_keys = if target == "classic" {
            let schema = service
                .element_schema(widget.context("widget_type required for classic migration")?)
                .await?;
            Some(
                schema["controls"]
                    .as_object()
                    .context("registered widget schema has no controls")?
                    .keys()
                    .cloned()
                    .collect(),
            )
        } else {
            None
        };
        let atomic_conversion = if target == "atomic" {
            let parsed = elementor_planning::parse_css_for_migration(
                args["css_text"].as_str(),
                args["source_url"].as_str(),
            )
            .await?;
            let evidence = match (args["source_url"].as_str(), args["selector"].as_str()) {
                (Some(url), Some(selector)) => {
                    Some(elementor_planning::css_matched_styles(url, selector).await?)
                }
                _ => None,
            };
            let mut conversions = std::collections::HashMap::new();
            for (index, rule) in parsed.rules.iter().enumerate() {
                let breakpoint = rule.contexts.iter().find_map(|context| {
                    let (kind, query) = context.split_once(": ")?;
                    (kind == "@media")
                        .then(|| breakpoints.get(query.trim()).cloned())
                        .flatten()
                });
                let exact = args["target_element_id"].is_string()
                    && args["selector"].as_str() == Some(rule.selector.as_str())
                    && (rule.contexts.is_empty()
                        || (rule.contexts.len() == 1 && breakpoint.is_some()))
                    && rule.priority.values().all(String::is_empty);
                let context_evidence = if let (Some(url), Some(selector), Some(name)) = (
                    args["source_url"].as_str(),
                    args["selector"].as_str(),
                    breakpoint.as_deref(),
                ) {
                    let width = breakpoint_widths
                        .get(name)
                        .copied()
                        .unwrap_or(767)
                        .saturating_sub(1)
                        .max(1);
                    Some(
                        elementor_planning::css_matched_styles_at_width(url, selector, width)
                            .await?,
                    )
                } else {
                    evidence.clone()
                };
                let declarations = if exact {
                    let computed = context_evidence
                        .as_ref()
                        .and_then(|e| e["computedStyle"].as_array());
                    rule.declarations
                        .iter()
                        .filter(|(property, value)| {
                            computed.is_some_and(|styles| {
                                styles.iter().any(|entry| {
                                    entry["name"] == property.as_str()
                                        && entry["value"].as_str() == value.as_str()
                                })
                            })
                        })
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect::<serde_json::Map<String, Value>>()
                } else {
                    Default::default()
                };
                let key = format!("rule-{index}");
                let response = wp
                    .post(
                        "elementor/v1/css-to-atomic",
                        &json!({"blocks":{key.clone():declarations}}),
                    )
                    .await?;
                let converted = response
                    .pointer(&format!("/data/{key}"))
                    .context("native CSS converter response omitted converted rule")?;
                conversions.insert(
                    index,
                    AtomicCssConversion {
                        settings: converted["props"].clone(),
                        custom_css: converted
                            .get("customCss")
                            .filter(|v| !v.is_null() && *v != "")
                            .cloned(),
                        rejected: converted["rejected"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect(),
                        capability: "elementor-native-css-converter".into(),
                    },
                );
            }
            Some(conversions)
        } else {
            None
        };
        let css_text = args["css_text"].as_str().map(str::to_owned);
        output(
            elementor_planning::plan_css_migration(CssMigrationInput {
                context,
                document_tree: elements,
                css_text,
                source_url: args["source_url"].as_str().map(str::to_owned),
                selector: args["selector"].as_str().map(str::to_owned),
                target_element_id: args["target_element_id"].as_str().map(str::to_owned),
                matched_styles: None,
                target: target.to_owned(),
                widget_type: widget.map(str::to_owned),
                classic_allowed_keys,
                atomic_conversion,
                page_settings,
                custom_css_capability,
                breakpoints,
            })
            .await?,
        )
    }
}

#[async_trait]
impl Tool for ElementorDesignSystem {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "elementor_design_system",
            description: "Inspect Elementor design resources and plan revision-bound native resource patches.",
            input_schema: json!({"type":"object","required":["page_id","operation_id"],"properties":{"page_id":{"type":"integer"},"operation_id":{"type":"string"},"requested_patches":{"type":"array","items":{"type":"object","required":["kind","id","after"],"properties":{"kind":{"type":"string","enum":["class","variable","kit","page_settings","template"]},"id":{"type":"string"},"after":{"type":"object"}}}}}}),
        }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let requested = requested_resource_patches(&args)?;
        let (context, _) = context(&args, wp).await?;
        let service = crate::elementor::ElementorService::new(wp);
        let capabilities = service.capabilities().await?;
        let runtime = service.runtime().await?;
        if !requested.is_empty() && capabilities["resource_transactions"] != true {
            anyhow::bail!("native resource transactions are unavailable for requested patches");
        }
        for (_, kind) in &requested {
            let resource_key = match *kind {
                "class" => "classes",
                "variable" => "variables",
                "kit" => "kit_settings",
                "page_settings" => "page_settings",
                "template" => "draft_templates",
                _ => anyhow::bail!("unsupported design-system resource kind"),
            };
            if capabilities["resources"][resource_key] != true {
                anyhow::bail!("native capability unavailable for requested {kind} resource");
            }
        }
        let mut patches = Vec::with_capacity(requested.len());
        for (mut patch, kind) in requested {
            let logical_template_create =
                kind == "template" && (patch.id == "new" || patch.id.starts_with("new_pattern_"));
            let state = if logical_template_create {
                json!({"revision":"absent","value":null})
            } else {
                wp.get(&format!(
                    "mcp-for-page-builders/v1/resources/{kind}/{}",
                    patch.id
                ))
                .await?
            };
            patch.expected_revision = state["revision"].as_str().map(str::to_owned);
            patch.before = state.get("value").filter(|value| !value.is_null()).cloned();
            if !patch
                .expected_revision
                .as_deref()
                .is_some_and(|revision| !revision.trim().is_empty())
            {
                anyhow::bail!("resource {kind}/{} did not return a revision", patch.id);
            }
            patches.push(patch);
        }
        let component_ability = native_component_ability(&capabilities);
        let synchronized_components = synchronized_component_status(component_ability.as_ref());
        let registered = json!({"capabilities":capabilities,"runtime":runtime,"style_schema":service.style_schema().await?,"synchronized_components":synchronized_components});
        output(elementor_planning::plan_elementor_design_system(
            DesignSystemInput {
                context,
                native_api_available: capabilities["resource_transactions"] == true,
                registered: Some(registered),
                requested_patches: patches,
            },
        )?)
    }
}

#[async_trait]
impl Tool for ExtractElementorPatterns {
    fn def(&self) -> ToolDef {
        def(
            "extract_elementor_patterns",
            "Find repeated page structures and propose revision-bound portable template resources.",
        )
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let (context, elements) = context(&args, wp).await?;
        let capabilities = ElementorService::new(wp).capabilities().await?;
        let template_api_available = capabilities["resources"]["draft_templates"] == true;
        let logical_template_create_ids_available =
            capabilities["logical_template_create_ids"] == true;
        let native_component_ability = native_component_ability(&capabilities);
        output(
            elementor_planning::plan_extract_elementor_patterns_with_capability(
                ExtractPatternsInput {
                    context,
                    elements,
                    template_api_available,
                    template_revision: template_api_available.then(|| "absent".to_owned()),
                    native_component_ability,
                },
                logical_template_create_ids_available,
            )?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        native_component_ability, requested_resource_patches, synchronized_component_status,
    };
    use serde_json::json;

    #[test]
    fn component_discovery_requires_registered_mutation_schema_and_permission() {
        let ability = json!({
            "id":"elementor/manage-component", "available":true, "permission":true,
            "input_schema":{"type":"object","required":["action"],"properties":{"action":{"type":"string"}}},
            "output_schema":{"type":"object","properties":{"success":{"type":"boolean"}}}
        });
        let capabilities = json!({"abilities":[
            {"id":"elementor/list-components","available":true,"permission":true}, ability
        ]});
        assert_eq!(
            native_component_ability(&capabilities).unwrap()["schema_verified"],
            true
        );

        let denied = json!({"abilities":[{
            "id":"elementor/manage-component", "available":true, "permission":false,
            "input_schema":{"type":"object","required":["action"],"properties":{"action":{"type":"string"}}},
            "output_schema":{"type":"object"}
        }]});
        assert_eq!(
            native_component_ability(&denied).unwrap()["schema_verified"],
            false
        );
    }

    #[test]
    fn component_discovery_requires_nonempty_object_output_schema() {
        for output_schema in [
            json!(null),
            json!([]),
            json!({"type":"string"}),
            json!({"type":"object"}),
        ] {
            let capabilities = json!({"abilities":[{
                "id":"elementor/manage-component", "available":true, "permission":true,
                "input_schema":{"type":"object","required":["action"],"properties":{"action":{"type":"string"}}},
                "output_schema":output_schema
            }]});
            assert_eq!(
                native_component_ability(&capabilities).unwrap()["schema_verified"],
                false
            );
        }
    }

    #[test]
    fn discovered_component_ability_does_not_claim_an_executable_sync_adapter() {
        let ability = json!({
            "id":"elementor/manage-component", "available":true, "permission":true,
            "input_schema":{"type":"object","required":["action"],"properties":{"action":{"type":"string"}}},
            "output_schema":{"type":"object","properties":{"success":{"type":"boolean"}}}
        });
        let discovered = native_component_ability(&json!({"abilities":[ability]})).unwrap();
        assert_eq!(discovered["schema_verified"], true);
        let status = synchronized_component_status(Some(&discovered));
        assert_eq!(status["schema_verified"], true);
        assert_eq!(status["available"], false);
        assert!(
            status["reason"]
                .as_str()
                .unwrap()
                .contains("No checked native Component adapter")
        );
    }

    #[test]
    fn requested_resource_patch_parser_rejects_malformed_or_ambiguous_batches() {
        assert!(requested_resource_patches(&json!({})).unwrap().is_empty());
        for args in [
            json!({"requested_patches":{}}),
            json!({"requested_patches":[null]}),
            json!({"requested_patches":[{"kind":"class","id":" ","after":{}}]}),
            json!({"requested_patches":[{"kind":"class","id":"collection/../kit","after":{}}]}),
            json!({"requested_patches":[{"kind":"class","id":"collection?x=1","after":{}}]}),
            json!({"requested_patches":[{"kind":"variable","id":"variable-1","after":{}}]}),
            json!({"requested_patches":[{"kind":"kit","id":"active","after":{}}]}),
            json!({"requested_patches":[{"kind":"page_settings","id":"0","after":{}}]}),
            json!({"requested_patches":[{"kind":"page_settings","id":format!("{}", "9".repeat(128)),"after":{}}]}),
            json!({"requested_patches":[{"kind":"page","id":"p1","after":{}}]}),
            json!({"requested_patches":[{"kind":"class","id":"c1","expected_revision":"caller-rev","after":{}}]}),
            json!({"requested_patches":[{"kind":"class","id":"c1","before":{},"after":{}}]}),
            json!({"requested_patches":[{"kind":"class","id":"c1","after":{}},{"kind":"class","id":"c1","after":{}}]}),
        ] {
            assert!(requested_resource_patches(&args).is_err(), "{args}");
        }
    }

    #[test]
    fn requested_resource_patch_parser_accepts_only_bridge_write_scopes() {
        let parsed = requested_resource_patches(&json!({"requested_patches":[
            {"kind":"class","id":"collection","after":{}},
            {"kind":"variable","id":"collection","after":{}},
            {"kind":"kit","id":"settings","after":{}},
            {"kind":"page_settings","id":"123","after":{}},
            {"kind":"template","id":"456","after":{}},
            {"kind":"template","id":"new","after":{}},
            {"kind":"template","id":"new_pattern_2","after":{}}
        ]}))
        .unwrap();
        assert_eq!(
            parsed.iter().map(|(_, kind)| *kind).collect::<Vec<_>>(),
            [
                "class",
                "variable",
                "kit",
                "page_settings",
                "template",
                "template",
                "template"
            ]
        );
    }
}
