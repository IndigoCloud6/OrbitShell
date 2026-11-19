use std::{
    ffi::{OsStr, OsString},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    ptr::{null, null_mut},
};

use gpui::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tokio::sync::mpsc;
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS, HWND, LPARAM,
        LRESULT, WPARAM,
    },
    System::{
        DataExchange::COPYDATASTRUCT,
        Registry::{
            HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
            RegCloseKey, RegCreateKeyExW, RegOpenKeyExW, RegSetValueExW,
        },
        Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW},
    },
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            AllowSetForegroundWindow, EnumWindows, GetPropW, GetWindowThreadProcessId, IsIconic,
            RemovePropW, SMTO_ABORTIFHUNG, SMTO_BLOCK, SW_RESTORE, SendMessageTimeoutW,
            SetForegroundWindow, SetPropW, ShowWindow, WM_COPYDATA, WM_NCDESTROY,
        },
    },
};
use zeroize::Zeroizing;

const PROPERTY: &str = "OrbitShell.GPUI.ExternalLaunch.v1";
const SUBCLASS: usize = 2;
const COPYDATA_ID: usize = 0x4f524231;
const MAX_PACKET: usize = 256 * 1024;
pub type Arguments = Zeroizing<Vec<String>>;

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain([0]).collect()
}

fn dev_executable() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/orbitshell-gpui-terminal.exe")
}

fn dev_slots() -> [PathBuf; 2] {
    let prototype = Path::new(env!("CARGO_MANIFEST_DIR"));
    ["a", "b"].map(|slot| {
        prototype
            .join("../../work/gpui-dev")
            .join(slot)
            .join("orbitshell-gpui-terminal.exe")
    })
}

fn app_paths() -> Option<(PathBuf, Vec<PathBuf>)> {
    let current = std::env::current_exe().ok()?;
    let canonical = current.canonicalize().ok()?;
    let dev = dev_executable();
    let mut allowed: Vec<_> = std::iter::once(dev.clone())
        .chain(dev_slots())
        .filter_map(|path| path.canonicalize().ok())
        .collect();
    if allowed.contains(&canonical) && dev.is_file() {
        Some((dev, allowed))
    } else {
        allowed.clear();
        allowed.push(canonical);
        Some((current, allowed))
    }
}

struct Search<'a> {
    paths: Vec<PathBuf>,
    property: Vec<u16>,
    packet: &'a [u8],
    delivered: bool,
}

