use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::Engine;
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;

use crate::args::{str_arg, u64_arg};
use crate::cdp;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::types::tool_result::ToolContent;
use crate::wp::WpClient;

use super::{cdp_screenshot, unix_timestamp};

pub struct UiQualityAudit;

const DISCOVER_SELECTORS_JS: &str = r#"
(()=> {
  const sels = new Set();
  const semantic = ['header', 'nav', 'main', 'footer', 'section', 'article', 'aside', 'h1', 'h2'];
  semantic.forEach((tag) => {
    if (document.querySelector(tag)) sels.add(tag);
  });
  document.querySelectorAll('[class*=hero],[class*=header],[class*=footer],[class*=banner],[class*=nav]').forEach((el) => {
    if (el.id) {
      sels.add('#' + el.id);
      return;
    }
    if (!el.className || typeof el.className !== 'string') return;
    const c = el.className.split(/\s+/).find((cls) => /hero|header|footer|banner|nav/.test(cls));
    if (c) sels.add('.' + c);
  });
  for (const child of document.body.children) {
    if (child.id) {
      sels.add('#' + child.id);
      continue;
    }
    if (child.className && typeof child.className === 'string') {
      const cls = child.className.split(/\s+/)[0];
      if (cls) sels.add('.' + cls);
    }
  }
  return JSON.stringify([...sels]);
})()
"#;

