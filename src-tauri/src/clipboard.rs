/// A null result means this platform still uses the WebView clipboard adapter.
#[tauri::command]
pub async fn clipboard_read_text() -> Result<Option<String>, String> {
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(native::read)
            .await.map_err(|_| "读取剪贴板未完成，请重试粘贴。".to_string())?
            .map(Some)
    }
    #[cfg(not(windows))]
    { Ok(None) }
}

#[tauri::command]
pub async fn clipboard_write_text(window: tauri::WebviewWindow, text: String) -> Result<bool, String> {
    #[cfg(windows)]
    {
        // EmptyClipboard requires a real owner window before SetClipboardData can succeed.
        let owner = window.hwnd().map_err(|_| "应用窗口暂不可用，请重新打开窗口后复制。".to_string())?.0 as usize;
        tauri::async_runtime::spawn_blocking(move || native::write(owner, &text))
            .await.map_err(|_| "写入剪贴板未完成，请重试复制。".to_string())?
            .map(|()| true)
    }
    #[cfg(not(windows))]
    { let _ = (window, text); Ok(false) }
}

#[cfg(windows)]
mod native {
    use std::{mem::size_of, ptr, thread, time::Duration};
    use windows_sys::Win32::{
        Foundation::{GlobalFree, HGLOBAL, HWND},
        System::{
            DataExchange::{CloseClipboard, CountClipboardFormats, EmptyClipboard, GetClipboardData,
                IsClipboardFormatAvailable, OpenClipboard, SetClipboardData},
            Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
            Ole::CF_UNICODETEXT,
        },
    };

    const MAX_TEXT_BYTES: usize = 16 * 1024 * 1024;

    struct Clipboard;
    impl Clipboard {
        fn open(owner: HWND) -> Result<Self, String> {
            // Clipboard managers and other applications may hold the clipboard briefly.
            // Retry on the blocking worker, without blocking the UI or retrying a paste.
            for attempt in 0..8 {
                if unsafe { OpenClipboard(owner) } != 0 { return Ok(Self); }
                if attempt < 7 { thread::sleep(Duration::from_millis(15)); }
            }
            Err("剪贴板正被其他程序占用，请稍后重试。".into())
        }
    }
    impl Drop for Clipboard {
        fn drop(&mut self) { unsafe { CloseClipboard(); } }
    }

    struct MemoryLock(HGLOBAL);
    impl Drop for MemoryLock {
        fn drop(&mut self) { unsafe { GlobalUnlock(self.0); } }
    }

    struct OwnedMemory(HGLOBAL);
    impl Drop for OwnedMemory {
        fn drop(&mut self) {
            if !self.0.is_null() { unsafe { GlobalFree(self.0); } }
        }
    }

    pub fn read() -> Result<String, String> {
        let _clipboard = Clipboard::open(ptr::null_mut())?;
        if unsafe { IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT)) } == 0 {
            if unsafe { CountClipboardFormats() } == 0 { return Ok(String::new()); }
            return Err("剪贴板中没有可粘贴的文本，请先复制文字。".into());
        }
        let data = unsafe { GetClipboardData(u32::from(CF_UNICODETEXT)) };
        if data.is_null() { return Err("无法取得剪贴板文本，请重新复制后再粘贴。".into()); }
        let bytes = unsafe { GlobalSize(data) };
        if bytes > MAX_TEXT_BYTES { return Err("剪贴板文本过大，请分段粘贴。".into()); }
        if bytes < size_of::<u16>() || bytes % size_of::<u16>() != 0 {
            return Err("剪贴板文本格式无效，请重新复制后再粘贴。".into());
        }
        let pointer = unsafe { GlobalLock(data) };
        if pointer.is_null() { return Err("无法读取剪贴板文本，请重新复制后再粘贴。".into()); }
        let _lock = MemoryLock(data);
        // The handle belongs to Windows. Copy only within its allocation, then unlock before closing.
        let units = unsafe { std::slice::from_raw_parts(pointer.cast::<u16>(), bytes / size_of::<u16>()) };
        let end = units.iter().position(|unit| *unit == 0)
            .ok_or_else(|| "剪贴板文本不完整，请重新复制后再粘贴。".to_string())?;
        String::from_utf16(&units[..end])
            .map_err(|_| "剪贴板文本编码无效，请重新复制后再粘贴。".into())
    }

    pub fn write(owner: usize, text: &str) -> Result<(), String> {
        if text.contains('\0') { return Err("复制内容包含空字符，无法写入文本剪贴板。".into()); }
        if text.len() > MAX_TEXT_BYTES { return Err("复制内容过大，请分段复制。".into()); }
        let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = units.len().checked_mul(size_of::<u16>())
            .filter(|bytes| *bytes <= MAX_TEXT_BYTES)
            .ok_or_else(|| "复制内容过大，请分段复制。".to_string())?;
        let mut memory = OwnedMemory(unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) });
        if memory.0.is_null() { return Err("无法准备剪贴板内容，请重试复制。".into()); }
        let pointer = unsafe { GlobalLock(memory.0) };
        if pointer.is_null() { return Err("无法准备剪贴板内容，请重试复制。".into()); }
        let lock = MemoryLock(memory.0);
        unsafe { ptr::copy_nonoverlapping(units.as_ptr(), pointer.cast::<u16>(), units.len()); }
        drop(lock);
        // Allocate and fill before clearing, preserving the old clipboard on preparation errors.
        let _clipboard = Clipboard::open(owner as HWND)?;
        if unsafe { EmptyClipboard() } == 0 {
            return Err("无法更新系统剪贴板，请重试复制。".into());
        }
        if unsafe { SetClipboardData(u32::from(CF_UNICODETEXT), memory.0) }.is_null() {
            return Err("无法写入系统剪贴板，请重试复制。".into());
        }
        // SetClipboardData transfers ownership to Windows only on success.
        memory.0 = ptr::null_mut();
        Ok(())
    }
}
