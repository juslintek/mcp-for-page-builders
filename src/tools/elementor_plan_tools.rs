use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::elementor::ElementorService;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::types::elementor_plan::{ElementorPlan, PlanCheck};
use crate::wp::WpClient;

pub struct ApplyElementorPlan;
pub struct RestoreElementorSnapshot;

#[async_trait]
impl Tool for ApplyElementorPlan {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "apply_elementor_plan",
            description: "Apply a revision-checked full Elementor document plan.",
            input_schema: json!({
                "type":"object","required":["plan"],"properties":{"plan":{
                    "type":"object","required":["site_url","page_id","expected_revision","operation_id"],
                    "properties":{
                        "site_url":{"type":"string"},"page_id":{"type":"integer"},
                        "expected_revision":{"type":"string"},"operation_id":{"type":"string"},
                        "document_patch":{"type":"array"},"resource_patches":{"type":"array"},
                        "retained":{"type":"array"},"source":{},"checks":{"type":"array"}
                    }
                }}
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let plan: ElementorPlan =
            serde_json::from_value(args.get("plan").cloned().context("plan required")?)?;
        validate_checks(&plan.checks)?;
        validate_selectors_in_browser(&plan.checks).await?;
        let plan_site = canonical_site(&plan.site_url)?;
        let connected_site = canonical_site(wp.base_url())?;
        if plan_site != connected_site {
            anyhow::bail!(
                "Plan site_url does not match the connected WordPress site; no mutation was made"
            );
        }
        let mut create_ids = std::collections::HashSet::new();
        for patch in &plan.resource_patches {
            if patch.id == "new" || patch.id.starts_with("new_") {
                if !matches!(
                    patch.kind,
                    crate::types::elementor_plan::ResourceKind::Page
                        | crate::types::elementor_plan::ResourceKind::Template
                ) || !valid_create_resource_id(&patch.id)
                    || patch.expected_revision.as_deref() != Some("absent")
                    || patch.before.is_some()
                    || !create_ids.insert(patch.id.as_str())
                {
                    anyhow::bail!(
                        "Invalid new resource ID or creation precondition; no mutation was made"
                    );
                }
            }
        }
        let new_resource = plan.page_id == 0
            && plan.document_patch.is_none()
            && !plan.resource_patches.is_empty()
            && plan.resource_patches.iter().all(|patch| {
                matches!(
                    patch.kind,
                    crate::types::elementor_plan::ResourceKind::Page
                        | crate::types::elementor_plan::ResourceKind::Template
                ) && valid_create_resource_id(&patch.id)
                    && patch.expected_revision.as_deref() == Some("absent")
                    && patch.before.is_none()
            })
            && plan
                .resource_patches
                .iter()
                .map(|patch| patch.id.as_str())
                .collect::<std::collections::HashSet<_>>()
                .len()
                == plan.resource_patches.len()
            && plan.expected_revision == "absent";
        if !new_resource && plan.expected_revision.trim().is_empty() {
            anyhow::bail!("expected_revision is required; no mutation was made");
        }
        if !(8..=128).contains(&plan.operation_id.len())
            || !plan
                .operation_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            anyhow::bail!(
                "operation_id must contain 8 to 128 letters, digits, underscores, or hyphens; no mutation was made"
            );
        }
        if plan.document_patch.is_none() && plan.resource_patches.is_empty() {
            return Ok(ToolResult::error(serde_json::to_string(
                &json!({"status":"rejected","error":"A document_patch or resource_patches are required.","mutations_made":false}),
            )?));
        }
        if plan.page_id == 0 && !new_resource {
            anyhow::bail!(
                "page_id 0 is allowed only for new page or template resources; no mutation was made"
            );
        }
        if new_resource && !plan.checks.is_empty() {
            anyhow::bail!("Post creation checks require an existing page; no mutation was made");
        }
        let has_document_patch = plan.document_patch.is_some();
        let mut completed_preflight = None;
        if !plan.resource_patches.is_empty() || has_document_patch {
            let body = json!({"operation_id":plan.operation_id,"page_id":plan.page_id,"expected_revision":if new_resource {"absent"} else {plan.expected_revision.as_str()},"original_expected_revision":plan.expected_revision,"document_patch":plan.document_patch,"resource_patches":plan.resource_patches,"document_snapshot":has_document_patch,"validate_only":true});
            match wp
                .request(
                    "POST",
                    "mcp-for-page-builders/v1/resources/apply",
                    Some(&body),
                    None,
                )
                .await
            {
                Ok(receipt) => completed_preflight = Some(receipt),
                Err(error) => {
                    let detail = error
                        .chain()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(": ");
                    if detail.contains("stale_revision") {
                        return Ok(ToolResult::error(serde_json::to_string(
                            &json!({"status":"rejected","error":detail,"page_id":plan.page_id,"mutations_made":false}),
                        )?));
                    }
                    anyhow::bail!("{detail}; Resource preflight failed; no mutations were made");
                }
            }
        }
        let replay_receipt = completed_preflight
            .as_ref()
            .filter(|receipt| receipt["valid"] == true && receipt["idempotent_replay"] == true);
        let mut document_revision = replay_receipt
            .and_then(|receipt| receipt["current_revision"].as_str())
            .map(str::to_owned);
        if replay_receipt.is_none() {
            if let Some(document) = plan.document_patch.as_ref() {
                let elements: Vec<crate::types::Element> = serde_json::from_value(document.clone())
                    .context("document_patch must be a canonical full Elementor tree")?;
                let save_receipt = match wp.request("POST", &format!("mcp-for-page-builders/v1/document/{}/apply", plan.page_id), Some(&json!({"document":elements,"expected_revision":plan.expected_revision,"operation_id":plan.operation_id})), None).await {
                Ok(receipt) if receipt["saved"] == true && receipt["readback_verified"] == true && receipt["cache_regenerated"] == true => receipt,
                Ok(receipt) => {
                    let error = anyhow::anyhow!("Elementor save returned an incomplete result: {}", receipt);
                    let current_revision = ElementorService::new(wp).get_tree(plan.page_id).await.ok().and_then(|tree| tree.revision);
                    let persisted = receipt["saved"] == true;
                    return Ok(ToolResult::error(serde_json::to_string(&json!({"status":if persisted {"partial_failure"} else {"rejected"},"error":error.to_string(),"page_id":plan.page_id,"current_revision":current_revision,"snapshot_operation_id":plan.operation_id,"mutations_made":persisted}))?));
                }
                Err(error) => {
                let current_revision = ElementorService::new(wp)
                    .get_tree(plan.page_id)
                    .await
                    .ok()
                    .and_then(|tree| tree.revision);
                let persisted = error
                    .downcast_ref::<crate::elementor::DocumentPersistedError>()
                    .is_some();
                return Ok(ToolResult::error(serde_json::to_string(
                    &json!({"status":if persisted {"partial_failure"} else {"rejected"},"error":error.to_string(),"page_id":plan.page_id,"current_revision":current_revision,"snapshot_operation_id":plan.operation_id,"mutations_made":persisted}),
                )?));
                }
            };
                document_revision = save_receipt["revision"].as_str().map(str::to_owned);
                if document_revision.is_none() {
                    return Ok(ToolResult::error(serde_json::to_string(
                        &json!({"status":"partial_failure","error":"Checked document save receipt omitted revision.","page_id":plan.page_id,"snapshot_operation_id":plan.operation_id,"mutations_made":true,"restore_required":true}),
                    )?));
                }
            }
        }
        let resources = if plan.resource_patches.is_empty() && !has_document_patch {
            None
        } else if let Some(receipt) = replay_receipt {
            Some(receipt.clone())
        } else {
            let body = json!({"operation_id":plan.operation_id,"page_id":plan.page_id,"expected_revision":if new_resource {"absent"} else {document_revision.as_deref().unwrap_or(&plan.expected_revision)},"original_expected_revision":plan.expected_revision,"document_patch":plan.document_patch,"resource_patches":plan.resource_patches,"document_snapshot":has_document_patch});
            match wp
                .request(
                    "POST",
                    "mcp-for-page-builders/v1/resources/apply",
                    Some(&body),
                    None,
                )
                .await
            {
                Ok(value) if value["status"] == "partial_failure" => {
                    return Ok(ToolResult::error(serde_json::to_string(
                        &json!({"status":"partial_failure","error":value["error"],"completed_resources":value["completed"],"failed_resource":value["failed_resource"],"completed_document_revision":document_revision,"restore_operation_id":plan.operation_id,"mutations_made":true}),
                    )?));
                }
                Ok(value)
                    if value["applied"] == true
                        && value["resources"].is_array()
                        && (value["idempotent_replay"] != true || value["valid"] == true) =>
                {
                    Some(value)
                }
                Ok(value) => {
                    return Ok(ToolResult::error(serde_json::to_string(
                        &json!({"status":"partial_failure","error":"Native resource apply returned an incomplete success receipt; the persisted state must be inspected before retrying.","receipt":value,"completed_document_revision":document_revision,"restore_operation_id":plan.operation_id,"resource_restore_operation_id":plan.operation_id,"mutations_made":document_revision.is_some().then_some(true),"inspection_required":true}),
                    )?));
                }
                Err(error) => {
                    return Ok(ToolResult::error(serde_json::to_string(
                        &json!({"status":"partial_failure","error":error.to_string(),"completed_document_revision":document_revision,"restore_operation_id":plan.operation_id,"resource_restore_operation_id":plan.operation_id,"mutations_made":document_revision.is_some().then_some(true),"inspection_required":true}),
                    )?));
                }
            }
        };
        if replay_receipt.is_some() && !plan.checks.is_empty() {
            let results = match verify_checks(wp, plan.page_id, &plan.checks).await {
                Ok(results) if results.iter().all(|result| result["passed"] == true) => results,
                Ok(results) => {
                    return Ok(ToolResult::error(serde_json::to_string(
                        &json!({"status":"verification_failed","error":"One or more requested checks failed after the plan was saved.","page_id":plan.page_id,"completed_revision":document_revision,"operation_id":plan.operation_id,"restore_operation_id":plan.operation_id,"mutations_made":false,"verification":results}),
                    )?));
                }
                Err(error) => {
                    return Ok(ToolResult::error(serde_json::to_string(
                        &json!({"status":"partial_failure","error":error.to_string(),"page_id":plan.page_id,"completed_revision":document_revision,"operation_id":plan.operation_id,"restore_operation_id":plan.operation_id,"mutations_made":false}),
                    )?));
                }
            };
            return Ok(ToolResult::text(serde_json::to_string(
                &json!({"status":"applied_and_verified","applied":true,"page_id":plan.page_id,"operation_id":plan.operation_id,"current_revision":document_revision,"resources":resources,"idempotent_replay":true,"verification":results}),
            )?));
        }
        if replay_receipt.is_some() {
            return Ok(ToolResult::text(serde_json::to_string(
                &json!({"applied":true,"page_id":plan.page_id,"operation_id":plan.operation_id,"current_revision":document_revision,"resources":resources,"idempotent_replay":true,"verification":{"status":"not_requested"}}),
            )?));
        }
        if new_resource {
            let created = resources
                .as_ref()
                .and_then(|value| value["resources"].as_array())
                .cloned()
                .unwrap_or_default();
            return Ok(ToolResult::text(serde_json::to_string(
                &json!({"status":"created","resources":created,"operation_id":plan.operation_id,"idempotent_replay":resources.as_ref().is_some_and(|value| value["idempotent_replay"] == true)}),
            )?));
        }
        if !plan.checks.is_empty() {
            let results = match verify_checks(wp, plan.page_id, &plan.checks).await {
                Ok(results) if results.iter().all(|result| result["passed"] == true) => results,
                Ok(results) => {
                    return Ok(ToolResult::error(serde_json::to_string(
                        &json!({"status":"verification_failed","error":"One or more requested checks failed after the plan was saved.","page_id":plan.page_id,"completed_revision":document_revision,"operation_id":plan.operation_id,"restore_operation_id":plan.operation_id,"mutations_made":true,"verification":results}),
                    )?));
                }
                Err(error) => {
                    return Ok(ToolResult::error(serde_json::to_string(
                        &json!({"status":"partial_failure","error":error.to_string(),"page_id":plan.page_id,"completed_revision":document_revision,"operation_id":plan.operation_id,"restore_operation_id":plan.operation_id,"mutations_made":true}),
                    )?));
                }
            };
            return Ok(ToolResult::text(serde_json::to_string(
                &json!({"status":"applied_and_verified","applied":true,"page_id":plan.page_id,"operation_id":plan.operation_id,"current_revision":document_revision,"resources":resources,"verification":results}),
            )?));
        }
        Ok(ToolResult::text(serde_json::to_string(
            &json!({"applied":true,"page_id":plan.page_id,"operation_id":plan.operation_id,"current_revision":document_revision,"resources":resources,"verification":{"status":"not_requested"}}),
        )?))
    }
}

