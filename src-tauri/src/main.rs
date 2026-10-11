#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod state;
mod audio;
mod receiver;

use state::{AppSnapshot, AppState, OperationResult};
use tauri::{Emitter, Manager, State};

#[tauri::command]
fn get_snapshot(state: State<'_, AppState>) -> AppSnapshot {
    state.snapshot()
}

#[tauri::command]
async fn refresh_devices(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    let snapshot = state.refresh_devices().await?;
    let _ = app.emit("anpod://snapshot", &snapshot);
    Ok(snapshot)
}

#[tauri::command]
async fn connect_device(app: tauri::AppHandle, state: State<'_, AppState>, serial: String) -> Result<OperationResult, String> {
    state.connect_device(&app, serial).await
}

#[tauri::command]
async fn connect_device_only(app: tauri::AppHandle, state: State<'_, AppState>, serial: String) -> Result<OperationResult, String> {
    state.connect_device_only(&app, serial).await
}

#[tauri::command]
async fn switch_device(app: tauri::AppHandle, state: State<'_, AppState>, serial: String) -> Result<OperationResult, String> {
    state.switch_device(&app, serial).await
}

#[tauri::command]
async fn disconnect(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<OperationResult, String> {
    state.disconnect(&app).await
}

#[tauri::command]
fn select_device(state: State<'_, AppState>, serial: String) -> Result<AppSnapshot, String> {
    state.select_device(&serial)
}

#[tauri::command]
async fn set_output(app: tauri::AppHandle, state: State<'_, AppState>, output: String) -> Result<AppSnapshot, String> {
    state.set_output(&app, &output).await
}

#[tauri::command]
async fn cancel_operation(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    Ok(state.cancel_operation(&app).await)
}

#[tauri::command]
fn update_setting(state: State<'_, AppState>, key: String, value: serde_json::Value) -> Result<AppSnapshot, String> {
    state.update_setting(&key, value)?;
    Ok(state.snapshot())
}

#[tauri::command]
async fn connect_wifi(state: State<'_, AppState>, endpoint: String) -> Result<OperationResult, String> {
    state.connect_wifi(&endpoint).await
}

#[tauri::command]
async fn pair_wifi(state: State<'_, AppState>, endpoint: String, code: String) -> Result<OperationResult, String> {
    state.pair_wifi(&endpoint, &code).await
}

#[tauri::command]
async fn discover_wifi(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    state.discover_wifi().await
}

#[tauri::command]
fn qr_credentials(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    state.qr_credentials()
}

#[tauri::command]
async fn pair_qr(app: tauri::AppHandle, state: State<'_, AppState>, instance_name: String, password: String) -> Result<OperationResult, String> {
    state.pair_qr(&app, instance_name, password).await
}

#[tauri::command]
async fn receiver_status(state: State<'_, AppState>, serial: String) -> Result<receiver::ReceiverStatus, String> {
    state.receiver_status(&serial).await
}

#[tauri::command]
async fn receiver_install(state: State<'_, AppState>, serial: String) -> Result<receiver::ReceiverStatus, String> {
    state.receiver_install(&serial).await
}

#[tauri::command]
async fn receiver_start(state: State<'_, AppState>, serial: String) -> Result<receiver::ReceiverStatus, String> {
    state.receiver_start(&serial).await
}

#[tauri::command]
async fn receiver_health(state: State<'_, AppState>, serial: String) -> Result<receiver::ReceiverStatus, String> {
    state.receiver_health(&serial).await
}

#[tauri::command]
async fn receiver_stop(state: State<'_, AppState>, serial: String) -> Result<receiver::ReceiverStatus, String> {
    state.receiver_stop(&serial).await
}

#[tauri::command]
fn cancel_qr_pairing(state: State<'_, AppState>) {
    state.cancel_qr_pairing();
}

#[tauri::command]
async fn reconfigure_session(app: tauri::AppHandle, state: State<'_, AppState>, kind: String, value: String) -> Result<AppSnapshot, String> {
    state.reconfigure_session(&app, &kind, &value).await
}

#[tauri::command]
fn show_main_window(window: tauri::WebviewWindow) -> Result<(), String> {
    window.show().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())
}

#[tauri::command]
fn open_logs(state: State<'_, AppState>) -> Result<(), String> {
    state.open_logs()
}

#[tauri::command]
async fn close_to_tray(app: tauri::AppHandle, window: tauri::WebviewWindow, state: State<'_, AppState>) -> Result<(), String> {
    if state.snapshot().settings.minimize_to_tray {
        window.hide().map_err(|error| error.to_string())
    } else {
        state.disconnect(&app).await?;
        app.exit(0);
        Ok(())
    }
}

static SHOW_EVENT: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static INSTANCE_MUTEX: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

