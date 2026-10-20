#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[allow(dead_code)]
mod aoa;
#[allow(dead_code)]
mod aoa_owner;
#[allow(dead_code)]
mod aoa_winusb;
mod audio;
mod receiver;
mod state;
// Receiver-side protocol, session, and handshake code below is a cross-language
// test oracle for the Java Receiver: the shipped binary only drives the host
// side, so clippy sees those items as dead. Keep the allowance until W6-4 moves
// the oracle into a #[cfg(test)] module. network_interface has no test-only
// items and stays under the gate.
#[allow(dead_code)]
mod discovery;
#[allow(dead_code)]
mod network;
mod network_interface;
#[allow(dead_code)]
mod p5c;
mod presence;
#[allow(dead_code)]
mod protocol;
#[allow(dead_code)]
mod security;

use state::{AoaDeviceTarget, AppSnapshot, AppState, OperationResult};
use tauri::{Emitter, Manager, State};

#[tauri::command]
fn get_snapshot(state: State<'_, AppState>) -> AppSnapshot {
    state.snapshot()
}

#[tauri::command]
async fn refresh_devices(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    let snapshot = state.refresh_devices().await?;
    let _ = app.emit("anpod://snapshot", &snapshot);
    Ok(snapshot)
}

#[tauri::command]
async fn list_aoa_targets(state: State<'_, AppState>) -> Result<Vec<AoaDeviceTarget>, String> {
    state.list_aoa_targets().await
}

#[tauri::command]
async fn start_aoa_audio(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    target_id: String,
    binding_reference: String,
) -> Result<AppSnapshot, String> {
    state
        .start_aoa_audio(&app, &target_id, &binding_reference)
        .await
}

#[tauri::command]
async fn connect_device(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    serial: String,
) -> Result<OperationResult, String> {
    state.connect_device(&app, serial).await
}

#[tauri::command]
async fn connect_device_only(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    serial: String,
) -> Result<OperationResult, String> {
    state.connect_device_only(&app, serial).await
}

#[tauri::command]
async fn switch_device(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    serial: String,
) -> Result<OperationResult, String> {
    state.switch_device(&app, serial).await
}