fn valid_create_resource_id(id: &str) -> bool {
    if id == "new" {
        return true;
    }
    let Some(suffix) = id.strip_prefix("new_pattern_") else {
        return false;
    };
    suffix
        .parse::<u32>()
        .is_ok_and(|number| number >= 2 && number.to_string() == suffix)
}

fn validate_checks(checks: &[PlanCheck]) -> Result<()> {
    for check in checks {
        let (selector, viewport) = match check {
            PlanCheck::ElementPresent { element_id } => {
                if element_id.trim().is_empty() {
                    anyhow::bail!("element_present.element_id must not be empty");
                }
                (None, None)
            }
            PlanCheck::ComputedStyle {
                selector,
                property,
                expected,
                viewport,
            } => {
                validate_selector(selector)?;
                if expected.trim().is_empty() || !valid_css_property(property) {
                    anyhow::bail!(
                        "computed_style requires a nonempty expected value and supported CSS property syntax"
                    );
                }
                (Some(selector), viewport.as_ref())
            }
            PlanCheck::NoHorizontalOverflow { viewport } => (None, viewport.as_ref()),
            PlanCheck::PageHeight {
                viewport,
                expected,
                max_delta_percent,
            } => {
                if *expected == 0
                    || !max_delta_percent.is_finite()
                    || *max_delta_percent < 0.0
                    || *max_delta_percent > 100.0
                {
                    anyhow::bail!(
                        "page_height expected must be positive and max_delta_percent must be finite and between 0 and 100"
                    );
                }
                (None, viewport.as_ref())
            }
            PlanCheck::BoundingBox {
                selector,
                width,
                height,
                tolerance_px,
                viewport,
            } => {
                validate_selector(selector)?;
                if !width.is_finite()
                    || !height.is_finite()
                    || *width < 0.0
                    || *height < 0.0
                    || !tolerance_px.is_finite()
                    || *tolerance_px < 0.0
                    || *tolerance_px > 1000.0
                {
                    anyhow::bail!(
                        "bounding_box dimensions and tolerance must be finite and nonnegative, with tolerance at most 1000px"
                    );
                }
                (Some(selector), viewport.as_ref())
            }
        };
        let _ = selector;
        if let Some(viewport) = viewport
            && (!(320..=3840).contains(&viewport.width) || !(240..=2160).contains(&viewport.height))
        {
            anyhow::bail!("viewport width/height are outside supported bounds");
        }
    }
    Ok(())
}

