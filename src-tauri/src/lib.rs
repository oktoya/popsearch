mod history;
mod keyhook;
mod office;
mod provider_cli;
mod selection;
mod settings;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, PhysicalPosition, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

fn show_window(app: &AppHandle) {
    show_window_with(app, selection::Selection::default());
}

fn show_window_with(app: &AppHandle, selection: selection::Selection) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };

    if let Ok(cursor) = app.cursor_position() {
        if let Ok(monitors) = app.available_monitors() {
            let target = monitors.iter().find(|m| {
                let p = m.position();
                let s = m.size();
                cursor.x >= p.x as f64
                    && cursor.x < p.x as f64 + s.width as f64
                    && cursor.y >= p.y as f64
                    && cursor.y < p.y as f64 + s.height as f64
            });
            if let Some(m) = target {
                if let Ok(win_size) = win.outer_size() {
                    let p = m.position();
                    let s = m.size();
                    let x = p.x + (s.width as i32 - win_size.width as i32) / 2;
                    let y = p.y + (s.height as f64 * 0.2) as i32;
                    let _ = win.set_position(PhysicalPosition::new(x, y));
                }
            }
        }
    }

    let _r_show = win.show();
    #[cfg(debug_assertions)]
    eprintln!("[popsearch-debug] show() ok={}", _r_show.is_ok());
    let _r_focus = win.set_focus();
    #[cfg(debug_assertions)]
    eprintln!("[popsearch-debug] set_focus() ok={}", _r_focus.is_ok());
    let _ = app.emit(
        "popsearch://shown",
        serde_json::json!({ "selection": selection.text, "image": selection.image }),
    );
}

fn toggle_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        if win.is_visible().unwrap_or(false) && win.is_focused().unwrap_or(false) {
            #[cfg(debug_assertions)]
            eprintln!("[popsearch-debug] toggle: hide (visible+focused)");
            let _ = win.hide();
            return;
        }
    }
    let capture = settings::get_settings(app.clone()).capture_selection;
    let own_focus = app
        .get_webview_window("main")
        .map(|w| w.is_focused().unwrap_or(false))
        .unwrap_or(false);
    #[cfg(target_os = "macos")]
    let app_active = objc2_app_kit::NSRunningApplication::currentApplication().isActive();
    #[cfg(not(target_os = "macos"))]
    let app_active = false;
    #[cfg(all(debug_assertions, target_os = "macos"))]
    eprintln!("[popsearch-debug] toggle: app_active={}", app_active);
    if capture && !own_focus && !app_active {
        #[cfg(debug_assertions)]
        eprintln!("[popsearch-debug] toggle: capture thread");
        let app = app.clone();
        std::thread::spawn(move || {
            let sel = selection::capture_selection();
            let urls = sel.image_urls.clone();
            show_window_with(&app, sel);
            for url in urls {
                if let Some(img) = selection::fetch_image(&url) {
                    let _ = app.emit("popsearch://selection-image", img);
                }
            }
        });
    } else {
        #[cfg(debug_assertions)]
        eprintln!("[popsearch-debug] toggle: direct show");
        show_window(app);
    }
}

static SUPPRESS_HIDE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[tauri::command]
fn set_suppress_hide(value: bool) {
    SUPPRESS_HIDE.store(value, std::sync::atomic::Ordering::SeqCst);
}

#[tauri::command]
fn hide_window(window: tauri::WebviewWindow) {
    let _ = window.hide();
}

#[tauri::command]
fn ask(
    app: AppHandle,
    manager: tauri::State<provider_cli::Manager>,
    text: String,
    model: String,
    web_search: bool,
    attachments: Vec<provider_cli::AttachmentInput>,
) {
    manager.ask(&app, &model, web_search, &text, &attachments);
}

#[tauri::command]
async fn extract_office(name: String, data: String) -> Result<String, String> {
    let bytes = office::decode_base64(&data)
        .ok_or_else(|| format!("파일을 읽을 수 없습니다: {}", name))?;
    office::extract(&name, &bytes)
}

#[tauri::command]
fn new_conversation(app: AppHandle, manager: tauri::State<provider_cli::Manager>) {
    manager.new_conversation(&app);
}

#[tauri::command]
async fn cli_login(
    app: AppHandle,
    manager: tauri::State<'_, provider_cli::Manager>,
) -> Result<(), String> {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || provider_cli::run_login(&app2))
        .await
        .map_err(|e| format!("로그인 실행 실패: {e}"))??;
    manager.reset_sessions(&app);
    Ok(())
}

#[tauri::command]
fn reset_cli_sessions(app: AppHandle, manager: tauri::State<provider_cli::Manager>) {
    manager.reset_sessions(&app);
}

#[tauri::command]
fn get_system_prompt() -> &'static str {
    provider_cli::SYSTEM_PROMPT
}

struct HotkeyError(std::sync::Mutex<Option<String>>);

#[tauri::command]
fn get_hotkey_error(state: tauri::State<HotkeyError>) -> Option<String> {
    state.0.lock().ok().and_then(|g| g.clone())
}

