use eframe::egui;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

enum Target {
    Window(egui::Context),
    #[cfg(all(unix, not(target_os = "macos")))]
    Parked,
    Nobody,
}

static TARGET: Mutex<Target> = Mutex::new(Target::Nobody);
static FRAMES: AtomicU64 = AtomicU64::new(0);

pub fn point_at_window(ctx: egui::Context) {
    let mut target = TARGET.lock().unwrap();
    *target = Target::Window(ctx);
    #[cfg(all(unix, not(target_os = "macos")))]
    parked::forget_show_request();
}

#[cfg(not(windows))]
pub fn window_context() -> Option<egui::Context> {
    match &*TARGET.lock().unwrap() {
        Target::Window(ctx) => Some(ctx.clone()),
        _ => None,
    }
}

pub fn wake() {
    let ctx = match &*TARGET.lock().unwrap() {
        Target::Window(ctx) => Some(ctx.clone()),
        #[cfg(all(unix, not(target_os = "macos")))]
        Target::Parked => {
            parked::ring();
            None
        }
        Target::Nobody => None,
    };
    if let Some(ctx) = ctx {
        ctx.request_repaint();
    }
}

pub fn frame_drawn() {
    FRAMES.fetch_add(1, Ordering::SeqCst);
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn frames_drawn() -> u64 {
    FRAMES.load(Ordering::SeqCst)
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn point_at_parked() {
    *TARGET.lock().unwrap() = Target::Parked;
    parked::ring();
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn bring_back_if_parked() {
    let target = TARGET.lock().unwrap();
    if matches!(&*target, Target::Parked) {
        parked::request_show();
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
pub use parked::{Next, request_quit, request_show, wait_parked};

#[cfg(all(unix, not(target_os = "macos")))]
mod parked {
    use std::sync::{Condvar, Mutex};
    use std::time::Duration;

    #[derive(Debug, PartialEq, Eq)]
    pub enum Next {
        Stay,
        Show,
        Quit,
    }

    #[derive(Default)]
    pub struct Mailbox {
        rung: bool,
        show: bool,
        quit: bool,
    }

    impl Mailbox {
        pub fn ring(&mut self) {
            self.rung = true;
        }

        pub fn wish_show(&mut self) {
            self.show = true;
            self.rung = true;
        }

        pub fn wish_quit(&mut self) {
            self.quit = true;
            self.rung = true;
        }

        pub fn forget_show(&mut self) {
            self.show = false;
        }

        pub fn next(&mut self) -> Next {
            if self.quit {
                Next::Quit
            } else if std::mem::take(&mut self.show) {
                Next::Show
            } else {
                Next::Stay
            }
        }
    }

    static MAILBOX: Mutex<Mailbox> = Mutex::new(Mailbox {
        rung: false,
        show: false,
        quit: false,
    });
    static BELL: Condvar = Condvar::new();

    pub fn ring() {
        MAILBOX.lock().unwrap().ring();
        BELL.notify_all();
    }

    pub fn request_show() {
        MAILBOX.lock().unwrap().wish_show();
        BELL.notify_all();
    }

    pub fn request_quit() {
        MAILBOX.lock().unwrap().wish_quit();
        BELL.notify_all();
    }

    pub fn forget_show_request() {
        MAILBOX.lock().unwrap().forget_show();
    }

    pub fn wait_parked(timeout: Option<Duration>) -> (Next, bool) {
        let mailbox = MAILBOX.lock().unwrap();
        let mut mailbox = match timeout {
            Some(timeout) => BELL.wait_timeout_while(mailbox, timeout, |mailbox| !mailbox.rung).unwrap().0,
            None => BELL.wait_while(mailbox, |mailbox| !mailbox.rung).unwrap(),
        };
        let rung = std::mem::take(&mut mailbox.rung);
        (mailbox.next(), rung)
    }
}

#[cfg(test)]
mod tests {
    #[cfg(all(unix, not(target_os = "macos")))]
    use super::parked::{Mailbox, Next};

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn quitting_beats_showing_and_a_show_is_served_once() {
        let mut mailbox = Mailbox::default();
        assert_eq!(mailbox.next(), Next::Stay);
        mailbox.wish_show();
        assert_eq!(mailbox.next(), Next::Show);
        assert_eq!(mailbox.next(), Next::Stay);
        mailbox.wish_show();
        mailbox.wish_quit();
        assert_eq!(mailbox.next(), Next::Quit);
        assert_eq!(mailbox.next(), Next::Quit);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn a_show_asked_while_the_window_is_opening_is_forgotten() {
        let mut mailbox = Mailbox::default();
        mailbox.wish_show();
        mailbox.forget_show();
        assert_eq!(mailbox.next(), Next::Stay);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn a_ring_before_the_wait_is_not_lost_and_a_quiet_wait_times_out() {
        use std::time::{Duration, Instant};
        super::parked::ring();
        let started = Instant::now();
        assert!(super::parked::wait_parked(Some(Duration::from_secs(5))).1);
        assert!(started.elapsed() < Duration::from_secs(1));
        let started = Instant::now();
        assert!(!super::parked::wait_parked(Some(Duration::from_millis(60))).1);
        assert!(started.elapsed() >= Duration::from_millis(50));
    }

    #[test]
    fn frames_are_counted() {
        let before = super::FRAMES.load(std::sync::atomic::Ordering::SeqCst);
        super::frame_drawn();
        assert!(super::FRAMES.load(std::sync::atomic::Ordering::SeqCst) > before);
    }
}