fn validate_selector(selector: &str) -> Result<()> {
    let selector = selector.trim();
    if selector.is_empty() || selector.len() > 512 || selector.chars().any(char::is_control) {
        anyhow::bail!("selector must be nonempty CSS syntax up to 512 characters");
    }
    let mut brackets = 0i32;
    let mut parens = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in selector.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '[' => brackets += 1,
            ']' => {
                brackets -= 1;
                if brackets < 0 {
                    anyhow::bail!("selector has unmatched closing bracket");
                }
            }
            '(' => parens += 1,
            ')' => {
                parens -= 1;
                if parens < 0 {
                    anyhow::bail!("selector has unmatched closing parenthesis");
                }
            }
            _ => {}
        }
    }
    if quote.is_some() || brackets != 0 || parens != 0 {
        anyhow::bail!("selector has unclosed quote, bracket, or parenthesis");
    }
    Ok(())
}

async fn validate_selectors_in_browser(checks: &[PlanCheck]) -> Result<()> {
    let selectors: Vec<&str> = checks
        .iter()
        .filter_map(|check| match check {
            PlanCheck::ComputedStyle { selector, .. } | PlanCheck::BoundingBox { selector, .. } => {
                Some(selector.as_str())
            }
            _ => None,
        })
        .collect();
    if selectors.is_empty() {
        return Ok(());
    }
    let browser = crate::cdp::browser().await?;
    let page = browser.new_page("about:blank").await?;
    let validation = async {
        for selector in selectors {
            let literal = serde_json::to_string(selector)?;
            let script = format!("(()=>{{try{{document.querySelector({literal});return JSON.stringify({{valid:true}})}}catch(e){{return JSON.stringify({{valid:false,error:e.message}})}}}})()");
            let output: String = page.evaluate(script).await?.into_value()?;
            let outcome: Value = serde_json::from_str(&output)?;
            if outcome["valid"] != true { anyhow::bail!("invalid CSS selector: {}", outcome["error"].as_str().unwrap_or("browser rejected selector")); }
        }
        Ok(())
    }.await;
    let closed = page.close().await;
    match (validation, closed) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error).context("failed to close selector validation page"),
        (Err(error), Err(close_error)) => anyhow::bail!(
            "{error:#}; also failed to close selector validation page: {close_error:#}"
        ),
    }
}