#[tauri::command]
fn set_hotkey(
    app: AppHandle,
    state: tauri::State<HotkeyError>,
    accelerator: String,
) -> Result<(), String> {
    const FAIL: &str = "이 단축키를 등록할 수 없습니다. 다른 프로그램이 사용 중일 수 있습니다.";
    let mut current = settings::get_settings(app.clone());
    let has_error = state.0.lock().map(|g| g.is_some()).unwrap_or(false);
    if current.hotkey == accelerator && !has_error {
        return Ok(());
    }
    let gs = app.global_shortcut();
    gs.register(accelerator.as_str()).map_err(|_| FAIL.to_string())?;
    if current.hotkey != accelerator {
        let _ = gs.unregister(current.hotkey.as_str());
    }
    let old = current.hotkey.clone();
    current.hotkey = accelerator.clone();
    if let Err(e) = settings::write_settings(&app, &current) {
        let _ = gs.unregister(accelerator.as_str());
        if old != accelerator {
            let _ = gs.register(old.as_str());
        }
        return Err(e);
    }
    #[cfg(windows)]
    keyhook::set_hotkey(accelerator.as_str());
    if let Ok(mut g) = state.0.lock() {
        *g = None;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_window(app);
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    #[cfg(debug_assertions)]
                    eprintln!("[popsearch-debug] shortcut event: {:?}", event.state());
                    if event.state() == ShortcutState::Pressed {
                        toggle_window(app);
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![
            hide_window,
            set_suppress_hide,
            ask,
            extract_office,
            new_conversation,
            reset_cli_sessions,
            cli_login,
            get_system_prompt,
            get_hotkey_error,
            set_hotkey,
            settings::get_settings,
            settings::set_settings,
            settings::is_release,
            settings::get_api_key,
            settings::set_api_key,
            history::history_add,
            history::history_list,
            history::history_get,
            history::history_delete,
            history::save_text_file
        ])
        .on_window_event(|window, event| {
            #[cfg(debug_assertions)]
            if let WindowEvent::Focused(f) = event {
                if window.label() == "main" {
                    eprintln!(
                        "[popsearch-debug] Focused({}) suppress={}",
                        f,
                        SUPPRESS_HIDE.load(std::sync::atomic::Ordering::SeqCst)
                    );
                }
            }
            if let WindowEvent::Focused(false) = event {
                if window.label() == "main"
                    && !SUPPRESS_HIDE.load(std::sync::atomic::Ordering::SeqCst)
                {
                    #[cfg(debug_assertions)]
                    eprintln!("[popsearch-debug] hide executed");
                    let _ = window.hide();
                }
            }
        })
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            #[cfg(target_os = "macos")]
            {
                use tauri::menu::{PredefinedMenuItem, Submenu};
                let app_menu = Submenu::with_items(
                    app,
                    app.package_info().name.clone(),
                    true,
                    &[
                        &PredefinedMenuItem::about(app, None, None)?,
                        &PredefinedMenuItem::separator(app)?,
                        &PredefinedMenuItem::services(app, None)?,
                        &PredefinedMenuItem::separator(app)?,
                        &PredefinedMenuItem::hide_others(app, None)?,
                        &PredefinedMenuItem::separator(app)?,
                        &PredefinedMenuItem::quit(app, None)?,
                    ],
                )?;
                let edit_menu = Submenu::with_items(
                    app,
                    "Edit",
                    true,
                    &[
                        &PredefinedMenuItem::undo(app, None)?,
                        &PredefinedMenuItem::redo(app, None)?,
                        &PredefinedMenuItem::separator(app)?,
                        &PredefinedMenuItem::cut(app, None)?,
                        &PredefinedMenuItem::copy(app, None)?,
                        &PredefinedMenuItem::paste(app, None)?,
                        &PredefinedMenuItem::select_all(app, None)?,
                    ],
                )?;
                app.set_menu(Menu::with_items(app, &[&app_menu, &edit_menu])?)?;
            }

            let manager = provider_cli::Manager::new();
            manager.warm_up(app.handle());
            app.manage(manager);
            app.manage(history::open(app.handle())?);

            let startup_settings = settings::get_settings(app.handle().clone());
            settings::apply_autostart(app.handle(), startup_settings.autostart);
            let hotkey = startup_settings.hotkey;
            let hotkey_error = match app.global_shortcut().register(hotkey.as_str()) {
                Ok(()) => None,
                Err(e) => Some(e.to_string()),
            };
            #[cfg(windows)]
            {
                keyhook::install();
                if hotkey_error.is_none() {
                    keyhook::set_hotkey(hotkey.as_str());
                } else {
                    keyhook::disable();
                }
            }
            let failed = hotkey_error.is_some();
            app.manage(HotkeyError(std::sync::Mutex::new(hotkey_error)));
            if failed {
                show_window(app.handle());
            }

            let open = MenuItem::with_id(app, "open", "열기", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "종료", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;

            let mut tray = TrayIconBuilder::new()
                .tooltip("팝써치")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_window(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                app.state::<provider_cli::Manager>().kill_all();
            }
        });
}
