use crate::app::Action;
use crate::art::Art;
use crate::ui::{self, OVERLAY_WIDTH, OverlayView};
use eframe::egui::{self, Event, PointerButton, RawInput, Rect, pos2, vec2};
use eframe::{egui_glow, glow};
use glutin::config::{Config, ConfigTemplateBuilder, GlConfig};
use glutin::context::{ContextApi, ContextAttributesBuilder, PossiblyCurrentContext, Version};
use glutin::display::{Display, DisplayApiPreference, GlDisplay};
use glutin::prelude::*;
use glutin::surface::{Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use smithay_client_toolkit::{
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry, delegate_seat, registry_handlers,
};
use std::cell::RefCell;
use std::ffi::CString;
use std::io::{ErrorKind, Read, Write};
use std::num::NonZeroU32;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::ptr::NonNull;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use wayland_client::backend::WaylandError;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_output, wl_pointer, wl_registry, wl_registry::WlRegistry, wl_seat, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::relative_pointer::zv1::client::zwp_relative_pointer_manager_v1::{self, ZwpRelativePointerManagerV1};
use wayland_protocols::wp::relative_pointer::zv1::client::zwp_relative_pointer_v1::{self, ZwpRelativePointerV1};

const LEFT_BUTTON: u32 = 0x110;
const NAMESPACE: &str = "tubefast-overlay";
const LAYER_SHELL_INTERFACE: &str = "zwlr_layer_shell_v1";

#[derive(Clone, PartialEq)]
pub struct Frame {
    pub view: OverlayView,
    pub opacity: f32,
    pub position: [f32; 2],
}

enum Command {
    Show(Frame),
    Hide,
    Quit,
}

enum Report {
    Action(Box<Action>),
    Closed,
}

pub fn supported() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        let Ok(connection) = Connection::connect_to_env() else {
            return false;
        };
        let Ok((globals, _queue)) = registry_queue_init::<Probe>(&connection) else {
            return false;
        };
        globals
            .contents()
            .with_list(|list| list.iter().any(|global| global.interface == LAYER_SHELL_INTERFACE))
    })
}

struct Probe;