fn valid_css_property(property: &str) -> bool {
    let p = property.trim();
    !p.is_empty()
        && p.len() <= 128
        && (p.starts_with("--")
            && p[2..]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || p.bytes().all(|b| b.is_ascii_lowercase() || b == b'-'))
}

async fn verify_checks(wp: &WpClient, page_id: u64, checks: &[PlanCheck]) -> Result<Vec<Value>> {
    let tree = ElementorService::new(wp).get_tree(page_id).await?;
    if checks
        .iter()
        .all(|check| matches!(check, PlanCheck::ElementPresent { .. }))
    {
        return Ok(checks.iter().map(|check| {
            let PlanCheck::ElementPresent { element_id } = check else { unreachable!() };
            let present = tree.find(element_id).is_some();
            json!({"check":"element_present","element_id":element_id,"actual":present,"passed":present})
        }).collect());
    }
    let metadata = wp
        .get(&format!("wp/v2/pages/{page_id}?context=edit"))
        .await
        .context("read native page link for visual verification")?;
    let link = metadata["link"]
        .as_str()
        .context("native page metadata omitted its link")?;
    let site = reqwest::Url::parse(wp.base_url())?;
    let url = site.join(link)?;
    if url.scheme() != site.scheme()
        || url.host_str() != site.host_str()
        || url.port_or_known_default() != site.port_or_known_default()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        anyhow::bail!("native page link is not a same-origin HTTP URL");
    }
    let mut results = Vec::new();
    for check in checks {
        if let PlanCheck::ElementPresent { element_id } = check {
            let present = tree.find(element_id).is_some();
            results.push(json!({"check":"element_present","element_id":element_id,"actual":present,"passed":present}));
            continue;
        }
        let viewport = match check {
            PlanCheck::ComputedStyle { viewport, .. }
            | PlanCheck::NoHorizontalOverflow { viewport }
            | PlanCheck::PageHeight { viewport, .. }
            | PlanCheck::BoundingBox { viewport, .. } => viewport.as_ref(),
            PlanCheck::ElementPresent { .. } => unreachable!(),
        };
        let width = viewport.map_or(900, |v| v.width);
        let height = viewport.map_or(900, |v| v.height);
        let browser = crate::cdp::browser().await?;
        let page = browser.new_page("about:blank").await?;
        let verified = async {
            page.execute(chromiumoxide::cdp::browser_protocol::network::EnableParams::default()).await?;
            let cache = chromiumoxide::cdp::browser_protocol::network::SetCacheDisabledParams::builder().cache_disabled(true).build().map_err(|error| anyhow::anyhow!("invalid cache setting: {error}"))?;
            page.execute(cache).await?;
            let mobile = viewport.is_some_and(|v| v.mobile);
            let metrics = chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams::builder().width(width).height(height).device_scale_factor(1.0).mobile(mobile).build().map_err(|error| anyhow::anyhow!("invalid CDP viewport: {error}"))?;
            page.execute(metrics).await?;
            page.goto(url.as_str()).await?;
            let guard = json!({"origin":site.origin().ascii_serialization(),"pageId":page_id});
            let ready_script = format!("(() => {{ const g={}; const n=performance.getEntriesByType('navigation')[0]; const id=new RegExp('(?:^|\\\\s)(?:page-id-|postid-)' + g.pageId + '(?:\\\\s|$)'); return JSON.stringify({{url:location.href,origin:location.origin,ok:location.origin===g.origin && id.test(document.body.className) && !!n && n.responseStatus >= 200 && n.responseStatus < 400}}); }})()", guard);
            let guard_result: String = page.evaluate(ready_script).await?.into_value()?;
            let guard_result: Value = serde_json::from_str(&guard_result)?;
            if guard_result["ok"] != true { anyhow::bail!("visual verification page identity failed: redirected, wrong page, or unsuccessful response ({})", guard_result["url"]); }
            page.evaluate("(async()=>{await document.fonts.ready; for(let y=0;y<document.documentElement.scrollHeight;y+=Math.max(200,innerHeight-100)){scrollTo(0,y); await new Promise(r=>setTimeout(r,80))} await Promise.all([...document.images].map(i=>new Promise((resolve,reject)=>{if(i.complete)return i.naturalWidth?resolve():reject(new Error('image failed to load'));i.addEventListener('load',resolve,{once:true});i.addEventListener('error',()=>reject(new Error('image failed to load')),{once:true});setTimeout(()=>reject(new Error('image load timed out')),3000)}))); await Promise.race([Promise.all(document.getAnimations().map(a=>a.finished)),new Promise(r=>setTimeout(r,1200))]); scrollTo(0,0); await new Promise(r=>setTimeout(r,250)); return 'ready'})()").await?;
            let (script, name) = check_script(check)?;
            let output: String = page.evaluate(script).await?.into_value()?;
            let actual: Value = serde_json::from_str(&output)?;
            let passed = actual["passed"] == true;
            Ok::<_, anyhow::Error>(json!({"check":name,"viewport":{"width":width,"height":height},"actual":actual,"passed":passed}))
        }.await;
        let closed = page.close().await;
        let result = match (verified, closed) {
            (Ok(value), Ok(())) => value,
            (Err(error), Ok(())) => return Err(error),
            (Ok(_), Err(error)) => {
                return Err(error).context("failed to close visual verification page");
            }
            (Err(error), Err(close_error)) => anyhow::bail!(
                "{error:#}; also failed to close visual verification page: {close_error:#}"
            ),
        };
        results.push(result);
    }
    Ok(results)
}

