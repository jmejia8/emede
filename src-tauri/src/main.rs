// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    emede_lib::handle_cli_flags();
    emede_lib::apply_gpu_setting();
    emede_lib::run()
}
