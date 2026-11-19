use crate::{
    session::LiveSession,
    transfer::{self, TransferProgress},
};
use std::{
    mem::{size_of, ManuallyDrop},
    os::windows::ffi::OsStrExt,
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;
use windows::{
    core::{implement, w, Error, IUnknown, Ref, Result, BOOL, HRESULT, PCWSTR},
    Win32::{
        Foundation::{
            CloseHandle, GlobalFree, COLORREF, DATA_S_SAMEFORMATETC, DRAGDROP_S_CANCEL,
            DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, DV_E_FORMATETC, E_ABORT, E_FAIL,
            E_NOTIMPL, E_OUTOFMEMORY, E_POINTER, HANDLE, OLE_E_ADVISENOTSUPPORTED, POINT,
            RPC_S_CALLPENDING, SIZE, S_OK,
        },
        Graphics::Gdi::{
            CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS,
        },
        Storage::FileSystem::FILE_ATTRIBUTE_NORMAL,
        System::{
            Com::{
                CoCreateInstance, CoWaitForMultipleHandles, IAdviseSink, IDataObject,
                IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA, IStream, CLSCTX_INPROC_SERVER,
                COWAIT_DISPATCH_CALLS, COWAIT_DISPATCH_WINDOW_MESSAGES, COWAIT_INPUTAVAILABLE,
                DATADIR_GET, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0, STGM_READ,
                STGM_SHARE_DENY_WRITE, TYMED_HGLOBAL, TYMED_ISTREAM,
            },
            DataExchange::RegisterClipboardFormatW,
            Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
            Ole::{
                DoDragDrop, IDropSource, IDropSource_Impl, OleInitialize, OleUninitialize,
                DROPEFFECT, DROPEFFECT_COPY,
            },
            SystemServices::MODIFIERKEYS_FLAGS,
            Threading::{CreateEventW, SetEvent},
        },
        UI::{
            Input::KeyboardAndMouse::{GetAsyncKeyState, ReleaseCapture, VK_ESCAPE, VK_LBUTTON},
            Shell::{
                CLSID_DragDropHelper, IDragSourceHelper, SHCreateDataObject, SHCreateStdEnumFmtEtc,
                SHCreateStreamOnFileEx, FD_ATTRIBUTES, FD_PROGRESSUI, FILEDESCRIPTORW,
                FILEGROUPDESCRIPTORW, SHDRAGIMAGE,
            },
        },
    },
};

static DRAG_ACTIVE: AtomicBool = AtomicBool::new(false);
pub struct DragLease;
impl DragLease {
    pub fn acquire() -> std::result::Result<Self, String> {
        DRAG_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self)
            .map_err(|_| "已有系统拖放正在进行，请先松开鼠标或按 Esc 取消。".into())
    }
}
impl Drop for DragLease {
    fn drop(&mut self) {
        DRAG_ACTIVE.store(false, Ordering::Release);
    }
}
fn left_button_down() -> bool {
    unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0 }
}
fn escape_down() -> bool {
    unsafe { GetAsyncKeyState(VK_ESCAPE.0 as i32) < 0 }
}

struct Completion(HANDLE);
// Kernel event handles can be signalled from any thread. The Arc<Job> owns
// the handle until both the UI waiter and the download worker release it.
unsafe impl Send for Completion {}
unsafe impl Sync for Completion {}
impl Drop for Completion {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
struct Job {
    completion: Completion,
    result: Mutex<Option<std::result::Result<u64, String>>>,
}

pub fn filename(path: &str) -> std::result::Result<String, String> {
    let name = path
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty() && !matches!(*name, "." | ".."))
        .ok_or("远程文件路径无效")?;
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if !path.starts_with('/')
        || name.encode_utf16().count() >= 260
        || name.ends_with(['.', ' '])
        || name.chars().any(|c| {
            c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
        })
        || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
    {
        return Err("此远程文件名不能直接保存到 Windows，请使用下载按钮指定合法文件名。".into());
    }
    Ok(name.into())
}
pub struct DragOptions {
    pub runtime: Handle,
    pub scale: f64,
    pub cancel: CancellationToken,
    pub progress: Box<dyn Fn(TransferProgress) + Send + Sync>,
    pub should_stop: Box<dyn Fn() -> bool + Send + Sync>,
}