fn check_script(check: &PlanCheck) -> Result<(String, &'static str)> {
    let (name, body) = match check {
        PlanCheck::ComputedStyle { selector, property, expected, .. } => ("computed_style", format!("const e=document.querySelector({}); if(!e) return JSON.stringify({{passed:false,error:'selector_missing'}}); const actual=getComputedStyle(e).getPropertyValue({}).trim(); return JSON.stringify({{actual,expected:{},passed:actual==={}}});", serde_json::to_string(selector)?, serde_json::to_string(property)?, serde_json::to_string(expected)?, serde_json::to_string(expected)?)),
        PlanCheck::NoHorizontalOverflow { .. } => ("no_horizontal_overflow", "const actual=Math.max(document.documentElement.scrollWidth,document.body.scrollWidth); const expected=innerWidth; return JSON.stringify({actual,expected,passed:actual<=expected+1});".into()),
        PlanCheck::PageHeight { expected, max_delta_percent, .. } => ("page_height", format!("const actual=Math.max(document.documentElement.scrollHeight,document.body.scrollHeight); const expected={}; const delta=Math.abs(actual-expected)/expected*100; return JSON.stringify({{actual,expected,delta_percent:delta,max_delta_percent:{},passed:delta<={}}});", expected, max_delta_percent, max_delta_percent)),
        PlanCheck::BoundingBox { selector, width, height, tolerance_px, .. } => ("bounding_box", format!("const e=document.querySelector({}); if(!e) return JSON.stringify({{passed:false,error:'selector_missing'}}); const r=e.getBoundingClientRect(); const actual={{width:r.width,height:r.height}}; const expected={{width:{},height:{}}}; const tolerance={}; return JSON.stringify({{actual,expected,tolerance_px:tolerance,passed:Math.abs(r.width-expected.width)<=tolerance && Math.abs(r.height-expected.height)<=tolerance}});", serde_json::to_string(selector)?, width, height, tolerance_px)),
        PlanCheck::ElementPresent { .. } => anyhow::bail!("element_present uses the persisted Elementor tree"),
    };
    Ok((format!("(()=>{{ {} }})()", body), name))
}

