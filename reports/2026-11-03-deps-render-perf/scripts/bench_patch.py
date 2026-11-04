#!/usr/bin/env python3
"""Apply the throwaway timing instrumentation to an emede checkout.

Usage: bench_patch.py <checkout-root>

Identical text is inserted into both arms (v0.2.17 and the branch), anchored on
code that is byte-identical in both, so the instrumentation cost is the same.
Active only when EMEDE_BENCH is set: the window is parked off-screen (as
`--print` does) and the process prints one JSON line and exits once the
document's dynamic regions (MathJax + Mermaid) have finished and painted.
"""
import sys
from pathlib import Path

root = Path(sys.argv[1])


def sub(path, old, new, count=1):
    p = root / path
    s = p.read_text()
    n = s.count(old)
    if n != count:
        sys.exit(f"{path}: anchor found {n} times, expected {count}:\n{old}")
    p.write_text(s.replace(old, new))


# --- main.js -----------------------------------------------------------------
sub(
    "src/main.js",
    "async function boot() {\n",
    "async function boot() {\n  (window.__bench ??= {}).boot = performance.now();\n",
)
sub(
    "src/main.js",
    '  const openToken = ++activeOpenToken;\n  setReaderState("loading");\n\n  try {\n'
    '    const result = await invoke("render_markdown", { path });\n'
    "    await applyDocument(result, { openToken });\n",
    '  const openToken = ++activeOpenToken;\n  setReaderState("loading");\n\n  try {\n'
    "    (window.__bench ??= {}).invoke_start = performance.now();\n"
    '    const result = await invoke("render_markdown", { path });\n'
    "    window.__bench.invoke_end = performance.now();\n"
    "    window.__bench.html_bytes = result.html.length;\n"
    "    await applyDocument(result, { openToken });\n"
    "    window.__bench.apply_end = performance.now();\n",
)
sub(
    "src/main.js",
    "async function renderDynamicRegions(openToken) {\n"
    "  await Promise.allSettled([renderMermaid(), typesetMath()]);\n"
    "  if (openToken !== undefined && openToken !== activeOpenToken) return;\n"
    "  await enableOffscreenSkipping(openToken);\n}",
    "async function renderDynamicRegions(openToken) {\n"
    "  const __b = (window.__bench ??= {});\n"
    "  __b.dyn_start = performance.now();\n"
    "  await Promise.allSettled([\n"
    "    renderMermaid().finally(() => { __b.mermaid_end = performance.now(); }),\n"
    "    typesetMath().finally(() => { __b.math_end = performance.now(); }),\n"
    "  ]);\n"
    "  __b.dyn_settled = performance.now();\n"
    "  if (openToken !== undefined && openToken !== activeOpenToken) return;\n"
    "  await enableOffscreenSkipping(openToken);\n"
    "  __b.dyn_end = performance.now();\n"
    "  await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));\n"
    "  __b.painted = performance.now();\n"
    "  __b.time_origin = performance.timeOrigin;\n"
    "  __b.mermaid_svgs = contentEl.querySelectorAll('div.mermaid svg').length;\n"
    "  __b.math_nodes = contentEl.querySelectorAll('mjx-container').length;\n"
    '  void invoke("bench_report", { json: JSON.stringify(__b) }).catch(() => {});\n'
    "}",
)

# --- markdown.rs: time the Rust side of the render command --------------------
sub(
    "src-tauri/src/markdown.rs",
    "    let result = crate::changes::render_markdown_tracked(&path, &baselines, &app)?;\n",
    "    let __t = std::time::Instant::now();\n"
    "    let result = crate::changes::render_markdown_tracked(&path, &baselines, &app)?;\n"
    "    BENCH_RENDER_NS.store(__t.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);\n",
)
p = root / "src-tauri/src/markdown.rs"
p.write_text(
    p.read_text()
    + "\npub static BENCH_RENDER_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);\n"
)

# --- lib.rs: park the window, report and exit -----------------------------------
sub(
    "src-tauri/src/lib.rs",
    "                if print_mode {\n",
    '                if print_mode || std::env::var_os("EMEDE_BENCH").is_some() {\n',
)
sub("src-tauri/src/lib.rs", "            print_ready,\n", "            print_ready,\n            bench_report,\n")
p = root / "src-tauri/src/lib.rs"
p.write_text(
    p.read_text()
    + r'''
fn bench_vmhwm_kb(pid: &str) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

/// Peak RSS (kB) of this process and of its direct children (the WebKit
/// web/network processes).
fn bench_memory() -> (u64, u64) {
    let me = std::process::id().to_string();
    let mut children = 0;
    if let Ok(dir) = std::fs::read_dir("/proc") {
        for e in dir.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{name}/stat")) else { continue };
            let after = &stat[stat.rfind(')').map(|i| i + 2).unwrap_or(0)..];
            if after.split_whitespace().nth(1) == Some(me.as_str()) {
                children += bench_vmhwm_kb(&name);
            }
        }
    }
    (bench_vmhwm_kb("self"), children)
}

#[tauri::command]
fn bench_report(json: String) {
    if std::env::var_os("EMEDE_BENCH").is_none() {
        return;
    }
    let (self_kb, child_kb) = bench_memory();
    let ns = crate::markdown::BENCH_RENDER_NS.load(std::sync::atomic::Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    println!(
        "EMEDE_BENCH {{\"js\":{json},\"rust_render_ms\":{},\"rss_main_kb\":{self_kb},\"rss_children_kb\":{child_kb},\"report_epoch_ms\":{now}}}",
        ns as f64 / 1e6
    );
    use std::io::Write;
    let _ = std::io::stdout().flush();
    std::process::exit(0);
}
'''
)
print(f"patched {root}")