struct DragState {
    options: DragOptions,
    live: Arc<LiveSession>,
    remote: String,
    name: String,
    folder: tempfile::TempDir,
    cancel: CancellationToken,
    cached: Mutex<Option<Arc<Job>>>,
}
impl DragState {
    fn contents(self: &Arc<Self>) -> Result<IStream> {
        let local = self.folder.path().join(&self.name);
        let job = {
            let mut cached = self
                .cached
                .lock()
                .map_err(|_| Error::from_hresult(E_FAIL))?;
            if let Some(job) = cached.as_ref() {
                job.clone()
            } else {
                let completion =
                    Completion(unsafe { CreateEventW(None, true, false, PCWSTR::null()) }?);
                let job = Arc::new(Job {
                    completion,
                    result: Mutex::new(None),
                });
                *cached = Some(job.clone());
                let state = self.clone();
                let pending = job.clone();
                let destination = local.clone();
                self.options.runtime.spawn(async move {
                    let result = transfer::download(
                        &state.live,
                        &state.remote,
                        &destination,
                        false,
                        |event| {
                            (state.options.progress)(event);
                        },
                        &state.cancel,
                    )
                    .await
                    .map(|outcome| outcome.bytes)
                    .map_err(|error| format!("拖出下载失败：{error:#}"));
                    if let Ok(mut completed) = pending.result.lock() {
                        *completed = Some(result);
                    }
                    unsafe {
                        let _ = SetEvent(pending.completion.0);
                    }
                });
                job
            }
        };
        // Never hold cached/result locks across COM dispatch: Explorer may
        // make another IDataObject request while the UI is pumping messages.
        loop {
            if escape_down() {
                self.cancel.cancel();
            }
            if (self.options.should_stop)() {
                self.cancel.cancel();
            }
            if self.cancel.is_cancelled() || self.live.cancel.is_cancelled() {
                return Err(E_ABORT.into());
            }
            if let Some(result) = job
                .result
                .lock()
                .map_err(|_| Error::from_hresult(E_FAIL))?
                .clone()
            {
                result.map_err(|message| Error::new(E_FAIL, message))?;
                break;
            }
            let flags = (COWAIT_DISPATCH_CALLS.0
                | COWAIT_DISPATCH_WINDOW_MESSAGES.0
                | COWAIT_INPUTAVAILABLE.0) as u32;
            if let Err(error) = unsafe { CoWaitForMultipleHandles(flags, 100, &[job.completion.0]) }
            {
                if error.code() != RPC_S_CALLPENDING {
                    return Err(error);
                }
            }
        }
        let wide: Vec<u16> = local.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            SHCreateStreamOnFileEx(
                PCWSTR(wide.as_ptr()),
                (STGM_READ | STGM_SHARE_DENY_WRITE).0,
                0,
                false,
                None::<&IStream>,
            )
        }
    }
}

#[implement(IDataObject)]
struct VirtualFile {
    state: Arc<DragState>,
    descriptor: u16,
    contents: u16,
    shell_data: Option<IDataObject>,
}
impl VirtualFile {
    fn formats(&self) -> [FORMATETC; 2] {
        [
            FORMATETC {
                cfFormat: self.descriptor,
                dwAspect: DVASPECT_CONTENT.0,
                lindex: -1,
                tymed: TYMED_HGLOBAL.0 as u32,
                ..Default::default()
            },
            FORMATETC {
                cfFormat: self.contents,
                dwAspect: DVASPECT_CONTENT.0,
                lindex: 0,
                tymed: TYMED_ISTREAM.0 as u32,
                ..Default::default()
            },
        ]
    }

