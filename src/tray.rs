use std::sync::atomic::{AtomicBool, Ordering};

static QUITTING: AtomicBool = AtomicBool::new(false);

pub fn quitting() -> bool {
    QUITTING.load(Ordering::SeqCst)
}

pub fn begin_quit() {
    QUITTING.store(true, Ordering::SeqCst);
}

#[derive(Debug, PartialEq, Eq)]
pub enum Closing {
    Quit,
    Hide,
    Park,
}

pub fn closing(tray_running: bool, quitting: bool, wayland: bool, host_present: bool) -> Closing {
    if !tray_running || quitting {
        Closing::Quit
    } else if !wayland {
        Closing::Hide
    } else if host_present {
        Closing::Park
    } else {
        Closing::Quit
    }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
pub fn on_close_request(tray_running: bool) -> Closing {
    closing(tray_running, quitting(), false, true)
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn host_present() -> bool {
    host::present()
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn on_close_request(tray_running: bool) -> Closing {
    closing(tray_running, quitting(), on_wayland(), host::present())
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
static ON_WAYLAND: AtomicBool = AtomicBool::new(false);

#[cfg(not(windows))]
pub fn window_of(cc: &eframe::CreationContext<'_>) -> isize {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let wayland = matches!(cc.window_handle().map(|handle| handle.as_raw()), Ok(RawWindowHandle::Wayland(_)));
    ON_WAYLAND.store(wayland, Ordering::SeqCst);
    #[cfg(all(unix, not(target_os = "macos")))]
    activation::capture(cc);
    0
}

#[cfg(all(unix, not(target_os = "macos")))]
mod activation {
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
    use std::sync::Mutex;
    use wayland_client::backend::{Backend, ObjectId};
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::{wl_registry::WlRegistry, wl_surface::WlSurface};
    use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
    use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;

    struct Silent;

    impl Dispatch<WlRegistry, GlobalListContents> for Silent {
        fn event(
            _: &mut Self,
            _: &WlRegistry,
            _: <WlRegistry as Proxy>::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<XdgActivationV1, ()> for Silent {
        fn event(_: &mut Self, _: &XdgActivationV1, _: <XdgActivationV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
    }

    struct Activator {
        connection: Connection,
        activation: XdgActivationV1,
        surface: WlSurface,
        _queue: EventQueue<Silent>,
    }

    static ACTIVATOR: Mutex<Option<Activator>> = Mutex::new(None);

    fn connect(cc: &eframe::CreationContext<'_>) -> Option<Activator> {
        let window = cc.window_handle().ok()?.as_raw();
        let display = cc.display_handle().ok()?.as_raw();
        let (RawWindowHandle::Wayland(window), RawDisplayHandle::Wayland(display)) = (window, display) else {
            return None;
        };
        let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
        let connection = Connection::from_backend(backend);
        let (globals, queue) = registry_queue_init::<Silent>(&connection).ok()?;
        let activation: XdgActivationV1 = globals.bind(&queue.handle(), 1..=1, ()).ok()?;
        let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), window.surface.as_ptr().cast()) }.ok()?;
        let surface = WlSurface::from_id(&connection, id).ok()?;
        Some(Activator {
            connection,
            activation,
            surface,
            _queue: queue,
        })
    }

    pub fn capture(cc: &eframe::CreationContext<'_>) {
        release();
        if let Some(activator) = connect(cc) {
            *ACTIVATOR.lock().unwrap() = Some(activator);
        }
    }

    pub fn release() {
        if let Some(activator) = ACTIVATOR.lock().unwrap().take() {
            activator.activation.destroy();
            let _ = activator.connection.flush();
        }
    }

    fn activate(token: String) -> bool {
        match ACTIVATOR.lock().unwrap().as_ref() {
            Some(activator) => {
                activator.activation.activate(token, &activator.surface);
                activator.connection.flush().is_ok()
            }
            None => false,
        }
    }

    static STASHED: Mutex<Option<String>> = Mutex::new(None);

    fn token_of_tray_event() -> Option<String> {
        for _ in 0..20 {
            if let Some(token) = ksni::current_xdg_activation_token() {
                return Some(token);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        None
    }

    pub fn activate_from_tray_event() -> bool {
        token_of_tray_event().is_some_and(activate)
    }

    pub fn stash_token_of_tray_event() {
        *STASHED.lock().unwrap() = token_of_tray_event();
    }

    pub fn activate_stashed() -> bool {
        let stashed = STASHED.lock().unwrap().take();
        stashed.is_some_and(activate)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn present_new_window(ctx: &eframe::egui::Context) {
    use eframe::egui::ViewportCommand;
    ctx.send_viewport_cmd(ViewportCommand::Focus);
    if !activation::activate_stashed() {
        ctx.send_viewport_cmd(ViewportCommand::RequestUserAttention(
            eframe::egui::UserAttentionType::Informational,
        ));
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
pub use watchdog::save_before_forced_exit;

#[cfg(all(unix, not(target_os = "macos")))]
mod watchdog {
    use std::sync::Mutex;
    use std::time::Duration;

    const GRACE: Duration = Duration::from_millis(1000);

    static BEFORE_EXIT: Mutex<Option<Box<dyn Fn() + Send>>> = Mutex::new(None);

    pub fn save_before_forced_exit(save: impl Fn() + Send + 'static) {
        *BEFORE_EXIT.lock().unwrap() = Some(Box::new(save));
    }

    pub fn frozen(frames_before: u64, frames_after: u64) -> bool {
        frames_before == frames_after
    }

    pub fn start() {
        let before = crate::wake::frames_drawn();
        let _ = std::thread::Builder::new()
            .name("quit-watchdog".to_owned())
            .spawn(move || {
                std::thread::sleep(GRACE);
                if !frozen(before, crate::wake::frames_drawn()) {
                    return;
                }
                if let Some(save) = BEFORE_EXIT.lock().unwrap().as_ref() {
                    save();
                }
                unsafe { libc::_exit(0) };
            });
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_window_that_drew_nothing_since_quit_began_is_frozen() {
            assert!(frozen(7, 7));
            assert!(!frozen(7, 8));
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod host {
    use ashpd::zbus::fdo::DBusProxy;
    use ashpd::zbus::names::BusName;
    use ashpd::zbus::proxy::{Builder, CacheProperties};
    use ashpd::zbus::{Connection, Proxy};
    use futures_lite::{StreamExt, future};
    use std::sync::atomic::{AtomicBool, Ordering};

    const WATCHER: &str = "org.kde.StatusNotifierWatcher";
    const PATH: &str = "/StatusNotifierWatcher";
    const HOST_FLAG: &str = "IsStatusNotifierHostRegistered";

    static PRESENT: AtomicBool = AtomicBool::new(false);

    pub fn present() -> bool {
        PRESENT.load(Ordering::SeqCst)
    }

    pub fn host_shown(watcher_owned: bool, host_registered: Option<bool>) -> bool {
        watcher_owned && host_registered.unwrap_or(true)
    }

    pub struct Watch {
        _stop: async_channel::Sender<()>,
    }

    impl Drop for Watch {
        fn drop(&mut self) {
            PRESENT.store(false, Ordering::SeqCst);
        }
    }

    pub fn host_lost(known: Option<bool>, now: bool) -> bool {
        known == Some(true) && !now
    }

    fn publish(known: &mut Option<bool>, now: bool) {
        PRESENT.store(now, Ordering::SeqCst);
        if host_lost(*known, now) {
            crate::wake::bring_back_if_parked();
        }
        *known = Some(now);
    }

    pub fn watch() -> Option<Watch> {
        let (keeper, stop) = async_channel::bounded(1);
        std::thread::Builder::new()
            .name("tray-host-watch".to_owned())
            .spawn(move || {
                let mut known = None;
                let asked_to_stop = future::block_on(follow(stop, &mut known)).unwrap_or(false);
                if !asked_to_stop {
                    publish(&mut known, false);
                }
            })
            .ok()?;
        Some(Watch { _stop: keeper })
    }

    async fn shown_by(dbus: &DBusProxy<'_>, watcher: &Proxy<'_>) -> bool {
        let owned = match BusName::try_from(WATCHER) {
            Ok(name) => dbus.name_has_owner(name).await.unwrap_or(false),
            Err(_) => false,
        };
        let registered = if owned {
            watcher.get_property::<bool>(HOST_FLAG).await.ok()
        } else {
            None
        };
        host_shown(owned, registered)
    }

    async fn follow(stop: async_channel::Receiver<()>, known: &mut Option<bool>) -> ashpd::zbus::Result<bool> {
        let connection = Connection::session().await?;
        let dbus = DBusProxy::new(&connection).await?;
        let watcher = Builder::<Proxy<'_>>::new(&connection)
            .destination(WATCHER)?
            .path(PATH)?
            .interface(WATCHER)?
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        let owner = dbus.receive_name_owner_changed_with_args(&[(0, WATCHER)]).await?.map(|_| ());
        let registered = watcher.receive_signal("StatusNotifierHostRegistered").await?.map(|_| ());
        let unregistered = watcher.receive_signal("StatusNotifierHostUnregistered").await?.map(|_| ());
        let mut changes = std::pin::pin!(owner.or(registered).or(unregistered));
        loop {
            publish(known, shown_by(&dbus, &watcher).await);
            let changed = async { changes.next().await.is_none().then_some(false) };
            let stopped = async {
                let _ = stop.recv().await;
                Some(true)
            };
            if let Some(asked_to_stop) = future::or(changed, stopped).await {
                return Ok(asked_to_stop);
            }
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn release_activation() {
    activation::release();
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
pub fn release_activation() {}

#[cfg(all(unix, not(target_os = "macos")))]
fn activated_by_token() -> bool {
    activation::activate_from_tray_event()
}

#[cfg(all(not(windows), not(all(unix, not(target_os = "macos")))))]
fn activated_by_token() -> bool {
    false
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
    #[cfg(all(unix, not(target_os = "macos")))]
    _host: Option<host::Watch>,
}

#[cfg(not(windows))]
fn reveal(ctx: &eframe::egui::Context) {
    use eframe::egui::ViewportCommand;
    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(ViewportCommand::Focus);
    if on_wayland() && !activated_by_token() {
        ctx.send_viewport_cmd(ViewportCommand::RequestUserAttention(
            eframe::egui::UserAttentionType::Informational,
        ));
    }
    ctx.request_repaint();
}

#[cfg(not(windows))]
fn show() {
    match crate::wake::window_context() {
        Some(ctx) => reveal(&ctx),
        None => ask_parked_app_for_a_window(),
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn ask_parked_app_for_a_window() {
    activation::stash_token_of_tray_event();
    crate::wake::request_show();
}

#[cfg(all(not(windows), not(all(unix, not(target_os = "macos")))))]
fn ask_parked_app_for_a_window() {}

#[cfg(not(windows))]
fn quit() {
    begin_quit();
    match crate::wake::window_context() {
        Some(ctx) => {
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
            ctx.request_repaint();
            #[cfg(all(unix, not(target_os = "macos")))]
            if on_wayland() {
                watchdog::start();
            }
        }
        None => {
            #[cfg(all(unix, not(target_os = "macos")))]
            crate::wake::request_quit();
        }
    }
}

#[cfg(not(windows))]
fn on_wayland() -> bool {
    ON_WAYLAND.load(Ordering::SeqCst)
}

#[cfg(not(windows))]
impl Tray {
    pub fn start(_: isize, _: eframe::egui::Context) -> Option<Self> {
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
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| match event.id.as_ref() {
            SHOW_ID => show(),
            QUIT_ID => quit(),
            _ => {}
        }));
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show();
            }
        }));
        Some(Self {
            _icon: icon,
            #[cfg(all(unix, not(target_os = "macos")))]
            _host: if on_wayland() { host::watch() } else { None },
        })
    }

    pub fn hide(&self) {
        if let Some(ctx) = crate::wake::window_context() {
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Visible(false));
        }
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
        for wayland in [false, true] {
            for host in [false, true] {
                assert_eq!(closing(false, false, wayland, host), Closing::Quit);
                assert_eq!(closing(true, true, wayland, host), Closing::Quit);
                assert_eq!(closing(false, true, wayland, host), Closing::Quit);
            }
        }
    }

    #[test]
    fn outside_wayland_closing_still_hides_the_window_in_place() {
        assert_eq!(closing(true, false, false, false), Closing::Hide);
        assert_eq!(closing(true, false, false, true), Closing::Hide);
    }

    #[test]
    fn on_wayland_the_window_goes_away_only_when_a_tray_host_can_bring_it_back() {
        assert_eq!(closing(true, false, true, true), Closing::Park);
        assert_eq!(closing(true, false, true, false), Closing::Quit);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn only_a_host_that_was_there_and_is_gone_counts_as_lost() {
        use host::host_lost;
        assert!(host_lost(Some(true), false));
        assert!(!host_lost(None, false));
        assert!(!host_lost(Some(false), false));
        assert!(!host_lost(Some(true), true));
        assert!(!host_lost(Some(false), true));
        assert!(!host_lost(None, true));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn a_tray_host_needs_an_owned_watcher_and_a_registered_host() {
        use host::host_shown;
        assert!(!host_shown(false, None));
        assert!(!host_shown(false, Some(true)));
        assert!(!host_shown(true, Some(false)));
        assert!(host_shown(true, Some(true)));
        assert!(host_shown(true, None));
    }
}
