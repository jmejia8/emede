use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::Duration;
use tiny_http::{Header, Method, Response, Server};

use crate::markdown;
use crate::settings;

// ── Public state ──────────────────────────────────────────────────────────────

pub struct ShareState(pub Mutex<ShareStateInner>);

pub struct ShareStateInner {
    server: Option<Arc<Server>>,
    join: Option<JoinHandle<()>>,
    port: Option<u16>,
    /// Shared with the server thread; keyed by hash.
    route_map: Arc<RwLock<HashMap<String, NoteRoute>>>,
    /// path → hash reverse index.
    path_to_hash: HashMap<String, String>,
    /// Most recently shared path (for backwards-compat get_share_status).
    last_path: Option<String>,
    /// Random key used to authenticate URLs; generated at server start.
    server_key: Option<String>,
}

impl Default for ShareStateInner {
    fn default() -> Self {
        Self {
            server: None,
            join: None,
            port: None,
            route_map: Arc::new(RwLock::new(HashMap::new())),
            path_to_hash: HashMap::new(),
            last_path: None,
            server_key: None,
        }
    }
}

#[derive(Clone)]
struct NoteRoute {
    path: String,
    title: String,
    hash: String,
}

#[derive(Clone, Serialize)]
pub struct ShareInfo {
    pub url: String,
    pub home_url: String,
    pub ip: String,
    pub port: u16,
    pub hash: String,
    pub title: String,
}

// ── LAN helpers ───────────────────────────────────────────────────────────────

/// Best-effort discovery of this machine's primary LAN address. Connecting a UDP
/// socket does not send any packets; it just lets the OS pick the outbound
/// interface so we can read its local address.
fn local_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let addr = socket.local_addr().ok()?;
    let ip = addr.ip();
    if ip.is_unspecified() {
        None
    } else {
        Some(ip)
    }
}

/// Check if anything is still listening on `port` on localhost.
fn is_port_alive(port: u16) -> bool {
    TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(50),
    )
    .is_ok()
}

// ── Persistent per-file hash storage ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShareRouteEntry {
    hash: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct ShareRouteFile {
    #[serde(default)]
    routes: HashMap<String, ShareRouteEntry>,
    /// Port we successfully bound to last time; tried first on the next start.
    #[serde(default)]
    preferred_port: Option<u16>,
    /// Random key that authenticates URLs for this server instance.
    #[serde(default)]
    key: Option<String>,
}

fn share_routes_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("emede")
        .join("share_routes.json")
}

fn load_share_routes() -> ShareRouteFile {
    crate::persist::load_json_or_backup(&share_routes_path())
}

fn save_share_routes(file: &ShareRouteFile) {
    match serde_json::to_string_pretty(file) {
        Ok(json) => {
            if let Err(e) = crate::persist::write_json_atomic(&share_routes_path(), &json) {
                eprintln!("emede: failed to save share routes: {e}");
            }
        }
        Err(e) => eprintln!("emede: failed to serialize share routes: {e}"),
    }
}

// ── Cross-instance active-shares registry ─────────────────────────────────────
//
// All running emede instances write their active shares to this shared JSON file
// so the home page at "/" can list notes from every instance on the machine.
// Stale entries (process crashed) are detected by checking if the port is still
// reachable, so no cleanup daemon is needed.

#[derive(Debug, Serialize, Deserialize, Default)]
struct ActiveSharesFile {
    instances: HashMap<String, InstanceEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct InstanceEntry {
    port: u16,
    notes: Vec<ActiveNoteEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ActiveNoteEntry {
    path: String,
    hash: String,
    title: String,
}

fn active_shares_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("emede")
        .join("active_shares.json")
}

fn load_active_shares() -> ActiveSharesFile {
    crate::persist::load_json_or_backup(&active_shares_path())
}

fn save_active_shares(file: &ActiveSharesFile) {
    match serde_json::to_string_pretty(file) {
        Ok(json) => {
            if let Err(e) = crate::persist::write_json_atomic(&active_shares_path(), &json) {
                eprintln!("emede: failed to save active shares: {e}");
            }
        }
        Err(e) => eprintln!("emede: failed to serialize active shares: {e}"),
    }
}

fn sync_active_shares(inner: &ShareStateInner) {
    let pid = std::process::id().to_string();
    let current_port = inner.port;
    let mut file = load_active_shares();

    // Prune dead instances while we're here.
    // Also prune entries whose port matches ours but belong to a different PID:
    // only one process can bind a port, so the other PID must be a stale entry
    // from a crashed instance.
    file.instances.retain(|p, entry| {
        if p == &pid {
            return true;
        }
        if !is_port_alive(entry.port) {
            return false;
        }
        // Same port as ours with a different PID → stale.
        if current_port == Some(entry.port) {
            return false;
        }
        true
    });

    let map = inner.route_map.read().unwrap_or_else(|e| e.into_inner());
    match inner.port {
        Some(port) if !map.is_empty() => {
            let notes: Vec<ActiveNoteEntry> = map
                .values()
                .map(|r| ActiveNoteEntry {
                    path: r.path.clone(),
                    hash: r.hash.clone(),
                    title: r.title.clone(),
                })
                .collect();
            file.instances.insert(pid, InstanceEntry { port, notes });
        }
        _ => {
            file.instances.remove(&pid);
        }
    }

    save_active_shares(&file);
}

// ── Hash / key generation ─────────────────────────────────────────────────────

/// Minimum length (hex chars) of a valid auth key. Keys persisted by older
/// versions were 8 chars (32-bit); anything shorter than a full 128-bit key is
/// treated as legacy and regenerated on next share.
const MIN_KEY_LEN: usize = 32;

/// Fill a buffer with cryptographically secure random bytes and hex-encode it.
/// A failure of the OS RNG is unrecoverable for a security token, so we panic
/// rather than fall back to weak entropy.
fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("OS random number generator unavailable");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Short, unguessable token used as the served route (64-bit).
fn random_hash() -> String {
    random_hex(8)
}

/// URL auth key, generated once per server instance so the same key protects
/// all notes (128-bit).
fn generate_key() -> String {
    random_hex(16)
}

// ── URL query key parsing ─────────────────────────────────────────────────────

/// Split a request URL into (path, extracted_key).
/// Returns the path portion and the value of the `key` query parameter if present.
fn parse_url_key(url: &str) -> (String, Option<String>) {
    let Some((path, query)) = url.split_once('?') else {
        return (url.to_string(), None);
    };
    let key = query
        .split('&')
        .find_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            match (parts.next(), parts.next()) {
                (Some("key"), Some(v)) => Some(v.to_string()),
                _ => None,
            }
        });
    (path.to_string(), key)
}

// ── Image inlining helpers ────────────────────────────────────────────────────

fn is_remote_src(src: &str) -> bool {
    let lower = src.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("data:")
        || lower.starts_with("mailto:")
        || lower.starts_with("tel:")
}