const ACCESSIBILITY_AUDIT_JS: &str = r#"
(()=> {
  const limit = 25;
  const toPath = (el) => {
    if (!el || !el.tagName) return '';
    const id = el.id ? '#' + el.id : '';
    const cls = (el.className && typeof el.className === 'string')
      ? '.' + el.className.trim().split(/\s+/).slice(0,2).join('.')
      : '';
    return (el.tagName.toLowerCase() + id + cls).slice(0, 140);
  };

  const uniquePush = (arr, item) => {
    if (arr.length >= limit) return;
    if (!arr.includes(item)) arr.push(item);
  };

  const parseRgb = (color) => {
    const m = color.match(/rgba?\((\d+),\s*(\d+),\s*(\d+)/i);
    if (!m) return null;
    return [Number(m[1]), Number(m[2]), Number(m[3])];
  };

  const luminance = ([r, g, b]) => {
    const convert = (v) => {
      const c = v / 255;
      return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
    };
    const [rs, gs, bs] = [convert(r), convert(g), convert(b)];
    return 0.2126 * rs + 0.7152 * gs + 0.0722 * bs;
  };

  const contrastRatio = (fg, bg) => {
    const f = parseRgb(fg);
    const b = parseRgb(bg);
    if (!f || !b) return null;
    const l1 = luminance(f);
    const l2 = luminance(b);
    const [lighter, darker] = l1 > l2 ? [l1, l2] : [l2, l1];
    return (lighter + 0.05) / (darker + 0.05);
  };

  const effectiveBackground = (el) => {
    let cur = el;
    while (cur) {
      const c = getComputedStyle(cur).backgroundColor;
      if (c && c !== 'transparent' && c !== 'rgba(0, 0, 0, 0)') return c;
      cur = cur.parentElement;
    }
    return 'rgb(255, 255, 255)';
  };

  const samples = {
    images_without_alt: [],
    inputs_without_label: [],
    buttons_without_name: [],
    links_without_name: [],
    heading_order_jumps: [],
    low_contrast_text: []
  };

  document.querySelectorAll('img').forEach((img) => {
    const alt = img.getAttribute('alt');
    if (alt == null || alt.trim() === '') uniquePush(samples.images_without_alt, toPath(img));
  });

  document.querySelectorAll('input,textarea,select').forEach((field) => {
    const id = field.getAttribute('id');
    const hasAria = (field.getAttribute('aria-label') || '').trim().length > 0;
    const hasLabel = id ? document.querySelector(`label[for="${id}"]`) : null;
    if (!hasAria && !hasLabel) uniquePush(samples.inputs_without_label, toPath(field));
  });

  document.querySelectorAll('button,[role="button"]').forEach((button) => {
    const txt = (button.textContent || '').trim();
    const aria = (button.getAttribute('aria-label') || '').trim();
    if (!txt && !aria) uniquePush(samples.buttons_without_name, toPath(button));
  });

  document.querySelectorAll('a[href]').forEach((link) => {
    const txt = (link.textContent || '').trim();
    const aria = (link.getAttribute('aria-label') || '').trim();
    if (!txt && !aria) uniquePush(samples.links_without_name, toPath(link));
  });

  let prevLevel = 0;
  document.querySelectorAll('h1,h2,h3,h4,h5,h6').forEach((heading) => {
    const level = Number(heading.tagName.slice(1));
    if (prevLevel && level - prevLevel > 1) {
      uniquePush(samples.heading_order_jumps, `${toPath(heading)} (${prevLevel}->${level})`);
    }
    prevLevel = level;
  });

  document.querySelectorAll('p,span,a,button,label,li,h1,h2,h3,h4,h5,h6').forEach((el) => {
    const txt = (el.textContent || '').trim();
    if (!txt) return;
    const style = getComputedStyle(el);
    const fg = style.color;
    const bg = effectiveBackground(el);
    const ratio = contrastRatio(fg, bg);
    if (ratio !== null && ratio < 4.5) {
      uniquePush(samples.low_contrast_text, `${toPath(el)} (ratio ${ratio.toFixed(2)})`);
    }
  });

  const counts = Object.fromEntries(Object.entries(samples).map(([k, arr]) => [k, arr.length]));
  return JSON.stringify({ counts, samples });
})()
"#;

#[async_trait]
impl Tool for UiQualityAudit {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "ui_quality_audit",
            description: "Run a UI quality audit for a page: visual snapshot, optional expectation mismatch check (against reference_url), accessibility issue sampling, and Lighthouse metrics via PageSpeed API.\n\nWorkflow: Use after building/updating a page to quickly identify visual mismatches, accessibility gaps, and performance/SEO quality risks.",
            input_schema: json!({
                "type": "object",
                "required": ["url"],
                "properties": {
                    "url": { "type": "string", "description": "Target page URL to audit" },
                    "reference_url": { "type": "string", "description": "Optional expectation/reference URL for visual mismatch comparison" },
                    "selectors": { "type": "array", "items": { "type": "string" }, "description": "Optional CSS selectors used for mismatch checks. Auto-discovered if omitted." },
                    "output_dir": { "type": "string", "description": "Directory for saved screenshots (default: /tmp)" },
                    "width": { "type": "integer", "default": 1440 },
                    "height": { "type": "integer", "default": 900 },
                    "pre_js": { "type": "string", "description": "JavaScript to execute on target page before capture/audit" },
                    "pre_js_reference": { "type": "string", "description": "JavaScript to execute on reference page before capture/audit" },
                    "wait_ms": { "type": "integer", "default": 0, "description": "Delay after pre_js execution before audit/capture" },
                    "run_pagespeed": { "type": "boolean", "default": true, "description": "Fetch Lighthouse metrics from Google PageSpeed API" },
                    "pagespeed_strategy": { "type": "string", "enum": ["desktop", "mobile"], "default": "desktop" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let url = str_arg(&args, "url").ok_or_else(|| anyhow::anyhow!("url required"))?;
        let reference_url = str_arg(&args, "reference_url");
        let output_dir = str_arg(&args, "output_dir").unwrap_or_else(|| "/tmp".to_string());
        let width = u32::try_from(u64_arg(&args, "width").unwrap_or(1440)).unwrap_or(1440);
        let height = u32::try_from(u64_arg(&args, "height").unwrap_or(900)).unwrap_or(900);
        let pre_js = str_arg(&args, "pre_js");
        let pre_js_reference = str_arg(&args, "pre_js_reference");
        let wait_ms = u64_arg(&args, "wait_ms").unwrap_or(0);
        let run_pagespeed = args
            .get("run_pagespeed")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let pagespeed_strategy = str_arg(&args, "pagespeed_strategy")
            .filter(|value| value == "mobile" || value == "desktop")
            .unwrap_or_else(|| "desktop".to_string());
        let selectors: Vec<String> = args
            .get("selectors")
            .and_then(Value::as_array)
            .map(|array| {
                array
                    .iter()
                    .filter_map(|value| value.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default();

        let dir = Path::new(&output_dir);
        let stamp = unix_timestamp();
        let target_image_path = dir.join(format!("ui-audit-target-{stamp}.png"));
        let (target_bytes, target_warning) = match cdp_screenshot(
            &url,
            &target_image_path,
            width,
            height,
            pre_js.as_deref(),
            wait_ms,
        )
        .await
        {
            Ok(value) => value,
            Err(error) => {
                return Ok(ToolResult::error(format!(
                    "UI quality audit requires a working Chrome/CDP session for visual analysis: {error:#}",
                )));
            }
        };

        let accessibility =
            run_accessibility_audit(&url, width, height, pre_js.as_deref(), wait_ms).await?;

        let mut warnings = Vec::new();
        if let Some(warning) = target_warning {
            warnings.push(format!("target_page: {warning}"));
        }

        let mut reference_bytes: Option<Vec<u8>> = None;
        let mut reference_path: Option<String> = None;
        if let Some(reference) = reference_url.as_ref() {
            let ref_path = dir.join(format!("ui-audit-reference-{stamp}.png"));
            match cdp_screenshot(
                reference,
                &ref_path,
                width,
                height,
                pre_js_reference.as_deref(),
                wait_ms,
            )
            .await
            {
                Ok((bytes, warning)) => {
                    reference_bytes = Some(bytes);
                    reference_path = Some(ref_path.to_string_lossy().into_owned());
                    if let Some(w) = warning {
                        warnings.push(format!("reference_page: {w}"));
                    }
                }
                Err(error) => {
                    warnings.push(format!("reference screenshot failed: {error:#}"));
                }
            }
        }

        let visual_mismatch = if let Some(reference) = reference_url.as_ref() {
            match run_visual_mismatch_audit(
                &url,
                reference,
                &selectors,
                pre_js.as_deref(),
                pre_js_reference.as_deref(),
                wait_ms,
            )
            .await
            {
                Ok(report) => Some(report),
                Err(error) => {
                    warnings.push(format!("visual mismatch audit failed: {error:#}"));
                    None
                }
            }
        } else {
            None
        };

        let lighthouse = if run_pagespeed {
            match run_pagespeed_lighthouse(&url, &pagespeed_strategy).await {
                Ok(value) => Some(value),
                Err(error) => {
                    warnings.push(format!("lighthouse/pagespeed unavailable: {error:#}"));
                    None
                }
            }
        } else {
            None
        };

        let recommendations = build_recommendations(
            &accessibility,
            visual_mismatch.as_ref(),
            lighthouse.as_ref(),
        );

        let mut report = json!({
            "url": url,
            "target_screenshot": target_image_path.to_string_lossy().to_string(),
            "reference_url": reference_url,
            "reference_screenshot": reference_path,
            "accessibility": accessibility,
            "visual_mismatch": visual_mismatch,
            "lighthouse": lighthouse,
            "recommendations": recommendations,
        });
        if !warnings.is_empty() {
            report["warnings"] = json!(warnings);
        }

        let mut content = vec![ToolContent::Text {
            text: serde_json::to_string_pretty(&report)?,
        }];
        content.push(ToolContent::Image {
            data: base64::engine::general_purpose::STANDARD.encode(&target_bytes),
            mime_type: "image/png".to_string(),
        });
        if let Some(bytes) = reference_bytes {
            content.push(ToolContent::Image {
                data: base64::engine::general_purpose::STANDARD.encode(&bytes),
                mime_type: "image/png".to_string(),
            });
        }

        Ok(ToolResult::mixed(content))
    }
}

async fn run_accessibility_audit(
    url: &str,
    width: u32,
    height: u32,
    pre_js: Option<&str>,
    wait_ms: u64,
) -> Result<Value> {
    let (page, _) = cdp::open_page_with_js(url, width, height, pre_js, wait_ms).await?;
    let payload: String = page
        .evaluate(ACCESSIBILITY_AUDIT_JS)
        .await
        .context("CDP accessibility audit evaluate failed")?
        .into_value()
        .context("Failed to decode accessibility audit result")?;
    let report: Value =
        serde_json::from_str(&payload).context("Failed to parse accessibility audit JSON")?;
    Ok(report)
}

async fn run_visual_mismatch_audit(
    url_a: &str,
    url_b: &str,
    selectors: &[String],
    pre_js_a: Option<&str>,
    pre_js_b: Option<&str>,
    wait_ms: u64,
) -> Result<Value> {
    let (page_a, _) = cdp::open_page_with_js(url_a, 1440, 900, pre_js_a, wait_ms).await?;
    let (page_b, _) = cdp::open_page_with_js(url_b, 1440, 900, pre_js_b, wait_ms).await?;

    let mut effective_selectors = selectors.to_vec();
    if effective_selectors.is_empty() {
        let selectors_a: String = page_a.evaluate(DISCOVER_SELECTORS_JS).await?.into_value()?;
        let selectors_b: String = page_b.evaluate(DISCOVER_SELECTORS_JS).await?.into_value()?;
        let mut merged: Vec<String> = serde_json::from_str(&selectors_a)?;
        let extra: Vec<String> = serde_json::from_str(&selectors_b)?;
        for selector in extra {
            if !merged.contains(&selector) {
                merged.push(selector);
            }
        }
        effective_selectors = merged;
    }

    if effective_selectors.is_empty() {
        anyhow::bail!("No selectors found for visual mismatch audit");
    }

    let scan_js = build_visual_scan_js(&effective_selectors)?;
    let scan_a: String = page_a.evaluate(scan_js.clone()).await?.into_value()?;
    let scan_b: String = page_b.evaluate(scan_js).await?.into_value()?;
    let data_a: Value = serde_json::from_str(&scan_a)?;
    let data_b: Value = serde_json::from_str(&scan_b)?;

    let mut differences = Vec::new();
    let mut matched: u32 = 0;
    let mut total: u32 = 0;

    for selector in &effective_selectors {
        let value_a = data_a.get(selector);
        let value_b = data_b.get(selector);
        match (value_a, value_b) {
            (Some(Value::Null) | None, Some(Value::Null) | None) => {}
            (Some(_), Some(Value::Null) | None) => {
                total += 1;
                differences.push(json!({"selector": selector, "issue": "missing on reference", "severity": "high"}));
            }
            (Some(Value::Null) | None, Some(_)) => {
                total += 1;
                differences.push(
                    json!({"selector": selector, "issue": "missing on target", "severity": "high"}),
                );
            }
            (Some(target), Some(reference)) => {
                if let (Some(box_a), Some(box_b)) = (target.get("box"), reference.get("box")) {
                    for dim in ["w", "h"] {
                        total += 1;
                        let a = box_a.get(dim).and_then(Value::as_i64).unwrap_or_default();
                        let b = box_b.get(dim).and_then(Value::as_i64).unwrap_or_default();
                        if (a - b).abs() > 6 {
                            differences.push(json!({
                                "selector": selector,
                                "property": if dim == "w" { "width" } else { "height" },
                                "target": a,
                                "reference": b,
                                "severity": if (a - b).abs() > 24 { "high" } else { "medium" },
                            }));
                        } else {
                            matched += 1;
                        }
                    }
                }
                if let (Some(styles_a), Some(styles_b)) =
                    (target.get("styles"), reference.get("styles"))
                    && let (Some(map_a), Some(map_b)) = (styles_a.as_object(), styles_b.as_object())
                {
                    for (key, value) in map_a {
                        total += 1;
                        let other = map_b.get(key).unwrap_or(&Value::Null);
                        if value == other {
                            matched += 1;
                        } else {
                            differences.push(json!({
                                "selector": selector,
                                "property": key,
                                "target": value,
                                "reference": other,
                                "severity": "medium",
                            }));
                        }
                    }
                }
            }
        }
    }

    let score = if total > 0 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            (f64::from(matched) / f64::from(total) * 100.0) as u32
        }
    } else {
        100
    };
    let high_issues = differences
        .iter()
        .filter(|value| value.get("severity").and_then(Value::as_str) == Some("high"))
        .count();
    let medium_issues = differences
        .iter()
        .filter(|value| value.get("severity").and_then(Value::as_str) == Some("medium"))
        .count();

    Ok(json!({
        "match_score": score,
        "total_checks": total,
        "selectors_used": effective_selectors,
        "high_issues": high_issues,
        "medium_issues": medium_issues,
        "differences": differences,
    }))
}

fn build_visual_scan_js(selectors: &[String]) -> Result<String> {
    let selectors_json =
        serde_json::to_string(selectors).context("Failed to serialize selectors")?;
    Ok(format!(
        r"(()=>{{const selectors={selectors_json};const result={{}};selectors.forEach((sel)=>{{const el=document.querySelector(sel);if(!el){{result[sel]=null;return;}}const s=getComputedStyle(el);const b=el.getBoundingClientRect();result[sel]={{box:{{x:Math.round(b.x),y:Math.round(b.y),w:Math.round(b.width),h:Math.round(b.height)}},styles:{{'background-color':s.backgroundColor,color:s.color,'font-size':s.fontSize,'font-weight':s.fontWeight,display:s.display,gap:s.gap}},text:(el.textContent||'').trim().substring(0,100)}};}});return JSON.stringify(result);}})()",
    ))
}

async fn run_pagespeed_lighthouse(url: &str, strategy: &str) -> Result<Value> {
    let mut endpoint =
        reqwest::Url::parse("https://www.googleapis.com/pagespeedonline/v5/runPagespeed")
            .context("Failed to build PageSpeed endpoint")?;
    {
        let mut query = endpoint.query_pairs_mut();
        query.append_pair("url", url);
        query.append_pair("category", "performance");
        query.append_pair("category", "accessibility");
        query.append_pair("category", "best-practices");
        query.append_pair("category", "seo");
        query.append_pair("strategy", strategy);
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(45))
        .build()
        .context("Failed to build HTTP client")?;
    let response = client
        .get(endpoint)
        .send()
        .await
        .context("PageSpeed request failed")?;
    if !response.status().is_success() {
        anyhow::bail!("PageSpeed returned {}", response.status());
    }
    let payload: Value = response
        .json()
        .await
        .context("Failed to parse PageSpeed JSON")?;

    let categories = payload
        .pointer("/lighthouseResult/categories")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let audits = payload
        .pointer("/lighthouseResult/audits")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let metrics = json!({
        "first_contentful_paint": audits.pointer("/first-contentful-paint/displayValue").cloned().unwrap_or(Value::Null),
        "largest_contentful_paint": audits.pointer("/largest-contentful-paint/displayValue").cloned().unwrap_or(Value::Null),
        "speed_index": audits.pointer("/speed-index/displayValue").cloned().unwrap_or(Value::Null),
        "total_blocking_time": audits.pointer("/total-blocking-time/displayValue").cloned().unwrap_or(Value::Null),
        "cumulative_layout_shift": audits.pointer("/cumulative-layout-shift/displayValue").cloned().unwrap_or(Value::Null),
        "interactive": audits.pointer("/interactive/displayValue").cloned().unwrap_or(Value::Null),
    });

    Ok(json!({
        "source": "google_pagespeed_lighthouse",
        "strategy": strategy,
        "scores": {
            "performance": to_lighthouse_score(categories.pointer("/performance/score")),
            "accessibility": to_lighthouse_score(categories.pointer("/accessibility/score")),
            "best_practices": to_lighthouse_score(categories.pointer("/best-practices/score")),
            "seo": to_lighthouse_score(categories.pointer("/seo/score")),
        },
        "metrics": metrics,
    }))
}

fn to_lighthouse_score(value: Option<&Value>) -> Option<u64> {
    let score = value.and_then(Value::as_f64)?;
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    {
        Some((score * 100.0).round() as u64)
    }
}

fn build_recommendations(
    accessibility: &Value,
    visual_mismatch: Option<&Value>,
    lighthouse: Option<&Value>,
) -> Vec<String> {
    let mut recommendations = Vec::new();

    let images_without_alt = accessibility
        .pointer("/counts/images_without_alt")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    if images_without_alt > 0 {
        recommendations.push(format!(
            "Add descriptive alt text to {images_without_alt} image(s) to improve screen-reader accessibility."
        ));
    }

    let unlabeled_inputs = accessibility
        .pointer("/counts/inputs_without_label")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    if unlabeled_inputs > 0 {
        recommendations.push(format!(
            "Associate labels or aria-labels for {unlabeled_inputs} form control(s)."
        ));
    }

    let low_contrast = accessibility
        .pointer("/counts/low_contrast_text")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    if low_contrast > 0 {
        recommendations.push(format!(
            "Fix low-contrast text ({low_contrast} sample(s)) to meet WCAG AA (target contrast ratio >= 4.5:1)."
        ));
    }

    if let Some(mismatch) = visual_mismatch {
        let score = mismatch
            .get("match_score")
            .and_then(Value::as_u64)
            .unwrap_or(100);
        if score < 90 {
            recommendations.push(format!(
                "Visual parity score is {score}%. Align spacing, typography, and dimensions with the reference for better consistency."
            ));
        }
    }

    if let Some(light) = lighthouse {
        let perf = light
            .pointer("/scores/performance")
            .and_then(Value::as_u64)
            .unwrap_or(100);
        let acc = light
            .pointer("/scores/accessibility")
            .and_then(Value::as_u64)
            .unwrap_or(100);
        if perf < 80 {
            recommendations.push(format!(
                "Lighthouse performance score is {perf}. Optimize LCP/TBT by reducing render-blocking assets and script work."
            ));
        }
        if acc < 90 {
            recommendations.push(format!(
                "Lighthouse accessibility score is {acc}. Address semantic labels, heading structure, and contrast issues."
            ));
        }
    }

    if recommendations.is_empty() {
        recommendations.push(
            "No major quality blockers detected in this pass. Next: validate interaction states (hover/focus/error) and mobile breakpoints.".to_string(),
        );
    }

    recommendations
}
