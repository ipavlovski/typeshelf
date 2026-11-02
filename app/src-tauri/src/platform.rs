//! OS integration: session-end hook, font-caching app detection, reveal in file manager.

#[cfg(not(windows))]
use anyhow::Result;

/// Apps that read the font list once at start-up and need a restart to see changes.
#[cfg_attr(not(windows), allow(dead_code))]
const CACHING_APPS: &[(&str, &str)] = &[
    ("photoshop.exe", "Photoshop"),
    ("illustrator.exe", "Illustrator"),
    ("indesign.exe", "InDesign"),
    ("afterfx.exe", "After Effects"),
    ("figma.exe", "Figma"),
    ("inkscape.exe", "Inkscape"),
    ("designer.exe", "Affinity Designer"),
    ("photo.exe", "Affinity Photo"),
];

#[cfg(windows)]
pub use windows::*;

#[cfg(not(windows))]
pub fn font_caching_apps() -> Vec<String> {
    Vec::new()
}

#[cfg(not(windows))]
pub fn install_session_end_hook(_app: tauri::AppHandle) {}

#[cfg(not(windows))]
pub fn enable_logon_reconcile() -> Result<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn reveal(path: &str) -> Result<()> {
    std::process::Command::new("open").args(["-R", path]).spawn()?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn reveal(path: &str) -> Result<()> {
    let dir = std::path::Path::new(path).parent().unwrap_or(std::path::Path::new("/"));
    std::process::Command::new("xdg-open").arg(dir).spawn()?;
    Ok(())
}

#[cfg(windows)]
mod windows {
    use std::sync::OnceLock;

    use anyhow::Result;
    use tauri::{AppHandle, Manager};
    use windows_sys::Win32::Foundation::{CloseHandle, HWND, INVALID_HANDLE_VALUE, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG, RegisterClassW, TranslateMessage,
        WM_ENDSESSION, WM_QUERYENDSESSION, WNDCLASSW, WS_OVERLAPPED,
    };

    use super::CACHING_APPS;
    use crate::state::AppState;

    pub fn font_caching_apps() -> Vec<String> {
        let mut found = Vec::new();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                return found;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
            let mut ok = Process32FirstW(snap, &mut entry) != 0;
            while ok {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                let exe = String::from_utf16_lossy(&entry.szExeFile[..len]).to_lowercase();
                if let Some((_, name)) = CACHING_APPS.iter().find(|(e, _)| *e == exe)
                    && !found.iter().any(|f| f == name)
                {
                    found.push((*name).to_owned());
                }
                ok = Process32NextW(snap, &mut entry) != 0;
            }
            CloseHandle(snap);
        }
        found
    }

    pub fn reveal(path: &str) -> Result<()> {
        std::process::Command::new("explorer.exe").arg(format!("/select,{path}")).spawn()?;
        Ok(())
    }

    pub fn enable_logon_reconcile() -> Result<()> {
        let exe = std::env::current_exe()?;
        ts_activate::windows::enable_logon_reconcile(&exe, "--reconcile-logon")
    }

    static APP: OnceLock<AppHandle> = OnceLock::new();

    /// Windows sends WM_QUERYENDSESSION/WM_ENDSESSION only to top-level windows
    /// (not message-only ones), so a hidden one listens for logoff and shutdown and
    /// removes session fonts. Best effort: the logon reconcile is the guarantee.
    pub fn install_session_end_hook(app: AppHandle) {
        let _ = APP.set(app);
        std::thread::spawn(|| unsafe {
            let class: Vec<u16> = "typeshelf-session\0".encode_utf16().collect();
            let instance = GetModuleHandleW(std::ptr::null());
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return;
            }
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            WM_QUERYENDSESSION => 1,
            WM_ENDSESSION => {
                if wparam != 0
                    && let Some(app) = APP.get()
                {
                    let _ = app.state::<AppState>().deactivate_session();
                }
                0
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }
}
