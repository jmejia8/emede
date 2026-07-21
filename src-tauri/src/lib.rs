pub mod cli;
mod markdown;
mod persist;
mod recents;
mod settings;
mod share;
mod view_state;
mod watcher;

use std::sync::Mutex;
use tauri::{window::Color, AppHandle, Emitter, Manager, Url};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_window_state::StateFlags;

/// The markdown file (if any) the frontend should open on startup.
struct StartupFile(Mutex<Option<String>>);

/// When present, this invocation is a headless PDF export: the value is the
/// output path. The frontend polls it via `get_print_target` to switch into the
/// print-and-exit flow instead of revealing a window.
struct PrintTarget(Mutex<Option<String>>);

pub use cli::Mode;

/// Parse the process arguments into a [`cli::Mode`].
pub fn parse_cli() -> Mode {
    cli::parse_args(std::env::args().skip(1))
}

fn is_app_url(url: &Url) -> bool {
    match url.scheme() {
        "tauri" => true,
        "http" | "https" => matches!(
            url.host_str(),
            Some("tauri.localhost") | Some("localhost") | Some("127.0.0.1")
        ),
        _ => false,
    }
}

fn is_app_entry(url: &Url) -> bool {
    matches!(url.path(), "" | "/" | "/index.html")
}

fn handle_navigation(app: &AppHandle, url: &Url) -> bool {
    let scheme = url.scheme();

    if matches!(scheme, "http" | "https" | "mailto" | "tel") {
        if is_app_url(url) {
            return is_app_entry(url);
        }
        let _ = app.opener().open_url(url.as_str(), None::<&str>);
        return false;
    }

    if scheme == "file" {
        // Don't hand arbitrary local paths to the OS opener. If a link points at
        // a markdown file, open it inside emede; otherwise block it entirely.
        let path = url.path();
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".md") || lower.ends_with(".markdown") {
            let _ = app.emit("file-to-open", path.to_string());
        }
        return false;
    }

    if is_app_url(url) {
        return is_app_entry(url);
    }

    scheme == "tauri" || scheme == "data"
}

/// Print the help message to stdout.
pub fn print_help() {
    let name = env!("CARGO_PKG_NAME");
    println!(
        "\
{name} {version}
{description}

Author:  {authors}
Repo:    {repository}

USAGE:
    {name} [FILE]...                 Open one or more markdown files, each in its own window
    {name} --share [FILE]...         Share notes on the local network (headless, no window)
    {name} --export <FILE> [-o OUT]  Write a self-contained HTML file (OUT '-' = stdout)
    {name} --print <FILE> [-o OUT]   Render a PDF (Linux only)
    {name} --list [--json]           List notes shared by any running emede instance

ARGS:
    <FILE>    Path to a markdown file (or an http(s) URL). Use '--' before a
              path that begins with '-'.

OPTIONS:
    -o, --output <PATH>  Output path for --export / --print
        --json           Machine-readable output for --list
    -h, --help           Print this help message and exit
    -v, --version        Print version information and exit

NOTES:
    Exported HTML inlines images but loads MathJax/Mermaid from a CDN, so math
    and diagrams require internet access to render in a browser.",
        name = name,
        version = env!("CARGO_PKG_VERSION"),
        description = env!("CARGO_PKG_DESCRIPTION"),
        authors = env!("CARGO_PKG_AUTHORS"),
        repository = env!("CARGO_PKG_REPOSITORY"),
    );
}

/// Print the version string to stdout.
pub fn print_version() {
    println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
}

pub fn apply_gpu_setting() {
    #[cfg(target_os = "linux")]
    {
        let settings = settings::load_settings();
        if !settings.gpu_acceleration {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
            std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
        }
    }
}

// ── Headless CLI entry points ──────────────────────────────────────────────────

/// Derive a default output path next to `file`, swapping the extension for `ext`.
fn default_out_path(file: &str, ext: &str) -> String {
    let p = std::path::Path::new(file);
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".to_string());
    match p.parent().filter(|d| !d.as_os_str().is_empty()) {
        Some(dir) => dir
            .join(format!("{stem}.{ext}"))
            .to_string_lossy()
            .into_owned(),
        None => format!("{stem}.{ext}"),
    }
}

/// `emede --share ...`: serve the notes on the LAN, headless, until Ctrl+C.
pub fn run_share(files: Vec<String>) -> ! {
    match share::serve_files_headless(&files) {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("{}: {e}", env!("CARGO_PKG_NAME"));
            std::process::exit(1);
        }
    }
}

