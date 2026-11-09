use std::sync::atomic::{AtomicBool, Ordering};

pub const SUPPORTED: bool = cfg!(windows);
static QUITTING: AtomicBool = AtomicBool::new(false);

pub fn quitting() -> bool {
    QUITTING.load(Ordering::SeqCst)
}

pub fn begin_quit() {
    QUITTING.store(true, Ordering::SeqCst);
}

pub fn hides_on_close(tray_running: bool, quitting: bool) -> bool {
    tray_running && !quitting
}

#[cfg(windows)]
pub fn window_of(cc: &eframe::CreationContext<'_>) -> isize {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match cc.window_handle().map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::Win32(window)) => window.hwnd.get(),
        _ => 0,
    }
}

#[cfg(not(windows))]
pub fn window_of(_: &eframe::CreationContext<'_>) -> isize {
    0
}

#[cfg(windows)]
pub struct Tray {
    window: isize,
    thread: u32,
    worker: Option<std::thread::JoinHandle<()>>,
}

#[cfg(windows)]
mod native {
    use super::begin_quit;
    use std::cell::RefCell;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, GetCursorPos, HWND_MESSAGE, LoadIconW,
        MF_STRING, PostMessageW, RegisterClassW, SW_RESTORE, SW_SHOW, SW_SHOWMINNOACTIVE, SetForegroundWindow, ShowWindow, TPM_RETURNCMD,
        TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP, WM_CLOSE, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
    };

    pub const ICON_RESOURCE: usize = 1;
    pub const TRAY_MESSAGE: u32 = WM_APP + 1;
    const ICON_ID: u32 = 1;
    const SHOW_ID: usize = 1;
    const QUIT_ID: usize = 2;

    #[derive(Clone, Copy)]
    pub enum Choice {
        Show,
        Quit,
    }

    pub struct Handler {
        pub window: isize,
        pub wake: Box<dyn Fn()>,
    }

    thread_local! {
        pub static HANDLER: RefCell<Option<Handler>> = const { RefCell::new(None) };
    }

    fn apply(handler: &Handler, choice: Choice) {
        let window = handler.window as HWND;
        unsafe {
            match choice {
                Choice::Show => {
                    ShowWindow(window, SW_SHOW);
                    ShowWindow(window, SW_RESTORE);
                    SetForegroundWindow(window);
                }
                Choice::Quit => {
                    begin_quit();
                    ShowWindow(window, SW_SHOWMINNOACTIVE);
                    PostMessageW(window, WM_CLOSE, 0, 0);
                }
            }
        }
        (handler.wake)();
    }

    pub fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    unsafe fn pick_from_menu(window: HWND) -> Option<Choice> {
        unsafe {
            let menu = CreatePopupMenu();
            AppendMenuW(menu, MF_STRING, SHOW_ID, wide("Show Tubefast").as_ptr());
            AppendMenuW(menu, MF_STRING, QUIT_ID, wide("Quit").as_ptr());
            let mut cursor = POINT { x: 0, y: 0 };
            GetCursorPos(&mut cursor);
            SetForegroundWindow(window);
            let picked = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, cursor.x, cursor.y, 0, window, null());
            PostMessageW(window, WM_NULL, 0, 0);
            DestroyMenu(menu);
            match picked as usize {
                SHOW_ID => Some(Choice::Show),
                QUIT_ID => Some(Choice::Quit),
                _ => None,
            }
        }
    }

    unsafe extern "system" fn window_procedure(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if message == TRAY_MESSAGE {
            let choice = match lparam as u32 {
                WM_LBUTTONUP => Some(Choice::Show),
                WM_RBUTTONUP => unsafe { pick_from_menu(window) },
                _ => None,
            };
            if let Some(choice) = choice {
                HANDLER.with(|handler| {
                    if let Some(handler) = handler.borrow().as_ref() {
                        apply(handler, choice);
                    }
                });
            }
            return 0;
        }
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

    pub unsafe fn create_window() -> HWND {
        unsafe {
            let instance = GetModuleHandleW(null());
            let class_name = wide("TubefastTray");
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_procedure),
                hInstance: instance,
                lpszClassName: class_name.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassW(&class);
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                null_mut(),
                instance,
                null(),
            )
        }
    }

    fn icon_data(window: HWND) -> NOTIFYICONDATAW {
        unsafe {
            let mut data: NOTIFYICONDATAW = std::mem::zeroed();
            data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            data.hWnd = window;
            data.uID = ICON_ID;
            data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
            data.uCallbackMessage = TRAY_MESSAGE;
            data.hIcon = LoadIconW(GetModuleHandleW(null()), ICON_RESOURCE as *const u16);
            for (slot, unit) in data.szTip.iter_mut().zip(wide(crate::APP_NAME)) {
                *slot = unit;
            }
            data
        }
    }

    pub unsafe fn add_icon(window: HWND) -> bool {
        unsafe { Shell_NotifyIconW(NIM_ADD, &icon_data(window)) != 0 }
    }

    pub unsafe fn remove_icon(window: HWND) {
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &icon_data(window));
            DestroyWindow(window);
        }
    }
}

#[cfg(windows)]
impl Tray {
    pub fn start(window: isize, wake: impl Fn() + Send + 'static) -> Option<Self> {
        use windows_sys::Win32::System::Threading::GetCurrentThreadId;
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG};

        let (ready, started) = std::sync::mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("tray".into())
            .spawn(move || unsafe {
                native::HANDLER.with(|handler| {
                    *handler.borrow_mut() = Some(native::Handler {
                        window,
                        wake: Box::new(wake),
                    })
                });
                let tray_window = native::create_window();
                let added = !tray_window.is_null() && native::add_icon(tray_window);
                let _ = ready.send(added.then(|| GetCurrentThreadId()));
                if !added {
                    return;
                }
                let mut message: MSG = std::mem::zeroed();
                while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                    windows_sys::Win32::UI::WindowsAndMessaging::DispatchMessageW(&message);
                }
                native::remove_icon(tray_window);
            })
            .ok()?;
        let thread = started.recv().ok()??;
        Some(Self {
            window,
            thread,
            worker: Some(worker),
        })
    }
}

#[cfg(windows)]
impl Tray {
    pub fn hide(&self) {
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(self.window as _, windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE)
        };
    }
}

#[cfg(windows)]
impl Drop for Tray {
    fn drop(&mut self) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
        unsafe { PostThreadMessageW(self.thread, WM_QUIT, 0, 0) };
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(not(windows))]
pub struct Tray;

#[cfg(not(windows))]
impl Tray {
    pub fn start(_: isize, _: impl Fn() + Send + 'static) -> Option<Self> {
        None
    }

    pub fn hide(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_hides_only_while_the_tray_runs_and_not_quitting() {
        assert!(hides_on_close(true, false));
        assert!(!hides_on_close(false, false));
        assert!(!hides_on_close(true, true));
        assert!(!hides_on_close(false, true));
    }
}
