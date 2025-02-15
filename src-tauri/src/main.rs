#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

extern crate core;

use crate::device_manager::DeviceManager;
use crate::session_manager::SessionManager;
use crate::spawn_manager::SpawnManager;
use dialog::DialogBox;
use tauri::Manager;

mod device_manager;
mod error;
mod event_channel;
mod plugins;
mod remote_files;
mod session_manager;
mod spawn_manager;

fn main() {
    env_logger::init();
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            if let Some(wnd) = app.get_window("main") {
                wnd.unminimize().unwrap_or(());
                wnd.set_focus().unwrap_or(());
            }
        }))
        .plugin(plugins::device::plugin("device-manager"))
        .plugin(plugins::cmd::plugin("remote-command"))
        .plugin(plugins::shell::plugin("remote-shell"))
        .plugin(plugins::file::plugin("remote-file"))
        .plugin(plugins::devmode::plugin("dev-mode"))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .manage(DeviceManager::default())
        .manage(SessionManager::default())
        .manage(SpawnManager::default())
        .on_page_load(|wnd, payload| {
            let spawns = wnd.state::<SpawnManager>();
            spawns.clear();
        })
        .run(tauri::generate_context!());
    if let Err(e) = result {
        dialog::Message::new("Unexpected error occurred")
            .title("webOS Dev Manager")
            .show()
            .expect("Unexpected error occurred while processing unexpected error :(");
    }
}
