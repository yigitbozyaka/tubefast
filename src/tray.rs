use std::sync::atomic::{AtomicBool, Ordering};

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
    finished: std::sync::mpsc::Receiver<()>,
}

#[cfg(windows)]
mod native {
    use super::begin_quit;
    use std::cell::{Cell, RefCell};
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, FindWindowW, GWL_EXSTYLE, GetCursorPos,
        GetForegroundWindow, GetWindowLongPtrW, IsIconic, LoadIconW, MF_STRING, PostMessageW, RegisterClassW, RegisterWindowMessageW,
        SW_HIDE, SW_RESTORE, SW_SHOWNA, SetForegroundWindow, SetWindowLongPtrW, ShowWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
        WM_APP, WM_CLOSE, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
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
        static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
    }

    fn set_cloak(window: HWND, hidden: bool) {
        let hidden = hidden as i32;
        unsafe { DwmSetWindowAttribute(window, DWMWA_CLOAK as u32, (&raw const hidden).cast(), size_of::<i32>() as u32) };
    }

    fn set_taskbar_button(window: HWND, shown: bool) {
        unsafe {
            let style = GetWindowLongPtrW(window, GWL_EXSTYLE) as u32;
            let style = if shown {
                (style | WS_EX_APPWINDOW) & !WS_EX_TOOLWINDOW
            } else {
                (style | WS_EX_TOOLWINDOW) & !WS_EX_APPWINDOW
            };
            SetWindowLongPtrW(window, GWL_EXSTYLE, style as isize);
            ShowWindow(window, SW_HIDE);
            ShowWindow(window, SW_SHOWNA);
        }
    }

    pub fn hide(window: HWND) {
        set_cloak(window, true);
        set_taskbar_button(window, false);
        unsafe {
            if GetForegroundWindow() == window {
                SetForegroundWindow(FindWindowW(wide("Shell_TrayWnd").as_ptr(), null()));
            }
        }
    }

    pub fn reveal(window: HWND) {
        set_taskbar_button(window, true);
        set_cloak(window, false);
    }

    fn apply(handler: &Handler, choice: Choice) {
        let window = handler.window as HWND;
        unsafe {
            match choice {
                Choice::Show => {
                    reveal(window);
                    if IsIconic(window) != 0 {
                        ShowWindow(window, SW_RESTORE);
                    }
                    SetForegroundWindow(window);
                }
                Choice::Quit => {
                    begin_quit();
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
            if menu.is_null() {
                return None;
            }
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
        if message == TASKBAR_CREATED.get() {
            unsafe { add_icon(window) };
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
            TASKBAR_CREATED.set(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()));
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                null(),
                0,
                0,
                0,
                0,
                0,
                null_mut(),
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
    pub fn start(window: isize, ctx: eframe::egui::Context) -> Option<Self> {
        use windows_sys::Win32::System::Threading::GetCurrentThreadId;
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG};

        let (ready, started) = std::sync::mpsc::channel();
        let (done, finished) = std::sync::mpsc::channel();
        let wake = move || ctx.request_repaint();
        std::thread::Builder::new()
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
                let _ = done.send(());
            })
            .ok()?;
        let thread = started.recv().ok()??;
        Some(Self { window, thread, finished })
    }
}

#[cfg(windows)]
impl Tray {
    pub fn hide(&self) {
        native::hide(self.window as _);
    }
}

#[cfg(windows)]
impl Drop for Tray {
    fn drop(&mut self) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
        unsafe { PostThreadMessageW(self.thread, WM_QUIT, 0, 0) };
        let _ = self.finished.recv_timeout(std::time::Duration::from_millis(300));
    }
}

#[cfg(not(windows))]
const SHOW_ID: &str = "show";
#[cfg(not(windows))]
const QUIT_ID: &str = "quit";

#[cfg(not(windows))]
pub struct Tray {
    _icon: tray_icon::TrayIcon,
    ctx: eframe::egui::Context,
    minimize_only: bool,
}

#[cfg(not(windows))]
fn reveal(ctx: &eframe::egui::Context) {
    use eframe::egui::ViewportCommand;
    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(ViewportCommand::Focus);
    ctx.request_repaint();
}

#[cfg(not(windows))]
fn quit(ctx: &eframe::egui::Context) {
    begin_quit();
    ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
    ctx.request_repaint();
}

#[cfg(not(windows))]
fn on_wayland() -> bool {
    let forced_x11 = std::env::var("WINIT_UNIX_BACKEND").is_ok_and(|backend| backend == "x11");
    std::env::var_os("WAYLAND_DISPLAY").is_some() && !forced_x11
}

#[cfg(not(windows))]
impl Tray {
    pub fn start(_: isize, ctx: eframe::egui::Context) -> Option<Self> {
        use tray_icon::menu::{Menu, MenuEvent, MenuItem};
        use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

        let menu = Menu::new();
        menu.append_items(&[
            &MenuItem::with_id(SHOW_ID, "Show Tubefast", true, None),
            &MenuItem::with_id(QUIT_ID, "Quit", true, None),
        ])
        .ok()?;
        let art = crate::ui::app_icon();
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip(crate::APP_NAME)
            .with_icon(Icon::from_rgba(art.rgba, art.width, art.height).ok()?)
            .build()
            .ok()?;
        let menu_ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| match event.id.as_ref() {
            SHOW_ID => reveal(&menu_ctx),
            QUIT_ID => quit(&menu_ctx),
            _ => {}
        }));
        let click_ctx = ctx.clone();
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                reveal(&click_ctx);
            }
        }));
        Some(Self {
            _icon: icon,
            ctx,
            minimize_only: on_wayland(),
        })
    }

    pub fn hide(&self) {
        use eframe::egui::ViewportCommand;
        let command = if self.minimize_only {
            ViewportCommand::Minimized(true)
        } else {
            ViewportCommand::Visible(false)
        };
        self.ctx.send_viewport_cmd(command);
    }
}

#[cfg(not(windows))]
impl Drop for Tray {
    fn drop(&mut self) {
        tray_icon::menu::MenuEvent::set_event_handler(None::<fn(tray_icon::menu::MenuEvent)>);
        tray_icon::TrayIconEvent::set_event_handler(None::<fn(tray_icon::TrayIconEvent)>);
    }
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
