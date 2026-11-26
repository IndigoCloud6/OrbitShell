use std::{
    ffi::{OsStr, OsString},
    hash::{DefaultHasher, Hash, Hasher},
    os::windows::ffi::{OsStrExt, OsStringExt},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    time::{Duration, Instant},
};

use gpui::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tokio::sync::mpsc;
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS, HWND, LPARAM,
        LRESULT, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT, WPARAM,
    },
    System::{
        DataExchange::COPYDATASTRUCT,
        Registry::{
            HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
            RegCloseKey, RegCreateKeyExW, RegOpenKeyExW, RegSetValueExW,
        },
        Threading::{
            CreateMutexW, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
            PROCESS_SYNCHRONIZE, ReleaseMutex, WaitForSingleObject,
        },
    },
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            AllowSetForegroundWindow, EnumWindows, GetPropW, GetWindowThreadProcessId, IsIconic,
            IDYES, MB_ICONWARNING, MB_OK, MB_YESNO, MessageBoxW,
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
pub const EXIT_FOR_UPDATE: &str = "--exit-for-update";
pub const UPDATE_EXIT_CANCELLED: &str = "update-exit-cancelled";
const UPDATE_EXIT_TIMEOUT_MS: u32 = 120_000;
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
    wait_for_exit: bool,
    found: bool,
    delivered: bool,
    cancelled: bool,
    wait_failed: bool,
}

pub struct InstanceGuard(OwnedHandle);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe { ReleaseMutex(self.0.as_raw_handle()) };
    }
}

/// Only the mutex owner starts GPUI. Contenders may arrive before its HWND is registered.
pub fn start_or_forward(arguments: &[String]) -> Result<Option<InstanceGuard>, &'static str> {
    let wait_for_exit = arguments.iter().any(|argument| argument == EXIT_FOR_UPDATE);
    let (app_path, paths) = app_paths().ok_or("无法确定 OrbitShell 的安装位置。")?;
    let mut identity = DefaultHasher::new();
    app_path
        .canonicalize()
        .map_err(|_| "无法确定 OrbitShell 的安装位置。")?
        .hash(&mut identity);
    let name = wide(format!(r"Local\OrbitShell.GPUI.{:016x}", identity.finish()));
    let raw = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
    if raw.is_null() {
        return Err("无法打开 OrbitShell 单实例锁。");
    }
    let mutex = unsafe { OwnedHandle::from_raw_handle(raw) };
    let packet = Zeroizing::new(serde_json::to_vec(arguments).map_err(|_| "无法读取启动参数。")?);
    if packet.len() > MAX_PACKET {
        return Err("启动参数过长。");
    }
    let mut search = Search {
        paths,
        property: wide(PROPERTY),
        packet: &packet,
        wait_for_exit,
        found: false,
        delivered: false,
        cancelled: false,
        wait_failed: false,
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut wait_ms = 0;
    loop {
        match unsafe { WaitForSingleObject(mutex.as_raw_handle(), wait_ms) } {
            WAIT_OBJECT_0 | WAIT_ABANDONED => return Ok(Some(InstanceGuard(mutex))),
            WAIT_TIMEOUT => {}
            _ => return Err("无法获取 OrbitShell 单实例锁。"),
        }
        // EnumWindows calls us synchronously. After any delivery attempt, the
        // request must not be replayed: a timed-out receiver may still handle it.
        unsafe {
            EnumWindows(
                Some(forward_to_window),
                (&mut search as *mut Search<'_>) as LPARAM,
            );
        }
        if search.cancelled {
            return Err(UPDATE_EXIT_CANCELLED);
        }
        if search.delivered {
            if search.wait_failed {
                return Err("OrbitShell 尚未退出。请完成保存后重试安装。");
            }
            return Ok(None);
        }
        if search.found || Instant::now() >= deadline {
            return Err(if wait_for_exit {
                "无法联系正在运行的 OrbitShell。请手动退出后重试安装。"
            } else {
                "已有 OrbitShell 实例，未能确认启动请求已送达。请查看已有窗口。"
            });
        }
        wait_ms = 75;
    }
}

pub fn show_startup_error(message: &str) {
    unsafe {
        MessageBoxW(
            null_mut(),
            wide(message).as_ptr(),
            wide("OrbitShell").as_ptr(),
            MB_OK | MB_ICONWARNING,
        );
    }
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
    let access = PROCESS_QUERY_LIMITED_INFORMATION
        | if search.wait_for_exit {
            PROCESS_SYNCHRONIZE
        } else {
            0
        };
    let process = unsafe { OpenProcess(access, 0, pid) };
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
        search.found = true;
        if search.wait_for_exit {
            let answer = unsafe {
                MessageBoxW(
                    null_mut(),
                    wide("继续安装会退出 OrbitShell 并断开当前 SSH 会话。是否继续？").as_ptr(),
                    wide("OrbitShell 安装程序").as_ptr(),
                    MB_YESNO | MB_ICONWARNING,
                )
            };
            if answer != IDYES {
                search.cancelled = true;
                unsafe { CloseHandle(process) };
                return 0;
            }
        }
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
        if search.delivered && search.wait_for_exit {
            search.wait_failed = unsafe {
                WaitForSingleObject(process, UPDATE_EXIT_TIMEOUT_MS)
            } != WAIT_OBJECT_0;
        }
    }
    unsafe {
        CloseHandle(process);
    }
    i32::from(!search.found)
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
