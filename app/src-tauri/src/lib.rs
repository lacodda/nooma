//! The nooma window.
//!
//! A shell around `nooma-core`: the library the CLI keeps is the library the
//! window searches, in the same place on disk. Nothing here reaches the
//! network — the plugins are the dialog that picks a folder, the opener that
//! hands a found file to its application, and the one that remembers where the
//! window was.

pub mod commands;

use tauri::Manager as _;

/// Start the application. `main.rs` is deliberately thin: everything lives
/// here, where tests can reach it without opening a window.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(commands::AppState::new())
        .setup(|app| {
            // The window is created hidden and shown by the page once it has
            // painted, so the first thing seen is the field, never a white
            // rectangle. A page that fails to load would leave it hidden, so
            // it is shown here as well after a moment regardless.
            if let Some(window) = app.get_webview_window("main") {
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(1500));
                    let _ = window.show();
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::status,
            commands::search,
            commands::update,
            commands::add_source,
            commands::open_document,
            commands::reveal_document,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the application");
}
