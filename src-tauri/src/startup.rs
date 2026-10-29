const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "AnPod";

pub fn set_launch_at_login(enabled: bool) -> Result<(), String> {
    let command = if enabled {
        quoted_executable(&std::env::current_exe().map_err(|error| error.to_string())?)
    } else {
        String::new()
    };
    sync_run_value(enabled, &command, set_run_value, delete_run_value)
}

pub fn reconcile_launch_at_login_on_startup(enabled: bool) -> Result<(), String> {
    reconcile_launch_at_login(enabled, std::env::current_exe, set_run_value)
}

fn reconcile_launch_at_login(
    enabled: bool,
    current_exe: impl FnOnce() -> Result<std::path::PathBuf, std::io::Error>,
    set: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(), String> {
    if !enabled {
        return Ok(());
    }
    let exe = current_exe().map_err(|error| error.to_string())?;
    set(&quoted_executable(&exe))
}

fn quoted_executable(exe: &std::path::Path) -> String {
    format!("\"{}\"", exe.display())
}

pub fn apply_launch_at_login(
    apply_os: impl FnOnce() -> Result<(), String>,
    save: impl FnOnce() -> Result<(), String>,
    rollback_os: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    apply_os()?;
    if let Err(error) = save() {
        if let Err(rollback_error) = rollback_os() {
            return Err(format!("{error}；恢复登录启动项失败：{rollback_error}"));
        }
        return Err(error);
    }
    Ok(())
}

fn sync_run_value(
    enabled: bool,
    command: &str,
    set: impl FnOnce(&str) -> Result<(), String>,
    delete: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if enabled {
        set(command)
    } else {
        delete()
    }
}

#[cfg(windows)]
fn set_run_value(command: &str) -> Result<(), String> {
    use windows::{
        core::PCWSTR,
        Win32::System::Registry::{
            RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
            REG_OPTION_NON_VOLATILE, REG_SZ,
        },
    };

    let key_path: Vec<u16> = format!("{RUN_KEY}\0").encode_utf16().collect();
    let value_name: Vec<u16> = format!("{RUN_VALUE}\0").encode_utf16().collect();
    let command: Vec<u16> = format!("{command}\0").encode_utf16().collect();
    let mut key = HKEY::default();
    unsafe {
        let error = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key_path.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        );
        if error.0 != 0 {
            return Err(format!("Windows registry error {}", error.0));
        }
        let error = RegSetValueExW(
            key,
            PCWSTR(value_name.as_ptr()),
            None,
            REG_SZ,
            Some(std::slice::from_raw_parts(
                command.as_ptr().cast(),
                command.len() * std::mem::size_of::<u16>(),
            )),
        );
        let _ = RegCloseKey(key);
        if error.0 == 0 {
            Ok(())
        } else {
            Err(format!("Windows registry error {}", error.0))
        }
    }
}

#[cfg(windows)]
fn delete_run_value() -> Result<(), String> {
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::ERROR_FILE_NOT_FOUND,
            System::Registry::{
                RegCloseKey, RegCreateKeyExW, RegDeleteValueW, HKEY, HKEY_CURRENT_USER,
                KEY_SET_VALUE, REG_OPTION_NON_VOLATILE,
            },
        },
    };

    let key_path: Vec<u16> = format!("{RUN_KEY}\0").encode_utf16().collect();
    let value_name: Vec<u16> = format!("{RUN_VALUE}\0").encode_utf16().collect();
    let mut key = HKEY::default();
    unsafe {
        let error = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key_path.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        );
        if error.0 != 0 {
            return Err(format!("Windows registry error {}", error.0));
        }
        let error = RegDeleteValueW(key, PCWSTR(value_name.as_ptr()));
        let result = if error.0 == 0 || error == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(format!("Windows registry error {}", error.0))
        };
        let _ = RegCloseKey(key);
        result
    }
}

#[cfg(not(windows))]
fn set_run_value(_: &str) -> Result<(), String> {
    Err("登录启动仅支持 Windows。".into())
}

#[cfg(not(windows))]
fn delete_run_value() -> Result<(), String> {
    Err("登录启动仅支持 Windows。".into())
}

#[cfg(test)]
mod tests {
    use super::{
        apply_launch_at_login, quoted_executable, reconcile_launch_at_login, sync_run_value,
    };
    use std::path::Path;

    #[test]
    fn launch_command_quotes_executable_path_and_propagates_os_failure() {
        assert_eq!(
            quoted_executable(Path::new(r"C:\Program Files\AnPod\AnPod.exe")),
            r#""C:\Program Files\AnPod\AnPod.exe""#
        );
        let mut registered = None;
        sync_run_value(
            true,
            &quoted_executable(Path::new(r"C:\Program Files\AnPod\AnPod.exe")),
            |command| {
                registered = Some(command.to_owned());
                Ok(())
            },
            || panic!("enabled launch must set the value"),
        )
        .unwrap();
        assert_eq!(
            registered.as_deref(),
            Some(r#""C:\Program Files\AnPod\AnPod.exe""#)
        );
        assert_eq!(
            sync_run_value(true, "exe", |_| Err("registry denied".into()), || Ok(())).unwrap_err(),
            "registry denied"
        );
    }

    #[test]
    fn disabling_launch_removes_only_the_named_value_and_propagates_failure() {
        let mut deleted = false;
        sync_run_value(
            false,
            "unused",
            |_| panic!("disabled launch must not set the value"),
            || {
                deleted = true;
                Ok(())
            },
        )
        .unwrap();
        assert!(deleted);
        assert_eq!(
            sync_run_value(
                false,
                "unused",
                |_| Ok(()),
                || Err("registry denied".into())
            )
            .unwrap_err(),
            "registry denied"
        );
    }

    #[test]
    fn os_failure_does_not_save_and_save_failure_restores_os_state() {
        let mut saved = false;
        assert_eq!(
            apply_launch_at_login(
                || Err("registry denied".into()),
                || {
                    saved = true;
                    Ok(())
                },
                || panic!("OS change was not applied"),
            )
            .unwrap_err(),
            "registry denied"
        );
        assert!(!saved);

        let mut restored = false;
        assert_eq!(
            apply_launch_at_login(
                || Ok(()),
                || Err("settings write failed".into()),
                || {
                    restored = true;
                    Ok(())
                },
            )
            .unwrap_err(),
            "settings write failed"
        );
        assert!(restored);
    }

    #[test]
    fn startup_reconcile_updates_old_path_and_skips_disabled_settings() {
        let mut registered = r#""C:\Old folder\AnPod.exe""#.to_owned();
        reconcile_launch_at_login(
            true,
            || Ok(Path::new(r"D:\Portable\AnPod.exe").to_owned()),
            |command| {
                registered = command.to_owned();
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(registered, r#""D:\Portable\AnPod.exe""#);

        reconcile_launch_at_login(
            false,
            || panic!("disabled startup must not resolve the executable"),
            |_| panic!("disabled startup must not touch the OS"),
        )
        .unwrap();
        assert_eq!(
            reconcile_launch_at_login(
                true,
                || Ok(Path::new(r"D:\Portable\AnPod.exe").to_owned()),
                |_| Err("registry denied".into())
            )
            .unwrap_err(),
            "registry denied"
        );
    }
}