impl Dispatch<WlRegistry, GlobalListContents> for Probe {
    fn event(_: &mut Self, _: &WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

pub struct Handle {
    commands: Sender<Command>,
    wake: UnixStream,
    reports: Receiver<Report>,
    sent: RefCell<Option<Frame>>,
    thread: Option<JoinHandle<()>>,
}

impl Handle {
    pub fn start(main: egui::Context, agent: ureq::Agent) -> Option<Self> {
        let (commands, received) = mpsc::channel();
        let (reporter, reports) = mpsc::channel();
        let (wake, wake_read) = UnixStream::pair().ok()?;
        wake.set_nonblocking(true).ok()?;
        wake_read.set_nonblocking(true).ok()?;
        let (ready_sender, ready) = mpsc::channel();
        let wake_for_egui = wake.try_clone().ok()?;
        let thread = std::thread::Builder::new()
            .name("overlay-wayland".to_owned())
            .spawn(move || run(main, agent, received, reporter, wake_read, wake_for_egui, ready_sender))
            .ok()?;
        if ready.recv().unwrap_or(false) {
            Some(Self {
                commands,
                wake,
                reports,
                sent: RefCell::new(None),
                thread: Some(thread),
            })
        } else {
            let _ = thread.join();
            None
        }
    }

    pub fn show(&self, frame: Frame) {
        if self.sent.borrow().as_ref() == Some(&frame) {
            return;
        }
        *self.sent.borrow_mut() = Some(frame.clone());
        self.send(Command::Show(frame));
    }

    pub fn hide(&self) {
        if self.sent.borrow_mut().take().is_some() {
            self.send(Command::Hide);
        }
    }

    pub fn actions(&self) -> Vec<Action> {
        let mut actions = Vec::new();
        while let Ok(report) = self.reports.try_recv() {
            match report {
                Report::Action(action) => actions.push(*action),
                Report::Closed => {
                    self.sent.borrow_mut().take();
                    actions.push(Action::ToggleOverlay);
                }
            }
        }
        actions
    }

    fn send(&self, command: Command) {
        let _ = self.commands.send(command);
        let _ = (&self.wake).write(&[1]);
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.send(Command::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Shown {
    gl_surface: Surface<WindowSurface>,
    layer: LayerSurface,
    size: (u32, u32),
    scale: i32,
    configured: bool,
}

struct Overlay {
    registry: RegistryState,
    seats: SeatState,
    outputs: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    qh: QueueHandle<Overlay>,
    gl_display: Display,
    gl_config: Config,
    gl_context: PossiblyCurrentContext,
    painter: Option<egui_glow::Painter>,
    egui: egui::Context,
    art: Art,
    main: egui::Context,
    reports: Sender<Report>,
    started: Instant,
    events: Vec<Event>,
    pointer: egui::Pos2,
    shown: Option<Shown>,
    frame: Option<Frame>,
    margins: (i32, i32),
    last_position: Option<[f32; 2]>,
    relative_manager: Option<ZwpRelativePointerManagerV1>,
    pressed: bool,
    travelled: (f64, f64),
    drag: Option<(f64, f64)>,
    scale: i32,
    dirty: bool,
    repaint_at: Option<Instant>,
}

impl Overlay {
    fn report(&self, report: Report) {
        let _ = self.reports.send(report);
        self.main.request_repaint();
    }

    fn follow_drag(&mut self) {
        let Some((top, left)) = self.drag.as_mut() else { return };
        *top = top.max(0.0);
        *left = left.max(0.0);
        let margins = (top.round() as i32, left.round() as i32);
        if margins != self.margins {
            self.margins = margins;
            self.apply_margins();
        }
    }

    fn apply_margins(&self) {
        if let Some(shown) = &self.shown {
            shown.layer.set_margin(self.margins.0, 0, 0, self.margins.1);
            shown.layer.commit();
        }
    }

    fn show(&mut self, frame: &Frame) -> Result<(), String> {
        let size = (OVERLAY_WIDTH as u32, frame.view.height().ceil() as u32);
        let surface = self.compositor.create_surface(&self.qh);
        let layer = self
            .layer_shell
            .create_layer_surface(&self.qh, surface.clone(), Layer::Overlay, Some(NAMESPACE), None);
        layer.set_anchor(Anchor::TOP | Anchor::LEFT);
        layer.set_size(size.0, size.1);
        layer.set_margin(self.margins.0, 0, 0, self.margins.1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();
        let pointer = NonNull::new(surface.id().as_ptr().cast()).ok_or("surface has no native handle")?;
        let handle = RawWindowHandle::Wayland(WaylandWindowHandle::new(pointer));
        let attributes = SurfaceAttributesBuilder::<WindowSurface>::new().build(handle, nonzero(size.0), nonzero(size.1));
        let gl_surface = unsafe { self.gl_display.create_window_surface(&self.gl_config, &attributes) }
            .map_err(|error| format!("cannot create the overlay surface: {error}"))?;
        self.shown = Some(Shown {
            gl_surface,
            layer,
            size,
            scale: 1,
            configured: false,
        });
        Ok(())
    }

    fn follow(&mut self, frame: Frame) {
        if self.shown.is_none() {
            self.margins = margins_of(frame.position);
            self.last_position = Some(frame.position);
            if let Err(error) = self.show(&frame) {
                eprintln!("overlay: {error}");
                self.report(Report::Closed);
                return;
            }
        } else if self.last_position != Some(frame.position) && self.drag.is_none() {
            self.margins = margins_of(frame.position);
            self.last_position = Some(frame.position);
            self.apply_margins();
        }
        self.frame = Some(frame);
        self.dirty = true;
    }

    fn hide(&mut self) {
        self.shown = None;
        self.drag = None;
        self.pressed = false;
        self.frame = None;
    }

    fn resize(&mut self, size: (u32, u32)) {
        let scale = self.scale.max(1) as u32;
        let Some(shown) = self.shown.as_mut() else { return };
        shown.size = size;
        shown.scale = scale as i32;
        shown.layer.set_size(size.0, size.1);
        shown.layer.wl_surface().set_buffer_scale(scale as i32);
        shown
            .gl_surface
            .resize(&self.gl_context, nonzero(size.0 * scale), nonzero(size.1 * scale));
        shown.layer.commit();
    }

    fn draw(&mut self) {
        let Some(frame) = self.frame.clone() else { return };
        let wanted = (OVERLAY_WIDTH as u32, frame.view.height().ceil() as u32);
        match &self.shown {
            Some(shown) if shown.configured => {
                if shown.size != wanted || shown.scale != self.scale {
                    self.resize(wanted);
                }
            }
            _ => return,
        }
        let Some(shown) = self.shown.as_ref() else { return };
        if self.gl_context.make_current(&shown.gl_surface).is_err() {
            return;
        }
        if self.painter.is_none() {
            let display = &self.gl_display;
            let context = unsafe {
                glow::Context::from_loader_function(|name| {
                    CString::new(name).map_or(std::ptr::null(), |name| display.get_proc_address(&name))
                })
            };
            match egui_glow::Painter::new(Arc::new(context), "", None, false) {
                Ok(painter) => self.painter = Some(painter),
                Err(error) => {
                    eprintln!("overlay: cannot start the renderer: {error}");
                    self.hide();
                    self.report(Report::Closed);
                    return;
                }
            }
        }
        let _ = shown.gl_surface.set_swap_interval(&self.gl_context, SwapInterval::DontWait);
        let scale = self.scale.max(1) as f32;
        let mut input = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(wanted.0 as f32, wanted.1 as f32))),
            time: Some(self.started.elapsed().as_secs_f64()),
            events: std::mem::take(&mut self.events),
            ..RawInput::default()
        };
        input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(scale);
        self.art.begin_frame(&self.egui);
        let mut actions = Vec::new();
        let output = self.egui.run(input, |ctx| {
            actions = ui::draw_overlay_card(ctx, &frame.view, &self.art, frame.opacity);
        });
        let commands = output.viewport_output.values().flat_map(|viewport| viewport.commands.iter());
        if commands
            .into_iter()
            .any(|command| matches!(command, egui::ViewportCommand::StartDrag))
        {
            self.drag = Some((self.margins.0 as f64 + self.travelled.1, self.margins.1 as f64 + self.travelled.0));
            self.follow_drag();
        }
        let delay = output.viewport_output.values().map(|viewport| viewport.repaint_delay).min();
        let painted = self.egui.tessellate(output.shapes, output.pixels_per_point);
        if let Some(painter) = self.painter.as_mut() {
            use glow::HasContext;
            unsafe {
                painter.gl().clear_color(0.0, 0.0, 0.0, 0.0);
                painter.gl().clear(glow::COLOR_BUFFER_BIT);
            }
            painter.paint_and_update_textures(
                [wanted.0 * scale as u32, wanted.1 * scale as u32],
                output.pixels_per_point,
                &painted,
                &output.textures_delta,
            );
        }
        if let Some(shown) = self.shown.as_ref() {
            let _ = shown.gl_surface.swap_buffers(&self.gl_context);
            shown.layer.commit();
        }
        match delay {
            Some(delay) if delay.is_zero() => self.dirty = true,
            Some(delay) if delay < Duration::from_secs(3600) => self.repaint_at = Some(Instant::now() + delay),
            _ => {}
        }
        for action in actions {
            self.report(Report::Action(Box::new(action)));
        }
    }
}

fn nonzero(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap_or(NonZeroU32::MIN)
}

fn margins_of(position: [f32; 2]) -> (i32, i32) {
    (position[1].max(0.0) as i32, position[0].max(0.0) as i32)
}

impl CompositorHandler for Overlay {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, factor: i32) {
        let ours = self.shown.as_ref().is_some_and(|shown| shown.layer.wl_surface() == surface);
        if ours && factor != self.scale {
            self.scale = factor.max(1);
            self.dirty = true;
        }
    }

    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}

    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}

    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for Overlay {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for Overlay {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.hide();
        self.report(Report::Closed);
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface, _: LayerSurfaceConfigure, _: u32) {
        if let Some(shown) = self.shown.as_mut() {
            shown.configured = true;
        }
        self.dirty = true;
    }
}

impl SeatHandler for Overlay {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seats
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer
            && let Ok(pointer) = self.seats.get_pointer(qh, &seat)
            && let Some(manager) = &self.relative_manager
        {
            manager.get_relative_pointer(&pointer, qh, ());
        }
    }

    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, _: Capability) {}

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for Overlay {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for event in events {
            let ours = self.shown.as_ref().is_some_and(|shown| shown.layer.wl_surface() == &event.surface);
            if !ours {
                continue;
            }
            let position = pos2(event.position.0 as f32, event.position.1 as f32);
            match event.kind {
                PointerEventKind::Enter { .. } => {
                    self.pointer = position;
                    self.events.push(Event::PointerMoved(position));
                }
                PointerEventKind::Motion { .. } => {
                    self.pointer = position;
                    self.events.push(Event::PointerMoved(position));
                }
                PointerEventKind::Leave { .. } => self.events.push(Event::PointerGone),
                PointerEventKind::Press { button, .. } | PointerEventKind::Release { button, .. } if button == LEFT_BUTTON => {
                    let pressed = matches!(event.kind, PointerEventKind::Press { .. });
                    self.events.push(Event::PointerButton {
                        pos: position,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    });
                    self.pressed = pressed;
                    self.travelled = (0.0, 0.0);
                    if !pressed && self.drag.take().is_some() {
                        let moved = [self.margins.1 as f32, self.margins.0 as f32];
                        self.last_position = Some(moved);
                        self.report(Report::Action(Box::new(Action::OverlayMoved(moved))));
                    }
                }
                _ => {}
            }
            self.dirty = true;
        }
    }
}

