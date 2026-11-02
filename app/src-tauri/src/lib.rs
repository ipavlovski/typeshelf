mod automation;
mod channel;
mod commands;
mod platform;
mod providers;
mod state;
mod thumbs;
mod updater;
mod watch;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, RunEvent, WindowEvent};

use state::AppState;

/// `--reconcile-logon` runs at logon (HKCU Run): clears the previous session's
/// fonts and exits without showing a window.
fn reconcile_logon() -> anyhow::Result<()> {
    let mut store = ts_store::Store::open(&state::data_dir()?.join("library.db"))?;
    if let Some(backend) = ts_activate::platform_for(channel::APP) {
        ts_activate::reconcile(&mut store, backend.as_ref(), true)?;
    }
    Ok(())
}

pub fn run() {
    if std::env::args().any(|a| a == "--reconcile-logon") {
        if let Err(e) = reconcile_logon() {
            eprintln!("reconcile failed: {e:#}");
            std::process::exit(1);
        }
        return;
    }

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .register_asynchronous_uri_scheme_protocol("thumb", thumbs::handle)
        .setup(|app| {
            let state = AppState::open(state::data_dir()?)?;
            app.manage(state);
            let _ = platform::enable_logon_reconcile();
            platform::install_session_end_hook(app.handle().clone());
            watch::restart(app.handle());
            automation::start(app.handle().clone());
            updater::init(app.handle());
            // Catch up with changes made while the app was closed (incremental: unchanged files are skipped).
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let state = handle.state::<AppState>();
                let folders = state.store.lock().unwrap().folders().unwrap_or_default();
                for folder in &folders {
                    let _ = commands::reindex(&handle, &state, folder);
                }
            });

            let show = MenuItem::with_id(app, "show", "Show typeshelf", true, None::<&str>)?;
            let deactivate = MenuItem::with_id(app, "deactivate", "Deactivate session fonts", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &deactivate, &quit])?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().cloned().expect("bundled icon"))
                .tooltip(if channel::IS_DEV { "typeshelfDev" } else { "typeshelf" })
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main(app),
                    "deactivate" => {
                        let _ = app.state::<AppState>().deactivate_session();
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, .. } = event {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps fonts active; quit from the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::search,
            commands::family_faces,
            commands::set_active,
            commands::set_pinned,
            commands::set_collection_active,
            commands::deactivate_all,
            commands::set_favourite,
            commands::library,
            commands::add_folder,
            commands::remove_folder,
            commands::collection_create,
            commands::collection_rename,
            commands::collection_delete,
            commands::collection_add,
            commands::get_settings,
            commands::set_settings,
            commands::font_bytes,
            commands::face_codepoints,
            commands::metric_histograms,
            commands::reveal,
            providers::fontbase_preview,
            providers::fontbase_import,
            providers::download_pending_fonts,
            providers::google_catalogue,
            providers::google_install,
            updater::app_info,
            updater::update_status,
            updater::check_for_updates,
            updater::install_release,
            updater::install_update,
            updater::list_releases,
        ])
        .build(tauri::generate_context!())
        .expect("error while building typeshelf");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            // Session fonts live as long as the app, like FontBase's "deactivate on exit".
            if let Some(state) = app.try_state::<AppState>() {
                let _ = state.deactivate_session();
            }
            updater::install_on_exit(app);
        }
    });
}

fn show_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}