#[async_trait]
impl Tool for RestoreElementorSnapshot {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "restore_elementor_snapshot",
            description: "Restore a generic Elementor snapshot or a native editor operation snapshot.",
            input_schema: json!({"type":"object","properties":{
                "snapshot_kind":{"type":"string","enum":["document","native_editor"],"default":"document"},
                "page_id":{"type":"integer","minimum":0},
                "operation_id":{"type":"string","pattern":"^[A-Za-z0-9_-]{8,128}$"},
                "expected_revision":{"type":"string"},
                "expected_state_fingerprint":{"type":"string","pattern":"^[A-Fa-f0-9]{64}$"}
            },"oneOf":[
                {"title":"Generic document snapshot","required":["page_id","operation_id","expected_revision"],"properties":{"snapshot_kind":{"enum":["document"]},"page_id":{"type":"integer","minimum":0}}},
                {"title":"Native editor snapshot","required":["snapshot_kind","page_id","operation_id","expected_state_fingerprint"],"properties":{"snapshot_kind":{"const":"native_editor"},"page_id":{"type":"integer","minimum":1},"operation_id":{"type":"string","pattern":"^[A-Za-z0-9_-]{8,128}$"},"expected_state_fingerprint":{"type":"string","pattern":"^[A-Fa-f0-9]{64}$"}}}
            ]}),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let snapshot_kind = match args.get("snapshot_kind") {
            None => "document",
            Some(Value::String(value)) => value.as_str(),
            Some(_) => anyhow::bail!("snapshot_kind must be a string"),
        };
        if snapshot_kind != "document" && snapshot_kind != "native_editor" {
            anyhow::bail!("snapshot_kind must be document or native_editor");
        }
        let operation_id = args
            .get("operation_id")
            .and_then(Value::as_str)
            .context("operation_id required")?;
        if !valid_editor_operation_id(operation_id) {
            anyhow::bail!(
                "operation_id must be 8 to 128 ASCII letters, digits, underscores, or hyphens"
            );
        }
        if snapshot_kind == "native_editor" {
            let page_id = args
                .get("page_id")
                .and_then(Value::as_u64)
                .filter(|id| *id > 0)
                .context("positive page_id required for native_editor snapshots")?;
            let expected = args
                .get("expected_state_fingerprint")
                .and_then(Value::as_str)
                .filter(|value| valid_sha256_fingerprint(value))
                .context("64-character hexadecimal expected_state_fingerprint required")?;
            let path = format!(
                "mcp-for-page-builders/v1/document/{page_id}/editor-operation/{operation_id}/restore"
            );
            let restore = wp
                .request(
                    "POST",
                    &path,
                    Some(&json!({"expected_state_fingerprint":expected})),
                    None,
                )
                .await;
            let restore = match restore {
                Ok(value) if value["restored"] == true => value,
                Ok(value) => {
                    return Ok(ToolResult::error(serde_json::to_string(
                        &json!({"partial_failure":true,"snapshot_kind":"native_editor","page_id":page_id,"operation_id":operation_id,"restore":value}),
                    )?));
                }
                Err(error) => {
                    return Ok(ToolResult::error(serde_json::to_string(
                        &json!({"partial_failure":true,"snapshot_kind":"native_editor","page_id":page_id,"operation_id":operation_id,"restore_error":error.to_string()}),
                    )?));
                }
            };
            let Some(restored_fingerprint) = restore["restored_fingerprint"].as_str() else {
                return Ok(ToolResult::error(serde_json::to_string(
                    &json!({"partial_failure":true,"snapshot_kind":"native_editor","page_id":page_id,"operation_id":operation_id,"restore":restore,"verification_error":"restore response omitted restored_fingerprint"}),
                )?));
            };
            if !valid_sha256_fingerprint(restored_fingerprint) {
                return Ok(ToolResult::error(serde_json::to_string(
                    &json!({"partial_failure":true,"snapshot_kind":"native_editor","page_id":page_id,"operation_id":operation_id,"restore":restore,"verification_error":"invalid restored_fingerprint"}),
                )?));
            }
            let receipt_path = format!(
                "mcp-for-page-builders/v1/document/{page_id}/editor-operation/{operation_id}"
            );
            let receipt = match wp.request("GET", &receipt_path, None, None).await {
                Ok(value) => value,
                Err(error) => {
                    return Ok(ToolResult::error(serde_json::to_string(
                        &json!({"partial_failure":true,"snapshot_kind":"native_editor","page_id":page_id,"operation_id":operation_id,"restored_fingerprint":restored_fingerprint,"restore":restore,"readback_error":error.to_string()}),
                    )?));
                }
            };
            let verified = receipt["status"] == "restored"
                && receipt["operation_id"].as_str() == Some(operation_id)
                && receipt["page_id"].as_u64() == Some(page_id)
                && receipt["post_id"].as_u64() == Some(page_id)
                && receipt["readback_verified"] == true
                && receipt["restored_fingerprint"].as_str() == Some(restored_fingerprint)
                && receipt["current_state_fingerprint"].as_str() == Some(restored_fingerprint);
            let output = json!({"restored":verified,"partial_failure":!verified,"snapshot_kind":"native_editor","page_id":page_id,"operation_id":operation_id,"restored_fingerprint":restored_fingerprint,"readback_verified":verified,"receipt":receipt});
            return Ok(if verified {
                ToolResult::text(serde_json::to_string(&output)?)
            } else {
                ToolResult::error(serde_json::to_string(&output)?)
            });
        }
        let page_id = args["page_id"].as_u64().context("page_id required")?;
        let expected_revision = args["expected_revision"]
            .as_str()
            .context("expected_revision required")?;
        let resources = wp
            .request(
                "POST",
                "mcp-for-page-builders/v1/resources/restore",
                Some(&json!({"operation_id":operation_id})),
                None,
            )
            .await?;
        if resources["restored"] == true && resources["document_snapshot"] == false {
            return Ok(ToolResult::text(serde_json::to_string(
                &json!({"restored":true,"resources":resources}),
            )?));
        }
        let result = ElementorService::new(wp)
            .restore_tree(page_id, operation_id, expected_revision)
            .await;
        let output = match result {
            Ok(document)
                if document["cache_regenerated"] == true && resources["restored"] != false =>
            {
                json!({"restored":true,"document":document,"resources":resources})
            }
            Ok(document) => {
                json!({"partial_failure":true,"document":document,"resources":resources})
            }
            Err(error) => {
                json!({"partial_failure":true,"document_error":error.to_string(),"resources":resources})
            }
        };
        Ok(ToolResult::text(serde_json::to_string(&output)?))
    }
}