#[tauri::command]
async fn disconnect(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<OperationResult, String> {
    state.disconnect(&app).await
}

#[tauri::command]
fn select_device(state: State<'_, AppState>, serial: String) -> Result<AppSnapshot, String> {
    state.select_device(&serial)
}

#[tauri::command]
async fn set_output(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    output: String,
) -> Result<AppSnapshot, String> {
    state.set_output(&app, &output).await
}

#[tauri::command]
async fn cancel_operation(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    Ok(state.cancel_operation(&app).await)
}

#[tauri::command]
fn update_setting(
    state: State<'_, AppState>,
    key: String,
    value: serde_json::Value,
) -> Result<AppSnapshot, String> {
    state.update_setting(&key, value)?;
    Ok(state.snapshot())
}

#[tauri::command]
async fn connect_wifi(
    state: State<'_, AppState>,
    endpoint: String,
) -> Result<OperationResult, String> {
    state.connect_wifi(&endpoint).await
}

#[tauri::command]
async fn pair_wifi(
    state: State<'_, AppState>,
    endpoint: String,
    code: String,
) -> Result<OperationResult, String> {
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
async fn pair_qr(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    instance_name: String,
    password: String,
) -> Result<OperationResult, String> {
    state.pair_qr(&app, instance_name, password).await
}

#[tauri::command]
async fn receiver_status(
    state: State<'_, AppState>,
    serial: String,
) -> Result<receiver::ReceiverStatus, String> {
    state.receiver_status(&serial).await
}

#[tauri::command]
async fn receiver_install(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    serial: String,
    operation_id: String,
) -> Result<receiver::ReceiverStatus, String> {
    state.receiver_install(&app, &serial, &operation_id).await
}

#[tauri::command]
async fn receiver_upgrade_usb(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    serial: String,
    binding_reference: String,
    operation_id: String,
) -> Result<receiver::ReceiverStatus, String> {
    state
        .receiver_upgrade_usb(&app, &serial, &binding_reference, &operation_id)
        .await
}

#[tauri::command]
async fn receiver_start(
    state: State<'_, AppState>,
    serial: String,
) -> Result<receiver::ReceiverStatus, String> {
    state.receiver_start(&serial).await
}

#[tauri::command]
async fn receiver_health(
    state: State<'_, AppState>,
    serial: String,
) -> Result<receiver::ReceiverStatus, String> {
    state.receiver_health(&serial).await
}

#[tauri::command]
async fn receiver_stop(
    state: State<'_, AppState>,
    serial: String,
) -> Result<receiver::ReceiverStatus, String> {
    state.receiver_stop(&serial).await
}

#[tauri::command]
async fn receiver_update_candidate(
    state: State<'_, AppState>,
) -> Result<Option<state::ReceiverUpdateCandidate>, String> {
    state.receiver_update_candidate().await
}

#[tauri::command]
fn network_interfaces(
    state: State<'_, AppState>,
) -> Result<Vec<network_interface::Ipv4Interface>, String> {
    state.network_interfaces()
}

#[tauri::command]
async fn receiver_network_interfaces(
    state: State<'_, AppState>,
    serial: String,
) -> Result<Vec<receiver::ReceiverInterface>, String> {
    state.receiver_network_interfaces(&serial).await
}

#[tauri::command]
fn receiver_binding_references(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    state.receiver_binding_references()
}

#[tauri::command]
async fn receiver_binding_reset_preview(
    state: State<'_, AppState>,
    serial: String,
) -> Result<receiver::BindingResetPreview, String> {
    state.receiver_binding_reset_preview(&serial).await
}

#[tauri::command]
async fn receiver_reset_binding(
    state: State<'_, AppState>,
    preview: receiver::BindingResetPreview,
) -> Result<receiver::BindingResetResult, String> {
    state.receiver_reset_binding(preview).await
}

#[tauri::command]
async fn discover_network_receivers(
    state: State<'_, AppState>,
    local_address: String,
) -> Result<Vec<discovery::Candidate>, String> {
    state.discover_network_receivers(&local_address).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn start_network_audio(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    local_address: String,
    peer: String,
    bootstrap_serial: Option<String>,
    receiver_address: Option<String>,
    receiver_prefix_length: Option<u8>,
    binding_reference: Option<String>,
    background_authorization_required: Option<bool>,
) -> Result<AppSnapshot, String> {
    state
        .start_network_audio(
            &app,
            &local_address,
            &peer,
            bootstrap_serial.as_deref(),
            receiver_address.as_deref(),
            receiver_prefix_length,
            binding_reference.as_deref(),
            background_authorization_required.unwrap_or(false),
        )
        .await
}

#[tauri::command]
async fn query_background_trust(
    state: State<'_, AppState>,
    target: state::BackgroundTrustTarget,
) -> Result<state::BackgroundTrustStatus, String> {
    state.query_background_trust(target).await
}

#[tauri::command]
async fn revoke_background_trust(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    target: state::BackgroundTrustTarget,
) -> Result<state::BackgroundTrustStatus, String> {
    state.revoke_background_trust(&app, target).await
}

#[tauri::command]
fn set_wireless_connection_offers(
    state: State<'_, AppState>,
    local_address: String,
    allowed: bool,
    binding_reference: Option<String>,
) -> Result<discovery::HostDiscoverySnapshot, String> {
    state.set_wireless_connection_offers(&local_address, allowed, binding_reference.as_deref())
}

#[tauri::command]
async fn accept_wireless_connection_offer(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    offer_id: String,
    binding_reference: String,
) -> Result<AppSnapshot, String> {
    state
        .accept_wireless_connection_offer(&app, &offer_id, &binding_reference)
        .await
}

#[tauri::command]
fn cancel_qr_pairing(state: State<'_, AppState>) {
    state.cancel_qr_pairing();
}

#[tauri::command]
async fn reconfigure_session(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    kind: String,
    value: String,
) -> Result<AppSnapshot, String> {
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
fn export_diagnostics(state: State<'_, AppState>) -> Result<String, String> {
    state.export_diagnostics()
}

#[tauri::command]
async fn close_to_tray(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
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
        fn CreateEventW(
            attributes: *const c_void,
            manual_reset: i32,
            initial_state: i32,
            name: *const u16,
        ) -> isize;
        fn SetEvent(handle: isize) -> i32;
        fn GetLastError() -> u32;
    }
    let mutex_name: Vec<u16> = "Local\\AnPod.SingleInstance\0".encode_utf16().collect();
    let show_name: Vec<u16> = "Local\\AnPod.ShowWindow\0".encode_utf16().collect();
    unsafe {
        let mutex = CreateMutexW(ptr::null(), 1, mutex_name.as_ptr());
        if mutex == 0 {
            return Err("无法创建 AnPod 单实例互斥锁。".into());
        }
        let exists = GetLastError() == ERROR_ALREADY_EXISTS;
        INSTANCE_MUTEX.store(mutex, std::sync::atomic::Ordering::Relaxed);
        let event = CreateEventW(ptr::null(), 0, 0, show_name.as_ptr());
        if event == 0 {
            return Err("无法创建 AnPod 显示窗口事件。".into());
        }
        SHOW_EVENT.store(event, std::sync::atomic::Ordering::Relaxed);
        if exists {
            let _ = SetEvent(event);
        }
        Ok(!exists)
    }
}

#[cfg(not(windows))]
fn claim_single_instance() -> Result<bool, String> {
    Ok(true)
}

#[cfg(not(test))]
fn main() {
    match claim_single_instance() {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            eprintln!("{error}");
            return;
        }
    }
    tauri::Builder::default()
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            refresh_devices,
            list_aoa_targets,
            start_aoa_audio,
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
            receiver_upgrade_usb,
            receiver_start,
            receiver_health,
            receiver_stop,
            receiver_update_candidate,
            network_interfaces,
            receiver_network_interfaces,
            receiver_binding_references,
            receiver_binding_reset_preview,
            receiver_reset_binding,
            discover_network_receivers,
            start_network_audio,
            query_background_trust,
            revoke_background_trust,
            set_wireless_connection_offers,
            accept_wireless_connection_offer,
            cancel_qr_pairing,
            reconfigure_session,
            show_main_window,
            open_logs,
            export_diagnostics,
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
                    } else {
                        break;
                    }
                }
            });
            let rate_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    if let Some(state) = rate_app.try_state::<AppState>() {
                        state.sample_audio_bitrate(&rate_app).await;
                    } else {
                        break;
                    }
                }
            });
            #[cfg(windows)]
            {
                let handle = SHOW_EVENT.load(std::sync::atomic::Ordering::Relaxed);
                let app_handle = app.handle().clone();
                std::thread::Builder::new()
                    .name("AnPod show window".into())
                    .spawn(move || unsafe {
                        #[link(name = "kernel32")]
                        unsafe extern "system" {
                            fn WaitForSingleObject(handle: isize, milliseconds: u32) -> u32;
                        }
                        while WaitForSingleObject(handle, 0xFFFF_FFFF) == 0 {
                            if let Some(window) = app_handle.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    })?;
            }
            let show =
                tauri::menu::MenuItem::with_id(app, "show", "打开 AnPod", true, None::<&str>)?;
            let disconnect =
                tauri::menu::MenuItem::with_id(app, "disconnect", "断开连接", true, None::<&str>)?;
            let quit = tauri::menu::MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = tauri::menu::Menu::with_items(app, &[&show, &disconnect, &quit])?;
            let _tray = tauri::tray::TrayIconBuilder::new()
                .icon(
                    app.default_window_icon()
                        .expect("Tauri window icon is configured")
                        .clone(),
                )
                .menu(&menu)
                .tooltip("AnPod")
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "disconnect" => {
                        let app = (*app).clone();
                        tauri::async_runtime::spawn(async move {
                            if let Some(state) = app.try_state::<AppState>() {
                                let _ = state.disconnect(&app).await;
                            }
                        });
                    }
                    "quit" => {
                        let app = (*app).clone();
                        tauri::async_runtime::spawn(async move {
                            if let Some(state) = app.try_state::<AppState>() {
                                let _ = state.disconnect(&app).await;
                            }
                            app.exit(0);
                        });
                    }
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
                    let Some(state) = app.try_state::<AppState>() else {
                        return;
                    };
                    if state.snapshot().settings.minimize_to_tray {
                        if let Some(window) = app.get_webview_window(&label) {
                            let _ = window.hide();
                        }
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