/// `emede --export ...`: write a self-contained HTML file (or stdout for "-").
pub fn run_export(file: String, out: Option<String>) -> ! {
    let html = match share::build_shared_page(&file) {
        Ok(html) => html,
        Err(e) => {
            eprintln!("{}: {e}", env!("CARGO_PKG_NAME"));
            std::process::exit(1);
        }
    };

    if out.as_deref() == Some("-") {
        print!("{html}");
        std::process::exit(0);
    }

    let path = out.unwrap_or_else(|| default_out_path(&file, "html"));
    match std::fs::write(&path, html) {
        Ok(()) => {
            println!("Wrote {path}");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("{}: failed to write {path}: {e}", env!("CARGO_PKG_NAME"));
            std::process::exit(1);
        }
    }
}

/// `emede --list [--json]`: list notes shared by any running emede instance.
pub fn run_list(json: bool) -> ! {
    let entries = share::list_active_shares();

    if json {
        println!(
            "{}",
            serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string())
        );
    } else if entries.is_empty() {
        eprintln!("No notes are currently being shared.");
    } else {
        let n = entries.len();
        println!("Shared notes ({n}):");
        for e in &entries {
            println!("  {}\n    {}", e.title, e.url);
        }
    }
    std::process::exit(0);
}

/// `emede --print ...`: render a PDF via the bundled WebView (Linux only).
pub fn run_print(file: String, out: Option<String>) {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (file, out);
        eprintln!(
            "{}: --print (PDF export) is currently Linux-only; use --export for HTML.",
            env!("CARGO_PKG_NAME")
        );
        std::process::exit(1);
    }

    #[cfg(target_os = "linux")]
    {
        // Fail early (before spinning up a WebView) if the note can't be rendered.
        if let Err(e) = markdown::render_markdown_any(&file) {
            eprintln!("{}: {e}", env!("CARGO_PKG_NAME"));
            std::process::exit(1);
        }
        let out_path = out.unwrap_or_else(|| default_out_path(&file, "pdf"));
        run_inner(vec![file], Some(out_path));
    }
}

// ── Windowed / WebView app ─────────────────────────────────────────────────────

/// Spawn a fresh emede process to open `path` in its own window.
fn spawn_window_for(path: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("failed to get current exe path: {e}"))?;
    std::process::Command::new(exe)
        .arg(path)
        .spawn()
        .map_err(|e| format!("failed to open new window: {e}"))?;
    Ok(())
}

/// `emede [FILE]...`: open the reader, one window per file.
pub fn run(files: Vec<String>) {
    run_inner(files, None);
}