fn mime_for_extension(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

/// Largest local image (bytes) that will be inlined into a shared page.
const MAX_INLINE_IMAGE_BYTES: u64 = 10 * 1024 * 1024;

/// Replace local-file `src` values on a single `<img>` tag with a base64 `data:`
/// URI so the served page is fully self-contained. Remote URLs are left as-is.
///
/// Inlining is confined to `base_dir` (the shared note's canonicalized parent
/// directory): a note can only inline images that live alongside it, never
/// arbitrary paths like `/etc/shadow` or `~/.ssh/id_rsa`. Canonicalizing the
/// image path first defeats `../` traversal and symlink escapes. If `base_dir`
/// is `None` (e.g. a remote note) nothing local is inlined.
fn inline_img_tag_src(tag: &str, base_dir: Option<&Path>) -> String {
    let lower = tag.to_ascii_lowercase();
    let Some(src_idx) = lower.find("src=") else {
        return tag.to_string();
    };

    let after_src = &tag[src_idx + 4..];
    let (quote, rest) = if let Some(rest) = after_src.strip_prefix('"') {
        ('"', rest)
    } else if let Some(rest) = after_src.strip_prefix('\'') {
        ('\'', rest)
    } else {
        return tag.to_string();
    };

    let Some(end_quote) = rest.find(quote) else {
        return tag.to_string();
    };

    let raw_src = &rest[..end_quote];
    if is_remote_src(raw_src) {
        return tag.to_string();
    }

    let path = Path::new(raw_src);
    if !path.is_absolute() {
        return tag.to_string();
    }

    // Confine to the note's directory. No base dir → never inline local files.
    let Some(base) = base_dir else {
        return tag.to_string();
    };
    let Ok(canon) = path.canonicalize() else {
        return tag.to_string();
    };
    if !canon.starts_with(base) {
        return tag.to_string();
    }

    // Skip oversized images rather than buffer them fully into memory.
    match std::fs::metadata(&canon) {
        Ok(meta) if meta.len() > MAX_INLINE_IMAGE_BYTES => return tag.to_string(),
        Ok(_) => {}
        Err(_) => return tag.to_string(),
    }

    let Ok(bytes) = std::fs::read(&canon) else {
        return tag.to_string();
    };

    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let data_uri = format!("data:{};base64,{}", mime_for_extension(&canon), encoded);
    let prefix = &tag[..src_idx + 4 + 1];
    format!("{prefix}{data_uri}{quote}{}", &rest[end_quote + 1..])
}

/// Inline every local `<img>` source in the rendered HTML as a `data:` URI,
/// confined to `base_dir` (see [`inline_img_tag_src`]).
fn inline_local_images(html: &str, base_dir: Option<&Path>) -> String {
    let mut result = String::with_capacity(html.len());
    let lower_html = html.to_ascii_lowercase();
    let mut search_from = 0;

    while let Some(rel) = lower_html[search_from..].find("<img") {
        let start = search_from + rel;
        let Some(tag_end_rel) = html[start..].find('>') else {
            result.push_str(&html[search_from..]);
            return result;
        };
        let end = start + tag_end_rel + 1;

        result.push_str(&html[search_from..start]);
        result.push_str(&inline_img_tag_src(&html[start..end], base_dir));
        search_from = end;
    }

    result.push_str(&html[search_from..]);
    result
}

// ── Page building ─────────────────────────────────────────────────────────────

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn font_size_pt(value: &str) -> u32 {
    value
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(12)
}

/// Render `path` (local file or remote URL) into a self-contained HTML page for LAN clients.
pub fn build_shared_page(path: &str) -> Result<String, String> {
    let result = markdown::render_markdown_any(path)?;
    // Only local notes get a confinement directory; remote notes never inline
    // local files.
    let base_dir = if markdown::is_remote_url(path) {
        None
    } else {
        Path::new(path).parent().and_then(|p| p.canonicalize().ok())
    };
    let content = inline_local_images(&result.html, base_dir.as_deref());
    let settings = settings::load_settings();

    let body_font = if settings.font_family.trim().is_empty() {
        "serif".to_string()
    } else {
        settings.font_family.clone()
    };
    let code_font = if settings.font_code.trim().is_empty() {
        "monospace".to_string()
    } else {
        settings.font_code.clone()
    };

    let page = SHARED_PAGE_TEMPLATE
        .replace("{{TITLE}}", &escape_html(&result.title))
        .replace("{{FG}}", &settings.color_fg)
        .replace("{{BG}}", &settings.color_bg)
        .replace("{{SIZE}}", &font_size_pt(&settings.font_size).to_string())
        .replace("{{FONT}}", &body_font)
        .replace("{{FONT_CODE}}", &code_font)
        .replace("{{USER}}", &escape_html(&host_user_label()))
        .replace("{{REPO}}", EMEDE_REPO_URL)
        .replace("{{CONTENT}}", &content);

    Ok(page)
}

/// The emede logo (a monochrome cat mark) embedded so the LAN-served home page
/// stays self-contained. Encoded to a `data:` URI once, on first use.
fn logo_data_uri() -> &'static str {
    use std::sync::OnceLock;
    static LOGO: OnceLock<String> = OnceLock::new();
    LOGO.get_or_init(|| {
        let bytes = include_bytes!("../../src/assets/logo.png");
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        format!("data:image/png;base64,{encoded}")
    })
}

/// Public URL of the emede source repository, linked from the shared home page.
const EMEDE_REPO_URL: &str = "https://github.com/jmejia8/emede";

/// The footer line shown on shared pages, taken verbatim from the user's
/// setting (e.g. "Shared by jesus"). The hostname is deliberately omitted to
/// avoid disclosing a targetable machine name to everyone on the LAN. Falls
/// back to the OS username, then to a generic label.
fn host_user_label() -> String {
    let configured = settings::load_settings().share_username.trim().to_string();
    if !configured.is_empty() {
        return configured;
    }
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
        .map(|u| format!("Shared by {u}"))
        .unwrap_or_else(|| "Shared by someone".to_string())
}

fn build_home_page(
    route_map: &HashMap<String, NoteRoute>,
    ip: &str,
    port: u16,
    key: &str,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let pid = std::process::id().to_string();

    // Collect (title, url, filename) across all live instances.
    let mut entries: Vec<(String, String, String)> = Vec::new();

    for note in route_map.values() {
        let url = format!("http://{}:{}/{}?key={}", ip, port, note.hash, key);
        let filename = Path::new(&note.path)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| note.path.clone());
        entries.push((note.title.clone(), url, filename));
    }

    let shares = load_active_shares();
    for (instance_pid, entry) in &shares.instances {
        if *instance_pid == pid {
            continue;
        }
        if !is_port_alive(entry.port) {
            continue;
        }
        for note in &entry.notes {
            let url = format!("http://{}:{}/{}?key={}", ip, entry.port, note.hash, key);
            let filename = Path::new(&note.path)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| note.path.clone());
            entries.push((note.title.clone(), url, filename));
        }
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let body = if entries.is_empty() {
        r#"<p class="empty">No notes are currently being shared…</p>"#.to_string()
    } else {
        let mut s = String::from("<ul>");
        for (title, url, filename) in &entries {
            s.push_str(&format!(
                r#"<li><a href="{}"><span class="title">{}</span><span class="path">{}</span></a></li>"#,
                escape_html(url),
                escape_html(title),
                escape_html(filename),
            ));
        }
        s.push_str("</ul>");
        s
    };

    let html = format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>emede — shared notes</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&family=Playfair+Display:wght@500;700&display=swap" rel="stylesheet">
