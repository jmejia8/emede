// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use emede_lib::cli::Mode;

fn main() {
    match emede_lib::parse_cli() {
        Mode::Help => emede_lib::print_help(),
        Mode::Version => emede_lib::print_version(),
        Mode::Error(msg) => {
            eprintln!("{msg}");
            std::process::exit(2);
        }
        // Headless modes: do their work and exit before any window is built.
        Mode::List { json } => emede_lib::run_list(json),
        Mode::Export { file, out } => emede_lib::run_export(file, out),
        Mode::Share(files) => emede_lib::run_share(files),
        // Modes that need the WebView.
        Mode::Print { file, out } => {
            emede_lib::apply_gpu_setting();
            emede_lib::run_print(file, out);
        }
        Mode::Open(files) => {
            emede_lib::apply_gpu_setting();
            emede_lib::run(files);
        }
    }
}
