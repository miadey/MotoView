// Bzzz desktop shell. The main window (configured in tauri.conf.json) loads the
// Bzzz canister URL directly, so the desktop app is the same server-driven UI
// that the web and mobile clients use — one source of truth, zero duplicated UI.

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .run(tauri::generate_context!())
        .expect("error while running the Bzzz desktop app");
}