<style>
  *, *::before, *::after {{ box-sizing: border-box; }}
  :root {{
    --color-bg: #faf8f5;
    --color-surface: #ffffff;
    --color-fg: #1a1a1a;
    --color-muted: #8b8782;
    --color-border: #e6e1db;
    --color-border-hover: #cdc7bf;
    --color-link: #3d5a80;
    --color-link-hover: #1a365d;
    --focus-ring: rgba(61, 90, 128, 0.35);
    --logo-invert: 0;
    --shadow-sm: 0 1px 3px rgba(0,0,0,0.04), 0 1px 2px rgba(0,0,0,0.03);
    --shadow-md: 0 4px 12px rgba(0,0,0,0.05), 0 2px 4px rgba(0,0,0,0.03);
  }}
  /* Dark palette: applied when the OS prefers dark AND the user hasn't forced
     a theme, or when the user explicitly selects dark. */
  @media (prefers-color-scheme: dark) {{
    :root:not([data-theme="light"]) {{
      --color-bg: #17171a;
      --color-surface: #212127;
      --color-fg: #e8e6e3;
      --color-muted: #928e88;
      --color-border: #33333b;
      --color-border-hover: #4a4a55;
      --color-link: #9db8dc;
      --color-link-hover: #c3d4ee;
      --focus-ring: rgba(157, 184, 220, 0.4);
      --logo-invert: 1;
      --shadow-sm: 0 1px 3px rgba(0,0,0,0.3), 0 1px 2px rgba(0,0,0,0.24);
      --shadow-md: 0 4px 12px rgba(0,0,0,0.36), 0 2px 4px rgba(0,0,0,0.28);
    }}
  }}
  :root[data-theme="dark"] {{
    --color-bg: #17171a;
    --color-surface: #212127;
    --color-fg: #e8e6e3;
    --color-muted: #928e88;
    --color-border: #33333b;
    --color-border-hover: #4a4a55;
    --color-link: #9db8dc;
    --color-link-hover: #c3d4ee;
    --focus-ring: rgba(157, 184, 220, 0.4);
    --logo-invert: 1;
    --shadow-sm: 0 1px 3px rgba(0,0,0,0.3), 0 1px 2px rgba(0,0,0,0.24);
    --shadow-md: 0 4px 12px rgba(0,0,0,0.36), 0 2px 4px rgba(0,0,0,0.28);
  }}
  html {{
    touch-action: manipulation;
    -webkit-tap-highlight-color: transparent;
    color-scheme: light dark;
  }}
  body {{
    font-family: 'Inter', system-ui, -apple-system, sans-serif;
    max-width: 38rem;
    margin: 0 auto;
    padding: 3rem 1.25rem 5rem;
    background: var(--color-bg);
    color: var(--color-fg);
    line-height: 1.6;
    -webkit-font-smoothing: antialiased;
  }}
  header {{
    text-align: center;
    margin-bottom: 2.5rem;
  }}
  .logo {{
    width: 64px;
    height: 64px;
    margin: 0 auto 1rem;
    display: block;
    filter: invert(var(--logo-invert));
  }}
  .theme-toggle {{
    position: fixed;
    top: 1rem;
    right: 1rem;
    width: 40px;
    height: 40px;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border: 1px solid var(--color-border);
    border-radius: 50%;
    background: var(--color-surface);
    color: var(--color-fg);
    box-shadow: var(--shadow-sm);
    cursor: pointer;
    transition: border-color 0.15s ease, box-shadow 0.15s ease;
  }}
  .theme-toggle:hover {{
    border-color: var(--color-border-hover);
    box-shadow: var(--shadow-md);
  }}
  .theme-toggle:focus-visible {{
    outline: none;
    box-shadow: 0 0 0 3px var(--focus-ring), var(--shadow-sm);
    border-color: var(--color-link);
  }}
  .theme-toggle svg {{
    width: 20px;
    height: 20px;
  }}
  /* Only the icon matching the active mode is shown. */
  .theme-toggle .icon {{ display: none; }}
  :root:not([data-theme]) .theme-toggle .icon-auto,
  :root[data-theme="auto"] .theme-toggle .icon-auto,
  :root[data-theme="light"] .theme-toggle .icon-light,
  :root[data-theme="dark"] .theme-toggle .icon-dark {{ display: block; }}
  h1 {{
    font-family: 'Playfair Display', Georgia, serif;
    font-size: 1.8rem;
    font-weight: 700;
    margin: 0 0 0.35rem;
    letter-spacing: -0.01em;
    text-wrap: balance;
  }}
  .sub {{
    color: var(--color-muted);
    font-size: 0.9rem;
    margin: 0;
    font-weight: 400;
  }}
  ul {{
    list-style: none;
    padding: 0;
    margin: 0;
    display: grid;
    gap: 0.6rem;
  }}
  li {{
    margin: 0;
    transition: transform 0.15s ease, box-shadow 0.15s ease;
  }}
  li a {{
    display: block;
    text-decoration: none;
    padding: 0.9rem 1.1rem;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 10px;
    box-shadow: var(--shadow-sm);
    transition: box-shadow 0.15s ease, border-color 0.15s ease;
  }}
  li a:hover {{
    box-shadow: var(--shadow-md);
    border-color: var(--color-border-hover);
  }}
  li a:focus-visible {{
    outline: none;
    box-shadow: 0 0 0 3px var(--focus-ring), var(--shadow-md);
    border-color: var(--color-link);
  }}
  .title {{
    font-size: 1.05rem;
    font-weight: 500;
    color: var(--color-link);
    display: block;
    margin-bottom: 0.2rem;
  }}
  li a:hover .title {{
    color: var(--color-link-hover);
  }}
  .path {{
    font-size: 0.78rem;
    color: var(--color-muted);
    display: block;
  }}
  .empty {{
    text-align: center;
    color: var(--color-muted);
    font-style: italic;
    padding: 3rem 0;
  }}
  footer {{
    margin-top: 3.5rem;
    padding-top: 1.5rem;
    border-top: 1px solid var(--color-border);
    text-align: center;
    font-size: 0.8rem;
    color: var(--color-muted);
    line-height: 1.7;
  }}
  footer .who {{
    display: block;
    margin-bottom: 0.35rem;
  }}
  footer a {{
    color: var(--color-link);
    text-decoration: none;
    font-weight: 500;
  }}
  footer a:hover {{
    color: var(--color-link-hover);
    text-decoration: underline;
  }}
  footer a:focus-visible {{
    outline: none;
    border-radius: 3px;
    box-shadow: 0 0 0 3px var(--focus-ring);
    color: var(--color-link-hover);
  }}
  @media (prefers-reduced-motion: reduce) {{
    li, li a, .theme-toggle {{
      transition: none;
    }}
  }}
</style>
</head>
<body>
<button id="theme-toggle" class="theme-toggle" type="button" aria-label="Switch color theme" title="Switch color theme">
  <svg class="icon icon-auto" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M12 3a9 9 0 0 0 0 18z" fill="currentColor" stroke="none"/></svg>
  <svg class="icon icon-light" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="4.5"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/></svg>
  <svg class="icon icon-dark" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z"/></svg>
</button>
<header>
<img class="logo" src="{logo}" alt="emede logo" width="64" height="64">
<h1>Shared Notes</h1>
<p class="sub">Notes currently shared on this network</p>
</header>
{body}
<footer>
<span class="who">{user}</span>
<span>Powered by <a href="{repo}" target="_blank" rel="noopener noreferrer">emede</a></span>
</footer>
<script>
  (function () {{
    var root = document.documentElement;
    var btn = document.getElementById("theme-toggle");
    var order = ["auto", "light", "dark"];
    var labels = {{ auto: "System theme", light: "Light theme", dark: "Dark theme" }};

    function apply(mode) {{
      if (mode === "auto") {{
        root.removeAttribute("data-theme");
      }} else {{
        root.setAttribute("data-theme", mode);
      }}
      btn.setAttribute("aria-label", labels[mode] + " (click to switch)");
      btn.setAttribute("title", labels[mode]);
    }}

    var saved = localStorage.getItem("emede-share-theme");
    var current = order.indexOf(saved) !== -1 ? saved : "auto";
    apply(current);

    btn.addEventListener("click", function () {{
      current = order[(order.indexOf(current) + 1) % order.length];
      localStorage.setItem("emede-share-theme", current);
      apply(current);
    }});
  }})();
</script>
</body>
</html>"##,
        logo = logo_data_uri(),
        user = escape_html(&host_user_label()),
        repo = EMEDE_REPO_URL,
    );

    html_response(html, 200)
}