impl Dispatch<ZwpRelativePointerManagerV1, ()> for Overlay {
    fn event(
        _: &mut Self,
        _: &ZwpRelativePointerManagerV1,
        _: zwp_relative_pointer_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpRelativePointerV1, ()> for Overlay {
    fn event(
        state: &mut Self,
        _: &ZwpRelativePointerV1,
        event: zwp_relative_pointer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let zwp_relative_pointer_v1::Event::RelativeMotion { dx, dy, .. } = event else {
            return;
        };
        if !state.pressed {
            return;
        }
        state.travelled = (state.travelled.0 + dx, state.travelled.1 + dy);
        if let Some((top, left)) = state.drag.as_mut() {
            *top += dy;
            *left += dx;
            state.follow_drag();
        }
    }
}

impl ProvidesRegistryState for Overlay {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }

    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(Overlay);
delegate_output!(Overlay);
delegate_layer!(Overlay);
delegate_seat!(Overlay);
delegate_pointer!(Overlay);
delegate_registry!(Overlay);

fn run(
    main: egui::Context,
    agent: ureq::Agent,
    commands: Receiver<Command>,
    reports: Sender<Report>,
    mut wake_read: UnixStream,
    wake_for_egui: UnixStream,
    ready: Sender<bool>,
) {
    let setup = connect(main, agent, reports, wake_for_egui);
    let (mut queue, mut overlay) = match setup {
        Ok(setup) => setup,
        Err(error) => {
            eprintln!("overlay: {error}");
            let _ = ready.send(false);
            return;
        }
    };
    let _ = ready.send(true);
    let failure = serve(&mut queue, &mut overlay, &commands, &mut wake_read);
    if let Err(error) = failure {
        eprintln!("overlay: {error}");
        overlay.report(Report::Closed);
    }
}

type Setup = (EventQueue<Overlay>, Overlay);

fn connect(main: egui::Context, agent: ureq::Agent, reports: Sender<Report>, wake: UnixStream) -> Result<Setup, String> {
    let connection = Connection::connect_to_env().map_err(|error| format!("cannot reach the compositor: {error}"))?;
    let (globals, queue) = registry_queue_init::<Overlay>(&connection).map_err(|error| error.to_string())?;
    let qh = queue.handle();
    let display = NonNull::new(connection.backend().display_ptr().cast()).ok_or("compositor connection has no native handle")?;
    let display_handle = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(display));
    let gl_display = unsafe { Display::new(display_handle, DisplayApiPreference::Egl) }.map_err(|error| format!("no EGL: {error}"))?;
    let template = ConfigTemplateBuilder::new().with_alpha_size(8).build();
    let gl_config = unsafe { gl_display.find_configs(template) }
        .map_err(|error| format!("no EGL config: {error}"))?
        .reduce(|best, next| if next.num_samples() < best.num_samples() { next } else { best })
        .ok_or("no EGL config")?;
    let gles = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::Gles(Some(Version::new(3, 0))))
        .build(None);
    let not_current = unsafe { gl_display.create_context(&gl_config, &ContextAttributesBuilder::new().build(None)) }
        .or_else(|_| unsafe { gl_display.create_context(&gl_config, &gles) })
        .map_err(|error| format!("no GL context: {error}"))?;
    let gl_context = not_current.treat_as_possibly_current();
    let egui = egui::Context::default();
    ui::install(&egui);
    let waker = Arc::new(wake);
    egui.set_request_repaint_callback(move |info| {
        if info.delay.is_zero() {
            let _ = (&*waker).write(&[1]);
        }
    });
    let art = Art::new(egui.clone(), agent);
    let overlay = Overlay {
        registry: RegistryState::new(&globals),
        seats: SeatState::new(&globals, &qh),
        outputs: OutputState::new(&globals, &qh),
        compositor: CompositorState::bind(&globals, &qh).map_err(|error| error.to_string())?,
        layer_shell: LayerShell::bind(&globals, &qh).map_err(|_| "the compositor has no layer shell".to_owned())?,
        qh: qh.clone(),
        gl_display,
        gl_config,
        gl_context,
        painter: None,
        egui,
        art,
        main,
        reports,
        started: Instant::now(),
        events: Vec::new(),
        pointer: pos2(0.0, 0.0),
        shown: None,
        frame: None,
        margins: (0, 0),
        last_position: None,
        relative_manager: globals.bind(&qh, 1..=1, ()).ok(),
        pressed: false,
        travelled: (0.0, 0.0),
        drag: None,
        scale: 1,
        dirty: false,
        repaint_at: None,
    };
    Ok((queue, overlay))
}

