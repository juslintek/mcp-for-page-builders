# wp-sqlite-elementor-server

Zero-dependency WordPress + Elementor development server. Single Alpine container, no MySQL, no Apache.

**FrankenPHP (Caddy) · PHP 8.4 · SQLite · Elementor · ProElements · hello-elementor**

## Quick Start

```bash
docker run -p 8080:8080 \
  -e WP_ADMIN_USER=admin \
  -e WP_ADMIN_PASS=admin \
  juslintek/wp-sqlite-elementor-server:latest
```

Open http://localhost:8080 — WordPress is ready with Elementor active.

## What's Inside

| Component | Version | Notes |
|---|---|---|
| FrankenPHP | Latest | Caddy + PHP in one process, HTTP/2, HTTP/3 |
| PHP | 8.4 (ZTS) | Thread-safe, OPcache enabled |
| WordPress | Latest | `/wp` |
| SQLite Database Integration | Latest | No MySQL needed |
| Elementor | Latest | Symlinked from `/opt/elementor-stack/` |
| ProElements | Latest | Symlinked from `/opt/elementor-stack/` |
| hello-elementor theme | Latest | Activated on auto-setup |

## Environment Variables

| Variable | Default | Description |
|---|---|---|
| `WP_ADMIN_USER` | *(none)* | Set to enable auto-setup. Omit for manual web install. |
| `WP_ADMIN_PASS` | `admin` | Admin password |
| `WP_ADMIN_EMAIL` | `admin@localhost` | Admin email |
| `WP_TITLE` | `MCP for Page Builders` | Site title |
| `WP_DEBUG` | `false` | Enable WordPress debug mode |

### Auto-setup vs Manual

- **With `WP_ADMIN_USER`**: WordPress installs automatically on first run. hello-elementor activated, default themes removed, permalinks configured, application password created at `/wp/app-password.txt`.
- **Without `WP_ADMIN_USER`**: Container starts PHP server, you complete setup via http://localhost:8080.

## Domain-Agnostic

WordPress URL is determined dynamically from the request `Host` header. No domain lock-in — works from:
- `http://localhost:8080`
- `http://192.168.1.100:8080`
- `https://my-site.example.com` (behind reverse proxy)

## Volumes & Mounts

```bash
# Persist database
docker run -p 8080:8080 \
  -v ./data/database:/wp/wp-content/database \
  -e WP_ADMIN_USER=admin \
  juslintek/wp-sqlite-elementor-server:latest

# Persist uploads
docker run -p 8080:8080 \
  -v ./data/database:/wp/wp-content/database \
  -v ./data/uploads:/wp/wp-content/uploads \
  -e WP_ADMIN_USER=admin \
  juslintek/wp-sqlite-elementor-server:latest

# Custom plugins
docker run -p 8080:8080 \
  -v ./data/database:/wp/wp-content/database \
  -v ./my-plugins:/wp/wp-content/plugins \
  -e WP_ADMIN_USER=admin \
  juslintek/wp-sqlite-elementor-server:latest

# Full wp-content mount (core stack still loads from /opt)
docker run -p 8080:8080 \
  -v ./wp-content:/wp/wp-content \
  -e WP_ADMIN_USER=admin \
  juslintek/wp-sqlite-elementor-server:latest
```

| Mount Point | Purpose |
|---|---|
| `/wp/wp-content/database` | SQLite database file |
| `/wp/wp-content/uploads` | Media uploads |
| `/wp/wp-content/plugins` | Custom plugins (Elementor/ProElements load from /opt regardless) |
| `/wp/wp-content/themes` | Custom themes |
| `/wp/wp-content` | Full wp-content (mu-plugins recreated on every boot) |

## Docker Compose

```yaml
services:
  wordpress:
    image: juslintek/wp-sqlite-elementor-server:latest
    ports:
      - "8080:8080"
    environment:
      WP_ADMIN_USER: admin
      WP_ADMIN_PASS: admin
      WP_DEBUG: "true"
    volumes:
      - ./data/database:/wp/wp-content/database
      - ./data/uploads:/wp/wp-content/uploads
```

Core stack lives in `/opt/elementor-stack/` and is symlinked into `wp-content/plugins/` on every boot — immune to `wp-content` mounts.

Architectures: `linux/amd64`, `linux/arm64`

## Architecture

```
FrankenPHP (Caddy + PHP 8.4 ZTS in one process)
  ├── HTTP/2, HTTP/3, gzip/br/zstd compression
  ├── OPcache enabled
  └── Serves /wp on :8080
/wp/                              WordPress root
├── wp-config.php                 Auto-generated, domain-agnostic
├── router.php                    PHP built-in server router
└── wp-content/
    ├── database/.ht.sqlite       SQLite database (mountable)
    ├── uploads/                  Media (mountable)
    ├── plugins/                  User plugins (mountable)
    ├── themes/                   User themes (mountable)
    ├── db.php                    SQLite drop-in (auto-created)
    └── mu-plugins/
        ├── elementor-stack.php   Loads Elementor + ProElements from /opt
        └── mcp-for-page-builders-config.php  REST API meta, app passwords

/opt/elementor-stack/             Baked into image, immune to mounts
├── sqlite-database-integration/
├── elementor/
├── pro-elements/
└── hello-elementor/
```

## Application Password

When using auto-setup, an application password is created and saved to `/wp/app-password.txt`. Read it for API access:

```bash
# From host
docker exec <container> cat /wp/app-password.txt

# Use with REST API
curl -u admin:<app-password> http://localhost:8080/wp-json/wp/v2/pages
```

## Building

```bash
# Local build
docker build -f Dockerfile.test -t wp-sqlite-elementor-server .

# Multi-arch build + push
docker buildx build --platform linux/amd64,linux/arm64 \
  -f Dockerfile.test -t juslintek/wp-sqlite-elementor-server:latest --push .
```

## Why FrankenPHP?

| | PHP built-in server | Apache/Nginx + FPM | FrankenPHP |
|---|---|---|---|
| Processes | 1 (single-threaded) | 2+ (web server + FPM pool) | 1 (threaded) |
| HTTP/2 | ❌ | ✅ (with config) | ✅ (automatic) |
| HTTP/3 | ❌ | ❌ (needs QUIC proxy) | ✅ (automatic) |
| Compression | ❌ | ✅ (with modules) | ✅ (zstd + br + gzip) |
| Worker mode | ❌ | ❌ | ✅ (keeps app in memory) |
| Config | None | nginx.conf + php-fpm.conf | Caddyfile (6 lines) |

## License

BSL-1.1 — free to use, cannot resell as a product. See [LICENSE](LICENSE).