/// The key-request ("unlock") page shown when a request arrives without a valid
/// key. It is a plain `<form method="get" action="/">` whose single `key` input
/// becomes the `?key=` query parameter on submit: a correct key lands on the
/// home page, a wrong one bounces back here. `attempted` is true when the request
/// carried a (wrong) key, so we can show an inline error instead of a cold prompt.
fn build_unlock_page(attempted: bool) -> Response<std::io::Cursor<Vec<u8>>> {
    let error = if attempted {
        r#"<p id="key-error" class="error" role="alert">That key didn't match. Try again.</p>"#
    } else {
        ""
    };
    let invalid_attrs = if attempted {
        r#" aria-invalid="true" aria-describedby="key-error""#
    } else {
        ""
    };

    let html = format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>emede — enter key</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&family=Playfair+Display:wght@500;700&display=swap" rel="stylesheet">
<style>
  *, *::before, *::after {{ box-sizing: border-box; }}
  :root {{
    --color-bg: #faf8f5;
    --color-surface: #ffffff;
    --color-fg: #1a1a1a;
    --color-muted: #8b8782;
    --color-border: #e6e1db;
    --color-border-hover: #cdc7bf;
    --color-link: #3d5a80;
    --color-link-hover: #1a365d;
    --color-error: #b3261e;
    --focus-ring: rgba(61, 90, 128, 0.35);
    --logo-invert: 0;
    --shadow-sm: 0 1px 3px rgba(0,0,0,0.04), 0 1px 2px rgba(0,0,0,0.03);
    --shadow-md: 0 4px 12px rgba(0,0,0,0.05), 0 2px 4px rgba(0,0,0,0.03);
  }}
  @media (prefers-color-scheme: dark) {{
    :root:not([data-theme="light"]) {{
      --color-bg: #17171a;
      --color-surface: #212127;
      --color-fg: #e8e6e3;
      --color-muted: #928e88;
      --color-border: #33333b;
      --color-border-hover: #4a4a55;
      --color-link: #9db8dc;
      --color-link-hover: #c3d4ee;
      --color-error: #f2b8b5;
      --focus-ring: rgba(157, 184, 220, 0.4);
      --logo-invert: 1;
      --shadow-sm: 0 1px 3px rgba(0,0,0,0.3), 0 1px 2px rgba(0,0,0,0.24);
      --shadow-md: 0 4px 12px rgba(0,0,0,0.36), 0 2px 4px rgba(0,0,0,0.28);
    }}
  }}
  :root[data-theme="dark"] {{
    --color-bg: #17171a;
    --color-surface: #212127;
    --color-fg: #e8e6e3;
    --color-muted: #928e88;
    --color-border: #33333b;
    --color-border-hover: #4a4a55;
    --color-link: #9db8dc;
    --color-link-hover: #c3d4ee;
    --color-error: #f2b8b5;
    --focus-ring: rgba(157, 184, 220, 0.4);
    --logo-invert: 1;
    --shadow-sm: 0 1px 3px rgba(0,0,0,0.3), 0 1px 2px rgba(0,0,0,0.24);
    --shadow-md: 0 4px 12px rgba(0,0,0,0.36), 0 2px 4px rgba(0,0,0,0.28);
  }}
  html {{
    touch-action: manipulation;
    -webkit-tap-highlight-color: transparent;
    color-scheme: light dark;
  }}
  body {{
    font-family: 'Inter', system-ui, -apple-system, sans-serif;
    min-height: 100dvh;
    margin: 0;
    padding: 3rem 1.25rem 5rem;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    background: var(--color-bg);
    color: var(--color-fg);
    line-height: 1.6;
    -webkit-font-smoothing: antialiased;
  }}
  .card {{
    width: 100%;
    max-width: 24rem;
    text-align: center;
  }}
  .logo {{
    width: 64px;
    height: 64px;
    margin: 0 auto 1.25rem;
    display: block;
    filter: invert(var(--logo-invert));
  }}
  h1 {{
    font-family: 'Playfair Display', Georgia, serif;
    font-size: 1.6rem;
    font-weight: 700;
    margin: 0 0 0.35rem;
    letter-spacing: -0.01em;
    text-wrap: balance;
  }}
  .sub {{
    color: var(--color-muted);
    font-size: 0.9rem;
    margin: 0 0 2rem;
  }}
  form {{
    display: grid;
    gap: 0.75rem;
    text-align: left;
  }}
  label {{
    font-size: 0.85rem;
    font-weight: 500;
  }}
  input[type="password"] {{
    width: 100%;
    font: inherit;
    padding: 0.7rem 0.9rem;
    background: var(--color-surface);
    color: var(--color-fg);
    border: 1px solid var(--color-border);
    border-radius: 10px;
    box-shadow: var(--shadow-sm);
    transition: border-color 0.15s ease, box-shadow 0.15s ease;
  }}
  input[type="password"]:hover {{
    border-color: var(--color-border-hover);
  }}
  input[type="password"]:focus-visible {{
    outline: none;
    border-color: var(--color-link);
    box-shadow: 0 0 0 3px var(--focus-ring), var(--shadow-sm);
  }}
  input[aria-invalid="true"] {{
    border-color: var(--color-error);
  }}
  .error {{
    margin: 0;
    font-size: 0.82rem;
    color: var(--color-error);
  }}
  button {{
    font: inherit;
    font-weight: 600;
    margin-top: 0.25rem;
    padding: 0.7rem 1rem;
    color: #fff;
    background: var(--color-link);
    border: 1px solid transparent;
    border-radius: 10px;
    cursor: pointer;
    box-shadow: var(--shadow-sm);
    transition: background 0.15s ease, box-shadow 0.15s ease;
  }}
  button:hover {{
    background: var(--color-link-hover);
    box-shadow: var(--shadow-md);
  }}
  button:focus-visible {{
    outline: none;
    box-shadow: 0 0 0 3px var(--focus-ring), var(--shadow-sm);
  }}
  footer {{
    margin-top: 3rem;
    text-align: center;
    font-size: 0.8rem;
    color: var(--color-muted);
    line-height: 1.7;
  }}
  footer .who {{
    display: block;
    margin-bottom: 0.35rem;
  }}
  footer a {{
    color: var(--color-link);
    text-decoration: none;
    font-weight: 500;
  }}
  footer a:hover {{
    color: var(--color-link-hover);
    text-decoration: underline;
  }}
  footer a:focus-visible {{
    outline: none;
    border-radius: 3px;
    box-shadow: 0 0 0 3px var(--focus-ring);
    color: var(--color-link-hover);
  }}
  @media (prefers-reduced-motion: reduce) {{
    input, button {{ transition: none; }}
  }}
</style>
</head>
<body>
<main class="card">
<img class="logo" src="{logo}" alt="emede logo" width="64" height="64">
<h1>Enter key</h1>
<p class="sub">This note is shared privately. Enter the key to continue.</p>
<form method="get" action="/">
  <label for="key">Key</label>
  <input id="key" name="key" type="password" autocomplete="off" spellcheck="false"
         autocapitalize="off" autocorrect="off" required autofocus{invalid}>
  {error}
  <button type="submit">Unlock</button>
</form>
</main>
<footer>
<span class="who">{user}</span>
<span>Powered by <a href="{repo}" target="_blank" rel="noopener noreferrer">emede</a></span>
</footer>
</body>
</html>"##,
        logo = logo_data_uri(),
        invalid = invalid_attrs,
        error = error,
        user = escape_html(&host_user_label()),
        repo = EMEDE_REPO_URL,
    );

    html_response(html, if attempted { 401 } else { 200 })
}

// ── Server lifecycle ──────────────────────────────────────────────────────────

/// Preferred default port. All notes in an instance share one port.
const DEFAULT_PORT: u16 = 7777;

