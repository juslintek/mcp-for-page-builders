use anyhow::{Context, Result};
use chromiumoxide::cdp::browser_protocol::{css, dom};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::{
    cdp,
    tools::css_map::{AtomicCssConversion, map_css_to_elementor_target},
    types::elementor_plan::{ElementorPlan, ResourceKind, ResourcePatch, RetainedElement},
};

const MAX_DEPTH: usize = 64;
const MAX_ELEMENTS: usize = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanContext {
    pub site_url: String,
    pub page_id: u64,
    pub expected_revision: String,
    pub operation_id: String,
}

#[derive(Debug, Clone)]
pub struct CssMigrationInput {
    pub context: PlanContext,
    pub document_tree: Vec<Value>,
    pub css_text: Option<String>,
    pub source_url: Option<String>,
    pub selector: Option<String>,
    pub target_element_id: Option<String>,
    /// CDP CSS.getMatchedStylesForNode response supplied by the existing browser inspector.
    pub matched_styles: Option<Value>,
    pub target: String,
    pub widget_type: Option<String>,
    pub classic_allowed_keys: Option<HashSet<String>>,
    pub atomic_conversion: Option<HashMap<usize, AtomicCssConversion>>,
    pub page_settings: Option<(Value, String)>,
    pub custom_css_capability: bool,
    pub breakpoints: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CssRuleEvidence {
    pub selector: String,
    pub declarations: Map<String, Value>,
    pub priority: BTreeMap<String, String>,
    pub contexts: Vec<String>,
    #[serde(default)]
    pub source_stylesheet: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CssMigrationResult {
    pub plan: ElementorPlan,
    pub inventory: Vec<CssRuleEvidence>,
    pub unresolved_imports: Vec<String>,
    pub matched_styles: Option<Value>,
}

/// Parse CSS with the browser CSSOM and produce a revision-bound proposal. No page writes occur here.
pub async fn plan_css_migration(mut input: CssMigrationInput) -> Result<CssMigrationResult> {
    validate_context(&input.context)?;
    if input.css_text.is_some() == input.source_url.is_some() {
        anyhow::bail!("provide exactly one of css_text or source_url");
    }
    if input
        .css_text
        .as_ref()
        .is_some_and(|css| css.len() > 1_000_000)
    {
        anyhow::bail!("CSS input exceeds 1 MB");
    }
    if input.selector.is_some() && input.source_url.is_none() {
        anyhow::bail!("selector provenance requires source_url");
    }
    let parsed = parse_cssom(input.css_text.as_deref(), input.source_url.as_deref()).await?;
    let matched_styles = match (&input.source_url, &input.selector, &input.matched_styles) {
        (Some(url), Some(selector), None) => {
            Some(inspect_matched_styles(url, selector, 1280).await?)
        }
        (_, _, evidence) => evidence.clone(),
    };
    if input
        .target_element_id
        .as_ref()
        .is_some_and(|id| !valid_element_id(id))
    {
        anyhow::bail!("target_element_id must be a 7- or 8-character alphanumeric Elementor id");
    }
    let mut plan = empty_plan(&input.context);
    let matched_exact_rule = input.target_element_id.is_some()
        && input.selector.as_ref().is_some_and(|selector| {
            parsed
                .rules
                .iter()
                .any(|rule| rule.selector == selector.as_str())
        })
        && input.selector.as_deref().is_some_and(|selector| {
            matched_styles
                .as_ref()
                .is_some_and(|evidence| matched_selector(evidence, selector))
        });
    let mut all_settings = Map::new();
    let mut atomic_variants: BTreeMap<(String, Option<String>), Map<String, Value>> =
        BTreeMap::new();
    let mut all_residual = Vec::new();
    let mut custom_css = Vec::new();
    let mut retained = Vec::new();
    let mut unresolved_rules = Vec::new();
    for (index, rule) in parsed.rules.iter().enumerate() {
        if let Some(value) = input
            .atomic_conversion
            .as_ref()
            .and_then(|conversions| conversions.get(&index))
            .and_then(|conversion| conversion.custom_css.as_ref())
        {
            custom_css
                .push(json!({"selector":rule.selector,"contexts":rule.contexts,"customCss":value}));
        }
        let exact_target_rule =
            matched_exact_rule && input.selector.as_deref() == Some(rule.selector.as_str());
        let flat = rule
            .declarations
            .iter()
            .filter(|(property, value)| {
                exact_target_rule
                    && matched_styles.as_ref().is_some_and(|evidence| {
                        computed_style_value(evidence, property) == value.as_str()
                    })
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Map<_, _>>();
        let nonwinning = rule
            .declarations
            .keys()
            .filter(|property| !flat.contains_key(*property))
            .collect::<Vec<_>>();
        let atomic = input
            .atomic_conversion
            .as_ref()
            .and_then(|conversions| conversions.get(&index));
        let filtered_atomic = if input.target == "atomic" {
            atomic.map(|conversion| AtomicCssConversion {
                settings: if flat.is_empty()
                    && conversion.settings.as_object().is_none_or(Map::is_empty)
                {
                    json!({})
                } else {
                    conversion.settings.clone()
                },
                custom_css: conversion.custom_css.clone(),
                rejected: conversion.rejected.clone(),
                capability: conversion.capability.clone(),
            })
        } else {
            None
        };
        let (mapped, residual) = map_css_to_elementor_target(
            &flat,
            &input.target,
            input.widget_type.as_deref(),
            input.classic_allowed_keys.as_ref(),
            filtered_atomic.as_ref(),
        )?;
        let breakpoint = rule.contexts.iter().find_map(|context| {
            let (kind, query) = context.split_once(": ")?;
            (kind == "@media")
                .then(|| input.breakpoints.get(query.trim()).cloned())
                .flatten()
        });
        let state = [":hover", ":focus", ":active"].iter().find_map(|suffix| {
            rule.selector
                .strip_suffix(suffix)
                .map(|_| suffix[1..].to_owned())
        });
        if let Some(object) = mapped.as_object().filter(|object| !object.is_empty()) {
            if exact_target_rule
                && (rule.contexts.is_empty() || (rule.contexts.len() == 1 && breakpoint.is_some()))
                && rule.priority.values().all(String::is_empty)
            {
                if input.target == "atomic" && (breakpoint.is_some() || state.is_some()) {
                    atomic_variants
                        .entry((
                            breakpoint.clone().unwrap_or_else(|| "desktop".into()),
                            state.clone(),
                        ))
                        .or_default()
                        .extend(object.clone());
                } else if rule.contexts.is_empty() {
                    for (key, value) in object {
                        all_settings.insert(key.clone(), value.clone());
                    }
                }
            } else {
                // Elementor settings do not encode arbitrary selector, media and state ASTs safely.
                retained.push(RetainedElement { element_id: format!("css-rule-{index}"), reason: "selector, responsive context, pseudo-state, or !important priority needs a scoped CSS capability".into() });
            }
        }
        let has_residual = !residual.is_empty() || !nonwinning.is_empty();
        if has_residual {
            let mut residual = residual;
            residual.extend(nonwinning.iter().map(|property| {
                format!("{property}: not the computed winning value for this target")
            }));
            all_residual.push(
                json!({"selector":rule.selector,"contexts":rule.contexts,"declarations":residual}),
            );
            retained.push(RetainedElement {
                element_id: rule.selector.clone(),
                reason: "CSS declarations could not be represented by the active Elementor schema"
                    .into(),
            });
        }
        if !exact_target_rule
            || (!rule.contexts.is_empty() && breakpoint.is_none())
            || rule.priority.values().any(|priority| !priority.is_empty())
            || has_residual
        {
            unresolved_rules.push(json!({"selector":rule.selector,"contexts":rule.contexts,"declarations":rule.declarations,"priority":rule.priority}));
        }
    }
    for rule in &unresolved_rules {
        if !retained
            .iter()
            .any(|entry| entry.element_id == rule["selector"].as_str().unwrap_or_default())
        {
            retained.push(RetainedElement {
                element_id: rule["selector"].as_str().unwrap_or_default().to_owned(),
                reason: "CSS rule selector or context is unresolved and remains a proposal".into(),
            });
        }
    }
    let has_atomic_variants = !atomic_variants.is_empty();
    if input.target == "atomic" && has_atomic_variants {
        let target_id = input
            .target_element_id
            .as_deref()
            .context("target_element_id required for responsive Atomic styles")?;
        let variants = atomic_variants.into_iter().map(|((breakpoint, state), props)| json!({"meta":{"breakpoint":breakpoint,"state":state},"props":props})).collect();
        if !merge_element_style_variants(&mut input.document_tree, target_id, variants) {
            anyhow::bail!(
                "CSS target element was not found or its native Atomic styles were invalid or unbound"
            );
        }
    }
    let mut scoped_rules = Vec::new();
    for (index, rule) in parsed.rules.iter().enumerate() {
        let exact_base = matched_exact_rule
            && input.selector.as_deref() == Some(rule.selector.as_str())
            && rule.contexts.is_empty()
            && rule.priority.values().all(String::is_empty);
        let converted_variant = input.target == "atomic"
            && rule.contexts.len() == 1
            && rule.contexts[0].starts_with("@media:")
            && input
                .atomic_conversion
                .as_ref()
                .and_then(|conversions| conversions.get(&index))
                .is_some_and(|conversion| {
                    conversion
                        .settings
                        .as_object()
                        .is_some_and(|props| !props.is_empty())
                        && conversion.rejected.is_empty()
                });
        if exact_base || converted_variant {
            continue;
        }
        let declarations = rule
            .declarations
            .iter()
            .map(|(property, value)| {
                let important = rule
                    .priority
                    .get(property)
                    .is_some_and(|priority| priority == "important");
                let css_value = value.as_str().unwrap_or_default();
                format!(
                    "{property}: {css_value}{};",
                    if important { " !important" } else { "" }
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        let mut css = format!("{} {{ {declarations} }}", rule.selector);
        for context in rule.contexts.iter().rev() {
            if let Some((kind, condition)) = context.split_once(": ") {
                let condition = condition
                    .trim()
                    .strip_prefix(kind)
                    .unwrap_or(condition.trim())
                    .trim();
                css = format!("{kind} {condition} {{ {css} }}");
            }
        }
        scoped_rules.push(css);
    }
    if !scoped_rules.is_empty() {
        if let Some((settings, revision)) = input.page_settings.as_ref().filter(|(settings, _)| {
            input.custom_css_capability && settings.get("custom_css").is_some()
        }) {
            let mut after = settings.clone();
            let existing = after["custom_css"].as_str().unwrap_or_default();
            after["custom_css"] = json!(format!("{existing}\n{}", scoped_rules.join("\n")));
            plan.resource_patches.push(ResourcePatch {
                kind: ResourceKind::PageSettings,
                id: input.context.page_id.to_string(),
                expected_revision: Some(revision.clone()),
                before: Some(settings.clone()),
                after,
            });
        } else {
            retained.push(RetainedElement {
                element_id: format!("page-{}-css", input.context.page_id),
                reason:
                    "The active page settings schema does not expose a scoped custom_css resource"
                        .into(),
            });
        }
    }
    plan.retained = retained;
    plan.retained.extend(custom_css.iter().map(|proposal| RetainedElement { element_id: input.target_element_id.clone().unwrap_or_else(|| proposal["selector"].as_str().unwrap_or("css-rule").to_owned()), reason:"native CSS custom properties remain a scoped proposal and were not applied to the document".into() }));
    if !all_settings.is_empty() || has_atomic_variants {
        let target_id = input
            .target_element_id
            .as_deref()
            .context("target_element_id required for an executable CSS patch")?;
        let mut tree = input.document_tree;
        let merged = if input.target == "atomic" {
            all_settings.is_empty() || merge_element_styles(&mut tree, target_id, &all_settings)
        } else {
            merge_element_settings(&mut tree, target_id, &all_settings)
        };
        if !merged {
            anyhow::bail!("CSS target element was not found in the fetched document");
        }
        plan.document_patch = Some(json!(tree));
    }
    plan.source = Some(
        json!({"kind":"cssom_migration", "inventory_count":parsed.rules.len(), "unresolved_imports":parsed.imports, "diagnostics":parsed.diagnostics, "matched_styles_available":matched_styles.is_some(), "unresolved_rules":unresolved_rules, "custom_css_proposals":custom_css}),
    );
    plan.checks = Vec::new();
    if let Some(url) = &input.source_url {
        plan.source
            .as_mut()
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("source_url".into(), json!(url));
    }
    if let Some(selector) = &input.selector {
        plan.source
            .as_mut()
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("selector".into(), json!(selector));
    }
    Ok(CssMigrationResult {
        plan,
        inventory: parsed.rules,
        unresolved_imports: parsed.imports,
        matched_styles,
    })
}

async fn inspect_matched_styles(url: &str, selector: &str, width: u32) -> Result<Value> {
    let page = cdp::open_page(url, width, 800).await?;
    let document = page
        .execute(dom::GetDocumentParams::default())
        .await?
        .result
        .root;
    let node = page
        .execute(dom::QuerySelectorParams::new(document.node_id, selector))
        .await?
        .result
        .node_id;
    let _ = page.execute(css::EnableParams {}).await?;
    let matched = page
        .execute(css::GetMatchedStylesForNodeParams::new(node))
        .await?
        .result;
    let computed = page
        .execute(css::GetComputedStyleForNodeParams::new(node))
        .await?
        .result
        .computed_style;
    let mut evidence =
        serde_json::to_value(matched).context("failed to serialize CDP matched styles")?;
    evidence["computedStyle"] =
        serde_json::to_value(computed).context("failed to serialize CDP computed styles")?;
    Ok(evidence)
}

pub async fn css_matched_styles(url: &str, selector: &str) -> Result<Value> {
    inspect_matched_styles(url, selector, 1280).await
}

pub async fn css_matched_styles_at_width(url: &str, selector: &str, width: u32) -> Result<Value> {
    inspect_matched_styles(url, selector, width).await
}

fn computed_style_value<'a>(styles: &'a Value, property: &str) -> Option<&'a str> {
    styles["computedStyle"]
        .as_array()?
        .iter()
        .find(|entry| entry["name"] == property)?
        .get("value")?
        .as_str()
}

fn matched_selector(styles: &Value, selector: &str) -> bool {
    styles["matchedCSSRules"].as_array().is_some_and(|rules| {
        rules.iter().any(|entry| {
            let matching = entry["matchingSelectors"].as_array();
            entry["rule"]["selectorList"]["selectors"]
                .as_array()
                .is_some_and(|selectors| {
                    selectors.iter().enumerate().any(|(index, candidate)| {
                        let text = candidate.as_str().or_else(|| candidate["text"].as_str());
                        text == Some(selector)
                            && matching.is_some_and(|indices| {
                                indices
                                    .iter()
                                    .any(|value| value.as_u64() == Some(index as u64))
                            })
                    })
                })
        })
    })
}

#[derive(Debug, Deserialize)]
pub struct CssomOutput {
    pub rules: Vec<CssRuleEvidence>,
    pub imports: Vec<String>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

fn plain_paragraph_html(html: &str) -> bool {
    paragraph_inner(html).is_some()
}

fn paragraph_inner(html: &str) -> Option<&str> {
    let trimmed = html.trim();
    if trimmed.starts_with("<p>")
        && trimmed.ends_with("</p>")
        && !trimmed[3..trimmed.len() - 4].contains('<')
    {
        Some(&trimmed[3..trimmed.len() - 4])
    } else {
        None
    }
}

pub async fn parse_css_for_migration(
    css_text: Option<&str>,
    source_url: Option<&str>,
) -> Result<CssomOutput> {
    parse_cssom(css_text, source_url).await
}

async fn parse_cssom(css_text: Option<&str>, source_url: Option<&str>) -> Result<CssomOutput> {
    let page = if let Some(url) = source_url {
        cdp::open_page(url, 1280, 800).await?
    } else {
        cdp::open_page("about:blank", 1280, 800).await?
    };
    let script = if let Some(css) = css_text {
        let css_json = serde_json::to_string(css)?;
        format!(
            r#"(async()=>{{const style=document.createElement('style');style.textContent={css_json};document.head.append(style);const s=style.sheet;if(!s)throw Error('CSSStyleSheet unavailable');const out=walk(s.cssRules,[],'inline CSS');await checkImports(out);return JSON.stringify(out);function walk(rules,ctx,source){{const out={{rules:[],imports:[],diagnostics:[]}};for(const r of rules){{if(r.type===3){{out.imports.push(r.href);try{{const nested=walk(r.styleSheet.cssRules,ctx,r.href);out.rules.push(...nested.rules);out.diagnostics.push(...nested.diagnostics)}}catch(e){{out.diagnostics.push('Imported stylesheet rules inaccessible: '+r.href)}}continue}}if(r.type===1){{const declarations={{}},priority={{}};for(let i=0;i<r.style.length;i++){{const p=r.style[i];declarations[p]=r.style.getPropertyValue(p).trim();priority[p]=r.style.getPropertyPriority(p)}}out.rules.push({{selector:r.selectorText,declarations,priority,contexts:ctx,source_stylesheet:source}});continue}}if(r.cssRules){{const type=r.constructor.name;const label=r.cssText.split('{{')[0].trim();const context=(type==='CSSMediaRule'?'@media':type==='CSSSupportsRule'?'@supports':type==='CSSLayerBlockRule'?'@layer':type==='CSSContainerRule'?'@container':'@unknown '+type)+': '+label;if(type.startsWith('CSS')&&!['CSSMediaRule','CSSSupportsRule','CSSLayerBlockRule','CSSContainerRule','CSSImportRule'].includes(type))out.diagnostics.push('Unsupported at-rule '+label);const nested=walk(r.cssRules,ctx.concat([context]),source);out.rules.push(...nested.rules);out.imports.push(...nested.imports);out.diagnostics.push(...nested.diagnostics)}}}}return out}}async function checkImports(out){{const statuses=await Promise.all(out.imports.map(async href=>{{try{{const response=await fetch(href);return response.ok?null:href}}catch(e){{out.diagnostics.push('Import status inaccessible: '+href);return null}}}}));out.imports=statuses.filter(Boolean)}}}})()"#
        )
    } else {
        r#"(async()=>{const out={rules:[],imports:[],diagnostics:[]};function walk(rules,ctx,source){for(const r of rules){if(r.type===3){out.imports.push(r.href);try{walk(r.styleSheet.cssRules,ctx,r.href)}catch(e){out.diagnostics.push('Imported stylesheet rules inaccessible: '+r.href)}continue}if(r.type===1){const declarations={},priority={};for(let i=0;i<r.style.length;i++){const p=r.style[i];declarations[p]=r.style.getPropertyValue(p).trim();priority[p]=r.style.getPropertyPriority(p)}out.rules.push({selector:r.selectorText,declarations,priority,contexts:ctx,source_stylesheet:source});continue}if(r.cssRules){const type=r.constructor.name,label=r.cssText.split('{')[0].trim(),context=(type==='CSSMediaRule'?'@media':type==='CSSSupportsRule'?'@supports':type==='CSSLayerBlockRule'?'@layer':type==='CSSContainerRule'?'@container':'@unknown '+type)+': '+label;if(type.startsWith('CSS')&&!['CSSMediaRule','CSSSupportsRule','CSSLayerBlockRule','CSSContainerRule','CSSImportRule'].includes(type))out.diagnostics.push('Unsupported at-rule '+label);walk(r.cssRules,ctx.concat([context]),source)}}}for(const s of document.styleSheets){try{walk(s.cssRules,[],s.href||'inline stylesheet')}catch(e){out.diagnostics.push('Stylesheet inaccessible: '+(s.href||'inline stylesheet'))}}const statuses=await Promise.all(out.imports.map(async href=>{try{const response=await fetch(href);return response.ok?null:href}catch(e){out.diagnostics.push('Import status inaccessible: '+href);return null}}));out.imports=statuses.filter(Boolean);return JSON.stringify(out)})()"#.into()
    };
    let result: String = page
        .evaluate(script)
        .await
        .context("CSSOM evaluation failed")?
        .into_value()
        .context("CSSOM returned no string")?;
    serde_json::from_str(&result).context("CSSOM parser returned invalid JSON")
}

#[derive(Debug, Clone)]
pub struct AtomicRuntimeSchema {
    /// Runtime property type names used for compatibility checks.
    pub widget_props: HashMap<String, HashMap<String, String>>,
    /// Recursive native schemas returned by the active Elementor service.
    pub widget_prop_schemas: HashMap<String, Map<String, Value>>,
    pub element_versions: HashMap<String, String>,
    /// Runtime style property type names.
    pub style_props: HashMap<String, HashMap<String, String>>,
    pub style_schemas: Map<String, Value>,
    pub classic_controls: HashMap<String, HashSet<String>>,
    pub render_capabilities: Map<String, Value>,
}

impl AtomicRuntimeSchema {
    pub fn from_runtime(runtime: &Value, styles: &Value) -> Self {
        let mut schema = Self {
            widget_props: HashMap::new(),
            widget_prop_schemas: HashMap::new(),
            element_versions: HashMap::new(),
            style_props: HashMap::new(),
            style_schemas: Map::new(),
            classic_controls: HashMap::new(),
            render_capabilities: runtime
                .get("render_capabilities")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
        };
        if let Some(elements) = runtime.get("elements").and_then(Value::as_object) {
            for (name, entry) in elements {
                if let Some(controls) = entry.get("controls").and_then(Value::as_object) {
                    schema
                        .classic_controls
                        .insert(name.clone(), controls.keys().cloned().collect());
                }
                if let Some(version) = entry
                    .pointer("/initial_config/version")
                    .and_then(Value::as_str)
                {
                    schema.element_versions.insert(name.clone(), version.into());
                }
                if let Some(props) = entry
                    .pointer("/initial_config/atomic_props_schema")
                    .and_then(Value::as_object)
                {
                    schema.widget_props.insert(
                        name.clone(),
                        props
                            .iter()
                            .filter_map(|(key, value)| {
                                prop_type(value).map(|typ| (key.clone(), typ))
                            })
                            .collect(),
                    );
                }
            }
        }
        if let Some(props) = styles
            .pointer("/atomic_style_schema")
            .and_then(Value::as_object)
        {
            schema.style_props.insert(
                "*".into(),
                props
                    .iter()
                    .filter_map(|(key, value)| prop_type(value).map(|typ| (key.clone(), typ)))
                    .collect(),
            );
            schema.style_schemas = props.clone();
        }
        schema
    }
    pub fn add_element_schema(&mut self, name: &str, value: &Value) {
        if let Some(props) = value
            .get("props_schema")
            .or_else(|| value.pointer("/initial_config/atomic_props_schema"))
            .and_then(Value::as_object)
        {
            self.widget_props.insert(
                name.into(),
                props
                    .iter()
                    .filter_map(|(key, value)| prop_type(value).map(|typ| (key.clone(), typ)))
                    .collect(),
            );
            self.widget_prop_schemas.insert(name.into(), props.clone());
        }
        if let Some(version) = value
            .pointer("/initial_config/version")
            .and_then(Value::as_str)
        {
            self.element_versions.insert(name.into(), version.into());
        }
    }
    pub fn add_classic_schema(&mut self, name: &str, value: &Value) {
        if let Some(controls) = value.get("controls").and_then(Value::as_object) {
            self.classic_controls
                .insert(name.into(), controls.keys().cloned().collect());
        }
    }

    fn attributes_rendering_available(&self) -> bool {
        self.render_capabilities
            .get("attributes")
            .and_then(|capability| capability.get("available"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    fn classic_control_available(&self, element: &str, control: &str) -> bool {
        self.classic_controls
            .get(element)
            .is_some_and(|controls| controls.contains(control))
    }
}

fn prop_type(value: &Value) -> Option<String> {
    value
        .get("$$type")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("default")
                .and_then(|default| default.get("$$type"))
                .and_then(Value::as_str)
        })
        .or_else(|| value.get("type").and_then(Value::as_str))
        .or_else(|| value.get("key").and_then(Value::as_str))
        .map(str::to_owned)
}

fn merge_element_settings(elements: &mut [Value], id: &str, settings: &Map<String, Value>) -> bool {
    for element in elements {
        if element["id"].as_str() == Some(id) {
            if !element["settings"].is_object() {
                element["settings"] = json!({});
            }
            element["settings"]
                .as_object_mut()
                .unwrap()
                .extend(settings.clone());
            return true;
        }
        if let Some(children) = element["elements"].as_array_mut()
            && merge_element_settings(children, id, settings)
        {
            return true;
        }
    }
    false
}

fn native_styles_by_id(element: &Value) -> Option<Map<String, Value>> {
    let mut styles = Map::new();
    match element.get("styles") {
        None => {}
        Some(Value::Object(entries)) => {
            for (id, style) in entries {
                if !valid_native_style_definition(id, style)
                    || styles.insert(id.clone(), style.clone()).is_some()
                {
                    return None;
                }
            }
        }
        Some(Value::Array(entries)) => {
            for style in entries {
                let id = style.get("id")?.as_str()?;
                if !valid_native_style_definition(id, style)
                    || styles.insert(id.to_owned(), style.clone()).is_some()
                {
                    return None;
                }
            }
        }
        Some(_) => return None,
    }
    if !styles.is_empty() {
        let classes = element
            .pointer("/settings/classes")
            .filter(|classes| classes["$$type"] == "classes")?
            .get("value")?
            .as_array()?;
        let bound = classes
            .iter()
            .map(Value::as_str)
            .collect::<Option<HashSet<_>>>()?;
        if styles.keys().any(|id| !bound.contains(id.as_str())) {
            return None;
        }
    }
    Some(styles)
}

fn valid_native_style_definition(id: &str, style: &Value) -> bool {
    if id.is_empty()
        || !id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
        || style["id"].as_str() != Some(id)
        || style["type"] != "class"
    {
        return false;
    }
    let Some(variants) = style["variants"].as_array() else {
        return false;
    };
    let mut seen = HashSet::new();
    variants.iter().all(|variant| {
        let Some(breakpoint) = variant.pointer("/meta/breakpoint").and_then(Value::as_str) else {
            return false;
        };
        let Some(props) = variant["props"].as_object() else {
            return false;
        };
        let state = variant["meta"]["state"].as_str().unwrap_or_default();
        let key = (breakpoint, state);
        !props.is_empty()
            && (variant["meta"]["state"].is_null() || variant["meta"]["state"].as_str().is_some())
            && props.values().all(|prop| prop["$$type"].as_str().is_some())
            && seen.insert(key)
    })
}

fn merge_element_styles(elements: &mut [Value], id: &str, props: &Map<String, Value>) -> bool {
    for element in elements {
        if element["id"].as_str() == Some(id) {
            let class_id = format!("mcp-css-{id}");
            let Some(mut styles) = native_styles_by_id(element) else {
                return false;
            };
            if !merge_element_class_binding(element, &class_id) {
                return false;
            }
            if let Some(style) = styles.get_mut(&class_id) {
                let variant = style["variants"].as_array_mut().and_then(|items| {
                    items.iter_mut().find(|variant| {
                        variant["meta"]["breakpoint"] == "desktop"
                            && variant["meta"]["state"].is_null()
                    })
                });
                let Some(variant) = variant else {
                    return false;
                };
                let Some(existing) = variant["props"].as_object_mut() else {
                    return false;
                };
                existing.extend(props.clone());
            } else {
                styles.insert(class_id.clone(), json!({"id":class_id,"type":"class","label":class_id,"variants":[{"meta":{"breakpoint":"desktop","state":null},"props":props}]}));
            }
            element["styles"] = Value::Object(styles);
            return true;
        }
        if let Some(children) = element["elements"].as_array_mut()
            && merge_element_styles(children, id, props)
        {
            return true;
        }
    }
    false
}

fn merge_element_style_variants(elements: &mut [Value], id: &str, variants: Vec<Value>) -> bool {
    for element in elements {
        if element["id"].as_str() == Some(id) {
            let class_id = format!("mcp-css-{id}");
            let Some(mut styles) = native_styles_by_id(element) else {
                return false;
            };
            if !merge_element_class_binding(element, &class_id) {
                return false;
            }
            let style = styles.entry(class_id.clone()).or_insert_with(
                || json!({"id":class_id,"type":"class","label":class_id,"variants":[]}),
            );
            let Some(existing) = style["variants"].as_array_mut() else {
                return false;
            };
            for variant in variants {
                let meta = variant["meta"].clone();
                if let Some(old) = existing.iter_mut().find(|old| old["meta"] == meta) {
                    if let (Some(dst), Some(src)) =
                        (old["props"].as_object_mut(), variant["props"].as_object())
                    {
                        dst.extend(src.clone());
                    }
                } else {
                    existing.push(variant);
                }
            }
            element["styles"] = Value::Object(styles);
            return true;
        }
        if let Some(children) = element["elements"].as_array_mut()
            && merge_element_style_variants(children, id, variants.clone())
        {
            return true;
        }
    }
    false
}

fn merge_element_class_binding(element: &mut Value, class_id: &str) -> bool {
    if !element["settings"].is_object() {
        element["settings"] = json!({});
    }
    let settings = element["settings"].as_object_mut().unwrap();
    let classes = settings
        .entry("classes")
        .or_insert_with(|| json!({"$$type":"classes","value":[]}));
    if classes["$$type"] != "classes" || !classes["value"].is_array() {
        return false;
    }
    let values = classes["value"].as_array_mut().unwrap();
    if !values.iter().any(|value| value.as_str() == Some(class_id)) {
        values.push(json!(class_id));
    }
    true
}

fn encode_prop(schema: &Value, value: &Value, forced_type: Option<&str>) -> Option<Value> {
    let kind = schema["kind"].as_str()?;
    if kind == "union" {
        let options = schema["prop_types"].as_object()?;
        let selected = forced_type
            .and_then(|name| options.get(name).map(|schema| (name, schema)))
            .or_else(|| {
                value
                    .get("$$type")
                    .and_then(Value::as_str)
                    .and_then(|name| options.get(name).map(|schema| (name, schema)))
            })
            .or_else(|| match value {
                Value::String(_) => ["string", "url", "html-v3"]
                    .iter()
                    .find_map(|name| options.get(*name).map(|schema| (*name, schema))),
                Value::Bool(_) => options.get("boolean").map(|schema| ("boolean", schema)),
                Value::Number(_) => ["number", "image-attachment-id"]
                    .iter()
                    .find_map(|name| options.get(*name).map(|schema| (*name, schema))),
                Value::Object(fields) => options.iter().find_map(|(name, candidate)| {
                    candidate["kind"]
                        .as_str()
                        .filter(|kind| *kind == "object")?;
                    let shape = candidate["shape"].as_object()?;
                    (!shape.is_empty() && shape.keys().all(|key| fields.contains_key(key)))
                        .then_some((name.as_str(), candidate))
                }),
                _ => None,
            })?;
        return encode_prop(selected.1, value, Some(selected.0));
    }
    let key = schema["key"].as_str()?;
    let raw = match kind {
        "object" if key == "html-v3" && value.is_string() => {
            json!({"content":{"$$type":"string","value":value},"children":[]})
        }
        "object" if key == "size" => value.clone(),
        "object" => {
            let shape = schema["shape"].as_object()?;
            let input = value.as_object()?;
            let mut fields = Map::new();
            for (field, field_schema) in shape {
                if let Some(field_value) = input.get(field) {
                    if field_value.is_null() {
                        continue;
                    }
                    let selected = if field == "url" {
                        Some("url")
                    } else if field == "src" {
                        Some("image-src")
                    } else if field == "destination" {
                        Some("url")
                    } else if field == "isTargetBlank" {
                        Some("boolean")
                    } else if field == "tag" || field == "alt" {
                        Some("string")
                    } else {
                        None
                    };
                    fields.insert(
                        field.clone(),
                        encode_prop(field_schema, field_value, selected)?,
                    );
                } else if field_schema
                    .pointer("/settings/required")
                    .and_then(Value::as_bool)
                    == Some(true)
                {
                    return None;
                }
            }
            Value::Object(fields)
        }
        "string" | "number" | "boolean" if forced_type.is_none_or(|forced| forced == key) => {
            value.clone()
        }
        "plain" if forced_type.is_none_or(|forced| forced == key) => value.clone(),
        "array" if forced_type.is_none_or(|forced| forced == key) && value.is_array() => {
            let item_schema = schema.get("item_prop_type")?;
            Value::Array(
                value
                    .as_array()?
                    .iter()
                    .map(|item| encode_prop(item_schema, item, None))
                    .collect::<Option<Vec<_>>>()?,
            )
        }
        _ => return None,
    };
    Some(json!({"$$type":key,"value":raw}))
}

#[derive(Debug, Clone)]
pub struct ConvertElementsInput {
    pub context: PlanContext,
    pub direction: String,
    pub elements: Vec<Value>,
    pub schemas: Option<AtomicRuntimeSchema>,
}

/// Convert only known plain content widgets for which the active editor supplied atomic property schemas.
/// Unsupported references and descendants stay intact as their full original subtree.
pub fn plan_convert_elements(input: ConvertElementsInput) -> Result<ElementorPlan> {
    plan_convert_elements_for_ids(input, None)
}

pub fn plan_convert_elements_for_ids(
    input: ConvertElementsInput,
    element_ids: Option<Vec<String>>,
) -> Result<ElementorPlan> {
    validate_context(&input.context)?;
    if input.elements.is_empty() || input.elements.len() > MAX_ELEMENTS {
        anyhow::bail!("elements must contain 1 to {MAX_ELEMENTS} nodes");
    }
    if !matches!(
        input.direction.as_str(),
        "classic_to_atomic" | "atomic_to_classic"
    ) {
        anyhow::bail!("direction must be classic_to_atomic or atomic_to_classic");
    }
    validate_ids(&input.elements)?;
    let selected_ids = element_ids.as_deref();
    let mut paths = HashMap::new();
    collect_element_paths(&input.elements, &mut Vec::new(), &mut paths);
    let target_paths = if let Some(ids) = selected_ids {
        if ids.is_empty() {
            anyhow::bail!("element_ids must contain at least one element ID when supplied");
        }
        let mut unique = HashSet::new();
        let mut targets = Vec::with_capacity(ids.len());
        for id in ids {
            if !unique.insert(id.as_str()) {
                anyhow::bail!("duplicate requested Elementor element id: {id}");
            }
            targets.push((
                id,
                paths
                    .get(id)
                    .with_context(|| format!("requested Elementor element id not found: {id}"))?,
            ));
        }
        for (index, (left_id, left_path)) in targets.iter().enumerate() {
            for (right_id, right_path) in targets.iter().skip(index + 1) {
                if paths_overlap(left_path, right_path) {
                    anyhow::bail!(
                        "requested element IDs overlap by parent/child: {left_id} and {right_id}"
                    );
                }
            }
        }
        Some(
            targets
                .into_iter()
                .map(|(_, path)| path.clone())
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };
    let mut plan = empty_plan(&input.context);
    let mut retained = Vec::new();
    let mut converted = input.elements.clone();
    if let Some(paths) = target_paths {
        let mut staged = Vec::with_capacity(paths.len());
        for path in &paths {
            let element = element_at_path(&input.elements, path)
                .context("validated element path disappeared before conversion")?;
            if already_in_requested_form(element, &input.direction) {
                retained.push(RetainedElement {
                    element_id: element["id"].as_str().unwrap_or_default().to_owned(),
                    reason: format!(
                        "element is already in the requested {} form and was retained unchanged",
                        input.direction
                    ),
                });
                staged.push(element.clone());
                continue;
            }
            let mut target_retained = Vec::new();
            let converted = convert_element(
                element,
                &input.direction,
                input.schemas.as_ref(),
                &mut target_retained,
                0,
            )?;
            if !target_retained.is_empty() {
                retained.extend(target_retained);
            }
            staged.push(converted);
        }
        for (path, replacement) in paths.iter().zip(staged) {
            replace_element_at_path(&mut converted, path, replacement)
                .context("validated element path disappeared while composing document patch")?;
        }
    } else {
        converted = input
            .elements
            .iter()
            .map(|element| {
                convert_element(
                    element,
                    &input.direction,
                    input.schemas.as_ref(),
                    &mut retained,
                    0,
                )
            })
            .collect::<Result<Vec<_>>>()?;
    }
    plan.retained = retained;
    if plan.retained.is_empty() {
        plan.document_patch = Some(json!(converted));
    }
    plan.source = Some(
        json!({"kind":"element_conversion", "direction":input.direction, "runtime_schema_available":input.schemas.is_some(), "selected_element_ids":selected_ids.unwrap_or(&[]), "selection_scope":if selected_ids.is_some() {"selected_disjoint_subtrees"} else {"complete_document"}, "external_selector_style_dependencies_unverified":true, "external_dependency_note":"Document conversion does not verify stylesheets or selectors outside the canonical Elementor tree."}),
    );
    plan.checks = Vec::new();
    Ok(plan)
}

fn collect_element_paths(
    elements: &[Value],
    path: &mut Vec<usize>,
    paths: &mut HashMap<String, Vec<usize>>,
) {
    for (index, element) in elements.iter().enumerate() {
        path.push(index);
        if let Some(id) = element["id"].as_str() {
            paths.insert(id.to_owned(), path.clone());
        }
        if let Some(children) = element["elements"].as_array() {
            collect_element_paths(children, path, paths);
        }
        path.pop();
    }
}

fn element_at_path<'a>(elements: &'a [Value], path: &[usize]) -> Option<&'a Value> {
    let (&index, rest) = path.split_first()?;
    let element = elements.get(index)?;
    if rest.is_empty() {
        Some(element)
    } else {
        element_at_path(element["elements"].as_array()?, rest)
    }
}

fn replace_element_at_path(
    elements: &mut [Value],
    path: &[usize],
    replacement: Value,
) -> Option<()> {
    let (&index, rest) = path.split_first()?;
    let element = elements.get_mut(index)?;
    if rest.is_empty() {
        *element = replacement;
        Some(())
    } else {
        replace_element_at_path(element["elements"].as_array_mut()?, rest, replacement)
    }
}

fn already_in_requested_form(element: &Value, direction: &str) -> bool {
    match direction {
        "classic_to_atomic" => {
            matches!(
                element["widgetType"].as_str(),
                Some("e-heading" | "e-paragraph" | "e-image" | "e-button")
            ) || matches!(
                element["elType"].as_str(),
                Some("e-flexbox" | "e-grid" | "e-div-block")
            )
        }
        "atomic_to_classic" => {
            matches!(
                element["widgetType"].as_str(),
                Some("heading" | "text-editor" | "image" | "button")
            ) || matches!(
                element["elType"].as_str(),
                Some("section" | "column" | "container")
            )
        }
        _ => false,
    }
}

fn convert_element(
    element: &Value,
    direction: &str,
    schemas: Option<&AtomicRuntimeSchema>,
    retained: &mut Vec<RetainedElement>,
    depth: usize,
) -> Result<Value> {
    if depth > MAX_DEPTH {
        anyhow::bail!("element recursion exceeds {MAX_DEPTH}");
    }
    let id = element["id"].as_str().unwrap_or_default();
    let layout = matches!(
        element["elType"].as_str(),
        Some("section" | "column" | "container" | "wrapper" | "e-flexbox")
    );
    if let Some(reason) = conversion_subtree_unsafe_reason(element) {
        retained.push(RetainedElement {
            element_id: id.into(),
            reason: if layout {
                format!("layout subtree contains unsupported content: {reason}")
            } else {
                reason
            },
        });
        return Ok(element.clone());
    }
    if layout {
        let mut nested_retained = Vec::new();
        let converted = if direction == "classic_to_atomic" {
            classic_layout_to_atomic(element, schemas, &mut nested_retained, depth)
        } else {
            atomic_layout_to_classic(element, schemas, &mut nested_retained, depth, false)
        };
        if let Some(converted) = converted.filter(|_| nested_retained.is_empty()) {
            return Ok(converted);
        }
        let reason = nested_retained
            .first()
            .map(|nested| format!("layout subtree contains unsupported content: {}", nested.reason))
            .unwrap_or_else(|| "layout subtree contains settings or descendants that cannot be mapped with the active native style schema".into());
        retained.push(RetainedElement {
            element_id: id.into(),
            reason,
        });
        return Ok(element.clone());
    }
    let converted = match direction {
        "classic_to_atomic" => classic_to_atomic(element, schemas),
        _ => atomic_to_classic(element, schemas),
    };
    if let Some(converted) = converted {
        return Ok(converted);
    }
    if direction == "atomic_to_classic"
        && element["elType"].as_str() == Some("widget")
        && matches!(
            element["widgetType"].as_str(),
            Some("e-heading" | "e-paragraph" | "e-image" | "e-button")
        )
    {
        retained.push(RetainedElement {
            element_id: id.into(),
            reason: "Atomic widget subtree retained because the active Classic controls cannot represent its settings".into(),
        });
        return Ok(element.clone());
    }
    if matches!(element["elType"].as_str(), Some("e-grid" | "e-div-block")) {
        retained.push(RetainedElement { element_id: id.into(), reason: "section, column, or atomic layout requires a runtime mapping to preserve desktop and responsive layout".into() });
        return Ok(element.clone());
    }
    if element["elType"].as_str() == Some("widget")
        && matches!(
            element["widgetType"].as_str(),
            Some("heading" | "text-editor" | "image" | "button")
        )
    {
        retained.push(RetainedElement {
            element_id: id.into(),
            reason: if direction == "classic_to_atomic"
                && has_nonempty_classic_attributes(element)
                && !schemas.is_some_and(AtomicRuntimeSchema::attributes_rendering_available)
            {
                "nonempty Classic custom attributes retained because the active Atomic settings resolver did not verify their rendered output".into()
            } else {
                "matching active Atomic prop schema or lossless field mapping is unavailable"
                    .into()
            },
        });
    }
    if element["elType"].as_str() == Some("widget")
        && !matches!(
            element["widgetType"].as_str(),
            Some(
                "heading"
                    | "text-editor"
                    | "image"
                    | "button"
                    | "e-heading"
                    | "e-paragraph"
                    | "e-image"
                    | "e-button"
            )
        )
    {
        retained.push(RetainedElement {
            element_id: id.into(),
            reason: "widget has no lossless mapping in the active runtime schemas".into(),
        });
        return Ok(element.clone());
    }
    let mut clone = element.clone();
    if let Some(children) = clone.get_mut("elements").and_then(Value::as_array_mut) {
        for child in children {
            *child = convert_element(child, direction, schemas, retained, depth + 1)?;
        }
    }
    Ok(clone)
}

fn native_layout_style(id: &str, props: Map<String, Value>, breakpoint: &str) -> Option<Value> {
    if props.is_empty() {
        return None;
    }
    let variant = json!({"meta":{"breakpoint":breakpoint,"state":null},"props":props});
    Some(json!({"id":id,"type":"class","label":"layout","variants":[variant]}))
}

fn conversion_subtree_unsafe_reason(element: &Value) -> Option<String> {
    fn has_unsafe_reference(value: &Value) -> bool {
        match value {
            Value::Array(items) => items.iter().any(has_unsafe_reference),
            Value::Object(fields) => fields.iter().any(|(key, value)| {
                matches!(
                    key.as_str(),
                    "__dynamic__"
                        | "global_classes"
                        | "component_id"
                        | "template_id"
                        | "custom_logic"
                ) || value
                    .as_str()
                    .is_some_and(|text| text.contains("{{") || text.contains("[elementor-template"))
                    || has_unsafe_reference(value)
            }),
            _ => false,
        }
    }

    if element["interactions"]
        .as_array()
        .is_some_and(|items| !items.is_empty())
    {
        return Some("interactions need a runtime mapping".into());
    }
    if has_unsafe_reference(&element["settings"]) {
        return Some(
            "dynamic bindings, global references, or custom logic need runtime resolution".into(),
        );
    }
    if contains_unparsed_custom_css(element) {
        return Some("custom CSS is retained because raw selector and style-ID references cannot be safely remapped".into());
    }
    if element["elType"] == "widget"
        && !matches!(
            element["widgetType"].as_str(),
            Some(
                "heading"
                    | "text-editor"
                    | "image"
                    | "button"
                    | "e-heading"
                    | "e-paragraph"
                    | "e-image"
                    | "e-button"
            )
        )
    {
        return Some(
            "custom widget subtree retained because its runtime schema and behavior are unknown"
                .into(),
        );
    }
    let mut ids = HashSet::new();
    collect_subtree_ids(element, &mut ids);
    if contains_internal_element_id_reference(element, &ids) {
        return Some(
            "subtree contains an internal element ID reference that cannot be safely remapped"
                .into(),
        );
    }
    element["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(conversion_subtree_unsafe_reason)
}

fn encode_classes(
    schemas: &AtomicRuntimeSchema,
    element: &str,
    classes: &[String],
) -> Option<Value> {
    let schema = schemas.widget_prop_schemas.get(element)?.get("classes")?;
    encode_prop(schema, &json!(classes), Some("classes"))
}

fn classic_attributes(
    schemas: &AtomicRuntimeSchema,
    element: &str,
    settings: &Value,
) -> Option<Value> {
    let mut attributes = Vec::new();
    if let Some(lines) = settings.get("_attributes") {
        let lines = lines.as_str()?;
        let element_id = settings
            .get("_element_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty());
        for line in lines.lines() {
            let (key, value) = line.split_once('|')?;
            if key.is_empty()
                || !key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
                || key.starts_with("on")
                || key.eq_ignore_ascii_case("href")
                || value.contains(['|', '\n', '\r'])
                || (key.eq_ignore_ascii_case("id") && element_id.is_some())
                || attributes
                    .iter()
                    .any(|attr: &Value| attr["key"] == key.to_ascii_lowercase())
            {
                return None;
            }
            attributes.push(json!({"key":key.to_ascii_lowercase(),"value":value.trim()}));
        }
    }
    if attributes.is_empty() {
        return Some(Value::Null);
    }
    if !schemas.attributes_rendering_available() {
        return None;
    }
    let schema = schemas
        .widget_prop_schemas
        .get(element)?
        .get("attributes")?;
    encode_prop(schema, &json!(attributes), Some("attributes"))
}

fn has_nonempty_classic_attributes(element: &Value) -> bool {
    element["settings"]["_attributes"]
        .as_str()
        .is_some_and(|lines| lines.lines().any(|line| !line.trim().is_empty()))
}

fn classic_element_id(
    schemas: &AtomicRuntimeSchema,
    element: &str,
    settings: &Value,
) -> Option<Value> {
    let id = settings.get("_element_id")?.as_str()?;
    if id.contains('|') || id.contains('\n') || id.contains('\r') {
        return None;
    }
    if id.is_empty() {
        return Some(Value::Null);
    }
    let schema = schemas.widget_prop_schemas.get(element)?.get("_cssid")?;
    encode_prop(schema, &json!(id), Some("string"))
}

fn classic_link_value(settings: &Value, schema: &Value) -> Option<Value> {
    let link = settings.get("link")?;
    let link = link.as_object()?;
    if link.keys().any(|key| {
        !matches!(
            key.as_str(),
            "url" | "is_external" | "nofollow" | "custom_attributes"
        )
    }) {
        return None;
    }
    let url = link.get("url")?.as_str()?;
    let truthy = |value: &Value| match value {
        Value::Null => Some(false),
        Value::Bool(value) => Some(*value),
        Value::Number(value) => Some(value.as_f64().is_some_and(|number| number != 0.0)),
        Value::String(value) => Some(!value.is_empty() && value != "0"),
        Value::Array(_) | Value::Object(_) => None,
    };
    let is_external = link.get("is_external").map(truthy).unwrap_or(Some(false))?;
    let nofollow = link.get("nofollow").map(truthy).unwrap_or(Some(false))?;
    if nofollow
        || link
            .get("custom_attributes")
            .is_some_and(|value| !matches!(value, Value::String(value) if value.is_empty()))
    {
        return None;
    }
    encode_prop(
        schema,
        &json!({"destination":url,"isTargetBlank":is_external,"tag":"a"}),
        Some("link"),
    )
}

fn atomic_link_to_classic(value: &Value, schema: &Value) -> Option<(String, bool)> {
    let link = decode_typed_prop(value, schema)?;
    let fields = link.as_object()?;
    if fields
        .keys()
        .any(|key| !matches!(key.as_str(), "destination" | "isTargetBlank" | "tag"))
    {
        return None;
    }
    let destination = fields.get("destination")?.as_str()?;
    if fields
        .get("tag")
        .and_then(Value::as_str)
        .is_some_and(|tag| tag != "a")
    {
        return None;
    }
    let is_external = fields
        .get("isTargetBlank")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some((destination.to_owned(), is_external))
}

fn decode_typed_prop(value: &Value, schema: &Value) -> Option<Value> {
    if schema["kind"] == "union" {
        let typ = value["$$type"].as_str()?;
        return decode_typed_prop(value, schema["prop_types"].get(typ)?);
    }
    if value["$$type"].as_str()? != prop_type(schema).as_deref()? {
        return None;
    }
    let raw = value.get("value")?;
    match schema["kind"].as_str()? {
        "array" => Some(Value::Array(
            raw.as_array()?
                .iter()
                .map(|item| decode_typed_prop(item, schema.get("item_prop_type")?))
                .collect::<Option<Vec<_>>>()?,
        )),
        "object" => {
            let mut decoded = Map::new();
            for (key, field_schema) in schema["shape"].as_object()? {
                if let Some(field) = raw.get(key) {
                    decoded.insert(key.clone(), decode_typed_prop(field, field_schema)?);
                }
            }
            Some(Value::Object(decoded))
        }
        _ => Some(raw.clone()),
    }
}

fn atomic_attributes_to_classic(value: &Value, schema: &Value) -> Option<Map<String, Value>> {
    let entries = decode_typed_prop(value, schema)?.as_array()?.to_vec();
    let mut settings = Map::new();
    let mut lines = Vec::new();
    let mut seen = HashSet::new();
    for entry in entries {
        let key = entry["key"].as_str()?.to_ascii_lowercase();
        let value = entry["value"].as_str()?;
        if key.is_empty()
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || key.starts_with("on")
            || key == "href"
            || !seen.insert(key.clone())
            || value.contains(['|', '\n', '\r'])
        {
            return None;
        }
        if key == "id" {
            settings.insert("_element_id".into(), json!(value));
        } else {
            lines.push(format!("{key}|{value}"));
        }
    }
    if !lines.is_empty() {
        settings.insert("_attributes".into(), json!(lines.join("\n")));
    }
    Some(settings)
}

fn atomic_css_classes(
    value: &Value,
    schema: &Value,
    ignored: &HashSet<String>,
) -> Option<Vec<String>> {
    let classes = decode_typed_prop(value, schema)?
        .as_array()?
        .iter()
        .map(|value| value.as_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()?;
    if classes.iter().any(|name| {
        name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || (!ignored.contains(name) && name.starts_with("mcp-layout-"))
    }) {
        return None;
    }
    Some(
        classes
            .into_iter()
            .filter(|name| !ignored.contains(name))
            .collect(),
    )
}

fn classic_class_names(settings: &Value, key: &str) -> Option<Vec<String>> {
    let classes = settings.get(key)?.as_str()?;
    let names = classes
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    names
        .iter()
        .all(|name| {
            !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
        .then_some(names)
}

fn style_value(
    schemas: &AtomicRuntimeSchema,
    key: &str,
    value: Value,
    forced: Option<&str>,
) -> Option<Value> {
    encode_prop(schemas.style_schemas.get(key)?, &value, forced)
}

fn layout_props(
    schemas: &AtomicRuntimeSchema,
    settings: &Value,
    direction: &str,
    width: Option<Value>,
) -> Option<Map<String, Value>> {
    let mut props = Map::new();
    props.insert(
        "display".into(),
        style_value(schemas, "display", json!("flex"), Some("string"))?,
    );
    props.insert(
        "flex-direction".into(),
        style_value(schemas, "flex-direction", json!(direction), Some("string"))?,
    );
    if let Some(width) = width {
        props.insert("width".into(), style_value(schemas, "width", width, None)?);
    }
    if let Some(gap) = settings.get("flex_gap").or_else(|| settings.get("gap")) {
        let size = gap.get("size")?.clone();
        let unit = gap.get("unit")?.as_str()?;
        let side = json!({"size":size,"unit":unit});
        props.insert(
            "gap".into(),
            style_value(
                schemas,
                "gap",
                json!({"column":side,"row":side}),
                Some("layout-direction"),
            )?,
        );
    }
    Some(props)
}

fn classic_layout_to_atomic(
    element: &Value,
    schemas: Option<&AtomicRuntimeSchema>,
    retained: &mut Vec<RetainedElement>,
    depth: usize,
) -> Option<Value> {
    let schemas = schemas?;
    let kind = element["elType"].as_str()?;
    let settings = element.get("settings").filter(|v| v.is_object())?;
    if kind == "section" {
        let columns = element["elements"].as_array()?;
        if columns.is_empty()
            || columns.iter().any(|column| {
                column["elType"] != "column"
                    || column["settings"]["_column_size"].as_f64().is_none()
                    || ["_column_size_tablet", "_column_size_mobile"]
                        .iter()
                        .any(|key| {
                            column["settings"]
                                .get(key)
                                .is_some_and(|size| size.as_f64().is_none())
                        })
            })
        {
            return None;
        }
    }
    let (atomic, direction, width) = match kind {
        "section" => (
            "e-flexbox",
            settings
                .get("flex_direction")
                .map(Value::as_str)
                .unwrap_or(Some("row"))?,
            None,
        ),
        "column" => (
            "e-flexbox",
            "column",
            match settings.get("_column_size") {
                Some(value) => Some(json!({"size":value.as_f64()?,"unit":"%"})),
                None => None,
            },
        ),
        "container" | "wrapper" => (
            "e-flexbox",
            settings
                .get("flex_direction")
                .map(Value::as_str)
                .unwrap_or(Some("column"))?,
            None,
        ),
        _ => return None,
    };
    let supported = match kind {
        "section" => [
            "flex_direction",
            "flex_gap",
            "gap",
            "css_classes",
            "_element_id",
            "_attributes",
            "html_tag",
        ]
        .as_slice(),
        "column" => [
            "_column_size",
            "_column_size_tablet",
            "_column_size_mobile",
            "flex_gap",
            "gap",
            "css_classes",
            "_element_id",
            "_attributes",
            "html_tag",
        ]
        .as_slice(),
        _ => [
            "flex_direction",
            "flex_gap",
            "gap",
            "css_classes",
            "_element_id",
            "_attributes",
            "html_tag",
            "link",
        ]
        .as_slice(),
    };
    if settings
        .as_object()?
        .keys()
        .any(|key| !supported.contains(&key.as_str()))
    {
        return None;
    }
    if element
        .get("styles")
        .is_some_and(|styles| !styles.as_array().is_some_and(Vec::is_empty))
    {
        return None;
    }
    let version = schemas.element_versions.get(atomic)?;
    let mut styles = Map::new();
    let mut class_ids = Vec::new();
    let mut base = layout_props(schemas, settings, direction, width)?;
    let base_id = format!("mcp-layout-{}", element["id"].as_str()?);
    let base_style = native_layout_style(&base_id, std::mem::take(&mut base), "desktop")?;
    styles.insert(base_id.clone(), base_style);
    class_ids.push(base_id);
    for (suffix, breakpoint) in [("_tablet", "tablet"), ("_mobile", "mobile")] {
        if let Some(width) = settings.get(&format!("_column_size{suffix}")) {
            let width = width.as_f64()?;
            let mut props = Map::new();
            props.insert(
                "width".into(),
                style_value(schemas, "width", json!({"size":width,"unit":"%"}), None)?,
            );
            let class_id = format!("mcp-layout-{}-{breakpoint}", element["id"].as_str()?);
            let style = native_layout_style(&class_id, props, breakpoint)?;
            styles.insert(class_id.clone(), style);
            class_ids.push(class_id);
        }
    }
    let mut atomic_settings = Map::new();
    atomic_settings.insert(
        "classes".into(),
        encode_classes(schemas, atomic, &class_ids)?,
    );
    let default_tag = if kind == "section" { "section" } else { "div" };
    let tag = settings
        .get("html_tag")
        .map(Value::as_str)
        .unwrap_or(Some(default_tag))?;
    let tag = if tag.is_empty() { default_tag } else { tag };
    let tag_schema = schemas.widget_prop_schemas.get(atomic)?.get("tag")?;
    atomic_settings.insert(
        "tag".into(),
        encode_prop(tag_schema, &json!(tag), Some("string"))?,
    );
    if let Some(link) = settings.get("link").filter(|link| !link.is_null()) {
        if !matches!(kind, "container" | "wrapper") || tag != "a" {
            return None;
        }
        let link_schema = schemas.widget_prop_schemas.get(atomic)?.get("link")?;
        atomic_settings.insert(
            "link".into(),
            classic_link_value(&json!({"link":link}), link_schema)?,
        );
    }
    let classic_class_key = "css_classes";
    if let Some(classes) = classic_class_names(settings, classic_class_key) {
        if !classes.is_empty() {
            if class_ids.iter().any(|id| classes.contains(id)) {
                return None;
            }
            let mut all_classes = class_ids;
            all_classes.extend(classes);
            atomic_settings.insert(
                "classes".into(),
                encode_classes(schemas, atomic, &all_classes)?,
            );
        }
    } else if settings
        .get(classic_class_key)
        .is_some_and(|value| !value.as_str().is_some_and(str::is_empty))
    {
        return None;
    }
    if settings.get("_element_id").is_some() {
        let id = classic_element_id(schemas, atomic, settings)?;
        if !id.is_null() {
            atomic_settings.insert("_cssid".into(), id);
        }
    }
    let attributes = classic_attributes(schemas, atomic, settings)?;
    if !attributes.is_null() {
        atomic_settings.insert("attributes".into(), attributes);
    }
    let mut children = Vec::new();
    for child in element["elements"].as_array()? {
        children.push(
            convert_element(
                child,
                "classic_to_atomic",
                Some(schemas),
                retained,
                depth + 1,
            )
            .ok()?,
        );
    }
    Some(
        json!({"id":element["id"],"version":version,"elType":"e-flexbox","isInner":element["isInner"].as_bool().unwrap_or(false),"settings":atomic_settings,"editor_settings":element.get("editor_settings").cloned().unwrap_or_else(|| json!([])),"interactions":element.get("interactions").cloned().unwrap_or_else(|| json!([])),"styles":styles,"elements":children}),
    )
}

fn atomic_layout_to_classic(
    element: &Value,
    schemas: Option<&AtomicRuntimeSchema>,
    retained: &mut Vec<RetainedElement>,
    depth: usize,
    as_column: bool,
) -> Option<Value> {
    let schemas = schemas?;
    if element["elType"] != "e-flexbox"
        || schemas.element_versions.get("e-flexbox")? != element["version"].as_str()?
    {
        return None;
    }
    let schemas_for_element = schemas.widget_prop_schemas.get("e-flexbox")?;
    let settings_source = element["settings"].as_object()?;
    if settings_source.keys().any(|key| {
        !matches!(
            key.as_str(),
            "classes" | "attributes" | "_cssid" | "tag" | "link"
        )
    }) {
        return None;
    }
    let styles = native_styles_by_id(element)?;
    let style_ids = styles.keys().cloned().collect::<HashSet<_>>();
    let style_prefix = format!("mcp-layout-{}", element["id"].as_str()?);
    if style_ids.iter().any(|id| {
        id != &style_prefix
            && id != &format!("{style_prefix}-tablet")
            && id != &format!("{style_prefix}-mobile")
    }) {
        return None;
    }
    let mut classic_settings = Map::new();
    if let Some(classes) = settings_source.get("classes") {
        let class_schema = schemas_for_element.get("classes")?;
        let decoded_classes = decode_typed_prop(classes, class_schema)?;
        let encoded_classes = decoded_classes
            .as_array()?
            .iter()
            .map(|value| value.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()?;
        if style_ids.iter().any(|id| !encoded_classes.contains(id)) {
            return None;
        }
        let class_names = atomic_css_classes(classes, class_schema, &style_ids)?;
        if !class_names.is_empty() {
            classic_settings.insert("css_classes".into(), json!(class_names.join(" ")));
        }
    }
    if !style_ids.is_empty() && settings_source.get("classes").is_none() {
        return None;
    }
    if let Some(id) = settings_source.get("_cssid") {
        let value = decode_typed_prop(id, schemas_for_element.get("_cssid")?)?;
        classic_settings.insert("_element_id".into(), value);
    }
    let tag = match settings_source.get("tag") {
        Some(value) => Some(
            decode_typed_prop(value, schemas_for_element.get("tag")?)?
                .as_str()?
                .to_owned(),
        ),
        None => None,
    };
    let tag = tag.as_deref().unwrap_or("div");
    if as_column {
        if tag != "div" || settings_source.contains_key("link") {
            return None;
        }
    } else {
        classic_settings.insert("html_tag".into(), json!(tag));
    }
    if let Some(link) = settings_source.get("link") {
        if tag != "a" {
            return None;
        }
        let (url, is_external) = atomic_link_to_classic(link, schemas_for_element.get("link")?)?;
        classic_settings.insert(
            "link".into(),
            json!({"url":url,"is_external":is_external,"nofollow":""}),
        );
    }
    if let Some(attributes) = settings_source.get("attributes") {
        let attributes =
            atomic_attributes_to_classic(attributes, schemas_for_element.get("attributes")?)?;
        if settings_source.contains_key("_cssid") && attributes.contains_key("_element_id") {
            return None;
        }
        classic_settings.extend(attributes);
    }
    let mut width = None;
    let mut responsive_widths = Map::new();
    let mut direction = "row".to_owned();
    let mut gap = None;
    let mut display_seen = false;
    let mut direction_seen = false;
    let mut gap_seen = false;
    for style in styles.values() {
        for variant in style["variants"].as_array()? {
            let breakpoint = variant["meta"]["breakpoint"].as_str()?;
            if !variant["meta"]["state"].is_null() {
                return None;
            }
            let props = variant["props"].as_object()?;
            for (key, value) in props {
                let schema = schemas.style_schemas.get(key)?;
                let raw = decode_style_value(value, schema)?;
                match key.as_str() {
                    "display"
                        if breakpoint == "desktop"
                            && raw.as_str() == Some("flex")
                            && !display_seen =>
                    {
                        display_seen = true
                    }
                    "flex-direction" if breakpoint == "desktop" && !direction_seen => {
                        direction = raw.as_str()?.to_owned();
                        direction_seen = true;
                    }
                    "width" if breakpoint == "desktop" && width.is_none() => width = Some(raw),
                    "width" if matches!(breakpoint, "tablet" | "mobile") => {
                        if responsive_widths.contains_key(breakpoint) {
                            return None;
                        }
                        responsive_widths.insert(breakpoint.into(), raw);
                    }
                    "gap" if breakpoint == "desktop" && !gap_seen => {
                        if raw["column"] != raw["row"] {
                            return None;
                        }
                        gap = Some(raw["column"].clone());
                        gap_seen = true;
                    }
                    _ => return None,
                }
            }
        }
    }
    let source_children = element["elements"].as_array()?;
    let is_section = !source_children.is_empty()
        && source_children.iter().all(|child| {
            child["elType"] == "e-flexbox"
                && native_styles_by_id(child).is_some_and(|styles| {
                    styles.values().any(|style| {
                        style["variants"].as_array().is_some_and(|variants| {
                            variants
                                .iter()
                                .any(|variant| variant["props"]["width"].is_object())
                        })
                    })
                })
        });
    if !is_section && !as_column && !responsive_widths.is_empty() {
        return None;
    }
    let kind = if is_section { "section" } else { "container" };
    let classic_kind = if as_column { "column" } else { kind };
    if classic_settings.contains_key("_attributes")
        && !schemas.classic_control_available(classic_kind, "_attributes")
    {
        return None;
    }
    let mut children = Vec::new();
    for child in source_children {
        let next = if is_section {
            let mut column =
                atomic_layout_to_classic(child, Some(schemas), retained, depth + 1, true)?;
            column["elType"] = json!("column");
            column
        } else {
            convert_element(
                child,
                "atomic_to_classic",
                Some(schemas),
                retained,
                depth + 1,
            )
            .ok()?
        };
        children.push(next);
    }
    let mut settings = classic_settings;
    if !is_section {
        settings.insert("flex_direction".into(), json!(direction));
    }
    if let Some(width) = width {
        settings.insert("width".into(), width);
    }
    for (breakpoint, width) in responsive_widths {
        settings.insert(format!("_column_size_{breakpoint}"), width);
    }
    if is_section {
        for child in &mut children {
            let size = &child["settings"]["width"];
            if size["unit"].as_str()? != "%" {
                return None;
            }
            let mut settings = child["settings"].as_object()?.clone();
            let width = settings.remove("width")?;
            settings.insert("_column_size".into(), width["size"].clone());
            for breakpoint in ["tablet", "mobile"] {
                if let Some(width) = settings.remove(&format!("_column_size_{breakpoint}")) {
                    if width["unit"].as_str()? != "%" {
                        return None;
                    }
                    settings.insert(format!("_column_size_{breakpoint}"), width["size"].clone());
                }
            }
            child["settings"] = Value::Object(settings);
        }
    }
    if let Some(gap) = gap {
        settings.insert("flex_gap".into(), gap);
    }
    Some(json!({"id":element["id"],"elType":kind,"settings":settings,"elements":children}))
}

fn decode_style_value(value: &Value, schema: &Value) -> Option<Value> {
    if schema["kind"] == "union" {
        let typ = value["$$type"].as_str()?;
        return decode_style_value(value, schema["prop_types"].get(typ)?);
    }
    if value["$$type"].as_str()? != prop_type(schema).as_deref()? {
        return None;
    }
    let raw = value.get("value")?;
    match schema["kind"].as_str()? {
        "object" if schema["key"] == "size" => Some(raw.clone()),
        "object" => {
            let mut decoded = Map::new();
            for (key, field_schema) in schema["shape"].as_object()? {
                if let Some(field) = raw.get(key) {
                    decoded.insert(key.clone(), decode_style_value(field, field_schema)?);
                }
            }
            Some(Value::Object(decoded))
        }
        _ => Some(raw.clone()),
    }
}

fn classic_to_atomic(element: &Value, schemas: Option<&AtomicRuntimeSchema>) -> Option<Value> {
    let widget = element["widgetType"].as_str()?;
    if element["elType"] != "widget" {
        return None;
    }
    let (atomic, text_key, classic_text_key) = match widget {
        "heading" => ("e-heading", "title", "title"),
        "text-editor"
            if element["settings"]["editor"]
                .as_str()
                .is_some_and(plain_paragraph_html) =>
        {
            ("e-paragraph", "paragraph", "editor")
        }
        "image" => ("e-image", "image", "image"),
        "button" => ("e-button", "text", "text"),
        _ => return None,
    };
    let schema = schemas?;
    let version = schema.element_versions.get(atomic)?;
    let props = schema.widget_props.get(atomic)?;
    let settings = &element["settings"];
    if element
        .get("styles")
        .is_some_and(|styles| !styles.as_array().is_some_and(Vec::is_empty))
    {
        return None;
    }
    let text = if atomic == "e-image" {
        ""
    } else {
        settings.get(classic_text_key).and_then(Value::as_str)?
    };
    let text = if atomic == "e-paragraph" {
        paragraph_inner(text)?
    } else {
        text
    };
    let text_type = props.get(text_key)?;
    if contains_references(&settings[classic_text_key]) {
        return None;
    }
    let prop_schemas = &schema.widget_prop_schemas[atomic];
    let typed_text = if atomic == "e-image" {
        None
    } else {
        let forced = if matches!(text_type.as_str(), "html-v3" | "html") {
            "html-v3"
        } else {
            text_type.as_str()
        };
        Some(encode_prop(
            prop_schemas.get(text_key)?,
            &json!(text),
            Some(forced),
        )?)
    };
    let mut atomic_settings = Map::new();
    if let Some(typed_text) = typed_text {
        atomic_settings.insert(text_key.into(), typed_text);
    }
    if atomic == "e-image" {
        let image = &settings["image"];
        let url = image["url"].as_str()?;
        let image_id = image["id"].as_u64().filter(|id| *id > 0);
        let size = settings
            .get("image_size")
            .and_then(Value::as_str)
            .unwrap_or("full");
        if !matches!(size, "thumbnail" | "medium" | "large" | "full") {
            return None;
        }
        let image_type = props.get("image")?;
        let alt = settings
            .get("image_alt")
            .and_then(Value::as_str)
            .or_else(|| image.get("alt").and_then(Value::as_str))
            .unwrap_or("");
        let mut src = json!({"url":url,"alt":alt});
        if let Some(id) = image_id {
            src["id"] = json!(id);
        }
        let raw = json!({"src":src,"size":size});
        atomic_settings.insert(
            "image".into(),
            encode_prop(prop_schemas.get("image")?, &raw, Some(image_type))?,
        );
    }
    if widget == "heading" {
        let tag = settings
            .get("header_size")
            .and_then(Value::as_str)
            .unwrap_or("h2");
        if !matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
            return None;
        }
        if props.contains_key("tag") {
            atomic_settings.insert(
                "tag".into(),
                encode_prop(prop_schemas.get("tag")?, &json!(tag), Some("string"))?,
            );
        }
    }
    if let Some(class_value) = settings.get("_css_classes") {
        let class_value = class_value.as_str()?;
        let classes = class_value
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if classes.iter().any(|name| {
            name.is_empty()
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        }) {
            return None;
        }
        if !classes.is_empty() {
            atomic_settings.insert("classes".into(), encode_classes(schema, atomic, &classes)?);
        }
    }
    if settings.get("_element_id").is_some() {
        let id = classic_element_id(schema, atomic, settings)?;
        if !id.is_null() {
            atomic_settings.insert("_cssid".into(), id);
        }
    }
    if settings.get("_attributes").is_some() {
        let attributes = classic_attributes(schema, atomic, settings)?;
        if !attributes.is_null() {
            atomic_settings.insert("attributes".into(), attributes);
        }
    }
    let image_link_to = if atomic == "e-image" {
        match settings
            .get("link_to")
            .and_then(Value::as_str)
            .unwrap_or("none")
        {
            "none" => false,
            "custom" => true,
            _ => return None,
        }
    } else {
        true
    };
    if atomic == "e-image"
        && settings
            .get("open_lightbox")
            .is_some_and(|value| value.as_str() != Some("default"))
    {
        return None;
    }
    if image_link_to && !settings.get("link").is_none_or(Value::is_null) {
        let link_type = props.get("link")?;
        if link_type != "link" || contains_references(&settings["link"]) {
            return None;
        }
        let old = &settings["link"];
        let url = old.get("url").and_then(Value::as_str)?;
        let old = old.as_object()?;
        if old.keys().any(|key| {
            !matches!(
                key.as_str(),
                "url" | "is_external" | "nofollow" | "custom_attributes"
            )
        }) {
            return None;
        }
        let native_nonempty = |value: &Value| match value {
            Value::Null => Some(false),
            Value::Bool(value) => Some(*value),
            Value::Number(value) => Some(value.as_f64().is_some_and(|number| number != 0.0)),
            Value::String(value) => Some(!value.is_empty() && value != "0"),
            Value::Array(_) | Value::Object(_) => None,
        };
        let is_external = match old.get("is_external") {
            Some(value) => native_nonempty(value)?,
            None => false,
        };
        let nofollow = match old.get("nofollow") {
            Some(value) => native_nonempty(value)?,
            None => false,
        };
        let custom_attributes = old.get("custom_attributes");
        if nofollow
            || custom_attributes
                .is_some_and(|value| !matches!(value, Value::String(value) if value.is_empty()))
        {
            return None;
        }
        let link = json!({"destination":url,"isTargetBlank":is_external,"tag":"a"});
        atomic_settings.insert(
            "link".into(),
            encode_prop(prop_schemas.get("link")?, &link, Some("link"))?,
        );
    }
    if settings.as_object()?.keys().any(|key| {
        !matches!(
            key.as_str(),
            "title"
                | "text"
                | "editor"
                | "header_size"
                | "link"
                | "image"
                | "image_size"
                | "image_alt"
                | "link_to"
                | "open_lightbox"
                | "_css_classes"
                | "_element_id"
                | "_attributes"
        )
    }) {
        return None;
    }
    Some(
        json!({"id":element["id"], "version":version, "elType":"widget", "widgetType":atomic, "isInner":element["isInner"].as_bool().unwrap_or(false), "settings":atomic_settings, "editor_settings":element.get("editor_settings").cloned().unwrap_or_else(|| json!([])), "interactions":element.get("interactions").cloned().unwrap_or_else(|| json!([])), "styles":element.get("styles").cloned().unwrap_or_else(|| json!([])), "elements":[]}),
    )
}

fn atomic_to_classic(element: &Value, schemas: Option<&AtomicRuntimeSchema>) -> Option<Value> {
    if element["elType"] != "widget" {
        return None;
    }
    let atomic = element["widgetType"].as_str()?;
    let (widget, text_key, classic_key) = match atomic {
        "e-heading" => ("heading", "title", "title"),
        "e-paragraph" => ("text-editor", "paragraph", "editor"),
        "e-image" => ("image", "image", "image"),
        "e-button" => ("button", "text", "text"),
        _ => return None,
    };
    if element["elements"]
        .as_array()
        .is_some_and(|children| !children.is_empty())
        || !element["styles"].as_array().is_some_and(Vec::is_empty)
    {
        return None;
    }
    let schema = schemas?;
    if schema.element_versions.get(atomic)? != element["version"].as_str()? {
        return None;
    }
    let props = schema.widget_props.get(atomic)?;
    let prop_schemas = schema.widget_prop_schemas.get(atomic)?;
    let settings = element["settings"].as_object()?;
    if element["editor_settings"]
        .as_array()
        .is_some_and(|items| !items.is_empty())
    {
        return None;
    }
    let mut classic = Map::new();
    for (key, value) in settings {
        let typ = value["$$type"].as_str()?;
        if props.get(key)? != typ {
            return None;
        }
        let raw = value.get("value")?;
        if contains_references(raw) {
            return None;
        }
        match key.as_str() {
            key if key == text_key && matches!(typ, "string" | "html-v3" | "html") => {
                let text = if typ == "string" {
                    raw.clone()
                } else {
                    raw.pointer("/content/value").cloned()?
                };
                classic.insert(
                    classic_key.into(),
                    if widget == "text-editor" {
                        json!(format!("<p>{}</p>", text.as_str()?))
                    } else {
                        text
                    },
                );
            }
            "tag"
                if atomic == "e-heading"
                    && typ == "string"
                    && matches!(raw.as_str(), Some("h1" | "h2" | "h3" | "h4" | "h5" | "h6")) =>
            {
                classic.insert("header_size".into(), raw.clone());
            }
            "tag" if atomic == "e-paragraph" && typ == "string" && raw.as_str() == Some("p") => {}
            "tag" if atomic == "e-button" && typ == "string" => {
                let linked = settings.get("link").is_some();
                let expected = if linked { "a" } else { "button" };
                if raw.as_str()? != expected {
                    return None;
                }
            }
            "link" if typ == "link" => {
                let destination = raw["destination"]["value"]
                    .as_str()
                    .or_else(|| raw["destination"].as_str())?;
                if raw
                    .get("tag")
                    .and_then(|tag| tag["value"].as_str().or_else(|| tag.as_str()))
                    .is_some_and(|tag| tag != "a")
                {
                    return None;
                }
                classic.insert("link".into(), json!({"url":destination,"is_external":raw["isTargetBlank"]["value"],"nofollow":""}));
                if widget == "image" {
                    classic.insert("link_to".into(), json!("custom"));
                }
            }
            "image" if widget == "image" => {
                let src = raw["src"].get("value").unwrap_or(&raw["src"]);
                let url = src["url"]["value"]
                    .as_str()
                    .or_else(|| src["url"].as_str())?;
                let id = src["id"]["value"].as_u64().or_else(|| src["id"].as_u64());
                let alt = src["alt"]["value"]
                    .as_str()
                    .or_else(|| src["alt"].as_str())
                    .unwrap_or("");
                let mut image = json!({"url":url,"alt":alt});
                if let Some(id) = id {
                    image["id"] = json!(id);
                }
                classic.insert("image".into(), image);
                classic.insert("image_size".into(), raw["size"]["value"].clone());
            }
            "classes" if typ == "classes" => {
                let names =
                    atomic_css_classes(value, prop_schemas.get("classes")?, &HashSet::new())?;
                if !names.is_empty() {
                    classic.insert("_css_classes".into(), json!(names.join(" ")));
                }
            }
            "_cssid" if typ == "string" => {
                let id = raw.as_str()?;
                if id.contains(['|', '\n', '\r']) {
                    return None;
                }
                if !id.is_empty() {
                    classic.insert("_element_id".into(), json!(id));
                }
            }
            "attributes" if typ == "attributes" => {
                let attributes =
                    atomic_attributes_to_classic(value, prop_schemas.get("attributes")?)?;
                if settings.contains_key("_cssid") && attributes.contains_key("_element_id") {
                    return None;
                }
                if attributes.contains_key("_attributes")
                    && !schema.classic_control_available(widget, "_attributes")
                {
                    return None;
                }
                classic.extend(attributes);
            }
            _ => return None,
        }
    }
    if !classic.contains_key(classic_key) {
        return None;
    }
    Some(
        json!({"id":element["id"], "elType":"widget", "widgetType":widget, "isInner":element["isInner"], "editor_settings":element.get("editor_settings").cloned().unwrap_or_else(||json!([])), "interactions":element.get("interactions").cloned().unwrap_or_else(||json!([])), "settings":classic, "elements":[]}),
    )
}

fn unsafe_subtree_reason(element: &Value) -> Option<String> {
    if element
        .get("interactions")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty())
    {
        return Some(
            "interactions are retained because no safe interaction mapping was supplied".into(),
        );
    }
    if contains_references(&element["settings"]) {
        return Some("dynamic bindings, template/component/global-style references, or custom logic need runtime resolution".into());
    }
    if element["elType"].as_str() == Some("widget")
        && !matches!(
            element["widgetType"].as_str(),
            Some(
                "heading"
                    | "text-editor"
                    | "image"
                    | "button"
                    | "e-heading"
                    | "e-paragraph"
                    | "e-image"
                    | "e-button"
            )
        )
    {
        return Some(
            "custom widget subtree retained because its runtime schema and behavior are unknown"
                .into(),
        );
    }
    if element["elType"] != "e-flexbox"
        && !element.get("styles").is_none_or(|styles| {
            styles.as_array().is_some_and(Vec::is_empty)
                || styles.as_object().is_some_and(Map::is_empty)
        })
    {
        return Some(
            "style variants are retained because their exact runtime schema is unavailable".into(),
        );
    }
    None
}

fn subtree_unsafe_reason(element: &Value) -> Option<String> {
    unsafe_subtree_reason(element)
        .or_else(|| contains_unparsed_custom_css(element).then(|| {
            "custom CSS is retained because raw selector and style-ID references cannot be safely normalized".into()
        }))
        .or_else(|| {
            let mut ids = HashSet::new();
            collect_subtree_ids(element, &mut ids);
            contains_internal_element_id_reference(element, &ids).then(|| {
                "subtree contains an internal element ID reference that cannot be safely remapped".into()
            })
        })
        .or_else(|| {
        element["elements"]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(subtree_unsafe_reason)
            .map(|reason| format!("subtree contains an unsupported element: {reason}"))
        })
}

fn contains_unparsed_custom_css(value: &Value) -> bool {
    match value {
        Value::Array(values) => values.iter().any(contains_unparsed_custom_css),
        Value::Object(fields) => fields.iter().any(|(key, child)| {
            (matches!(key.as_str(), "customCss" | "custom_css")
                && child.as_str().is_some_and(|css| !css.trim().is_empty()))
                || contains_unparsed_custom_css(child)
        }),
        _ => false,
    }
}

fn collect_subtree_ids(element: &Value, ids: &mut HashSet<String>) {
    if let Some(id) = element["id"].as_str() {
        ids.insert(id.to_owned());
    }
    for child in element["elements"].as_array().into_iter().flatten() {
        collect_subtree_ids(child, ids);
    }
}

fn contains_internal_element_id_reference(element: &Value, ids: &HashSet<String>) -> bool {
    match element {
        Value::Object(object) => object.iter().any(|(key, value)| {
            (key != "id" && value.as_str().is_some_and(|text| ids.contains(text)))
                || contains_internal_element_id_reference(value, ids)
        }),
        Value::Array(items) => items
            .iter()
            .any(|value| contains_internal_element_id_reference(value, ids)),
        _ => false,
    }
}

fn contains_references(value: &Value) -> bool {
    match value {
        Value::Array(items) => items.iter().any(contains_references),
        Value::Object(map) => map.iter().any(|(key, value)| {
            key.starts_with("__dynamic__")
                || matches!(
                    key.as_str(),
                    "global_classes"
                        | "component_id"
                        | "template_id"
                        | "_element_id"
                        | "custom_logic"
                )
                || value
                    .as_str()
                    .is_some_and(|text| text.contains("{{") || text.contains("[elementor-template"))
                || contains_references(value)
        }),
        _ => false,
    }
}

fn validate_ids(elements: &[Value]) -> Result<()> {
    fn walk(
        elements: &[Value],
        depth: usize,
        count: &mut usize,
        seen: &mut HashSet<String>,
    ) -> Result<()> {
        if depth > MAX_DEPTH {
            anyhow::bail!("element recursion exceeds {MAX_DEPTH}");
        }
        for element in elements {
            *count += 1;
            if *count > MAX_ELEMENTS {
                anyhow::bail!("element count exceeds {MAX_ELEMENTS}");
            }
            let id = element
                .get("id")
                .and_then(Value::as_str)
                .context("element id required")?;
            if !matches!(id.len(), 7 | 8) || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
                anyhow::bail!("invalid Elementor element id: {id}");
            }
            if !seen.insert(id.into()) {
                anyhow::bail!("duplicate Elementor element id: {id}");
            }
            let children = element
                .get("elements")
                .and_then(Value::as_array)
                .context("element elements array required")?;
            walk(children, depth + 1, count, seen)?;
        }
        Ok(())
    }
    walk(elements, 0, &mut 0, &mut HashSet::new())
}

#[derive(Debug, Clone)]
pub struct DesignSystemInput {
    pub context: PlanContext,
    pub native_api_available: bool,
    pub registered: Option<Value>,
    pub requested_patches: Vec<ResourcePatch>,
}

pub(super) fn valid_design_system_resource_id(
    kind: ResourceKind,
    id: &str,
    expected_revision: Option<&str>,
) -> bool {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return false;
    }
    match kind {
        ResourceKind::Class | ResourceKind::Variable => {
            id == "collection" && expected_revision != Some("absent")
        }
        ResourceKind::Kit => id == "settings" && expected_revision != Some("absent"),
        ResourceKind::PageSettings => {
            id.parse::<i64>().is_ok_and(|number| number > 0) && expected_revision != Some("absent")
        }
        ResourceKind::Template => {
            if id.parse::<i64>().is_ok_and(|number| number > 0) {
                return expected_revision != Some("absent");
            }
            if expected_revision.is_some_and(|revision| revision != "absent") {
                return false;
            }
            id == "new"
                || id.strip_prefix("new_pattern_").is_some_and(|suffix| {
                    suffix
                        .parse::<u32>()
                        .is_ok_and(|number| number >= 2 && number.to_string() == suffix)
                })
        }
        ResourceKind::PostMetadata | ResourceKind::Page => false,
    }
}

pub fn plan_elementor_design_system(input: DesignSystemInput) -> Result<ElementorPlan> {
    validate_context(&input.context)?;
    let mut seen_resources = HashSet::new();
    for patch in &input.requested_patches {
        if !valid_design_system_resource_id(
            patch.kind,
            &patch.id,
            patch.expected_revision.as_deref(),
        ) {
            anyhow::bail!("unsupported or invalid resource id for {:?}", patch.kind);
        }
        if !patch
            .expected_revision
            .as_deref()
            .is_some_and(|revision| !revision.trim().is_empty())
        {
            anyhow::bail!(
                "resource {:?}/{} is missing a revision",
                patch.kind,
                patch.id
            );
        }
        if !seen_resources.insert((patch.kind as u8, patch.id.as_str())) {
            anyhow::bail!(
                "duplicate design-system resource {:?}/{}",
                patch.kind,
                patch.id
            );
        }
    }
    if !input.native_api_available && !input.requested_patches.is_empty() {
        anyhow::bail!("native resource transactions are unavailable for requested patches");
    }
    let mut plan = empty_plan(&input.context);
    plan.source = Some(
        json!({"kind":"elementor_design_system", "native_api_available":input.native_api_available, "registered":input.registered}),
    );
    plan.resource_patches = input.requested_patches;
    plan.checks = Vec::new();
    Ok(plan)
}

#[derive(Debug, Clone)]
pub struct ExtractPatternsInput {
    pub context: PlanContext,
    pub elements: Vec<Value>,
    pub template_api_available: bool,
    pub template_revision: Option<String>,
    pub native_component_ability: Option<Value>,
}

pub fn plan_extract_elementor_patterns(input: ExtractPatternsInput) -> Result<ElementorPlan> {
    plan_extract_elementor_patterns_with_capability(input, false)
}

pub fn plan_extract_elementor_patterns_with_capability(
    input: ExtractPatternsInput,
    logical_template_create_ids_available: bool,
) -> Result<ElementorPlan> {
    validate_context(&input.context)?;
    validate_ids(&input.elements)?;
    let mut used_ids = HashSet::new();
    fn reserve_ids(element: &Value, used_ids: &mut HashSet<String>) {
        if let Some(id) = element["id"].as_str() {
            used_ids.insert(id.to_owned());
        }
        for child in element["elements"].as_array().into_iter().flatten() {
            reserve_ids(child, used_ids);
        }
    }
    for element in &input.elements {
        reserve_ids(element, &mut used_ids);
    }
    let mut candidates: BTreeMap<String, Vec<PatternInstance>> = BTreeMap::new();
    let mut retained = Vec::new();
    for (index, element) in input.elements.iter().enumerate() {
        collect_pattern_candidates(element, &mut candidates, &mut retained, &mut vec![index], 0)?;
    }
    let mut candidates = candidates
        .into_iter()
        .filter(|(_, instances)| instances.len() > 1)
        .collect::<Vec<_>>();
    candidates.sort_by(|(left, left_instances), (right, right_instances)| {
        let left_size = subtree_size(&left_instances[0].source);
        let right_size = subtree_size(&right_instances[0].source);
        right_size.cmp(&left_size).then_with(|| left.cmp(right))
    });
    let mut accepted_paths: Vec<Vec<usize>> = Vec::new();
    let mut groups = Vec::new();
    for (signature, instances) in candidates {
        if instances.iter().any(|instance| {
            accepted_paths
                .iter()
                .any(|path| paths_overlap(path, &instance.path))
        }) {
            continue;
        }
        accepted_paths.extend(instances.iter().map(|instance| instance.path.clone()));
        let representative = remapped_pattern_representative(&instances[0].source, &mut used_ids);
        groups.push(json!({
            "signature":signature,
            "representative":representative,
            "instances":instances.iter().map(|instance| json!({
                "id":instance.source["id"],
                "bindings":pattern_bindings(&instance.source),
            })).collect::<Vec<_>>(),
        }));
    }
    let multi_template_creation_available = logical_template_create_ids_available;
    let mut plan = empty_plan(&input.context);
    plan.retained = retained;
    plan.source = Some(
        json!({"kind":"pattern_extraction", "groups":groups, "template_api_available":input.template_api_available, "multi_template_creation_available":multi_template_creation_available, "synchronization":"portable_template_only", "synchronized_components":{"available":false,"discovered_ability":input.native_component_ability,"reason":"No checked native Component adapter is implemented; extraction emits portable templates."}, "instance_content_preserved_as_bindings":true, "document_changed":false}),
    );
    if input.template_api_available {
        if let Some(revision) = input.template_revision {
            let groups = plan
                .source
                .as_ref()
                .and_then(|source| source["groups"].as_array())
                .into_iter()
                .flatten()
                .filter_map(|group| {
                    let representative = group.get("representative")?.clone();
                    let document = vec![representative];
                    Some((group.clone(), document))
                })
                .collect::<Vec<_>>();
            for (index, (group, document)) in groups.into_iter().enumerate() {
                if index > 0 && !multi_template_creation_available {
                    plan.resource_patches.clear();
                    break;
                }
                plan.resource_patches.push(ResourcePatch {
                    kind: ResourceKind::Template,
                    id: if index == 0 {
                        "new".into()
                    } else {
                        format!("new_pattern_{}", index + 1)
                    },
                    expected_revision: Some(revision.clone()),
                    before: None,
                    after: json!({"status":"draft","type":"section","title":format!("Extracted pattern {}", index + 1),"document":document,"source_group":group["signature"]}),
                });
            }
        }
    }
    plan.checks = Vec::new();
    Ok(plan)
}

#[derive(Debug)]
struct PatternInstance {
    path: Vec<usize>,
    source: Value,
}

fn remapped_pattern_representative(source: &Value, used_ids: &mut HashSet<String>) -> Value {
    fn collect_ids(
        element: &Value,
        used_ids: &mut HashSet<String>,
        replacements: &mut HashMap<String, String>,
    ) {
        let old_id = element["id"].as_str().unwrap_or_default();
        let mut new_id = crate::elementor::generate_id();
        while used_ids.contains(&new_id) {
            new_id = crate::elementor::generate_id();
        }
        used_ids.insert(new_id.clone());
        replacements.insert(old_id.to_owned(), new_id);
        for child in element["elements"].as_array().into_iter().flatten() {
            collect_ids(child, used_ids, replacements);
        }
    }

    fn apply_ids(element: &mut Value, replacements: &HashMap<String, String>) {
        if let Some(old_id) = element["id"].as_str()
            && let Some(new_id) = replacements.get(old_id)
        {
            element["id"] = json!(new_id);
        }
        if element["elType"] == "container" && element.get("isInner").is_none() {
            element["isInner"] = json!(false);
        }
        for child in element["elements"].as_array_mut().into_iter().flatten() {
            apply_ids(child, replacements);
        }
    }

    let mut replacements = HashMap::new();
    collect_ids(source, used_ids, &mut replacements);
    let mut representative = source.clone();
    apply_ids(&mut representative, &replacements);
    representative
}

fn pattern_bindings(element: &Value) -> Value {
    fn descendants(element: &Value) -> Vec<Value> {
        element["elements"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|child| {
                json!({
                    "id": child["id"],
                    "settings": child["settings"],
                    "elements": descendants(child),
                })
            })
            .collect()
    }

    json!({"settings":element["settings"], "elements":descendants(element)})
}

fn collect_pattern_candidates(
    element: &Value,
    groups: &mut BTreeMap<String, Vec<PatternInstance>>,
    retained: &mut Vec<RetainedElement>,
    path: &mut Vec<usize>,
    depth: usize,
) -> Result<()> {
    if depth > MAX_DEPTH {
        anyhow::bail!("element recursion exceeds {MAX_DEPTH}");
    }
    if let Some(reason) = subtree_unsafe_reason(element) {
        retained.push(RetainedElement {
            element_id: element["id"].as_str().unwrap_or_default().to_owned(),
            reason,
        });
        return Ok(());
    } else if let Some(signature) = pattern_signature(element) {
        groups.entry(signature).or_default().push(PatternInstance {
            path: path.clone(),
            source: element.clone(),
        });
    } else {
        retained.push(RetainedElement {
            element_id: element["id"].as_str().unwrap_or_default().to_owned(),
            reason: "native local style definitions or references are invalid or cannot be safely normalized".into(),
        });
        return Ok(());
    }
    if let Some(children) = element["elements"].as_array() {
        for (index, child) in children.iter().enumerate() {
            path.push(index);
            collect_pattern_candidates(child, groups, retained, path, depth + 1)?;
            path.pop();
        }
    }
    Ok(())
}

fn pattern_signature(element: &Value) -> Option<String> {
    if contains_unparsed_custom_css(element) {
        return None;
    }
    fn normalize_element(element: &Value) -> Option<Value> {
        let mut normalized = element.clone();
        if let Some(object) = normalized.as_object_mut() {
            object.remove("id");
            let mut local_ids = Vec::new();
            match object.get("styles") {
                Some(Value::Object(definitions)) => {
                    if !definitions.is_empty() {
                        native_styles_by_id(element)?;
                        local_ids.extend(definitions.keys().cloned());
                    }
                }
                Some(Value::Array(definitions)) => {
                    if !definitions.is_empty() {
                        native_styles_by_id(element)?;
                        local_ids.extend(
                            definitions
                                .iter()
                                .map(|definition| definition["id"].as_str().map(str::to_owned))
                                .collect::<Option<Vec<_>>>()?,
                        );
                    }
                }
                Some(Value::Null) | None => {}
                Some(_) => return None,
            }
            if !local_ids.is_empty() {
                let renamed = local_ids
                    .iter()
                    .enumerate()
                    .map(|(index, id)| (id.clone(), format!("local-class-{index}")))
                    .collect::<HashMap<_, _>>();
                match object.get_mut("styles")? {
                    Value::Object(definitions) => {
                        let mut rewritten = Map::new();
                        for (id, definition) in std::mem::take(definitions) {
                            let new_id = renamed.get(&id)?;
                            let mut definition = definition;
                            definition["id"] = json!(new_id);
                            if let Some(label) = definition.get_mut("label") {
                                *label = Value::Null;
                            }
                            rewritten.insert(new_id.clone(), definition);
                        }
                        *definitions = rewritten;
                    }
                    Value::Array(definitions) => {
                        for definition in definitions {
                            let id = definition["id"].as_str()?.to_owned();
                            definition["id"] = json!(renamed.get(&id)?);
                            if let Some(label) = definition.get_mut("label") {
                                *label = Value::Null;
                            }
                        }
                    }
                    _ => return None,
                }
                let classes = object
                    .get_mut("settings")
                    .and_then(Value::as_object_mut)
                    .and_then(|settings| settings.get_mut("classes"));
                let classes = classes?;
                if classes["$$type"] != "classes" {
                    return None;
                }
                let class_ids = classes.get_mut("value")?.as_array_mut()?;
                for class_id in class_ids {
                    let id = class_id.as_str()?;
                    if let Some(new_id) = renamed.get(id) {
                        *class_id = json!(new_id);
                    }
                }
            }
            if let Some(settings) = object.get_mut("settings").and_then(Value::as_object_mut) {
                for key in [
                    "title", "text", "editor", "image", "link", "url", "alt", "caption", "src",
                    "content", "media",
                ] {
                    if let Some(value) = settings.get_mut(key) {
                        *value = Value::Null;
                    }
                }
                if let Some(attributes) = settings
                    .get_mut("attributes")
                    .and_then(Value::as_object_mut)
                {
                    for key in ["title", "alt", "aria-label"] {
                        if let Some(value) = attributes.get_mut(key) {
                            *value = Value::Null;
                        }
                    }
                }
            }
            if let Some(title) = object
                .get_mut("editor_settings")
                .and_then(Value::as_object_mut)
                .and_then(|editor_settings| editor_settings.get_mut("title"))
            {
                *title = Value::Null;
            }
            if let Some(children) = object.get_mut("elements").and_then(Value::as_array_mut) {
                for child in children {
                    *child = normalize_element(child)?;
                }
            }
        }
        Some(normalized)
    }
    let mut normalized = normalize_element(element)?;
    canonicalize_pattern_fields(&mut normalized, false);
    Some(serde_json::to_string(&normalized).ok()?)
}

fn canonicalize_pattern_fields(value: &mut Value, style_definition_map: bool) {
    match value {
        Value::Array(values) => values
            .iter_mut()
            .for_each(|value| canonicalize_pattern_fields(value, false)),
        Value::Object(fields) => {
            for (key, value) in fields.iter_mut() {
                canonicalize_pattern_fields(value, key == "styles");
            }
            if !style_definition_map {
                let mut entries = std::mem::take(fields).into_iter().collect::<Vec<_>>();
                entries.sort_by(|left, right| left.0.cmp(&right.0));
                fields.extend(entries);
            }
        }
        _ => {}
    }
}

fn subtree_size(element: &Value) -> usize {
    1 + element["elements"]
        .as_array()
        .map_or(0, |children| children.iter().map(subtree_size).sum())
}

fn paths_overlap(left: &[usize], right: &[usize]) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn validate_context(context: &PlanContext) -> Result<()> {
    if context.site_url.len() > 2048
        || !(context.site_url.starts_with("http://") || context.site_url.starts_with("https://"))
    {
        anyhow::bail!("site_url must be an http(s) URL");
    }
    if context.page_id == 0
        || context.expected_revision.trim().is_empty()
        || context.expected_revision.len() > 256
        || context.operation_id.trim().is_empty()
        || context.operation_id.len() > 128
    {
        anyhow::bail!("page_id, expected_revision and operation_id are required");
    }
    Ok(())
}

fn valid_element_id(id: &str) -> bool {
    matches!(id.len(), 7 | 8) && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn empty_plan(context: &PlanContext) -> ElementorPlan {
    ElementorPlan {
        site_url: context.site_url.clone(),
        page_id: context.page_id,
        expected_revision: context.expected_revision.clone(),
        operation_id: context.operation_id.clone(),
        document_patch: None,
        resource_patches: Vec::new(),
        retained: Vec::new(),
        source: None,
        checks: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_style_merge_preserves_native_map_and_normalizes_legacy_arrays() {
        let original = json!({
            "id":"native-style",
            "type":"class",
            "label":"native style",
            "variants":[
                {"meta":{"breakpoint":"desktop","state":null},"props":{"width":{"$$type":"size","value":{"size":320,"unit":"px"}}}},
                {"meta":{"breakpoint":"mobile","state":null},"props":{"width":{"$$type":"size","value":{"size":280,"unit":"px"}}}},
                {"meta":{"breakpoint":"desktop","state":"hover"},"props":{"opacity":{"$$type":"number","value":0.8}}}
            ]
        });
        let base = json!({
            "id":"17f82ce",
            "elType":"e-flexbox",
            "settings":{"classes":{"$$type":"classes","value":["native-style"]}},
            "styles":{"native-style":original},
            "elements":[]
        });
        let variants = vec![
            json!({"meta":{"breakpoint":"tablet","state":null},"props":{"width":{"$$type":"size","value":{"size":60,"unit":"%"}}}}),
        ];
        let mut native_map = vec![base.clone()];
        assert!(merge_element_style_variants(
            &mut native_map,
            "17f82ce",
            variants.clone()
        ));
        assert_eq!(
            native_map[0]["styles"]["native-style"],
            base["styles"]["native-style"]
        );
        assert_eq!(
            native_map[0]["styles"]["mcp-css-17f82ce"]["variants"][0]["meta"]["breakpoint"],
            "tablet"
        );
        assert!(
            native_map[0]["settings"]["classes"]["value"]
                .as_array()
                .unwrap()
                .iter()
                .any(|class| class == "mcp-css-17f82ce")
        );

        let mut legacy = base;
        legacy["styles"] = json!([original]);
        assert!(merge_element_style_variants(
            std::slice::from_mut(&mut legacy),
            "17f82ce",
            variants
        ));
        assert!(legacy["styles"].is_object());
        assert_eq!(
            legacy["styles"]["native-style"]["variants"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn css_style_merge_rejects_ambiguous_or_unbound_style_ids_without_partial_binding() {
        let definition = json!({"id":"native-style","type":"class","variants":[{"meta":{"breakpoint":"desktop","state":null},"props":{"width":{"$$type":"size","value":{"size":320,"unit":"px"}}}}]});
        let cases = [
            json!({"wrong-key":definition}),
            json!({"native-style":definition}),
        ];
        for styles in cases {
            let mut node = json!({"id":"17f82ce","settings":{"classes":{"$$type":"classes","value":[]}},"styles":styles,"elements":[]});
            let before = node.clone();
            assert!(!merge_element_style_variants(
                std::slice::from_mut(&mut node),
                "17f82ce",
                vec![
                    json!({"meta":{"breakpoint":"tablet","state":null},"props":{"width":{"$$type":"size","value":{"size":60,"unit":"%"}}}})
                ]
            ));
            assert_eq!(node, before);
        }

        let duplicate = json!({"id":"native-style","type":"class","variants":[{"meta":{"breakpoint":"desktop","state":null},"props":{"width":{"$$type":"size","value":{"size":320,"unit":"px"}}}}]});
        let mut node = json!({"id":"17f82ce","settings":{"classes":{"$$type":"classes","value":["native-style"]}},"styles":[duplicate,duplicate],"elements":[]});
        let before = node.clone();
        assert!(!merge_element_style_variants(
            std::slice::from_mut(&mut node),
            "17f82ce",
            vec![
                json!({"meta":{"breakpoint":"tablet","state":null},"props":{"width":{"$$type":"size","value":{"size":60,"unit":"%"}}}})
            ]
        ));
        assert_eq!(node, before);
    }

    fn context() -> PlanContext {
        PlanContext {
            site_url: "https://example.test".into(),
            page_id: 12,
            expected_revision: "rev-1".into(),
            operation_id: "plan-1".into(),
        }
    }

    fn resource_patch(
        kind: crate::types::elementor_plan::ResourceKind,
        id: &str,
        revision: Option<&str>,
    ) -> crate::types::elementor_plan::ResourcePatch {
        crate::types::elementor_plan::ResourcePatch {
            kind,
            id: id.into(),
            expected_revision: revision.map(str::to_owned),
            before: None,
            after: json!({}),
        }
    }

    #[test]
    fn design_system_rejects_incomplete_or_invalid_resource_batches() {
        use crate::types::elementor_plan::ResourceKind;

        let complete = resource_patch(ResourceKind::Class, "collection", Some("rev-2"));
        let incomplete = resource_patch(ResourceKind::Variable, "var-a", None);
        assert!(
            plan_elementor_design_system(DesignSystemInput {
                context: context(),
                native_api_available: true,
                registered: None,
                requested_patches: vec![complete.clone(), incomplete],
            })
            .is_err()
        );

        for invalid in [
            resource_patch(ResourceKind::Class, "", Some("rev-2")),
            resource_patch(ResourceKind::Kit, "kit-a", Some("  ")),
            resource_patch(ResourceKind::Page, "page-a", Some("rev-2")),
        ] {
            assert!(
                plan_elementor_design_system(DesignSystemInput {
                    context: context(),
                    native_api_available: true,
                    registered: None,
                    requested_patches: vec![invalid],
                })
                .is_err()
            );
        }

        assert!(
            plan_elementor_design_system(DesignSystemInput {
                context: context(),
                native_api_available: true,
                registered: None,
                requested_patches: vec![complete.clone(), complete],
            })
            .is_err()
        );

        assert!(
            plan_elementor_design_system(DesignSystemInput {
                context: context(),
                native_api_available: false,
                registered: None,
                requested_patches: vec![resource_patch(
                    ResourceKind::Class,
                    "collection",
                    Some("rev-2")
                )],
            })
            .is_err()
        );
    }

    #[test]
    fn design_system_resource_ids_match_bridge_write_scopes() {
        use crate::types::elementor_plan::ResourceKind;

        let valid = [
            resource_patch(ResourceKind::Class, "collection", Some("rev-class")),
            resource_patch(ResourceKind::Variable, "collection", Some("rev-vars")),
            resource_patch(ResourceKind::Kit, "settings", Some("rev-kit")),
            resource_patch(ResourceKind::PageSettings, "123", Some("rev-page")),
            resource_patch(ResourceKind::Template, "456", Some("rev-template")),
            resource_patch(ResourceKind::Template, "new", Some("absent")),
            resource_patch(ResourceKind::Template, "new_pattern_2", Some("absent")),
        ];
        assert!(
            plan_elementor_design_system(DesignSystemInput {
                context: context(),
                native_api_available: true,
                registered: None,
                requested_patches: valid.to_vec(),
            })
            .is_ok()
        );

        let invalid = [
            resource_patch(ResourceKind::Class, "collection/../kit", Some("rev")),
            resource_patch(ResourceKind::Class, "collection?x=1", Some("rev")),
            resource_patch(ResourceKind::Variable, "variable-1", Some("rev")),
            resource_patch(ResourceKind::Kit, "active", Some("rev")),
            resource_patch(ResourceKind::PageSettings, "0", Some("rev")),
            resource_patch(ResourceKind::PageSettings, "12/../13", Some("rev")),
            resource_patch(
                ResourceKind::PageSettings,
                "9".repeat(128).as_str(),
                Some("rev"),
            ),
            resource_patch(ResourceKind::Template, "new_pattern_1", Some("absent")),
            resource_patch(ResourceKind::Template, "new_pattern_02", Some("absent")),
            resource_patch(
                ResourceKind::Template,
                "new_pattern_4294967296",
                Some("absent"),
            ),
            resource_patch(
                ResourceKind::Template,
                "9".repeat(128).as_str(),
                Some("rev"),
            ),
        ];
        for patch in invalid {
            assert!(
                plan_elementor_design_system(DesignSystemInput {
                    context: context(),
                    native_api_available: true,
                    registered: None,
                    requested_patches: vec![patch],
                })
                .is_err()
            );
        }
    }

    #[test]
    fn pattern_extraction_keeps_style_variants_separate_and_emits_one_representative() {
        let plan = plan_extract_elementor_patterns(ExtractPatternsInput {
            context: context(),
            elements: vec![
                json!({"id":"17f82ce","elType":"widget","widgetType":"heading","settings":{"title":"First","align":"left","_css_classes":"hero"},"elements":[]}),
                json!({"id":"27f82ce","elType":"widget","widgetType":"heading","settings":{"title":"Second","align":"left","_css_classes":"hero"},"elements":[]}),
                json!({"id":"37f82ce","elType":"widget","widgetType":"heading","settings":{"title":"Third","align":"right","_css_classes":"hero"},"elements":[]}),
            ],
            template_api_available: true,
            template_revision: Some("absent".into()),
            native_component_ability: None,
        }).unwrap();
        let groups = plan.source.as_ref().unwrap()["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0]["instances"].as_array().unwrap().len(), 2);
        assert_eq!(groups[0]["representative"]["settings"]["align"], "left");
        assert_eq!(
            groups[0]["instances"][1]["bindings"]["settings"]["title"],
            "Second"
        );
        assert_eq!(plan.resource_patches.len(), 1);
        assert_eq!(
            plan.resource_patches[0].after["document"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(plan.document_patch.is_none());
    }

    #[test]
    fn pattern_extraction_uses_stable_create_ids_for_distinct_groups() {
        let elements = ["left", "right"]
            .into_iter()
            .flat_map(|align| {
                ["First", "Second"].into_iter().map(move |title| {
                    json!({"id":format!("{}{}", if title == "First" { "1" } else { "2" }, if align == "left" { "7f82ce" } else { "8f82ce" }),"elType":"widget","widgetType":"heading","settings":{"title":title,"align":align},"elements":[]})
                })
            })
            .collect();
        let plan = plan_extract_elementor_patterns_with_capability(
            ExtractPatternsInput {
                context: context(),
                elements,
                template_api_available: true,
                template_revision: Some("absent".into()),
                native_component_ability: None,
            },
            true,
        )
        .unwrap();
        assert_eq!(
            plan.source.as_ref().unwrap()["groups"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            plan.resource_patches
                .iter()
                .map(|patch| patch.id.as_str())
                .collect::<Vec<_>>(),
            ["new", "new_pattern_2"]
        );
        assert!(
            plan.resource_patches
                .iter()
                .all(|patch| patch.expected_revision.as_deref() == Some("absent")
                    && patch.before.is_none())
        );
        assert!(plan.document_patch.is_none());
    }

    #[test]
    fn pattern_extraction_withholds_multi_create_plan_without_backend_capability() {
        let elements = ["left", "right"]
            .into_iter()
            .flat_map(|align| ["First", "Second"].into_iter().map(move |title| {
                json!({"id":format!("{}{}", if title == "First" { "1" } else { "2" }, if align == "left" { "7f82ce" } else { "8f82ce" }),"elType":"widget","widgetType":"heading","settings":{"title":title,"align":align},"elements":[]})
            })).collect();
        let plan = plan_extract_elementor_patterns(ExtractPatternsInput {
            context: context(),
            elements,
            template_api_available: true,
            template_revision: Some("absent".into()),
            native_component_ability: None,
        })
        .unwrap();
        assert_eq!(
            plan.source.as_ref().unwrap()["groups"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(plan.resource_patches.is_empty());
    }

    #[test]
    fn pattern_container_representative_includes_native_default() {
        let source = json!({"id":"17f82ce","elType":"container","settings":{"_css_classes":"card"},"elements":[{"id":"27f82ce","elType":"container","settings":{},"elements":[]},{"id":"37f82ce","elType":"container","isInner":true,"settings":{},"elements":[]}]});
        let mut used_ids = HashSet::new();
        let representative = remapped_pattern_representative(&source, &mut used_ids);
        assert_eq!(representative["isInner"], false);
        assert_eq!(representative["settings"], source["settings"]);
        assert_eq!(representative["elements"][0]["isInner"], false);
        assert_eq!(representative["elements"][1]["isInner"], true);
    }

    #[test]
    fn selected_conversion_preserves_unselected_tree_and_custom_sibling() {
        let schema = native_button_schema();
        let source = vec![
            json!({"id":"17f82ce","elType":"container","settings":{"flex_direction":"row"},"elements":[
                {"id":"27f82ce","elType":"widget","widgetType":"button","settings":{"text":"Read more","link":{"url":"https://example.test/guide"}},"elements":[]},
                {"id":"37f82ce","elType":"widget","widgetType":"custom-widget","settings":{"vendor":{"state":"keep"}},"elements":[]}
            ]}),
        ];
        let plan = plan_convert_elements_for_ids(
            ConvertElementsInput {
                context: context(),
                direction: "classic_to_atomic".into(),
                elements: source.clone(),
                schemas: Some(schema),
            },
            Some(vec!["27f82ce".into()]),
        )
        .unwrap();
        let patch = plan.document_patch.unwrap();
        assert_eq!(patch[0]["id"], "17f82ce");
        assert_eq!(patch[0]["settings"], source[0]["settings"]);
        assert_eq!(patch[0]["elements"][0]["widgetType"], "e-button");
        assert_eq!(patch[0]["elements"][1], source[0]["elements"][1]);
        assert!(plan.retained.is_empty());
    }

    #[test]
    fn selected_conversion_rejects_invalid_targets_before_building_a_patch() {
        let tree = vec![
            json!({"id":"17f82ce","elType":"container","settings":{},"elements":[
                {"id":"27f82ce","elType":"widget","widgetType":"heading","settings":{"title":"x"},"elements":[]}
            ]}),
        ];
        let plan = |element_ids| {
            plan_convert_elements_for_ids(
                ConvertElementsInput {
                    context: context(),
                    direction: "classic_to_atomic".into(),
                    elements: tree.clone(),
                    schemas: None,
                },
                element_ids,
            )
        };
        assert!(plan(Some(vec![])).is_err());
        assert!(plan(Some(vec!["missing1".into()])).is_err());
        assert!(plan(Some(vec!["27f82ce".into(), "27f82ce".into()])).is_err());
        assert!(plan(Some(vec!["17f82ce".into(), "27f82ce".into()])).is_err());
    }

    #[test]
    fn selected_unsupported_subtree_is_retained_without_executable_patch() {
        let tree = vec![
            json!({"id":"17f82ce","elType":"container","settings":{},"elements":[
                {"id":"27f82ce","elType":"widget","widgetType":"custom-widget","settings":{"vendor":{"mode":"preserve"}},"elements":[
                    {"id":"37f82ce","elType":"widget","widgetType":"heading","settings":{"title":"Nested"},"elements":[]}
                ]}
            ]}),
        ];
        let plan = plan_convert_elements_for_ids(
            ConvertElementsInput {
                context: context(),
                direction: "classic_to_atomic".into(),
                elements: tree.clone(),
                schemas: None,
            },
            Some(vec!["27f82ce".into()]),
        )
        .unwrap();
        assert!(plan.document_patch.is_none());
        assert_eq!(plan.retained.len(), 1);
        assert_eq!(plan.retained[0].element_id, "27f82ce");
        assert_eq!(
            tree[0]["elements"][0]["elements"][0]["settings"]["title"],
            "Nested"
        );
    }

    #[test]
    fn selected_element_already_in_requested_form_is_retained_unchanged() {
        let source = vec![
            json!({"id":"17f82ce","elType":"widget","widgetType":"e-heading","version":"4.2.3","settings":{"title":{"$$type":"html-v3","value":{"content":{"$$type":"string","value":"Keep me"}}},"vendor":"keep"},"elements":[]}),
        ];
        let plan = plan_convert_elements_for_ids(
            ConvertElementsInput {
                context: context(),
                direction: "classic_to_atomic".into(),
                elements: source.clone(),
                schemas: None,
            },
            Some(vec!["17f82ce".into()]),
        )
        .unwrap();
        assert!(plan.document_patch.is_none());
        assert_eq!(plan.retained.len(), 1);
        assert!(plan.retained[0].reason.contains("already in the requested"));
        assert_eq!(source[0]["settings"]["vendor"], "keep");
    }

    fn patterned_atomic(
        id: &str,
        style_id: &str,
        title: &str,
        editor_title: &str,
        color: &str,
    ) -> Value {
        json!({
            "id":id, "version":"4.2.3", "elType":"e-flexbox", "settings":{"classes":{"$$type":"classes","value":[style_id,"global-card"]},"title":title},
            "editor_settings":{"title":editor_title}, "elements":[],
            "styles":{
                (style_id):{"id":style_id,"type":"class","label":format!("Navigator {editor_title}"),"variants":[
                    {"meta":{"breakpoint":"desktop","state":null},"props":{"color":{"$$type":"color","value":color}}},
                    {"meta":{"breakpoint":"mobile","state":null},"props":{"width":{"$$type":"size","value":{"size":100,"unit":"%"}}}},
                    {"meta":{"breakpoint":"desktop","state":"hover"},"props":{"color":{"$$type":"color","value":"#ffffff"}}}
                ]}
            }
        })
    }

    #[test]
    fn native_pattern_signatures_rename_local_styles_and_preserve_instance_bindings() {
        let first = patterned_atomic(
            "17f82ce",
            "local-blue",
            "First content",
            "First Navigator",
            "#006a82",
        );
        let second = patterned_atomic(
            "27f82ce",
            "local-green",
            "Second content",
            "Second Navigator",
            "#006a82",
        );
        assert_eq!(pattern_signature(&first), pattern_signature(&second));
        let plan = plan_extract_elementor_patterns(ExtractPatternsInput {
            context: context(),
            elements: vec![first, second],
            template_api_available: false,
            template_revision: None,
            native_component_ability: None,
        })
        .unwrap();
        let group = &plan.source.as_ref().unwrap()["groups"][0];
        assert_eq!(group["instances"].as_array().unwrap().len(), 2);
        assert_eq!(
            group["instances"][0]["bindings"]["settings"]["title"],
            "First content"
        );
        assert_eq!(
            group["instances"][1]["bindings"]["settings"]["title"],
            "Second content"
        );
        assert_eq!(
            group["representative"]["settings"]["classes"]["value"][0],
            "local-blue"
        );
        assert_eq!(
            group["representative"]["styles"]["local-blue"]["id"],
            "local-blue"
        );
        assert_eq!(
            group["representative"]["settings"]["classes"]["value"][1],
            "global-card"
        );
        assert_eq!(
            group["representative"]["styles"]["local-blue"]["variants"][1]["meta"]["breakpoint"],
            "mobile"
        );
        assert_eq!(
            group["representative"]["styles"]["local-blue"]["variants"][2]["meta"]["state"],
            "hover"
        );
    }

    #[test]
    fn native_pattern_signatures_keep_global_classes_declarations_and_order_distinct() {
        let base = patterned_atomic("17f82ce", "local-one", "Content", "Navigator", "#006a82");
        let mut global_class =
            patterned_atomic("27f82ce", "local-two", "Other", "Other", "#006a82");
        global_class["settings"]["classes"]["value"][1] = json!("global-other");
        let declaration = patterned_atomic("37f82ce", "local-three", "Third", "Third", "#ff0000");
        let mut state = patterned_atomic("47f82ce", "local-four", "Fourth", "Fourth", "#006a82");
        state["styles"]["local-four"]["variants"][2]["meta"]["state"] = json!("focus");
        let mut order = patterned_atomic("57f82ce", "local-five", "Fifth", "Fifth", "#006a82");
        order["styles"]["local-five"]["variants"]
            .as_array_mut()
            .unwrap()
            .swap(0, 1);
        assert_ne!(pattern_signature(&base), pattern_signature(&global_class));
        assert_ne!(pattern_signature(&base), pattern_signature(&declaration));
        assert_ne!(pattern_signature(&base), pattern_signature(&state));
        assert_ne!(pattern_signature(&base), pattern_signature(&order));
    }

    #[test]
    fn native_pattern_signatures_retain_unsafe_id_references_and_css() {
        let mut css = patterned_atomic("17f82ce", "local-one", "Content", "Navigator", "#006a82");
        css["settings"]["customCss"] = json!("#17f82ce { color: red; }");
        assert!(pattern_signature(&css).is_none());
        assert!(subtree_unsafe_reason(&css).is_some());
        let mut custom_logic =
            patterned_atomic("27f82ce", "local-two", "Content", "Navigator", "#006a82");
        custom_logic["settings"]["custom_logic"] = json!({"expression":"custom"});
        assert!(subtree_unsafe_reason(&custom_logic).is_some());

        let mut invalid_definition =
            patterned_atomic("37f82ce", "local-three", "Content", "Navigator", "#006a82");
        invalid_definition["styles"]["local-three"]["id"] = json!("local-mismatch");
        let plan = plan_extract_elementor_patterns(ExtractPatternsInput {
            context: context(),
            elements: vec![invalid_definition],
            template_api_available: false,
            template_revision: None,
            native_component_ability: None,
        })
        .unwrap();
        assert_eq!(plan.retained[0].element_id, "37f82ce");
        assert!(
            plan.source.as_ref().unwrap()["groups"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn pattern_signatures_canonicalize_fields_but_preserve_local_style_order() {
        let first = patterned_atomic("17f82ce", "local-one", "Content", "Navigator", "#006a82");
        let reordered: Value = serde_json::from_str(r##"{"styles":{"local-two":{"id":"local-two","type":"class","label":"Navigator Navigator","variants":[{"meta":{"breakpoint":"desktop","state":null},"props":{"color":{"$$type":"color","value":"#006a82"}}},{"meta":{"breakpoint":"mobile","state":null},"props":{"width":{"$$type":"size","value":{"size":100,"unit":"%"}}}},{"meta":{"breakpoint":"desktop","state":"hover"},"props":{"color":{"$$type":"color","value":"#ffffff"}}}]}},"elements":[],"editor_settings":{"title":"Other Navigator"},"elType":"e-flexbox","version":"4.2.3","settings":{"title":"Other content","classes":{"value":["local-two","global-card"],"$$type":"classes"}},"id":"27f82ce"}"##).unwrap();
        assert_eq!(pattern_signature(&first), pattern_signature(&reordered));

        let ordered = json!({
            "id":"37f82ce", "elType":"e-flexbox", "settings":{"classes":{"$$type":"classes","value":["local-a","local-b"]}}, "elements":[],
            "styles":{
                "local-a":{"id":"local-a","type":"class","label":"A","variants":[{"meta":{"breakpoint":"desktop","state":null},"props":{"width":{"$$type":"size","value":{"size":20,"unit":"px"}}}}]},
                "local-b":{"id":"local-b","type":"class","label":"B","variants":[{"meta":{"breakpoint":"desktop","state":null},"props":{"width":{"$$type":"size","value":{"size":40,"unit":"px"}}}}]}
            }
        });
        let mut reversed = ordered.clone();
        let style_entries = ordered["styles"].as_object().unwrap().clone();
        let reversed_styles = Map::from_iter(style_entries.into_iter().rev());
        reversed["styles"] = Value::Object(reversed_styles);
        assert_ne!(pattern_signature(&ordered), pattern_signature(&reversed));
    }

    fn primitive(kind: &str, key: &str) -> Value {
        json!({"kind":kind,"key":key})
    }
    fn union(options: Map<String, Value>) -> Value {
        json!({"kind":"union","prop_types":options})
    }
    fn object(key: &str, shape: Map<String, Value>) -> Value {
        json!({"kind":"object","key":key,"shape":shape})
    }

    fn native_image_schema() -> Value {
        let mut src = Map::new();
        src.insert(
            "id".into(),
            union(Map::from_iter([(
                "image-attachment-id".into(),
                primitive("number", "image-attachment-id"),
            )])),
        );
        src.insert(
            "url".into(),
            union(Map::from_iter([("url".into(), primitive("string", "url"))])),
        );
        src.insert(
            "alt".into(),
            union(Map::from_iter([(
                "string".into(),
                primitive("string", "string"),
            )])),
        );
        let mut image = Map::new();
        image.insert(
            "src".into(),
            union(Map::from_iter([(
                "image-src".into(),
                object("image-src", src),
            )])),
        );
        image.insert(
            "size".into(),
            union(Map::from_iter([(
                "string".into(),
                primitive("string", "string"),
            )])),
        );
        union(Map::from_iter([("image".into(), object("image", image))]))
    }

    fn native_link_schema() -> Value {
        let mut link = Map::new();
        link.insert(
            "destination".into(),
            union(Map::from_iter([("url".into(), primitive("string", "url"))])),
        );
        link.insert(
            "isTargetBlank".into(),
            union(Map::from_iter([(
                "boolean".into(),
                primitive("boolean", "boolean"),
            )])),
        );
        link.insert(
            "tag".into(),
            union(Map::from_iter([(
                "string".into(),
                primitive("string", "string"),
            )])),
        );
        union(Map::from_iter([("link".into(), object("link", link))]))
    }

    fn native_attributes_schema() -> Value {
        union(Map::from_iter([(
            "attributes".into(),
            json!({
                "kind":"array",
                "key":"attributes",
                "item_prop_type":object("key-value", Map::from_iter([
                    ("key".into(), primitive("string", "string")),
                    ("value".into(), primitive("string", "string")),
                ])),
            }),
        )]))
    }

    fn native_layout_schema() -> AtomicRuntimeSchema {
        let class_schema = json!({"kind":"plain","key":"classes"});
        let mut styles = Map::new();
        for key in ["display", "flex-direction"] {
            styles.insert(
                key.into(),
                union(Map::from_iter([(
                    "string".into(),
                    primitive("string", "string"),
                )])),
            );
        }
        styles.insert(
            "width".into(),
            json!({
                "kind":"object",
                "key":"size",
                "shape":{
                    "size":primitive("number", "number"),
                    "unit":primitive("string", "string"),
                },
            }),
        );
        AtomicRuntimeSchema {
            widget_props: HashMap::from([(
                "e-flexbox".into(),
                HashMap::from([
                    ("classes".into(), "classes".into()),
                    ("_cssid".into(), "string".into()),
                    ("attributes".into(), "attributes".into()),
                    ("tag".into(), "string".into()),
                    ("link".into(), "link".into()),
                ]),
            )]),
            widget_prop_schemas: HashMap::from([(
                "e-flexbox".into(),
                Map::from_iter([
                    ("classes".into(), class_schema),
                    ("_cssid".into(), primitive("string", "string")),
                    ("attributes".into(), native_attributes_schema()),
                    (
                        "tag".into(),
                        union(Map::from_iter([(
                            "string".into(),
                            primitive("string", "string"),
                        )])),
                    ),
                    ("link".into(), native_link_schema()),
                ]),
            )]),
            element_versions: HashMap::from([("e-flexbox".into(), "4.2.3".into())]),
            style_props: HashMap::new(),
            style_schemas: styles,
            classic_controls: HashMap::from([
                ("section".into(), HashSet::from(["_attributes".into()])),
                ("column".into(), HashSet::from(["_attributes".into()])),
                ("container".into(), HashSet::from(["_attributes".into()])),
            ]),
            render_capabilities: Map::from_iter([("attributes".into(), json!({"available":true}))]),
        }
    }

    #[test]
    fn layout_conversion_binds_unique_native_styles_and_preserves_classes_ids_and_attributes() {
        let schema = native_layout_schema();
        let source = vec![
            json!({"id":"17f82ce","elType":"container","settings":{"flex_direction":"row","css_classes":"hero card","_element_id":"first","_attributes":"data-label|one","html_tag":"a","link":{"url":"https://example.test/wrapper","is_external":true}},"elements":[]}),
            json!({"id":"27f82ce","elType":"wrapper","settings":{"flex_direction":"column","css_classes":"footer","_element_id":"second","_attributes":"aria-label|Footer"},"elements":[]}),
            json!({"id":"37f82ce","elType":"section","settings":{},"elements":[
                {"id":"47f82ce","elType":"column","settings":{"_column_size":60},"elements":[]},
                {"id":"57f82ce","elType":"column","settings":{"_column_size":40},"elements":[]}
            ]}),
        ];
        let atomic = plan_convert_elements(ConvertElementsInput {
            context: context(),
            direction: "classic_to_atomic".into(),
            elements: source,
            schemas: Some(schema.clone()),
        })
        .unwrap()
        .document_patch
        .unwrap();
        let first_classes = atomic[0]["settings"]["classes"]["value"]
            .as_array()
            .unwrap();
        let second_classes = atomic[1]["settings"]["classes"]["value"]
            .as_array()
            .unwrap();
        assert!(
            first_classes
                .iter()
                .any(|class| class == "mcp-layout-17f82ce")
        );
        assert!(
            second_classes
                .iter()
                .any(|class| class == "mcp-layout-27f82ce")
        );
        assert_ne!(first_classes[0], second_classes[0]);
        assert_eq!(
            atomic[0]["styles"]["mcp-layout-17f82ce"]["id"],
            "mcp-layout-17f82ce"
        );
        assert_eq!(atomic[0]["settings"]["_cssid"]["value"], "first");

        let classic = plan_convert_elements(ConvertElementsInput {
            context: context(),
            direction: "atomic_to_classic".into(),
            elements: atomic.as_array().unwrap().clone(),
            schemas: Some(schema),
        })
        .unwrap()
        .document_patch
        .unwrap();
        assert_eq!(classic[0]["settings"]["css_classes"], "hero card");
        assert_eq!(classic[0]["settings"]["_element_id"], "first");
        assert_eq!(classic[0]["settings"]["_attributes"], "data-label|one");
        assert_eq!(classic[0]["settings"]["html_tag"], "a");
        assert_eq!(
            classic[0]["settings"]["link"]["url"],
            "https://example.test/wrapper"
        );
        assert_eq!(classic[0]["settings"]["link"]["is_external"], true);
        assert_eq!(classic[1]["settings"]["css_classes"], "footer");
        assert_eq!(classic[1]["settings"]["_element_id"], "second");
        assert_eq!(classic[1]["settings"]["_attributes"], "aria-label|Footer");
        assert_eq!(classic[2]["elType"], "section");
        assert_eq!(classic[2]["elements"][0]["elType"], "column");
        assert_eq!(
            classic[2]["elements"][0]["settings"]["_column_size"].as_f64(),
            Some(60.0)
        );
        assert_eq!(
            classic[2]["elements"][1]["settings"]["_column_size"].as_f64(),
            Some(40.0)
        );
    }

    fn native_button_schema() -> AtomicRuntimeSchema {
        AtomicRuntimeSchema {
            widget_props: HashMap::from([(
                "e-button".into(),
                HashMap::from([
                    ("text".into(), "html-v3".into()),
                    ("link".into(), "link".into()),
                    ("classes".into(), "classes".into()),
                    ("_cssid".into(), "string".into()),
                    ("attributes".into(), "attributes".into()),
                ]),
            )]),
            widget_prop_schemas: HashMap::from([(
                "e-button".into(),
                Map::from_iter([
                    ("text".into(), native_html_schema()),
                    ("link".into(), native_link_schema()),
                    ("classes".into(), json!({"kind":"plain","key":"classes"})),
                    ("_cssid".into(), primitive("string", "string")),
                    ("attributes".into(), native_attributes_schema()),
                ]),
            )]),
            element_versions: HashMap::from([("e-button".into(), "4.2.3".into())]),
            style_props: HashMap::new(),
            style_schemas: Map::new(),
            classic_controls: HashMap::from([(
                "button".into(),
                HashSet::from(["_attributes".into()]),
            )]),
            render_capabilities: Map::from_iter([("attributes".into(), json!({"available":true}))]),
        }
    }

    #[test]
    fn runtime_attributes_capability_defaults_to_unknown_and_reads_verified_state() {
        let styles = json!({});
        let unknown = AtomicRuntimeSchema::from_runtime(&json!({}), &styles);
        assert!(!unknown.attributes_rendering_available());

        let available = AtomicRuntimeSchema::from_runtime(
            &json!({
                "render_capabilities":{"attributes":{"available":true,"elementor_core_version":"4.2.3"}},
                "elements":{"container":{"controls":{"_attributes":{}}}}
            }),
            &styles,
        );
        assert!(available.attributes_rendering_available());
        assert!(available.classic_control_available("container", "_attributes"));

        let unavailable = AtomicRuntimeSchema::from_runtime(
            &json!({"render_capabilities":{"attributes":{"available":false}}}),
            &styles,
        );
        assert!(!unavailable.attributes_rendering_available());
    }

    #[test]
    fn native_url_flags_preserve_php_empty_semantics_and_unsupported_link_values() {
        let schema = native_button_schema();
        let classic = json!({"id":"17f82ce","elType":"widget","widgetType":"button","settings":{"text":"Open","link":{"url":"https://example.test/","is_external":"yes","nofollow":"","custom_attributes":""}},"elements":[]});
        let atomic =
            classic_to_atomic(&classic, Some(&schema)).expect("native URL defaults convert");
        assert_eq!(
            atomic["settings"]["link"]["value"]["isTargetBlank"]["value"],
            true
        );
        let restored = atomic_to_classic(&atomic, Some(&schema)).expect("Atomic URL converts back");
        assert_eq!(restored["settings"]["link"]["url"], "https://example.test/");
        assert_eq!(restored["settings"]["link"]["is_external"], true);
        assert_eq!(
            atomic_to_classic(&atomic, Some(&schema)).unwrap()["settings"]["link"]["is_external"],
            true
        );

        for empty in [json!(null), json!(false), json!(0), json!(""), json!("0")] {
            let mut input = classic.clone();
            input["settings"]["link"]["is_external"] = empty;
            input["settings"]["link"]["nofollow"] =
                input["settings"]["link"]["is_external"].clone();
            assert_eq!(
                classic_to_atomic(&input, Some(&schema)).unwrap()["settings"]["link"]["value"]["isTargetBlank"]
                    ["value"],
                false
            );
        }

        for truthy in [json!(true), json!(2), json!(-1), json!("new-window")] {
            let mut input = classic.clone();
            input["settings"]["link"]["is_external"] = truthy;
            assert_eq!(
                classic_to_atomic(&input, Some(&schema)).unwrap()["settings"]["link"]["value"]["isTargetBlank"]
                    ["value"],
                true
            );
        }

        for link in [
            json!({"url":"https://example.test/","nofollow":true}),
            json!({"url":"https://example.test/","nofollow":"yes"}),
            json!({"url":"https://example.test/","custom_attributes":"aria-label|open"}),
            json!({"url":"https://example.test/","custom_attributes":["x"]}),
            json!({"url":"https://example.test/","is_external":{"dynamic":true}}),
            json!({"url":"https://example.test/","is_external":[true]}),
        ] {
            let mut input = classic.clone();
            input["settings"]["link"] = link;
            let mut retained = Vec::new();
            let converted =
                convert_element(&input, "classic_to_atomic", Some(&schema), &mut retained, 0)
                    .unwrap();
            assert_eq!(
                serde_json::to_vec(&converted).unwrap(),
                serde_json::to_vec(&input).unwrap(),
                "unsupported link value changed"
            );
            assert_eq!(retained.len(), 1);
        }
    }

    #[test]
    fn widget_conversion_preserves_native_classes_css_id_custom_attributes_and_link() {
        let schema = native_button_schema();
        let source = json!({
            "id":"17f82ce",
            "elType":"widget",
            "widgetType":"button",
            "settings":{
                "text":"Read more",
                "link":{"url":"https://example.test/guide","is_external":true},
                "_css_classes":"cta primary",
                "_element_id":"guide-link",
                "_attributes":"aria-label|Read the guide"
            },
            "elements":[]
        });
        let atomic = classic_to_atomic(&source, Some(&schema)).unwrap();
        assert_eq!(
            atomic["settings"]["classes"]["value"],
            json!(["cta", "primary"])
        );
        assert_eq!(atomic["settings"]["_cssid"]["value"], "guide-link");
        assert_eq!(
            atomic["settings"]["attributes"]["value"][0]["value"]["key"]["value"],
            "aria-label"
        );

        let classic = atomic_to_classic(&atomic, Some(&schema)).unwrap();
        assert_eq!(classic["settings"]["_css_classes"], "cta primary");
        assert_eq!(classic["settings"]["_element_id"], "guide-link");
        assert_eq!(
            classic["settings"]["_attributes"],
            "aria-label|Read the guide"
        );
        assert_eq!(
            classic["settings"]["link"]["url"],
            "https://example.test/guide"
        );
        assert_eq!(classic["settings"]["link"]["is_external"], true);

        let mut malformed = source.clone();
        malformed["settings"]["_attributes"] = json!("data-note|left|right");
        let mut retained = Vec::new();
        let unchanged = convert_element(
            &malformed,
            "classic_to_atomic",
            Some(&schema),
            &mut retained,
            0,
        )
        .unwrap();
        assert_eq!(unchanged, malformed);
        assert_eq!(retained.len(), 1);
    }

    #[test]
    fn reverse_conversion_retains_the_whole_subtree_for_unknown_native_style_props() {
        let schema = native_layout_schema();
        let atomic = plan_convert_elements(ConvertElementsInput {
            context: context(),
            direction: "classic_to_atomic".into(),
            elements: vec![json!({"id":"17f82ce","elType":"container","settings":{"flex_direction":"column"},"elements":[]})],
            schemas: Some(schema.clone()),
        })
        .unwrap()
        .document_patch
        .unwrap();
        let mut source = atomic[0].clone();
        source["styles"]["mcp-layout-17f82ce"]["variants"][0]["props"]["opacity"] =
            json!({"$$type":"number","value":0.8});
        let plan = plan_convert_elements(ConvertElementsInput {
            context: context(),
            direction: "atomic_to_classic".into(),
            elements: vec![source.clone()],
            schemas: Some(schema),
        })
        .unwrap();
        assert!(plan.document_patch.is_none());
        assert_eq!(plan.retained.len(), 1);
        assert_eq!(plan.retained[0].element_id, "17f82ce");
        assert_eq!(plan.retained[0].reason.contains("layout subtree"), true);
        assert_eq!(
            source["styles"]["mcp-layout-17f82ce"]["variants"][0]["props"]["opacity"]["value"],
            0.8
        );
    }

    #[test]
    fn unverified_attribute_rendering_retains_the_complete_classic_subtree() {
        for capability in [None, Some(false)] {
            let mut schema = native_layout_schema();
            let button = native_button_schema();
            schema.widget_props.extend(button.widget_props);
            schema
                .widget_prop_schemas
                .extend(button.widget_prop_schemas);
            schema.element_versions.extend(button.element_versions);
            if let Some(available) = capability {
                schema
                    .render_capabilities
                    .insert("attributes".into(), json!({"available":available}));
            } else {
                schema.render_capabilities.remove("attributes");
            }
            let source = json!({
                "id":"17f82ce",
                "elType":"container",
                "settings":{"flex_direction":"column"},
                "elements":[{
                    "id":"27f82ce",
                    "elType":"widget",
                    "widgetType":"button",
                    "settings":{"text":"Open","_attributes":"aria-label|Open"},
                    "elements":[]
                }]
            });
            let plan = plan_convert_elements(ConvertElementsInput {
                context: context(),
                direction: "classic_to_atomic".into(),
                elements: vec![source.clone()],
                schemas: Some(schema),
            })
            .unwrap();
            assert!(plan.document_patch.is_none());
            assert_eq!(plan.retained[0].element_id, "17f82ce");
            assert!(plan.retained[0].reason.contains("layout subtree"));
            assert!(plan.retained[0].reason.contains("did not verify"));
            assert_eq!(plan.retained.len(), 1);
            assert_eq!(
                source["elements"][0]["settings"]["_attributes"],
                "aria-label|Open"
            );
        }
    }

    #[test]
    fn classic_attribute_target_requires_registered_control_but_empty_values_do_not() {
        let mut schema = native_button_schema();
        schema
            .render_capabilities
            .insert("attributes".into(), json!({"available":false}));
        let empty_classic = json!({"id":"17f82ce","elType":"widget","widgetType":"button","settings":{"text":"Open","_attributes":""},"elements":[]});
        let converted = convert_element(
            &empty_classic,
            "classic_to_atomic",
            Some(&schema),
            &mut Vec::new(),
            0,
        )
        .unwrap();
        assert_eq!(converted["widgetType"], "e-button");
        assert!(converted["settings"].get("attributes").is_none());

        let source = json!({"id":"27f82ce","elType":"widget","widgetType":"button","settings":{"text":"Open","_attributes":"aria-label|Open"},"elements":[]});
        let mut retained = Vec::new();
        let converted = convert_element(
            &source,
            "classic_to_atomic",
            Some(&schema),
            &mut retained,
            0,
        )
        .unwrap();
        assert_eq!(converted, source);
        assert_eq!(retained.len(), 1);

        let mut schema = native_button_schema();
        schema.classic_controls.remove("button");
        let atomic = classic_to_atomic(&source, Some(&schema)).unwrap();
        let mut retained = Vec::new();
        let converted = convert_element(
            &atomic,
            "atomic_to_classic",
            Some(&schema),
            &mut retained,
            0,
        )
        .unwrap();
        assert_eq!(converted, atomic);
        assert_eq!(retained.len(), 1);
    }

    fn native_html_schema() -> Value {
        let mut html = Map::new();
        html.insert("content".into(), primitive("string", "string"));
        html.insert("children".into(), primitive("plain", "unknown"));
        union(Map::from_iter([(
            "html-v3".into(),
            object("html-v3", html),
        )]))
    }

    #[test]
    fn negative_conversion_checks_preserve_unsafe_trees_and_reject_bad_ids() {
        let invalid = ConvertElementsInput {
            context: context(),
            direction: "classic_to_atomic".into(),
            elements: vec![
                json!({"elType":"widget","widgetType":"heading","settings":{},"elements":[]}),
            ],
            schemas: None,
        };
        assert!(plan_convert_elements(invalid).is_err());

        let section = json!({"id":"12345678","elType":"section","settings":{"_column_size":100,"_column_size_tablet":50},"elements":[
            {"id":"23456789","elType":"column","settings":{"_column_size":50},"elements":[
                {"id":"34567890","elType":"widget","widgetType":"custom-widget","settings":{"__dynamic__":{"field":"site_option"}},"elements":[]}
            ]}
        ]});
        let plan = plan_convert_elements(ConvertElementsInput {
            context: context(),
            direction: "classic_to_atomic".into(),
            elements: vec![section.clone()],
            schemas: None,
        })
        .unwrap();
        assert!(plan.document_patch.is_none());
        assert!(
            plan.retained
                .iter()
                .any(|item| item.reason.contains("layout subtree"))
        );

        let unsafe_elements = vec![
            json!({"id":"45678901","elType":"widget","widgetType":"heading","settings":{"__dynamic__":{"field":"site_option"}},"elements":[]}),
            json!({"id":"56789012","elType":"widget","widgetType":"custom-widget","settings":{},"elements":[]}),
        ];
        let plan = plan_convert_elements(ConvertElementsInput {
            context: context(),
            direction: "classic_to_atomic".into(),
            elements: unsafe_elements,
            schemas: None,
        })
        .unwrap();
        assert!(
            plan.retained
                .iter()
                .any(|item| item.reason.contains("dynamic bindings"))
        );
        assert!(
            plan.retained
                .iter()
                .any(|item| item.reason.contains("custom widget"))
        );
    }

    #[test]
    fn atomic_widget_serialization_uses_widget_base_and_accepts_seven_character_ids() {
        let mut versions = HashMap::new();
        versions.insert("e-heading".into(), "4.2.3".into());
        let mut props = HashMap::new();
        props.insert("tag".into(), "string".into());
        props.insert("title".into(), "html-v3".into());
        let schema = AtomicRuntimeSchema {
            widget_props: HashMap::from([("e-heading".into(), props)]),
            widget_prop_schemas: HashMap::from([(
                "e-heading".into(),
                Map::from_iter([
                    ("title".into(), native_html_schema()),
                    (
                        "tag".into(),
                        union(Map::from_iter([(
                            "string".into(),
                            primitive("string", "string"),
                        )])),
                    ),
                ]),
            )]),
            element_versions: versions,
            style_props: HashMap::new(),
            style_schemas: Map::new(),
            classic_controls: HashMap::new(),
            render_capabilities: Map::new(),
        };
        let element = json!({"id":"17f82ce","elType":"widget","widgetType":"heading","settings":{"title":"Hello","header_size":"h2"},"elements":[]});
        let plan = plan_convert_elements(ConvertElementsInput {
            context: context(),
            direction: "classic_to_atomic".into(),
            elements: vec![element],
            schemas: Some(schema),
        })
        .unwrap();
        let converted = &plan.document_patch.unwrap()[0];
        assert_eq!(converted["elType"], "widget");
        assert_eq!(converted["widgetType"], "e-heading");
        assert_eq!(converted["settings"]["title"]["$$type"], "html-v3");
    }

    #[test]
    fn image_media_and_button_link_roundtrip_through_registered_props() {
        let versions = HashMap::from([
            ("e-image".into(), "1".into()),
            ("e-button".into(), "1".into()),
        ]);
        let image_props = HashMap::from([("image".into(), "image".into())]);
        let button_props = HashMap::from([
            ("text".into(), "html-v3".into()),
            ("link".into(), "link".into()),
        ]);
        let schema = AtomicRuntimeSchema {
            widget_props: HashMap::from([
                ("e-image".into(), image_props),
                ("e-button".into(), button_props),
            ]),
            widget_prop_schemas: HashMap::from([
                (
                    "e-image".into(),
                    Map::from_iter([("image".into(), native_image_schema())]),
                ),
                (
                    "e-button".into(),
                    Map::from_iter([
                        ("text".into(), native_html_schema()),
                        ("link".into(), native_link_schema()),
                    ]),
                ),
            ]),
            element_versions: versions,
            style_props: HashMap::new(),
            style_schemas: Map::new(),
            classic_controls: HashMap::new(),
            render_capabilities: Map::new(),
        };
        let source = vec![
            json!({"id":"17f82ce","elType":"widget","widgetType":"image","settings":{"image":{"id":42,"url":"https://example.test/a.jpg","alt":"A"},"image_size":"large"},"elements":[]}),
            json!({"id":"27f82ce","elType":"widget","widgetType":"button","settings":{"text":"Open","link":{"url":"https://example.test/","is_external":true}},"elements":[]}),
        ];
        let converted = plan_convert_elements(ConvertElementsInput {
            context: context(),
            direction: "classic_to_atomic".into(),
            elements: source,
            schemas: Some(schema.clone()),
        })
        .unwrap();
        let atomic = converted
            .document_patch
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(atomic[0]["widgetType"], "e-image");
        assert_eq!(
            atomic[0]["settings"]["image"]["value"]["src"]["value"]["id"]["value"],
            42
        );
        assert_eq!(atomic[1]["widgetType"], "e-button");
        assert_eq!(
            atomic[1]["settings"]["link"]["value"]["isTargetBlank"]["value"],
            true
        );
        let reverse = plan_convert_elements(ConvertElementsInput {
            context: context(),
            direction: "atomic_to_classic".into(),
            elements: atomic,
            schemas: Some(schema),
        })
        .unwrap();
        let restored = reverse.document_patch.unwrap().as_array().unwrap().clone();
        assert_eq!(restored[0]["settings"]["image"]["id"], 42, "{restored:#?}");
        assert_eq!(restored[1]["settings"]["link"]["is_external"], true);
    }

    #[test]
    fn unsupported_image_settings_are_retained_without_an_executable_patch() {
        let schema = AtomicRuntimeSchema {
            widget_props: HashMap::from([(
                "e-image".into(),
                HashMap::from([("image".into(), "image".into())]),
            )]),
            widget_prop_schemas: HashMap::from([(
                "e-image".into(),
                Map::from_iter([("image".into(), native_image_schema())]),
            )]),
            element_versions: HashMap::from([("e-image".into(), "1".into())]),
            style_props: HashMap::new(),
            style_schemas: Map::new(),
            classic_controls: HashMap::new(),
            render_capabilities: Map::new(),
        };
        let plan = plan_convert_elements(ConvertElementsInput { context:context(), direction:"classic_to_atomic".into(), elements:vec![json!({"id":"17f82ce","elType":"widget","widgetType":"image","settings":{"image":{"url":"https://example.test/a.jpg"},"image_custom_dimension":{"width":400,"height":300},"caption":"A"},"elements":[]})], schemas:Some(schema) }).unwrap();
        assert!(plan.document_patch.is_none());
        assert_eq!(plan.retained[0].element_id, "17f82ce");
    }
}