fn serve(
    queue: &mut EventQueue<Overlay>,
    overlay: &mut Overlay,
    commands: &Receiver<Command>,
    wake_read: &mut UnixStream,
) -> Result<(), String> {
    loop {
        loop {
            match commands.try_recv() {
                Ok(Command::Show(frame)) => overlay.follow(frame),
                Ok(Command::Hide) => overlay.hide(),
                Ok(Command::Quit) | Err(TryRecvError::Disconnected) => return Ok(()),
                Err(TryRecvError::Empty) => break,
            }
        }
        queue.dispatch_pending(overlay).map_err(|error| error.to_string())?;
        if overlay.repaint_at.is_some_and(|at| at <= Instant::now()) {
            overlay.repaint_at = None;
            overlay.dirty = true;
        }
        if overlay.dirty {
            overlay.dirty = false;
            overlay.draw();
        }
        queue.flush().map_err(|error| error.to_string())?;
        let timeout = overlay.repaint_at.map_or(-1, |at| {
            at.saturating_duration_since(Instant::now()).as_millis().min(i32::MAX as u128) as i32
        });
        if let Some(guard) = queue.prepare_read() {
            let mut fds = [
                libc::pollfd {
                    fd: guard.connection_fd().as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: wake_read.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            let ready = unsafe { libc::poll(fds.as_mut_ptr(), 2, timeout) };
            if ready > 0 && fds[0].revents & libc::POLLIN != 0 {
                if let Err(error) = guard.read()
                    && !matches!(&error, WaylandError::Io(io) if io.kind() == ErrorKind::WouldBlock)
                {
                    return Err(error.to_string());
                }
            } else {
                drop(guard);
            }
        }
        let mut drained = [0u8; 64];
        while wake_read.read(&mut drained).is_ok_and(|read| read > 0) {}
        queue.dispatch_pending(overlay).map_err(|error| error.to_string())?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn margins_follow_the_saved_position_and_never_go_negative() {
        assert_eq!(margins_of([24.0, 48.0]), (48, 24));
        assert_eq!(margins_of([-5.0, -1.0]), (0, 0));
    }
}