/// Bind the share server. We deliberately bind `0.0.0.0` — LAN reachability is
/// the whole point of the feature. The exposure is bounded by three controls:
/// a 128-bit CSPRNG auth key required on every request, `Host`-header
/// validation (rejecting DNS-rebinding), and image inlining confined to each
/// note's own directory.
fn try_bind_server(preferred_port: Option<u16>) -> Result<(Arc<Server>, u16), String> {
    let mut candidates: Vec<String> = Vec::new();
    if let Some(p) = preferred_port {
        if p != DEFAULT_PORT {
            candidates.push(format!("0.0.0.0:{p}"));
        }
    }
    candidates.push(format!("0.0.0.0:{DEFAULT_PORT}"));
    candidates.push("0.0.0.0:0".to_string());

    for addr in &candidates {
        if let Ok(s) = Server::http(addr) {
            let port = s
                .server_addr()
                .to_ip()
                .map(|a| a.port())
                .ok_or_else(|| "failed to read server port".to_string())?;
            return Ok((Arc::new(s), port));
        }
    }

    Err("Failed to bind to any port".to_string())
}

fn stop_server(inner: &mut ShareStateInner) {
    if let Some(server) = inner.server.take() {
        server.unblock();
    }
    if let Some(join) = inner.join.take() {
        let _ = join.join();
    }
    inner.port = None;
    inner.server_key = None;
}

// ── Tauri commands ────────────────────────────────────────────────────────────

#[tauri::command]
pub fn start_share(
    path: String,
    state: tauri::State<ShareState>,
) -> Result<ShareInfo, String> {
    let result = markdown::render_markdown_any(&path)?;
    let title = result.title.clone();

    let mut inner = state.0.lock().map_err(|e| e.to_string())?;

    // If this path is already being served by a running server, return its
    // existing info. Guard on `port` too: if the reverse index still holds a
    // hash but the server isn't running (an inconsistent state), fall through
    // to the full start path below instead of panicking.
    if let (Some(hash), Some(port)) = (inner.path_to_hash.get(&path).cloned(), inner.port) {
        let ip = local_ip()
            .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
            .to_string();
        let key = inner.server_key.clone().unwrap_or_default();
        inner.last_path = Some(path.clone());
        return Ok(ShareInfo {
            url: format!("http://{ip}:{port}/{hash}?key={key}"),
            home_url: format!("http://{ip}:{port}/?key={key}"),
            ip,
            port,
            hash,
            title,
        });
    }

    let mut routes = load_share_routes();

    // Reuse the saved hash for this file so the URL stays stable across restarts.
    let hash = routes
        .routes
        .get(&path)
        .map(|e| e.hash.clone())
        .unwrap_or_else(random_hash);

    // Reuse the persisted key so URLs stay valid across restarts, but treat a
    // legacy weak key (older 32-bit, <32 hex chars) as invalid and regenerate.
    let key = routes
        .key
        .clone()
        .filter(|k| k.len() >= MIN_KEY_LEN)
        .unwrap_or_else(generate_key);

    // Start the server once; all subsequent notes share the same instance.
    let bound_port = if inner.server.is_none() {
        let (server, port) = try_bind_server(routes.preferred_port)?;

        let route_map_clone = Arc::clone(&inner.route_map);
        let thread_server = Arc::clone(&server);
        let server_port = port;
        let server_key = key.clone();

        let join = std::thread::spawn(move || {
            for request in thread_server.incoming_requests() {
                // Isolate each request: a panic while rendering one note must not
                // kill the accept loop and take down the whole share server.
                let response = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    handle_share_request(&request, &route_map_clone, server_port, &server_key)
                }))
                .unwrap_or_else(|_| html_response("Internal error".to_string(), 500));

                let _ = request.respond(response);
            }
        });

        inner.server = Some(server);
        inner.join = Some(join);
        inner.port = Some(port);
        inner.server_key = Some(key.clone());

        routes.key = Some(key.clone());

        if routes.preferred_port != Some(port) {
            routes.preferred_port = Some(port);
        }
        save_share_routes(&routes);

        port
    } else {
        // Server already running from an earlier share; reuse its port.
        inner
            .port
            .ok_or_else(|| "share server is running without a bound port".to_string())?
    };

    let port = bound_port;

    inner
        .route_map
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(
            hash.clone(),
            NoteRoute { path: path.clone(), title: title.clone(), hash: hash.clone() },
        );
    inner.path_to_hash.insert(path.clone(), hash.clone());
    inner.last_path = Some(path.clone());

    routes.routes.insert(path.clone(), ShareRouteEntry { hash: hash.clone() });
    save_share_routes(&routes);
    sync_active_shares(&inner);

    let ip = local_ip()
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
        .to_string();

    let key = inner.server_key.as_deref().unwrap_or("");

    Ok(ShareInfo {
        url: format!("http://{ip}:{port}/{hash}?key={key}"),
        home_url: format!("http://{ip}:{port}/?key={key}"),
        ip,
        port,
        hash,
        title,
    })
}

/// Stop sharing a single note. Stops the server if no notes remain.
#[tauri::command]
pub fn stop_share_note(
    path: String,
    state: tauri::State<ShareState>,
) -> Result<(), String> {
    let mut inner = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(hash) = inner.path_to_hash.remove(&path) {
        inner
            .route_map
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&hash);
    }
    if inner.last_path.as_deref() == Some(&path) {
        inner.last_path = inner.path_to_hash.keys().next().cloned();
    }
    if inner.path_to_hash.is_empty() {
        stop_server(&mut inner);
    }
    sync_active_shares(&inner);
    Ok(())
}

/// Stop sharing everything and shut down the server.
#[tauri::command]
pub fn stop_share(state: tauri::State<ShareState>) -> Result<(), String> {
    let mut inner = state.0.lock().map_err(|e| e.to_string())?;
    inner
        .route_map
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
    inner.path_to_hash.clear();
    inner.last_path = None;
    stop_server(&mut inner);
    sync_active_shares(&inner);
    Ok(())
}

/// Returns share info for the most recently shared note (backwards compat).
#[tauri::command]
pub fn get_share_status(state: tauri::State<ShareState>) -> Option<ShareInfo> {
    let inner = state.0.lock().ok()?;
    let path = inner.last_path.as_ref()?;
    let hash = inner.path_to_hash.get(path)?;
    let port = inner.port?;
    let map = inner.route_map.read().ok()?;
    let route = map.get(hash)?;
    let ip = local_ip()
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
        .to_string();
    let key = inner.server_key.as_deref().unwrap_or("");
    Some(ShareInfo {
        url: format!("http://{ip}:{port}/{hash}?key={key}"),
        home_url: format!("http://{ip}:{port}/?key={key}"),
        ip,
        port,
        hash: hash.clone(),
        title: route.title.clone(),
    })
}

/// Returns share info for a specific note path, or None if not currently shared.
#[tauri::command]
pub fn get_note_share_info(
    path: String,
    state: tauri::State<ShareState>,
) -> Option<ShareInfo> {
    let inner = state.0.lock().ok()?;
    let hash = inner.path_to_hash.get(&path)?;
    let port = inner.port?;
    let map = inner.route_map.read().ok()?;
    let route = map.get(hash)?;
    let ip = local_ip()
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
        .to_string();
    let key = inner.server_key.as_deref().unwrap_or("");
    Some(ShareInfo {
        url: format!("http://{ip}:{port}/{hash}?key={key}"),
        home_url: format!("http://{ip}:{port}/?key={key}"),
        ip,
        port,
        hash: hash.clone(),
        title: route.title.clone(),
    })
}

/// Content-Security-Policy applied to every served page. The shared pages use
/// inline `<script>`/`<style>` and pull MathJax + Mermaid from jsDelivr; the
/// home page additionally uses Google Fonts. Everything else is denied.
const SHARED_CSP: &str = "default-src 'none'; \
script-src 'unsafe-inline' https://cdn.jsdelivr.net; \
style-src 'unsafe-inline' https://fonts.googleapis.com https://cdn.jsdelivr.net; \
font-src https://fonts.gstatic.com https://cdn.jsdelivr.net; \
img-src data: http: https:; \
connect-src https://cdn.jsdelivr.net";

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("valid header")
}

