use std::collections::HashSet;

use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::wp::WpClient;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheScope {
    Post(u64),
    Global,
    All,
}

impl CacheScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Post(_) => "post",
            Self::Global => "global",
            Self::All => "all",
        }
    }

    pub fn post_id(&self) -> Option<u64> {
        match self {
            Self::Post(id) => Some(*id),
            Self::Global | Self::All => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DetectedCache {
    pub id: String,
    pub layer: String,
    pub kind: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheReport {
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_id: Option<u64>,
    pub bridge_available: bool,
    pub detected: Vec<DetectedCache>,
    pub cleared: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl CacheReport {
    pub fn to_summary(&self) -> Value {
        let mut obj = json!({
            "scope": self.scope,
            "bridge_available": self.bridge_available,
            "detected": self.detected,
            "cleared": self.cleared,
        });
        if let Some(id) = self.post_id {
            obj["post_id"] = json!(id);
        }
        if !self.warnings.is_empty() {
            obj["warnings"] = json!(self.warnings);
        }
        obj
    }

    pub fn summary_line(&self) -> String {
        let cleared_part = if self.cleared.is_empty() {
            "none".to_string()
        } else {
            self.cleared.join(", ")
        };
        if self.warnings.is_empty() {
            format!("\nCache cleared: {cleared_part}")
        } else {
            format!(
                "\nCache cleared: {cleared_part} (warnings: {})",
                self.warnings.join("; ")
            )
        }
    }
}

const HEADER_SIGNATURES: &[(&str, &str, &str, &str)] = &[
    (
        "x-litespeed-cache",
        "litespeed-cache-header",
        "LiteSpeed Cache",
        "page_cache",
    ),
    (
        "x-wp-rocket",
        "wp-rocket-header",
        "WP Rocket",
        "page_cache",
    ),
    (
        "x-rocket-nginx-serving-static",
        "rocket-nginx-header",
        "WP Rocket (Nginx)",
        "page_cache",
    ),
    ("cf-cache-status", "cloudflare-header", "Cloudflare", "cdn"),
    ("cf-apo-via", "cloudflare-apo-header", "Cloudflare APO", "cdn"),
    (
        "x-wp-cf-super-cache",
        "cf-super-cache-header",
        "Super Page Cache for Cloudflare",
        "cdn",
    ),
    (
        "x-fastcgi-cache",
        "nginx-fastcgi-header",
        "Nginx FastCGI Cache",
        "reverse_proxy",
    ),
    (
        "x-nginx-cache",
        "nginx-cache-header",
        "Nginx Cache",
        "reverse_proxy",
    ),
    ("x-varnish", "varnish-header", "Varnish", "reverse_proxy"),
    (
        "x-kinsta-cache",
        "kinsta-cache-header",
        "Kinsta Cache",
        "page_cache",
    ),
    (
        "x-pantheon-styx-hostname",
        "pantheon-styx-header",
        "Pantheon Styx",
        "reverse_proxy",
    ),
    (
        "x-sg-cache",
        "sg-cache-header",
        "SG Optimizer",
        "page_cache",
    ),
    (
        "x-siteground-cache",
        "siteground-cache-header",
        "SG Optimizer",
        "page_cache",
    ),
    ("x-breeze-cache", "breeze-header", "Breeze", "page_cache"),
    (
        "x-flying-press-cache",
        "flying-press-header",
        "FlyingPress",
        "page_cache",
    ),
    (
        "x-nitro-cache",
        "nitropack-header",
        "NitroPack",
        "page_cache",
    ),
    (
        "x-cache",
        "http-x-cache-header",
        "HTTP Proxy Cache (X-Cache)",
        "reverse_proxy",
    ),
    (
        "x-cache-status",
        "http-x-cache-status-header",
        "HTTP Proxy Cache (X-Cache-Status)",
        "reverse_proxy",
    ),
    (
        "x-proxy-cache",
        "http-x-proxy-cache-header",
        "HTTP Proxy Cache (X-Proxy-Cache)",
        "reverse_proxy",
    ),
    (
        "x-cache-enabled",
        "http-x-cache-enabled-header",
        "HTTP Proxy Cache (X-Cache-Enabled)",
        "reverse_proxy",
    ),
];

/// Detect caching layers from HTTP response headers.
pub fn detect_cache_headers(headers: &HeaderMap) -> Vec<DetectedCache> {
    let mut detected = Vec::new();
    let mut seen_ids = HashSet::new();

    for &(header_name, id, layer, kind) in HEADER_SIGNATURES {
        if let Some(val) = headers.get(header_name).and_then(|v| v.to_str().ok()) {
            let trimmed = val.trim();
            if !trimmed.is_empty() && seen_ids.insert(id) {
                detected.push(DetectedCache {
                    id: id.to_string(),
                    layer: layer.to_string(),
                    kind: kind.to_string(),
                    source: "http_header".to_string(),
                    detail: Some(format!("{header_name}: {trimmed}")),
                });
            }
        }
    }

    if !seen_ids.contains("varnish-header")
        && let Some(via) = headers.get("via").and_then(|v| v.to_str().ok())
        && via.to_ascii_lowercase().contains("varnish")
    {
        seen_ids.insert("varnish-header");
        detected.push(DetectedCache {
            id: "varnish-header".to_string(),
            layer: "Varnish".to_string(),
            kind: "reverse_proxy".to_string(),
            source: "http_header".to_string(),
            detail: Some(format!("via: {}", via.trim())),
        });
    }

    if detected.is_empty()
        && let Some(age_val) = headers.get("age").and_then(|v| v.to_str().ok())
        && let Ok(age_num) = age_val.trim().parse::<u64>()
        && age_num > 0
    {
        detected.push(DetectedCache {
            id: "http-age-header".to_string(),
            layer: "HTTP Cache (Age)".to_string(),
            kind: "reverse_proxy".to_string(),
            source: "http_header".to_string(),
            detail: Some(format!("age: {age_num}")),
        });
    }

    detected
}

fn parse_detected_list(val: &Value) -> Vec<DetectedCache> {
    val.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|item| serde_json::from_value::<DetectedCache>(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn merge_detected(bridge: Vec<DetectedCache>, headers: Vec<DetectedCache>) -> Vec<DetectedCache> {
    let mut merged = bridge;
    merged.extend(headers);
    merged
}

async fn probe_headers_non_fatal(wp: &WpClient) -> (Vec<DetectedCache>, Option<String>) {
    let base = wp.base_url();
    if base.is_empty() {
        return (Vec::new(), None);
    }
    match wp.probe_headers(base).await {
        Ok(headers) => (detect_cache_headers(&headers), None),
        Err(e) => (Vec::new(), Some(format!("HTTP header probe skipped: {e}"))),
    }
}

/// Detect active WordPress/Elementor caches via the bridge and HTTP response headers.
pub async fn detect_caches(wp: &WpClient) -> CacheReport {
    let (bridge_res, (header_detected, header_warn)) = tokio::join!(
        wp.get("mcp-for-page-builders/v1/cache/status"),
        probe_headers_non_fatal(wp)
    );

    let mut warnings = Vec::new();
    if let Some(w) = header_warn {
        warnings.push(w);
    }

    let (bridge_available, bridge_detected) = match bridge_res {
        Ok(val) => (true, parse_detected_list(&val["detected"])),
        Err(e) => {
            warnings.push(format!("Bridge cache status unavailable: {e}"));
            (false, Vec::new())
        }
    };

    CacheReport {
        scope: "detect".to_string(),
        post_id: None,
        bridge_available,
        detected: merge_detected(bridge_detected, header_detected),
        cleared: Vec::new(),
        warnings,
    }
}

/// Clear Elementor CSS and any detected WordPress/plugin caches for the given scope.
/// Never fails the caller's mutation; non-fatal issues are recorded in `warnings`.
pub async fn clear_related_caches(wp: &WpClient, scope: CacheScope) -> CacheReport {
    let mut payload = json!({ "scope": scope.as_str() });
    if let Some(id) = scope.post_id() {
        payload["post_id"] = json!(id);
    }

    let (bridge_res, (header_detected, header_warn)) = tokio::join!(
        wp.post("mcp-for-page-builders/v1/cache/clear", &payload),
        probe_headers_non_fatal(wp)
    );

    let mut warnings = Vec::new();
    if let Some(w) = header_warn {
        warnings.push(w);
    }

    match bridge_res {
        Ok(val) => {
            let bridge_detected = parse_detected_list(&val["detected"]);
            let cleared = val["cleared"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            if let Some(bw) = val["warnings"].as_array() {
                for w in bw {
                    if let Some(s) = w.as_str() {
                        warnings.push(s.to_string());
                    }
                }
            }
            CacheReport {
                scope: scope.as_str().to_string(),
                post_id: scope.post_id(),
                bridge_available: true,
                detected: merge_detected(bridge_detected, header_detected),
                cleared,
                warnings,
            }
        }
        Err(bridge_err) => {
            warnings.push(format!(
                "Bridge cache clear unavailable ({bridge_err}); falling back to Elementor CSS cache clear"
            ));
            let mut cleared = Vec::new();
            match wp.clear_elementor_cache().await {
                Ok(()) => cleared.push("Elementor CSS (fallback)".to_string()),
                Err(e) => warnings.push(format!("Elementor CSS cache clear failed: {e}")),
            }
            CacheReport {
                scope: scope.as_str().to_string(),
                post_id: scope.post_id(),
                bridge_available: false,
                detected: header_detected,
                cleared,
                warnings,
            }
        }
    }
}

/// Clear related caches for `scope` and attach `cache_cleared` summary to a JSON object response.
pub async fn clear_and_attach(wp: &WpClient, scope: CacheScope, resp: &mut Value) -> CacheReport {
    let report = clear_related_caches(wp, scope).await;
    if let Some(obj) = resp.as_object_mut() {
        obj.insert("cache_cleared".to_string(), report.to_summary());
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderName, HeaderValue};

    fn headers_from(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (k, v) in pairs {
            map.insert(
                HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        map
    }

    #[test]
    fn detects_multiple_cache_headers() {
        let headers = headers_from(&[
            ("x-litespeed-cache", "hit"),
            ("cf-cache-status", "DYNAMIC"),
            ("x-varnish", "12345 67890"),
            ("age", "42"),
        ]);
        let detected = detect_cache_headers(&headers);
        let ids: Vec<&str> = detected.iter().map(|d| d.id.as_str()).collect();
        assert!(ids.contains(&"litespeed-cache-header"));
        assert!(ids.contains(&"cloudflare-header"));
        assert!(ids.contains(&"varnish-header"));
        // Age should not add a duplicate generic entry when specific proxy/CDN headers exist.
        assert!(!ids.contains(&"http-age-header"));
    }

    #[test]
    fn detects_varnish_from_via_and_age_fallback() {
        let via_headers = headers_from(&[("via", "1.1 varnish (Varnish/6.0)")]);
        let via_detected = detect_cache_headers(&via_headers);
        assert_eq!(via_detected.len(), 1);
        assert_eq!(via_detected[0].id, "varnish-header");

        let age_headers = headers_from(&[("age", "120")]);
        let age_detected = detect_cache_headers(&age_headers);
        assert_eq!(age_detected.len(), 1);
        assert_eq!(age_detected[0].id, "http-age-header");

        let zero_age = headers_from(&[("age", "0")]);
        assert!(detect_cache_headers(&zero_age).is_empty());
    }

    #[test]
    fn detects_wp_rocket_and_fastcgi_headers() {
        let headers = headers_from(&[
            ("x-wp-rocket", "1"),
            ("x-fastcgi-cache", "HIT"),
            ("x-cache", "HIT from backend"),
        ]);
        let detected = detect_cache_headers(&headers);
        let ids: Vec<&str> = detected.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "wp-rocket-header",
                "nginx-fastcgi-header",
                "http-x-cache-header"
            ]
        );
    }

    #[test]
    fn cache_report_summary_and_line_omit_empty_optional_fields() {
        let report = CacheReport {
            scope: "post".to_string(),
            post_id: Some(42),
            bridge_available: true,
            detected: vec![],
            cleared: vec!["Elementor Post CSS (#42)".to_string()],
            warnings: vec![],
        };
        let summary = report.to_summary();
        assert_eq!(summary["scope"], "post");
        assert_eq!(summary["post_id"], 42);
        assert!(summary.get("warnings").is_none());
        assert_eq!(
            report.summary_line(),
            "\nCache cleared: Elementor Post CSS (#42)"
        );
    }

    #[tokio::test]
    async fn clear_related_caches_falls_back_gracefully_when_unreachable() {
        let wp = WpClient::new("http://127.0.0.1:1", "user", "password");
        let report = clear_related_caches(&wp, CacheScope::Post(7)).await;
        assert!(!report.bridge_available);
        assert_eq!(report.scope, "post");
        assert_eq!(report.post_id, Some(7));
        assert!(!report.warnings.is_empty());
    }

    async fn spawn_mock_wp(bridge_available: bool) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let first_line = req.lines().next().unwrap_or("");
                    let (status, extra_headers, body) = if first_line.starts_with("HEAD / ")
                        || first_line.starts_with("GET / ")
                    {
                        (
                            "200 OK",
                            "x-litespeed-cache: hit\r\ncf-cache-status: DYNAMIC\r\n",
                            "",
                        )
                    } else if first_line.contains("/mcp-for-page-builders/v1/cache/clear")
                        && bridge_available
                    {
                        (
                            "200 OK",
                            "Content-Type: application/json\r\n",
                            r#"{"scope":"post","post_id":12,"detected":[{"id":"elementor-css","layer":"Elementor CSS Cache","kind":"css_cache","source":"wordpress_bridge"},{"id":"litespeed-cache","layer":"LiteSpeed Cache","kind":"page_cache","source":"wordpress_bridge"}],"cleared":["Elementor Post CSS (#12)","LiteSpeed Cache (post #12)"],"warnings":[]}"#,
                        )
                    } else if first_line.contains("/mcp-for-page-builders/v1/cache/status")
                        && bridge_available
                    {
                        (
                            "200 OK",
                            "Content-Type: application/json\r\n",
                            r#"{"detected":[{"id":"wp-object-cache","layer":"WP Object Cache","kind":"object_cache","source":"wordpress_bridge","detail":"Redis"}]}"#,
                        )
                    } else if first_line.starts_with("DELETE ")
                        && first_line.contains("/elementor/v1/cache")
                    {
                        ("200 OK", "Content-Type: application/json\r\n", r#"true"#)
                    } else {
                        (
                            "404 Not Found",
                            "Content-Type: application/json\r\n",
                            r#"{"code":"rest_no_route"}"#,
                        )
                    };
                    let resp = format!(
                        "HTTP/1.1 {status}\r\n{extra_headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                });
            }
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn clear_and_attach_merges_bridge_and_http_headers() {
        let (base_url, handle) = spawn_mock_wp(true).await;
        let wp = WpClient::new(&base_url, "user", "password");
        let mut resp = json!({"applied": true});
        let report = clear_and_attach(&wp, CacheScope::Post(12), &mut resp).await;
        handle.abort();

        assert!(report.bridge_available);
        assert_eq!(
            report.cleared,
            vec!["Elementor Post CSS (#12)", "LiteSpeed Cache (post #12)"]
        );
        let ids: Vec<&str> = report.detected.iter().map(|d| d.id.as_str()).collect();
        assert!(ids.contains(&"elementor-css"));
        assert!(ids.contains(&"litespeed-cache"));
        assert!(ids.contains(&"litespeed-cache-header"));
        assert!(ids.contains(&"cloudflare-header"));
        assert_eq!(resp["cache_cleared"]["scope"], "post");
        assert_eq!(resp["cache_cleared"]["post_id"], 12);
    }

    #[tokio::test]
    async fn clear_related_caches_falls_back_to_elementor_delete_when_bridge_404() {
        let (base_url, handle) = spawn_mock_wp(false).await;
        let wp = WpClient::new(&base_url, "user", "password");
        let report = clear_related_caches(&wp, CacheScope::Global).await;
        handle.abort();

        assert!(!report.bridge_available);
        assert_eq!(report.cleared, vec!["Elementor CSS (fallback)"]);
        let ids: Vec<&str> = report.detected.iter().map(|d| d.id.as_str()).collect();
        assert!(ids.contains(&"litespeed-cache-header"));
        assert!(ids.contains(&"cloudflare-header"));
    }

    #[tokio::test]
    async fn detect_caches_combines_bridge_status_and_http_headers() {
        let (base_url, handle) = spawn_mock_wp(true).await;
        let wp = WpClient::new(&base_url, "user", "password");
        let report = detect_caches(&wp).await;
        handle.abort();

        assert!(report.bridge_available);
        assert!(report.cleared.is_empty());
        let ids: Vec<&str> = report.detected.iter().map(|d| d.id.as_str()).collect();
        assert!(ids.contains(&"wp-object-cache"));
        assert!(ids.contains(&"litespeed-cache-header"));
        assert!(ids.contains(&"cloudflare-header"));
    }
}
