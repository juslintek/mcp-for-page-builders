use anyhow::{Context, bail, ensure};
use std::{env, path::PathBuf, process::Command};

#[allow(dead_code)]
pub struct NativeTestConfig {
    pub url: String,
    pub user: String,
    pub password: String,
    pub container: String,
    pub docker_user: String,
    pub wp_path_arg: String,
    pub cdp_port: Option<u16>,
    pub chrome: Option<PathBuf>,
    pub host: String,
}

impl NativeTestConfig {
    pub fn load(require_cdp: bool) -> anyhow::Result<Self> {
        let required = |name: &str| -> anyhow::Result<String> {
            let value = env::var(name).with_context(|| format!("{name} is required"))?;
            ensure!(!value.trim().is_empty(), "{name} must not be empty");
            Ok(value)
        };

        ensure!(
            env::var("MCP_TEST_ALLOW_MUTATIONS").as_deref() == Ok("isolated-fixtures"),
            "set MCP_TEST_ALLOW_MUTATIONS=isolated-fixtures to enable disposable fixture mutations"
        );
        let url = required("WP_TEST_URL")?;
        let user = required("WP_TEST_USER")?;
        let password = required("WP_TEST_PASS")?;
        let container = required("MCP_TEST_CONTAINER")?;
        let docker_user = required("MCP_TEST_DOCKER_USER")?;
        let wp_path = required("MCP_TEST_WP_PATH")?;
        ensure!(
            container
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')),
            "MCP_TEST_CONTAINER must be a Docker container name"
        );
        ensure!(
            docker_user
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')),
            "MCP_TEST_DOCKER_USER must be a container user name"
        );
        ensure!(
            wp_path.starts_with('/')
                && !wp_path.split('/').any(|part| part == "..")
                && !wp_path.chars().any(char::is_whitespace),
            "MCP_TEST_WP_PATH must be an absolute path without parent traversal or whitespace"
        );

        let (host, project) = validate_target(
            &url,
            &container,
            env::var("MCP_TEST_DDEV_PROJECT").ok().as_deref(),
        )?;
        let inspected = Command::new("docker")
            .args([
                "inspect",
                "--format",
                "{{.State.Running}} {{index .Config.Labels \"com.ddev.site-name\"}}",
                &container,
            ])
            .output()
            .context("inspect configured DDEV container")?;
        ensure!(
            inspected.status.success(),
            "configured MCP_TEST_CONTAINER could not be inspected"
        );
        let container_identity = String::from_utf8(inspected.stdout)
            .context("Docker returned a non-UTF-8 container identity")?;
        ensure!(
            container_identity.trim() == format!("true {project}"),
            "configured container must be running and carry the matching DDEV site label"
        );
        if host.ends_with(".ddev.site") {
            ensure!(
                host.strip_suffix(".ddev.site") == Some(project.as_str()),
                "WP_TEST_URL host and MCP_TEST_CONTAINER must identify the same DDEV project"
            );
        } else if host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
        {
            ensure!(
                env::var("MCP_TEST_DDEV_PROJECT").as_deref() == Ok(project.as_str()),
                "loopback WP_TEST_URL requires MCP_TEST_DDEV_PROJECT matching MCP_TEST_CONTAINER"
            );
        } else {
            bail!("WP_TEST_URL must use localhost, a loopback address, or a local .ddev.site host");
        }

        let cdp_port = match env::var("MCP_TEST_CDP_PORT") {
            Ok(value) => Some(
                value
                    .parse()
                    .context("MCP_TEST_CDP_PORT must be a valid TCP port")?,
            ),
            Err(_) if require_cdp => bail!("MCP_TEST_CDP_PORT is required for the browser probe"),
            Err(_) => None,
        };
        if let Some(port) = cdp_port {
            ensure!(port != 0, "MCP_TEST_CDP_PORT must be between 1 and 65535");
        }
        let chrome = if require_cdp {
            Some(match env::var_os("MCP_TEST_CHROME") {
                Some(path) => PathBuf::from(path),
                None => [
                    "/usr/bin/google-chrome",
                    "/usr/bin/chromium",
                    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                ]
                .into_iter()
                .map(PathBuf::from)
                .find(|path| path.is_file())
                .context("set MCP_TEST_CHROME to an installed Chrome or Chromium executable")?,
            })
        } else {
            None
        };

        Ok(Self {
            url,
            user,
            password,
            container,
            docker_user,
            wp_path_arg: format!("--path={wp_path}"),
            cdp_port,
            chrome,
            host,
        })
    }
}