fn html_response(body: String, status: u16) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(body)
        .with_header(header("Content-Type", "text/html; charset=utf-8"))
        .with_header(header("Content-Security-Policy", SHARED_CSP))
        .with_header(header("X-Frame-Options", "DENY"))
        .with_header(header("X-Content-Type-Options", "nosniff"))
        .with_header(header("Referrer-Policy", "no-referrer"))
        .with_status_code(status)
}

/// Constant-time comparison of the query key against the server key, so an
/// attacker cannot recover the key byte-by-byte through response timing.
fn key_matches(candidate: Option<&str>, expected: &str) -> bool {
    use subtle::ConstantTimeEq;
    let Some(c) = candidate else {
        return false;
    };
    // The length check leaks only the (constant, 32-char) expected length.
    c.len() == expected.len() && c.as_bytes().ct_eq(expected.as_bytes()).into()
}

/// Build the response for one shared-server request. Pure function of the
/// request plus shared state, so it is safe to run inside `catch_unwind`.
fn handle_share_request(
    request: &tiny_http::Request,
    route_map: &Arc<RwLock<HashMap<String, NoteRoute>>>,
    server_port: u16,
    server_key: &str,
) -> Response<std::io::Cursor<Vec<u8>>> {
    if !host_header_ok(request) {
        return html_response("Forbidden".to_string(), 403);
    }
    if request.method() != &Method::Get {
        return html_response("Method not allowed".to_string(), 405);
    }

    let (url_path, query_key) = parse_url_key(request.url());
    if !key_matches(query_key.as_deref(), server_key) {
        // No/invalid key: ask for it instead of a dead end. The form submits the
        // key back as `?key=` (see `build_unlock_page`); a correct one reaches
        // the home page. `attempted` distinguishes a first visit from a retry.
        return build_unlock_page(query_key.is_some());
    }

    if url_path == "/" {
        let current_ip = local_ip()
            .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
            .to_string();
        let map = route_map.read().unwrap_or_else(|e| e.into_inner());
        build_home_page(&map, &current_ip, server_port, server_key)
    } else {
        let hash_str = url_path.trim_start_matches('/').to_string();
        let path_opt = {
            let map = route_map.read().unwrap_or_else(|e| e.into_inner());
            map.get(&hash_str).map(|r| r.path.clone())
        };
        match path_opt {
            Some(p) => match build_shared_page(&p) {
                Ok(html) => html_response(html, 200),
                Err(err) => html_response(format!("Render error: {err}"), 500),
            },
            None => html_response("Not found".to_string(), 404),
        }
    }
}

/// Is this `Host` header value acceptable? Accepts a bare IP literal or
/// `localhost` (optionally with a `:port`), rejects domain names.
fn host_value_ok(host: &str) -> bool {
    // Strip a trailing ":port". For IPv6 literals the host is bracketed
    // (`[::1]:7777`); unwrap the brackets before parsing.
    let hostname = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host)
    };

    hostname == "localhost" || hostname.parse::<IpAddr>().is_ok()
}

/// Validate the `Host` header to defeat DNS-rebinding attacks. Shared URLs
/// always target a bare IP literal (or localhost), so a request whose Host is a
/// domain name did not originate from one of our own URLs and is rejected.
fn host_header_ok(request: &tiny_http::Request) -> bool {
    match request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Host"))
        .map(|h| h.value.as_str())
    {
        // No Host header (HTTP/1.0) — allow; there is no rebinding vector.
        None => true,
        Some(host) => host_value_ok(host),
    }
}

const SHARED_PAGE_TEMPLATE: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="UTF-8" />
<meta name="viewport" content="width=device-width, initial-scale=1.0" />
<title>{{TITLE}} — emede</title>
<script>
  window.MathJax = {
    tex: {
      inlineMath: [["\\(", "\\)"], ["$", "$"]],
      displayMath: [["\\[", "\\]"], ["$$", "$$"]],
    },
    options: {
      skipHtmlTags: ["script", "noscript", "style", "textarea", "pre", "code"],
    },
  };
</script>
<script async src="https://cdn.jsdelivr.net/npm/mathjax@3/es5/tex-chtml.js"></script>
<style>
  :root {
    --color-fg: {{FG}};
    --color-bg: {{BG}};
    --font-size: {{SIZE}}pt;
    --reader-margin: 8%;
    --font-serif: {{FONT}};
    --font-code: {{FONT_CODE}};
    --color-muted: color-mix(in srgb, var(--color-fg) 52%, transparent);
    --color-link: color-mix(in srgb, #3d5a80 72%, var(--color-fg));
    --color-code-bg: color-mix(in srgb, var(--color-fg) 10%, var(--color-bg));
    --color-border: color-mix(in srgb, var(--color-fg) 16%, var(--color-bg));
  }
  * { box-sizing: border-box; }
  html, body {
    margin: 0;
    background: var(--color-bg);
    color: var(--color-fg);
    font-family: var(--font-serif);
    font-size: var(--font-size);
    line-height: 1.7;
  }
  .prose {
    max-width: 46rem;
    margin: 0 auto;
    padding: 3rem var(--reader-margin) 6rem;
    word-wrap: break-word;
  }
  .prose h1, .prose h2, .prose h3, .prose h4, .prose h5, .prose h6 {
    line-height: 1.25;
    margin: 1.6em 0 0.6em;
  }
  .prose h1 { font-size: 1.9em; }
  .prose h2 { font-size: 1.5em; }
  .prose h3 { font-size: 1.25em; }
  .prose p, .prose ul, .prose ol, .prose blockquote, .prose pre, .prose table, .prose figure {
    margin: 0 0 1em;
  }
  .prose a { color: var(--color-link); }
  .prose img { max-width: 100%; height: auto; }
  .prose code {
    font-family: var(--font-code);
    font-size: 0.9em;
    background: var(--color-code-bg);
    padding: 0.1em 0.35em;
    border-radius: 4px;
  }
  .prose pre {
    font-family: var(--font-code);
    background: var(--color-code-bg);
    padding: 1em;
    border-radius: 8px;
    overflow-x: auto;
  }
  .prose pre code { background: none; padding: 0; }
  .prose blockquote {
    margin-inline: 0;
    padding-left: 1em;
    border-left: 3px solid var(--color-border);
    color: var(--color-muted);
  }
  .prose table { border-collapse: collapse; width: 100%; display: block; overflow-x: auto; }
  .prose th, .prose td { border: 1px solid var(--color-border); padding: 0.4em 0.7em; }
  .prose hr { border: none; border-top: 1px solid var(--color-border); }
  .prose .mermaid { text-align: center; }

  .share-footer {
    max-width: 46rem;
    margin: 0 auto;
    padding: 1.5rem var(--reader-margin) 3rem;
    border-top: 1px solid var(--color-border);
    text-align: center;
    font-size: 0.8rem;
    color: var(--color-muted);
    line-height: 1.7;
  }
  .share-footer .who { display: block; margin-bottom: 0.35rem; }
  .share-footer a { color: var(--color-link); text-decoration: none; }
  .share-footer a:hover { text-decoration: underline; }

  .top-btns {
    position: fixed; top: 12px; right: 12px; z-index: 10;
    display: flex; gap: 8px;
  }
  .top-btns button {
    width: 40px; height: 40px; border-radius: 50%;
    border: 1px solid var(--color-border);
    background: var(--color-bg); color: var(--color-fg);
    font-size: 18px; cursor: pointer; display: flex;
    align-items: center; justify-content: center;
    transition: background 0.15s;
  }
  .top-btns button:hover { background: var(--color-code-bg); }
  #cfg-panel {
    position: fixed; top: 60px; right: 12px; z-index: 10;
    background: var(--color-bg); color: var(--color-fg);
    border: 1px solid var(--color-border); border-radius: 10px;
    padding: 14px; display: grid; gap: 12px; min-width: 200px;
    box-shadow: 0 8px 30px rgba(0,0,0,0.18);
  }
  #cfg-panel[hidden] { display: none; }
  #cfg-panel label { display: flex; align-items: center; justify-content: space-between; gap: 10px; font-size: 14px; }
  #cfg-panel input[type="color"] { width: 42px; height: 28px; padding: 0; border: 1px solid var(--color-border); background: none; }
  #cfg-size-label { min-width: 3ch; text-align: right; font-variant-numeric: tabular-nums; }