pub fn forward(arguments: &Arguments) -> bool {
    let Some((_, paths)) = app_paths() else {
        return false;
    };
    let Ok(packet) = serde_json::to_vec(&**arguments).map(Zeroizing::new) else {
        return false;
    };
    if packet.len() > MAX_PACKET {
        return false;
    }
    let mut search = Search {
        paths,
        property: wide(PROPERTY),
        packet: &packet,
        delivered: false,
    };
    // EnumWindows invokes the callback synchronously on this desktop.
    unsafe {
        EnumWindows(
            Some(forward_to_window),
            (&mut search as *mut Search<'_>) as LPARAM,
        );
    }
    search.delivered
}

unsafe extern "system" fn forward_to_window(hwnd: HWND, context: LPARAM) -> i32 {
    let search = unsafe { &mut *(context as *mut Search<'_>) };
    if unsafe { GetPropW(hwnd, search.property.as_ptr()) }.is_null() {
        return 1;
    }
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
    }
    if pid == 0 {
        return 1;
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return 1;
    }
    let mut image = vec![0u16; 32768];
    let mut len = image.len() as u32;
    let found =
        unsafe { QueryFullProcessImageNameW(process, 0, image.as_mut_ptr(), &mut len) } != 0;
    let path = found.then(|| PathBuf::from(OsString::from_wide(&image[..len as usize])));
    if path
        .and_then(|path| path.canonicalize().ok())
        .is_some_and(|path| search.paths.contains(&path))
    {
        let data = COPYDATASTRUCT {
            dwData: COPYDATA_ID,
            cbData: search.packet.len() as u32,
            lpData: search.packet.as_ptr().cast_mut().cast(),
        };
        let mut accepted = 0;
        unsafe {
            AllowSetForegroundWindow(pid);
            let sent = SendMessageTimeoutW(
                hwnd,
                WM_COPYDATA,
                0,
                (&data as *const COPYDATASTRUCT) as LPARAM,
                SMTO_ABORTIFHUNG | SMTO_BLOCK,
                1500,
                &mut accepted,
            );
            search.delivered = sent != 0 && accepted == COPYDATA_ID;
        }
    }
    unsafe {
        CloseHandle(process);
    }
    i32::from(!search.delivered)
}

pub fn register_window(window: &Window, sender: mpsc::Sender<Arguments>) -> bool {
    let Some(handle) = HasWindowHandle::window_handle(window).ok() else {
        return false;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return false;
    };
    let hwnd = handle.hwnd.get() as HWND;
    let state = Box::into_raw(Box::new(sender));
    unsafe {
        if SetWindowSubclass(hwnd, Some(window_proc), SUBCLASS, state as usize) == 0 {
            drop(Box::from_raw(state));
            return false;
        }
        if SetPropW(hwnd, wide(PROPERTY).as_ptr(), std::ptr::dangling_mut()) == 0 {
            RemoveWindowSubclass(hwnd, Some(window_proc), SUBCLASS);
            drop(Box::from_raw(state));
            return false;
        }
    }
    true
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    state: usize,
) -> LRESULT {
    let sender = state as *mut mpsc::Sender<Arguments>;
    match msg {
        WM_COPYDATA if lparam != 0 => {
            let data = unsafe { &*(lparam as *const COPYDATASTRUCT) };
            if data.dwData == COPYDATA_ID {
                if data.lpData.is_null() || !(1..=MAX_PACKET).contains(&(data.cbData as usize)) {
                    return 0;
                }
                let bytes = unsafe {
                    std::slice::from_raw_parts(data.lpData.cast::<u8>(), data.cbData as usize)
                };
                let Ok(args) = serde_json::from_slice::<Vec<String>>(bytes).map(Zeroizing::new)
                else {
                    return 0;
                };
                if unsafe { &*sender }.try_send(args).is_err() {
                    return 0;
                }
                unsafe {
                    if IsIconic(hwnd) != 0 {
                        ShowWindow(hwnd, SW_RESTORE);
                    }
                    SetForegroundWindow(hwnd);
                }
                return COPYDATA_ID as LRESULT;
            }
        }
        WM_NCDESTROY => unsafe {
            RemovePropW(hwnd, wide(PROPERTY).as_ptr());
            RemoveWindowSubclass(hwnd, Some(window_proc), id);
            drop(Box::from_raw(sender));
        },
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

const CAPABILITIES: &str = r"Software\OrbitShell\GPUI\Capabilities";

pub fn register_protocols() -> Result<(), String> {
    let (executable, _) = app_paths().ok_or("无法定位 GPUI 可执行文件")?;
    let command = format!("\"{}\" \"%1\"", executable.display());
    for (scheme, prog_id) in [
        ("ssh", "OrbitShell.GPUI.SSH"),
        ("orbitshell", "OrbitShell.GPUI.Link"),
    ] {
        write_protocol(prog_id, &command)?;
        write_values(
            &format!(r"{CAPABILITIES}\URLAssociations"),
            &[(scheme, prog_id)],
        )?;
        if !class_exists(scheme)? {
            write_protocol(scheme, &command)?;
        }
    }
    write_values(
        CAPABILITIES,
        &[
            ("ApplicationName", "OrbitShell GPUI"),
            ("ApplicationDescription", "OrbitShell SSH / SFTP 工作台"),
        ],
    )?;
    write_values(
        r"Software\RegisteredApplications",
        &[("OrbitShell GPUI", CAPABILITIES)],
    )
}

fn write_protocol(class: &str, command: &str) -> Result<(), String> {
    let key = format!(r"Software\Classes\{class}");
    write_values(
        &key,
        &[("", "OrbitShell GPUI 会话链接"), ("URL Protocol", "")],
    )?;
    write_values(&format!(r"{key}\shell\open\command"), &[("", command)])
}

fn class_exists(class: &str) -> Result<bool, String> {
    let mut key = null_mut();
    let result = unsafe {
        RegOpenKeyExW(
            HKEY_CLASSES_ROOT,
            wide(class).as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        )
    };
    match result {
        ERROR_SUCCESS => {
            unsafe {
                RegCloseKey(key);
            }
            Ok(true)
        }
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(false),
        code => Err(format!("无法读取系统链接关联（{code}）")),
    }
}

fn write_values(path: &str, values: &[(&str, &str)]) -> Result<(), String> {
    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
    let mut key = null_mut();
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            wide(path).as_ptr(),
            0,
            null(),
            0,
            KEY_SET_VALUE,
            null(),
            &mut key,
            null_mut(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!("无法注册系统链接入口（{status}）"));
    }
    let key = Key(key);
    for (name, value) in values {
        let data = wide(value);
        let status = unsafe {
            RegSetValueExW(
                key.0,
                wide(name).as_ptr(),
                0,
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(format!("无法写入系统链接入口（{status}）"));
        }
    }
    Ok(())
}