fn run_inner(files: Vec<String>, print_target: Option<String>) {
    let first = files.first().cloned();
    // Extra files each get their own window (spawned as separate processes).
    let extra: Vec<String> = files.iter().skip(1).cloned().collect();

    tauri::Builder::default()
        .plugin(
            tauri_plugin_window_state::Builder::new()
                // Keep visibility under frontend control so the window stays hidden
                // until the first frame is painted.
                .with_state_flags(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED)
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri::plugin::Builder::<tauri::Wry, ()>::new("external-links")
                .on_navigation(|webview, url| handle_navigation(webview.app_handle(), url))
                .build(),
        )
        .manage(StartupFile(Mutex::new(first)))
        .manage(PrintTarget(Mutex::new(print_target)))
        .manage(share::ShareState(Mutex::new(share::ShareStateInner::default())))
        .manage(watcher::WatcherState::default())
        .invoke_handler(tauri::generate_handler![
            markdown::render_markdown,
            markdown::render_markdown_url,
            settings::get_settings,
            settings::set_settings,
            settings::read_color_template,
            view_state::get_view_state,
            view_state::set_view_state,
            share::start_share,
            share::stop_share,
            share::stop_share_note,
            share::get_share_status,
            share::get_note_share_info,
            share::generate_share_qr,
            watcher::watch_document,
            watcher::unwatch_document,
            recents::get_recent_files,
            get_startup_file,
            get_print_target,
            print_ready,
            get_app_version,
            restart_app,
            open_in_new_window,
        ])
        .setup(move |app| {
            let print_mode = app
                .state::<PrintTarget>()
                .0
                .lock()
                .map(|g| g.is_some())
                .unwrap_or(false);

            if let Some(window) = app.get_webview_window("main") {
                let settings = settings::load_settings();
                let use_system_frame = settings.window_frame == "system";
                let _ = window.set_decorations(use_system_frame);
                if let Some(color) = hex_to_color(&settings.color_bg) {
                    let _ = window.set_background_color(Some(color));
                }

                // For headless PDF export, the WebView must be *realized* (mapped)
                // or WebKitGTK never lays out the page and the frontend's render
                // loop is throttled — so nothing ever prints. Park the window far
                // off-screen and show it: it renders normally but the user never
                // sees it, and the process exits as soon as the PDF is written.
                if print_mode {
                    let _ = window.set_skip_taskbar(true);
                    let _ = window.set_position(tauri::PhysicalPosition::new(-32000, -32000));
                    let _ = window.show();
                }
            }

            // Open any additional files in their own windows.
            for path in &extra {
                if let Err(e) = spawn_window_for(path) {
                    eprintln!("emede: {e}");
                }
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[tauri::command]
fn get_startup_file(state: tauri::State<StartupFile>) -> Option<String> {
    state.0.lock().ok()?.clone()
}

/// Returns the PDF output path when this invocation is a headless print export,
/// else `None`. The frontend uses this to switch into the print-and-exit flow.
#[tauri::command]
fn get_print_target(state: tauri::State<PrintTarget>) -> Option<String> {
    state.0.lock().ok()?.clone()
}

/// Called by the frontend once the document (including math and diagrams) has
/// finished rendering in print mode. Exports the WebView to PDF and exits.
#[tauri::command]
fn print_ready(app: tauri::AppHandle, state: tauri::State<PrintTarget>) -> Result<(), String> {
    let out = state
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .ok_or("not in print mode")?;

    let window = app
        .get_webview_window("main")
        .ok_or("no main window to print")?;

    // A successful print exits the process from the operation's signal handlers.
    // If setup fails, exit non-zero here so the headless process never hangs with
    // an invisible window.
    if let Err(e) = print_webview_to_pdf(&window, &out) {
        eprintln!("{}: {e}", env!("CARGO_PKG_NAME"));
        std::process::exit(1);
    }
    Ok(())
}

/// Export `window`'s WebView to a PDF file at `out`. On Linux this drives the
/// WebKitGTK print operation directly, configured to write to a file without a
/// dialog. The process exits from the print-operation signal handlers.
#[cfg(target_os = "linux")]
fn print_webview_to_pdf(window: &tauri::WebviewWindow, out: &str) -> Result<(), String> {
    use webkit2gtk::{PrintOperation, PrintOperationExt};

    let abs = std::path::Path::new(out);
    let abs = if abs.is_absolute() {
        abs.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(abs)
    };
    let uri = format!("file://{}", abs.to_string_lossy());

    window
        .with_webview(move |wv| {
            let webview = wv.inner();
            let print_op = PrintOperation::new(&webview);

            let settings = gtk::PrintSettings::new();
            // Select GTK's built-in "Print to File" backend and point it at our
            // PDF path. Setting only the URI isn't enough — without naming the
            // printer, the operation falls through to the default (physical)
            // printer via lpr.
            settings.set("printer", Some("Print to File"));
            settings.set("output-uri", Some(uri.as_str()));
            settings.set("output-file-format", Some("pdf"));
            print_op.set_print_settings(&settings);

            print_op.connect_finished(|_| {
                std::process::exit(0);
            });
            print_op.connect_failed(|_, err| {
                eprintln!("emede: PDF export failed: {err}");
                std::process::exit(1);
            });

            print_op.print();
            // Keep the operation alive so its async signals can fire.
            std::mem::forget(print_op);
        })
        .map_err(|e| format!("failed to access WebView for printing: {e}"))?;

    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn print_webview_to_pdf(_window: &tauri::WebviewWindow, _out: &str) -> Result<(), String> {
    Err("PDF export is only supported on Linux".to_string())
}

#[tauri::command]
fn get_app_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
fn open_in_new_window(path: String) -> Result<(), String> {
    spawn_window_for(&path)
}

#[tauri::command]
fn restart_app() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("failed to get current exe path: {e}"))?;
    let args: Vec<String> = std::env::args().collect();
    std::process::Command::new(exe)
        .args(args.iter().skip(1))
        .spawn()
        .map_err(|e| format!("failed to restart app: {e}"))?;
    std::process::exit(0);
}

fn hex_to_color(hex: &str) -> Option<Color> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }

    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color(r, g, b, 255))
}