#[cfg(windows)]
fn claim_single_instance() -> Result<bool, String> {
    use std::{ffi::c_void, ptr};
    const ERROR_ALREADY_EXISTS: u32 = 183;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateMutexW(attributes: *const c_void, initial_owner: i32, name: *const u16) -> isize;
        fn CreateEventW(attributes: *const c_void, manual_reset: i32, initial_state: i32, name: *const u16) -> isize;
        fn SetEvent(handle: isize) -> i32;
        fn GetLastError() -> u32;
    }
    let mutex_name: Vec<u16> = "Local\\AnPod.SingleInstance\0".encode_utf16().collect();
    let show_name: Vec<u16> = "Local\\AnPod.ShowWindow\0".encode_utf16().collect();
    unsafe {
        let mutex = CreateMutexW(ptr::null(), 1, mutex_name.as_ptr());
        if mutex == 0 { return Err("无法创建 AnPod 单实例互斥锁。".into()); }
        let exists = GetLastError() == ERROR_ALREADY_EXISTS;
        INSTANCE_MUTEX.store(mutex, std::sync::atomic::Ordering::Relaxed);
        let event = CreateEventW(ptr::null(), 0, 0, show_name.as_ptr());
        if event == 0 { return Err("无法创建 AnPod 显示窗口事件。".into()); }
        SHOW_EVENT.store(event, std::sync::atomic::Ordering::Relaxed);
        if exists { let _ = SetEvent(event); }
        Ok(!exists)
    }
}

#[cfg(not(windows))]
fn claim_single_instance() -> Result<bool, String> { Ok(true) }

fn main() {
    match claim_single_instance() {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => { eprintln!("{error}"); return; }
    }
    tauri::Builder::default()
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            refresh_devices,
            select_device,
            connect_device,
            connect_device_only,
            switch_device,
            disconnect,
            set_output,
            cancel_operation,
            update_setting,
            connect_wifi,
            pair_wifi,
            discover_wifi,
            qr_credentials,
            pair_qr,
            receiver_status,
            receiver_install,
            receiver_start,
            receiver_health,
            receiver_stop,
            cancel_qr_pairing,
            reconfigure_session,
            show_main_window,
            open_logs,
            close_to_tray
        ])
        .setup(|app| {
            let state = app.state::<AppState>();
            state.initialize(app.handle())?;
            let monitor_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    if let Some(state) = monitor_app.try_state::<AppState>() {
                        state.monitor_once(&monitor_app).await;
                    } else { break; }
                }
            });
            let rate_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    if let Some(state) = rate_app.try_state::<AppState>() {
                        state.sample_audio_bitrate(&rate_app).await;
                    } else { break; }
                }
            });
            #[cfg(windows)]
            {
                let handle = SHOW_EVENT.load(std::sync::atomic::Ordering::Relaxed);
                let app_handle = app.handle().clone();
                std::thread::Builder::new().name("AnPod show window".into()).spawn(move || unsafe {
                    #[link(name = "kernel32")]
                    unsafe extern "system" { fn WaitForSingleObject(handle: isize, milliseconds: u32) -> u32; }
                    while WaitForSingleObject(handle, 0xFFFF_FFFF) == 0 {
                        if let Some(window) = app_handle.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })?;
            }
            let show = tauri::menu::MenuItem::with_id(app, "show", "打开 AnPod", true, None::<&str>)?;
            let disconnect = tauri::menu::MenuItem::with_id(app, "disconnect", "断开连接", true, None::<&str>)?;
            let quit = tauri::menu::MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = tauri::menu::Menu::with_items(app, &[&show, &disconnect, &quit])?;
            let _tray = tauri::tray::TrayIconBuilder::new()
                .icon(app.default_window_icon().expect("Tauri window icon is configured").clone())
                .menu(&menu)
                .tooltip("AnPod")
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => if let Some(window) = app.get_webview_window("main") { let _ = window.show(); let _ = window.set_focus(); },
                    "disconnect" => { let app = (*app).clone(); tauri::async_runtime::spawn(async move { if let Some(state) = app.try_state::<AppState>() { let _ = state.disconnect(&app).await; } }); },
                    "quit" => { let app = (*app).clone(); tauri::async_runtime::spawn(async move { if let Some(state) = app.try_state::<AppState>() { let _ = state.disconnect(&app).await; } app.exit(0); }); },
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let app = window.app_handle().clone();
                let label = window.label().to_owned();
                tauri::async_runtime::spawn(async move {
                    let Some(state) = app.try_state::<AppState>() else { return; };
                    if state.snapshot().settings.minimize_to_tray {
                        if let Some(window) = app.get_webview_window(&label) { let _ = window.hide(); }
                    } else {
                        let _ = state.disconnect(&app).await;
                        app.exit(0);
                    }
                });
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to run AnPod");
}