fn valid_editor_operation_id(value: &str) -> bool {
    (8..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn valid_sha256_fingerprint(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn canonical_site(value: &str) -> Result<String> {
    let mut url = reqwest::Url::parse(value).context("site URL must be a valid absolute URL")?;
    let authority = value
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or_default())
        .unwrap_or_default();
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || authority.contains('@')
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        anyhow::bail!(
            "site URL must use HTTP or HTTPS and must not contain userinfo, a query, or a fragment"
        );
    }
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_site, valid_create_resource_id, valid_editor_operation_id,
        valid_sha256_fingerprint, validate_checks,
    };
    use crate::types::elementor_plan::PlanCheck;
    use serde_json::json;

    #[tokio::test]
    async fn restore_tool_schema_and_snapshot_kind_validation_are_explicit() {
        use crate::types::Tool;
        let tool = super::RestoreElementorSnapshot;
        let schema = tool.def().input_schema;
        assert_eq!(schema["properties"]["snapshot_kind"]["default"], "document");
        assert_eq!(
            schema["properties"]["snapshot_kind"]["enum"][1],
            "native_editor"
        );
        assert_eq!(schema["properties"]["page_id"]["minimum"], 0);
        assert_eq!(schema["oneOf"][1]["properties"]["page_id"]["minimum"], 1);
        let wp = crate::wp::WpClient::new("https://example.test", "user", "password");
        let outcome = tool.run(json!({"snapshot_kind":"unknown"}), &wp).await;
        assert!(
            outcome.is_err(),
            "unknown snapshot kind unexpectedly succeeded"
        );
        assert!(outcome.err().unwrap().to_string().contains("snapshot_kind"));
        for kind in [json!(null), json!(false), json!(4)] {
            assert!(tool.run(json!({"snapshot_kind":kind}), &wp).await.is_err());
        }
        let fingerprint = "a".repeat(64);
        assert!(tool.run(json!({"snapshot_kind":"native_editor","page_id":0,"operation_id":"guard-op_123","expected_state_fingerprint":fingerprint}), &wp).await.is_err());
        assert!(tool.run(json!({"snapshot_kind":"native_editor","page_id":"1","operation_id":"guard-op_123","expected_state_fingerprint":fingerprint}), &wp).await.is_err());
        assert!(tool.run(json!({"snapshot_kind":"native_editor","page_id":1,"operation_id":null,"expected_state_fingerprint":"{}"}), &wp).await.is_err());
        assert!(tool.run(json!({"snapshot_kind":"native_editor","page_id":1,"operation_id":"guard-op_123","expected_state_fingerprint":null}), &wp).await.is_err());
    }

    #[test]
    fn native_restore_identifiers_are_bounded_and_ascii() {
        assert!(valid_editor_operation_id("guard-op_123"));
        assert!(!valid_editor_operation_id("short"));
        assert!(!valid_editor_operation_id("über-long-id"));
        assert!(valid_sha256_fingerprint(&"a".repeat(64)));
        assert!(!valid_sha256_fingerprint(&"g".repeat(64)));
        assert!(!valid_sha256_fingerprint(&"a".repeat(63)));
    }

    #[test]
    fn canonical_site_normalizes_host_port_and_trailing_slash_but_preserves_path_case() {
        assert_eq!(
            canonical_site("HTTPS://Example.COM:443/Blog/").unwrap(),
            canonical_site("https://example.com/Blog").unwrap()
        );
        assert_ne!(
            canonical_site("https://example.com/Blog").unwrap(),
            canonical_site("https://example.com/blog").unwrap()
        );
    }

    #[test]
    fn canonical_site_rejects_userinfo_queries_fragments_and_non_http_schemes() {
        for url in [
            "https://user@example.com",
            "https://user:password@example.com",
            "https://example.com?",
            "https://example.com#",
            "ftp://example.com",
            "not a URL",
        ] {
            assert!(canonical_site(url).is_err(), "accepted {url}");
        }
    }

    #[test]
    fn create_resource_ids_accept_only_stable_pattern_aliases() {
        for id in ["new", "new_pattern_2", "new_pattern_18"] {
            assert!(valid_create_resource_id(id), "rejected {id}");
        }
        for id in [
            "New",
            "new_pattern_1",
            "new_pattern_02",
            "new_pattern_0",
            "new_pattern_x",
            "new_pattern_2x",
            "42",
        ] {
            assert!(!valid_create_resource_id(id), "accepted {id}");
        }
    }

    #[test]
    fn plan_checks_accept_known_shapes_and_reject_unknown_or_invalid_inputs() {
        let checks: Vec<PlanCheck> = serde_json::from_value(json!([
            {"check":"element_present","element_id":"abc1234"},
            {"check":"computed_style","selector":".hero h1","property":"color","expected":"red"},
            {"check":"no_horizontal_overflow"},
            {"check":"page_height","expected":900,"max_delta_percent":10.0},
            {"check":"bounding_box","selector":"#hero","width":300.0,"height":200.0,"tolerance_px":2.0}
        ])).unwrap();
        validate_checks(&checks).unwrap();
        for invalid in [
            json!([{"check":"old_metadata_check"}]),
            json!([{"check":"computed_style","selector":"[broken","property":"color","expected":"red"}]),
            json!([{"check":"page_height","expected":0,"max_delta_percent":10.0}]),
            json!([{"check":"no_horizontal_overflow","viewport":{"width":10,"height":900}}]),
        ] {
            let decoded = serde_json::from_value::<Vec<PlanCheck>>(invalid);
            assert!(decoded.is_err() || validate_checks(&decoded.unwrap()).is_err());
        }
    }
}