</style>
</head>
<body>
<div class="top-btns">
  <button id="home-btn" type="button" aria-label="Home" title="Shared notes home">⌂</button>
  <button id="cfg-toggle" type="button" aria-label="Display settings">⚙</button>
</div>
<div id="cfg-panel" hidden>
  <label>Text <input id="cfg-fg" type="color" /></label>
  <label>Background <input id="cfg-bg" type="color" /></label>
  <label>Size <input id="cfg-size" type="range" min="8" max="32" step="1" /><span id="cfg-size-label"></span></label>
</div>
<article class="prose">{{CONTENT}}</article>
<footer class="share-footer">
<span class="who">{{USER}}</span>
<span>Powered by <a href="{{REPO}}" target="_blank" rel="noopener noreferrer">emede</a></span>
</footer>
<script>
  (function () {
    var root = document.documentElement;
    var toggle = document.getElementById("cfg-toggle");
    var panel = document.getElementById("cfg-panel");
    var fg = document.getElementById("cfg-fg");
    var bg = document.getElementById("cfg-bg");
    var size = document.getElementById("cfg-size");
    var sizeLabel = document.getElementById("cfg-size-label");

    function cssVar(name) {
      return getComputedStyle(root).getPropertyValue(name).trim();
    }
    function normalizeHex(value, fallback) {
      return /^#[0-9a-fA-F]{6}$/.test(value) ? value : fallback;
    }

    var savedFg = localStorage.getItem("emede-share-fg");
    var savedBg = localStorage.getItem("emede-share-bg");
    var savedSize = localStorage.getItem("emede-share-size");

    var initFg = normalizeHex(savedFg, normalizeHex(cssVar("--color-fg"), "#2c2c2c"));
    var initBg = normalizeHex(savedBg, normalizeHex(cssVar("--color-bg"), "#faf8f5"));
    var initSize = parseInt(savedSize || cssVar("--font-size"), 10) || 12;

    function applyFg(v) { root.style.setProperty("--color-fg", v); fg.value = v; }
    function applyBg(v) { root.style.setProperty("--color-bg", v); bg.value = v; }
    function applySize(v) {
      root.style.setProperty("--font-size", v + "pt");
      size.value = v;
      sizeLabel.textContent = v + "pt";
    }

    applyFg(initFg);
    applyBg(initBg);
    applySize(initSize);

    toggle.addEventListener("click", function () { panel.hidden = !panel.hidden; });
    document.getElementById("home-btn").addEventListener("click", function () {
      var m = location.search.match(/[?&]key=([^&]+)/);
      var key = m ? m[1] : "";
      location.href = location.origin + "/" + (key ? "?key=" + key : "");
    });
    fg.addEventListener("input", function () {
      applyFg(fg.value);
      localStorage.setItem("emede-share-fg", fg.value);
      renderMermaid();
    });
    bg.addEventListener("input", function () {
      applyBg(bg.value);
      localStorage.setItem("emede-share-bg", bg.value);
      renderMermaid();
    });
    size.addEventListener("input", function () {
      applySize(size.value);
      localStorage.setItem("emede-share-size", size.value);
    });

    var mermaidLib = null;
    // Custom properties read back as their literal source text, so a value like
    // color-mix(...) reaches Mermaid unresolved and its color parser throws.
    // Paint the value onto a 1x1 canvas and read the pixel back. This forces the
    // browser to resolve any modern syntax (color-mix(), color(srgb ...), etc.)
    // down to plain 0-255 RGBA that Mermaid's color parser understands.
    var probeCanvas = document.createElement("canvas");
    probeCanvas.width = 1;
    probeCanvas.height = 1;
    var probeCtx = probeCanvas.getContext("2d");
    function resolveColor(value) {
      if (!probeCtx) return value;
      probeCtx.clearRect(0, 0, 1, 1);
      probeCtx.fillStyle = "#000";
      probeCtx.fillStyle = value;
      probeCtx.fillRect(0, 0, 1, 1);
      var d = probeCtx.getImageData(0, 0, 1, 1).data;
      return "rgb(" + d[0] + ", " + d[1] + ", " + d[2] + ")";
    }
    function resolveRgbData(value) {
      if (!probeCtx) return null;
      probeCtx.clearRect(0, 0, 1, 1);
      probeCtx.fillStyle = "#000";
      probeCtx.fillStyle = value;
      probeCtx.fillRect(0, 0, 1, 1);
      return probeCtx.getImageData(0, 0, 1, 1).data;
    }
    // Distinct multi-color palette (pie slices, cScale) tuned for the theme bg.
    // Low saturation + a blend toward the background keep it muted and on-theme.
    function autoPalette(count, bgData, dark) {
      var sat = 38;
      var light = dark ? 58 : 52;
      var blend = 0.28;
      var out = [];
      for (var i = 0; i < count; i++) {
        var hue = Math.round((i * 137.5) % 360);
        var c = resolveRgbData("hsl(" + hue + ", " + sat + "%, " + light + "%)");
        if (!c || !bgData) {
          out.push(resolveColor("hsl(" + hue + ", " + sat + "%, " + light + "%)"));
          continue;
        }
        var r = Math.round(c[0] * (1 - blend) + bgData[0] * blend);
        var g = Math.round(c[1] * (1 - blend) + bgData[1] * blend);
        var b = Math.round(c[2] * (1 - blend) + bgData[2] * blend);
        out.push("rgb(" + r + ", " + g + ", " + b + ")");
      }
      return out;
    }
    function renderMermaid() {
      var blocks = document.querySelectorAll("pre > code.language-mermaid, div.mermaid[data-src]");
      if (!blocks.length || !mermaidLib) return;
      var style = getComputedStyle(root);
      var fgc = resolveColor(style.getPropertyValue("--color-fg").trim());
      var bgc = resolveColor(style.getPropertyValue("--color-bg").trim());
      var codeBg = resolveColor(style.getPropertyValue("--color-code-bg").trim());
      var link = resolveColor(style.getPropertyValue("--color-link").trim());
      var bgData = resolveRgbData(style.getPropertyValue("--color-bg").trim());
      var dark = bgData
        ? (0.2126 * bgData[0] + 0.7152 * bgData[1] + 0.0722 * bgData[2]) / 255 < 0.5
        : true;
      var palette = autoPalette(12, bgData, dark);
      var themeVars = {
        background: bgc,
        primaryColor: codeBg,
        primaryTextColor: fgc,
        primaryBorderColor: fgc,
        lineColor: fgc,
        mainBkg: codeBg,
        nodeBorder: fgc,
        nodeTextColor: fgc,
        labelColor: fgc,
        edgeLabelBackground: bgc,
        linkColor: link,
        pieStrokeColor: fgc,
        pieOuterStrokeColor: fgc,
        pieSectionTextColor: fgc,
        pieTitleTextColor: fgc,
        pieOpacity: 1,
      };
      for (var p = 0; p < palette.length; p++) {
        themeVars["pie" + (p + 1)] = palette[p];
        themeVars["cScale" + p] = palette[p];
      }
      mermaidLib.initialize({
        startOnLoad: false,
        theme: "base",
        themeVariables: themeVars,
      });
      var nodes = [];
      document.querySelectorAll("pre > code.language-mermaid").forEach(function (code) {
        var div = document.createElement("div");
        div.className = "mermaid";
        div.dataset.src = code.textContent;
        div.textContent = code.textContent;
        code.parentElement.replaceWith(div);
        nodes.push(div);
      });
      document.querySelectorAll("div.mermaid[data-src]").forEach(function (div) {
        if (nodes.indexOf(div) !== -1) return;
        div.removeAttribute("data-processed");
        div.textContent = div.dataset.src;
        nodes.push(div);
      });
      mermaidLib.run({ nodes: nodes }).catch(function (e) { console.warn(e); });
    }

    var script = document.createElement("script");
    script.src = "https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.min.js";
    script.onload = function () { mermaidLib = window.mermaid; renderMermaid(); };
    script.onerror = function () { console.warn("Failed to load Mermaid"); };
    document.head.appendChild(script);
  })();
