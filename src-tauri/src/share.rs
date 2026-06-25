use base64::Engine as _;
use serde::Serialize;
use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};
use tiny_http::{Header, Method, Response, Server};

use crate::markdown;
use crate::settings;

pub struct ShareState(pub Mutex<Option<ShareHandle>>);

pub struct ShareHandle {
    server: Arc<Server>,
    join: Option<JoinHandle<()>>,
    info: ShareInfo,
}

#[derive(Clone, Serialize)]
pub struct ShareInfo {
    pub url: String,
    pub ip: String,
    pub port: u16,
    pub hash: String,
}

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

static SHARE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Short, hard-to-guess token used as the served route. Obscurity only.
fn random_hash() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let counter = SHARE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mixed = nanos ^ counter.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    format!("{:08x}", (mixed as u32))
}

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

/// Replace local-file `src` values on a single `<img>` tag with a base64 `data:`
/// URI so the served page is fully self-contained. Remote URLs are left as-is.
fn inline_img_tag_src(tag: &str) -> String {
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
    let Ok(bytes) = std::fs::read(path) else {
        return tag.to_string();
    };

    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let data_uri = format!("data:{};base64,{}", mime_for_extension(path), encoded);
    let prefix = &tag[..src_idx + 4 + 1];
    format!("{prefix}{data_uri}{quote}{}", &rest[end_quote + 1..])
}

/// Inline every local `<img>` source in the rendered HTML as a `data:` URI.
fn inline_local_images(html: &str) -> String {
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
        result.push_str(&inline_img_tag_src(&html[start..end]));
        search_from = end;
    }

    result.push_str(&html[search_from..]);
    result
}

fn escape_title(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
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

/// Render `path` into a single self-contained HTML page for LAN clients.
pub fn build_shared_page(path: &str) -> Result<String, String> {
    let result = markdown::render_markdown_inner(path)?;
    let content = inline_local_images(&result.html);
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
        .replace("{{TITLE}}", &escape_title(&result.title))
        .replace("{{FG}}", &settings.color_fg)
        .replace("{{BG}}", &settings.color_bg)
        .replace("{{SIZE}}", &font_size_pt(&settings.font_size).to_string())
        .replace("{{FONT}}", &body_font)
        .replace("{{FONT_CODE}}", &code_font)
        .replace("{{CONTENT}}", &content);

    Ok(page)
}

fn stop_handle(handle: ShareHandle) {
    handle.server.unblock();
    if let Some(join) = handle.join {
        let _ = join.join();
    }
}

#[tauri::command]
pub fn start_share(
    path: String,
    state: tauri::State<ShareState>,
) -> Result<ShareInfo, String> {
    // Validate the document renders before we start serving it.
    let _ = markdown::render_markdown_inner(&path)?;

    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(existing) = guard.take() {
        stop_handle(existing);
    }

    let server = Server::http("0.0.0.0:0").map_err(|e| e.to_string())?;
    let port = server
        .server_addr()
        .to_ip()
        .map(|addr| addr.port())
        .ok_or_else(|| "failed to read server port".to_string())?;
    let server = Arc::new(server);

    let ip = local_ip()
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
        .to_string();
    let hash = random_hash();
    let info = ShareInfo {
        url: format!("http://{ip}:{port}/{hash}"),
        ip,
        port,
        hash: hash.clone(),
    };

    let route = format!("/{hash}");
    let thread_server = Arc::clone(&server);
    let thread_path = path.clone();
    let join = std::thread::spawn(move || {
        for request in thread_server.incoming_requests() {
            let request_route = request.url().split('?').next().unwrap_or("").to_string();
            let is_match = request.method() == &Method::Get && request_route == route;

            let response = if is_match {
                match build_shared_page(&thread_path) {
                    Ok(html) => html_response(html, 200),
                    Err(err) => html_response(format!("Render error: {err}"), 500),
                }
            } else {
                html_response("Not found".to_string(), 404)
            };

            let _ = request.respond(response);
        }
    });

    *guard = Some(ShareHandle {
        server,
        join: Some(join),
        info: info.clone(),
    });

    Ok(info)
}

fn html_response(body: String, status: u16) -> Response<std::io::Cursor<Vec<u8>>> {
    let header = Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..])
        .expect("valid header");
    Response::from_string(body)
        .with_header(header)
        .with_status_code(status)
}

#[tauri::command]
pub fn stop_share(state: tauri::State<ShareState>) -> Result<(), String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(handle) = guard.take() {
        stop_handle(handle);
    }
    Ok(())
}

#[tauri::command]
pub fn get_share_status(state: tauri::State<ShareState>) -> Option<ShareInfo> {
    let guard = state.0.lock().ok()?;
    guard.as_ref().map(|handle| handle.info.clone())
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

  #cfg-toggle {
    position: fixed; top: 12px; right: 12px; z-index: 10;
    width: 40px; height: 40px; border-radius: 50%;
    border: 1px solid var(--color-border);
    background: var(--color-bg); color: var(--color-fg);
    font-size: 18px; cursor: pointer;
  }
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
<button id="cfg-toggle" type="button" aria-label="Display settings">⚙</button>
<div id="cfg-panel" hidden>
  <label>Text <input id="cfg-fg" type="color" /></label>
  <label>Background <input id="cfg-bg" type="color" /></label>
  <label>Size <input id="cfg-size" type="range" min="8" max="32" step="1" /><span id="cfg-size-label"></span></label>
</div>
<article class="prose">{{CONTENT}}</article>
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
    // Assigning the value to a real `color` and reading it back lets the browser
    // resolve it to a concrete rgb() that Mermaid can parse.
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
    function renderMermaid() {
      var blocks = document.querySelectorAll("pre > code.language-mermaid, div.mermaid[data-src]");
      if (!blocks.length || !mermaidLib) return;
      var style = getComputedStyle(root);
      var fgc = resolveColor(style.getPropertyValue("--color-fg").trim());
      var bgc = resolveColor(style.getPropertyValue("--color-bg").trim());
      var codeBg = resolveColor(style.getPropertyValue("--color-code-bg").trim());
      var link = resolveColor(style.getPropertyValue("--color-link").trim());
      mermaidLib.initialize({
        startOnLoad: false,
        theme: "base",
        themeVariables: {
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
        },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inlines_local_image_as_data_uri() {
        let dir = std::env::temp_dir().join("emede-share-img-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let img_path = dir.join("pic.png");
        // 1x1 transparent PNG bytes are not required; any bytes work for the test.
        std::fs::write(&img_path, b"\x89PNG\r\n\x1a\n test bytes").expect("write img");

        let html = format!(r#"<p><img src="{}" alt="x"></p>"#, img_path.display());
        let out = inline_local_images(&html);
        assert!(
            out.contains("data:image/png;base64,"),
            "expected inlined data uri, got: {out}"
        );
        assert!(!out.contains(&img_path.display().to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn leaves_remote_image_untouched() {
        let html = r#"<img src="https://example.com/a.png" alt="r">"#;
        assert_eq!(inline_local_images(html), html);
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