    fn shell_data_for(&self, format: &FORMATETC) -> Option<&IDataObject> {
        // The Shell stores drag-image metadata in registered formats. The
        // virtual file formats remain owned exclusively by this object.
        if format.cfFormat >= 0xC000
            && format.cfFormat != self.descriptor
            && format.cfFormat != self.contents
        {
            self.shell_data.as_ref()
        } else {
            None
        }
    }
}
impl IDataObject_Impl for VirtualFile_Impl {
    fn GetData(&self, input: *const FORMATETC) -> Result<STGMEDIUM> {
        let format = unsafe { input.as_ref() }.ok_or_else(|| Error::from_hresult(E_POINTER))?;
        if let Some(shell) = self.shell_data_for(format) {
            return unsafe { shell.GetData(input) };
        }
        self.QueryGetData(input).ok()?;
        if format.cfFormat == self.descriptor {
            let mut name = [0u16; 260];
            let encoded: Vec<_> = self.state.name.encode_utf16().collect();
            name[..encoded.len()].copy_from_slice(&encoded);
            let descriptor = FILEDESCRIPTORW {
                dwFlags: (FD_ATTRIBUTES.0 | FD_PROGRESSUI.0) as u32,
                dwFileAttributes: FILE_ATTRIBUTE_NORMAL.0,
                cFileName: name,
                ..Default::default()
            };
            let group = FILEGROUPDESCRIPTORW {
                cItems: 1,
                fgd: [descriptor],
            };
            unsafe {
                let memory = GlobalAlloc(GMEM_MOVEABLE, size_of::<FILEGROUPDESCRIPTORW>())?;
                let bytes = GlobalLock(memory);
                if bytes.is_null() {
                    let _ = GlobalFree(Some(memory));
                    return Err(E_OUTOFMEMORY.into());
                }
                ptr::write_unaligned(bytes.cast::<FILEGROUPDESCRIPTORW>(), group);
                let _ = GlobalUnlock(memory);
                Ok(STGMEDIUM {
                    tymed: TYMED_HGLOBAL.0 as u32,
                    u: STGMEDIUM_0 { hGlobal: memory },
                    pUnkForRelease: ManuallyDrop::new(None),
                })
            }
        } else {
            let stream = self.state.contents()?;
            Ok(STGMEDIUM {
                tymed: TYMED_ISTREAM.0 as u32,
                u: STGMEDIUM_0 {
                    pstm: ManuallyDrop::new(Some(stream)),
                },
                pUnkForRelease: ManuallyDrop::new(None),
            })
        }
    }
    fn GetDataHere(&self, _: *const FORMATETC, _: *mut STGMEDIUM) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn QueryGetData(&self, input: *const FORMATETC) -> HRESULT {
        let Some(format) = (unsafe { input.as_ref() }) else {
            return E_POINTER;
        };
        if let Some(shell) = self.shell_data_for(format) {
            return unsafe { shell.QueryGetData(input) };
        }
        if format.dwAspect != DVASPECT_CONTENT.0 || !format.ptd.is_null() {
            return DV_E_FORMATETC;
        }
        if self.formats().iter().any(|supported| {
            supported.cfFormat == format.cfFormat
                && supported.lindex == format.lindex
                && supported.tymed & format.tymed != 0
        }) {
            S_OK
        } else {
            DV_E_FORMATETC
        }
    }
    fn GetCanonicalFormatEtc(&self, _: *const FORMATETC, output: *mut FORMATETC) -> HRESULT {
        if let Some(output) = unsafe { output.as_mut() } {
            output.ptd = ptr::null_mut();
            DATA_S_SAMEFORMATETC
        } else {
            E_POINTER
        }
    }
    fn SetData(
        &self,
        input: *const FORMATETC,
        medium: *const STGMEDIUM,
        release: BOOL,
    ) -> Result<()> {
        let format = unsafe { input.as_ref() }.ok_or_else(|| Error::from_hresult(E_POINTER))?;
        if medium.is_null() {
            return Err(E_POINTER.into());
        }
        let shell = self
            .shell_data_for(format)
            .ok_or_else(|| Error::from_hresult(E_NOTIMPL))?;
        unsafe { shell.SetData(input, medium, release.as_bool()) }
    }
    fn EnumFormatEtc(&self, direction: u32) -> Result<IEnumFORMATETC> {
        if direction != DATADIR_GET.0 as u32 {
            return Err(E_NOTIMPL.into());
        }
        unsafe { SHCreateStdEnumFmtEtc(&self.formats()) }
    }
    fn DAdvise(&self, _: *const FORMATETC, _: u32, _: Ref<IAdviseSink>) -> Result<u32> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
    fn DUnadvise(&self, _: u32) -> Result<()> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
    fn EnumDAdvise(&self) -> Result<IEnumSTATDATA> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
}
#[implement(IDropSource)]
struct DragSource {
    state: Arc<DragState>,
}
impl IDropSource_Impl for DragSource_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, _: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool()
            || escape_down()
            || self.state.live.cancel.is_cancelled()
            || self.state.cancel.is_cancelled()
            || (self.state.options.should_stop)()
        {
            DRAGDROP_S_CANCEL
        } else if !left_button_down() {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }
    fn GiveFeedback(&self, _: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}
struct Ole;
impl Drop for Ole {
    fn drop(&mut self) {
        unsafe {
            OleUninitialize();
        }
    }
}

const DRAG_ICON_SIZE: i32 = 48;

fn document_pixel(x: i32, y: i32) -> u32 {
    const OUTLINE: u32 = 0xFF2855B6;
    const PAPER: u32 = 0xFFF2F6FF;
    const FOLD: u32 = 0xFFBFD3FF;
    if !(7..41).contains(&x) || !(2..46).contains(&y) || x - y > 25 {
        return 0;
    }
    if x < 9 || x >= 39 || y < 4 || y >= 44 || x - y >= 24 {
        return OUTLINE;
    }
    if x >= 27 && y < 16 {
        return FOLD;
    }
    if (14..34).contains(&x)
        && ((22..24).contains(&y) || (28..30).contains(&y) || (34..36).contains(&y))
    {
        return OUTLINE;
    }
    PAPER
}