</script>
</body>
</html>
"##;

// ── QR code generation ─────────────────────────────────────────────────────────

/// Generate an SVG QR code for the given URL string.
pub fn generate_qr_svg(url: &str) -> Result<String, String> {
    let code = qrcode::QrCode::new(url.as_bytes()).map_err(|e| e.to_string())?;
    let svg = code
        .render()
        .min_dimensions(400, 400)
        .dark_color(qrcode::render::svg::Color("#000000"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build();
    Ok(svg)
}

#[tauri::command]
pub fn generate_share_qr(url: String) -> Result<String, String> {
    generate_qr_svg(&url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inlines_local_image_as_data_uri() {
        let dir = std::env::temp_dir().join("emede-share-img-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let base = dir.canonicalize().expect("canonicalize dir");
        let img_path = base.join("pic.png");
        std::fs::write(&img_path, b"\x89PNG\r\n\x1a\n test bytes").expect("write img");

        let html = format!(r#"<p><img src="{}" alt="x"></p>"#, img_path.display());
        let out = inline_local_images(&html, Some(&base));
        assert!(
            out.contains("data:image/png;base64,"),
            "expected inlined data uri, got: {out}"
        );
        assert!(!out.contains(&img_path.display().to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn does_not_inline_image_outside_base_dir() {
        // An image referenced by absolute path outside the note's directory
        // (e.g. a secret file) must not be inlined.
        let base = std::env::temp_dir()
            .join("emede-share-confine-base")
            .canonicalize()
            .unwrap_or_else(|_| {
                let d = std::env::temp_dir().join("emede-share-confine-base");
                std::fs::create_dir_all(&d).unwrap();
                d.canonicalize().unwrap()
            });
        std::fs::create_dir_all(&base).ok();

        let secret_dir = std::env::temp_dir().join("emede-share-confine-secret");
        std::fs::create_dir_all(&secret_dir).expect("create secret dir");
        let secret = secret_dir.canonicalize().unwrap().join("secret.png");
        std::fs::write(&secret, b"\x89PNG top secret").expect("write secret");

        let html = format!(r#"<img src="{}" alt="x">"#, secret.display());
        let out = inline_local_images(&html, Some(&base));
        assert!(
            !out.contains("data:image"),
            "image outside base dir must not be inlined, got: {out}"
        );
        assert_eq!(out, html, "tag should be left untouched");

        let _ = std::fs::remove_dir_all(&secret_dir);
    }

    #[test]
    fn does_not_inline_oversized_image() {
        let dir = std::env::temp_dir().join("emede-share-oversize");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let base = dir.canonicalize().expect("canonicalize");
        let img_path = base.join("big.png");
        // One byte over the cap.
        let big = vec![0u8; (MAX_INLINE_IMAGE_BYTES + 1) as usize];
        std::fs::write(&img_path, &big).expect("write big img");

        let html = format!(r#"<img src="{}" alt="x">"#, img_path.display());
        let out = inline_local_images(&html, Some(&base));
        assert!(
            !out.contains("data:image"),
            "oversized image must not be inlined"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn leaves_remote_image_untouched() {
        let html = r#"<img src="https://example.com/a.png" alt="r">"#;
        assert_eq!(inline_local_images(html, None), html);
    }

    #[test]
    fn generates_128bit_key() {
        let key = generate_key();
        assert_eq!(key.len(), 32, "key should be 32 hex chars (128-bit)");
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(key.len() >= MIN_KEY_LEN);
        // Two keys must differ (astronomically unlikely to collide).
        assert_ne!(generate_key(), generate_key());
    }

    #[test]
    fn host_value_accepts_ips_rejects_domains() {
        assert!(host_value_ok("192.168.1.5"));
        assert!(host_value_ok("192.168.1.5:7777"));
        assert!(host_value_ok("127.0.0.1:7777"));
        assert!(host_value_ok("localhost"));
        assert!(host_value_ok("localhost:7777"));
        assert!(host_value_ok("[::1]:7777"));
        assert!(!host_value_ok("attacker.example"));
        assert!(!host_value_ok("evil.com:7777"));
        assert!(!host_value_ok("emede.local"));
    }

    #[test]
    fn key_matches_is_exact() {
        assert!(key_matches(Some("abc123"), "abc123"));
        assert!(!key_matches(Some("abc124"), "abc123"));
        assert!(!key_matches(Some("abc12"), "abc123"));
        assert!(!key_matches(Some("abc1234"), "abc123"));
        assert!(!key_matches(None, "abc123"));
    }

    #[test]
    fn home_page_includes_logo_and_theme_toggle() {
        let map = HashMap::new();
        let resp = build_home_page(&map, "192.168.1.20", 7777, "abcdef");
        let mut body = String::new();
        use std::io::Read;
        resp.into_reader().read_to_string(&mut body).unwrap();
        assert!(body.contains(r#"id="theme-toggle""#), "missing theme toggle");
        assert!(body.contains("data:image/png;base64,"), "missing inlined logo");
        assert!(body.contains(r#"alt="emede logo""#), "missing logo alt text");
        assert!(
            body.contains("prefers-color-scheme: dark"),
            "missing dark-mode support"
        );
    }

    #[test]
    fn unlock_page_has_get_form_logo_and_footer() {
        use std::io::Read;

        // First visit: no key attempted yet, so no error and a 200 status.
        let resp = build_unlock_page(false);
        assert_eq!(resp.status_code().0, 200);
        let mut body = String::new();
        resp.into_reader().read_to_string(&mut body).unwrap();
        assert!(
            body.contains(r#"<form method="get" action="/">"#),
            "missing GET form pointing at home"
        );
        assert!(body.contains(r#"name="key""#), "missing key input");
        assert!(body.contains("data:image/png;base64,"), "missing inlined logo");
        assert!(body.contains("Powered by"), "missing footer");
        assert!(!body.contains("key-error"), "should not show error on first visit");

        // Retry after a wrong key: inline error and a 401 status.
        let retry = build_unlock_page(true);
        assert_eq!(retry.status_code().0, 401);
        let mut retry_body = String::new();
        retry.into_reader().read_to_string(&mut retry_body).unwrap();
        assert!(retry_body.contains("key-error"), "missing error on retry");
        assert!(retry_body.contains(r#"aria-invalid="true""#), "missing invalid state");
    }

    #[test]
    fn font_size_pt_parses_value() {
        assert_eq!(font_size_pt("14pt"), 14);
        assert_eq!(font_size_pt("12"), 12);
        assert_eq!(font_size_pt("oops"), 12);
    }

    #[test]
    fn build_shared_page_includes_content_and_cdn() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../README.md");
        let page = build_shared_page(path).expect("build page");
        assert!(page.contains("class=\"prose\""), "missing prose wrapper");
        assert!(
            page.contains("cdn.jsdelivr.net/npm/mathjax"),
            "missing MathJax CDN script"
        );
        assert!(
            !page.contains("<script>alert"),
            "sanitization should strip inline scripts from content"
        );
        assert!(!page.contains("{{CONTENT}}"), "template token not replaced");
    }
}