fn validate_target(
    raw_url: &str,
    container: &str,
    loopback_project: Option<&str>,
) -> anyhow::Result<(String, String)> {
    let parsed = reqwest::Url::parse(raw_url).context("WP_TEST_URL must be a valid URL")?;
    ensure!(
        matches!(parsed.scheme(), "http" | "https")
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.query().is_none()
            && parsed.fragment().is_none()
            && parsed.path() == "/",
        "WP_TEST_URL must be an HTTP(S) site root without credentials, query, or fragment"
    );
    let host = parsed
        .host_str()
        .context("WP_TEST_URL must include a host")?
        .to_owned();
    ensure!(
        host != "karjera-recovery.ddev.site",
        "protected recovery site is forbidden for fixture tests"
    );
    let project = container
        .strip_prefix("ddev-")
        .and_then(|name| name.strip_suffix("-web"))
        .context("MCP_TEST_CONTAINER must identify a DDEV web container")?;
    ensure!(
        !project.is_empty(),
        "MCP_TEST_CONTAINER has no DDEV project name"
    );
    let loopback_host = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(&host)
        .parse::<std::net::IpAddr>()
        .is_ok_and(|ip| ip.is_loopback());
    if host.ends_with(".ddev.site") {
        ensure!(
            host.strip_suffix(".ddev.site") == Some(project),
            "WP_TEST_URL host and MCP_TEST_CONTAINER must identify the same DDEV project"
        );
    } else if host == "localhost" || loopback_host {
        ensure!(
            loopback_project == Some(project),
            "loopback WP_TEST_URL requires MCP_TEST_DDEV_PROJECT matching MCP_TEST_CONTAINER"
        );
    } else {
        bail!("WP_TEST_URL must use localhost, a loopback address, or a local .ddev.site host");
    }
    Ok((host, project.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::validate_target;

    #[test]
    fn accepts_only_local_urls_bound_to_the_configured_ddev_project() {
        let container = "ddev-karjera-mcp-integration-web";
        assert!(
            validate_target("https://karjera-mcp-integration.ddev.site", container, None).is_ok()
        );
        assert!(
            validate_target(
                "http://127.0.0.1:8080",
                container,
                Some("karjera-mcp-integration")
            )
            .is_ok()
        );
        assert!(
            validate_target(
                "http://[::1]:8080",
                container,
                Some("karjera-mcp-integration")
            )
            .is_ok()
        );
        for invalid in [
            "https://karjera-recovery.ddev.site",
            "https://karjera-mcp-integration.ddev.site/private",
            "https://karjera-mcp-integration.ddev.site/?x=1",
            "https://user:pass@karjera-mcp-integration.ddev.site",
            "https://example.invalid",
            "https://other-project.ddev.site",
            "http://192.0.2.4",
            "http://127.0.0.1:70000",
        ] {
            assert!(
                validate_target(invalid, container, Some("karjera-mcp-integration")).is_err(),
                "accepted {invalid}"
            );
        }
        assert!(validate_target("http://localhost", container, Some("another-project")).is_err());
        assert!(
            validate_target(
                "https://karjera-mcp-integration.ddev.site",
                "other-container",
                None
            )
            .is_err()
        );
    }
}