fn attach_drag_image(data: &IDataObject, scale: f64) -> Result<()> {
    let helper: IDragSourceHelper = unsafe {
        CoCreateInstance(
            &CLSID_DragDropHelper,
            None::<&IUnknown>,
            CLSCTX_INPROC_SERVER,
        )
    }?;
    let size = (f64::from(DRAG_ICON_SIZE) * scale)
        .round()
        .clamp(48.0, 144.0) as i32;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pixels = ptr::null_mut();
    let bitmap = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut pixels, None, 0) }?;
    if pixels.is_null() {
        unsafe {
            let _ = DeleteObject(bitmap.into());
        }
        return Err(E_OUTOFMEMORY.into());
    }
    // A top-down buffer with straight alpha. The Shell applies the alpha
    // multiplication when it attaches the image to the data object.
    let pixels =
        unsafe { std::slice::from_raw_parts_mut(pixels.cast::<u32>(), (size * size) as usize) };
    for y in 0..size {
        for x in 0..size {
            pixels[(y * size + x) as usize] =
                document_pixel(x * DRAG_ICON_SIZE / size, y * DRAG_ICON_SIZE / size);
        }
    }
    let image = SHDRAGIMAGE {
        sizeDragImage: SIZE { cx: size, cy: size },
        ptOffset: POINT {
            x: size / 8,
            y: size / 8,
        },
        hbmpDragImage: bitmap,
        crColorKey: COLORREF(u32::MAX),
    };
    let result = unsafe { helper.InitializeFromBitmap(&image, data) };
    if result.is_err() {
        // The Shell takes ownership when initialization succeeds.
        unsafe {
            let _ = DeleteObject(bitmap.into());
        }
    }
    result
}

pub fn run(
    live: Arc<LiveSession>,
    remote: String,
    name: String,
    options: DragOptions,
) -> std::result::Result<Option<u64>, String> {
    // IPC can arrive after a quick release. Do not enter an orphaned drag
    // loop when there is no physical mouse gesture left to transfer.
    if !left_button_down()
        || escape_down()
        || live.cancel.is_cancelled()
        || options.cancel.is_cancelled()
        || (options.should_stop)()
    {
        return Ok(None);
    }
    unsafe { OleInitialize(None) }.map_err(|error| format!("无法初始化系统拖放：{error}"))?;
    let _ole = Ole;
    unsafe {
        let _ = ReleaseCapture();
    }
    let folder = tempfile::Builder::new()
        .prefix("orbitshell-drag-")
        .tempdir()
        .map_err(|error| format!("无法准备下载临时目录：{error}"))?;
    let cancel = options.cancel.child_token();
    let state = Arc::new(DragState {
        options,
        live,
        remote,
        name,
        folder,
        cancel,
        cached: Mutex::new(None),
    });
    let descriptor = unsafe { RegisterClipboardFormatW(w!("FileGroupDescriptorW")) } as u16;
    let contents = unsafe { RegisterClipboardFormatW(w!("FileContents")) } as u16;
    if descriptor == 0 || contents == 0 {
        return Err("无法注册系统文件拖放格式".into());
    }
    let scale = state.options.scale;
    let shell_data = unsafe { SHCreateDataObject(None, None, None::<&IDataObject>) }.ok();
    let supports_preview = shell_data.is_some();
    let data: IDataObject = VirtualFile {
        state: state.clone(),
        descriptor,
        contents,
        shell_data,
    }
    .into();
    if supports_preview {
        // Download still works on targets without Windows Shell drag images.
        let _ = attach_drag_image(&data, scale);
    }
    let source: IDropSource = DragSource {
        state: state.clone(),
    }
    .into();
    let mut effect = DROPEFFECT::default();
    let status = unsafe { DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect) };
    let result = (|| {
        if state.cancel.is_cancelled() {
            return Ok(None);
        }
        let job = state
            .cached
            .lock()
            .map_err(|_| "文件拖放状态不可用")?
            .clone();
        let cached = match job {
            Some(job) => {
                let result = job.result.lock().map_err(|_| "文件拖放状态不可用")?.clone();
                result
            }
            None => None,
        };
        if let Some(Err(message)) = cached.as_ref() {
            return Err(message.clone());
        }
        if status == DRAGDROP_S_CANCEL {
            return Ok(None);
        }
        status
            .ok()
            .map_err(|error| format!("系统拖放未完成：{error}"))?;
        if effect.0 == 0 {
            return Ok(None);
        }
        match cached {
            Some(Ok(bytes)) => Ok(Some(bytes)),
            _ => Err("拖放目标没有请求文件内容，无法确认下载完成".into()),
        }
    })();
    state.cancel.cancel();
    result
}
