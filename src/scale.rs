pub const RELATIVE: [f32; 5] = [0.75, 1.0, 1.25, 1.5, 2.0];
pub const RELATIVE_MIN: f32 = 0.75;
pub const RELATIVE_MAX: f32 = 2.0;
const ABSOLUTE_MIN: f32 = 0.75;
const ABSOLUTE_GRID: [f32; 6] = [1.0, 1.25, 1.5, 2.0, 2.5, 3.0];
const ABSOLUTE_MAX: f32 = 3.0;
pub const UNKNOWN_FIT: f32 = 2.0;

pub fn label(scale: f32) -> String {
    format!("{}%", (scale * 100.0).round())
}

pub fn largest_fit(monitor: (f32, f32), native: f32) -> f32 {
    let width = monitor.0 / (crate::app::WINDOW_MIN[0] * native);
    let height = monitor.1 / (crate::app::WINDOW_MIN[1] * native);
    width.min(height).clamp(1.0, ABSOLUTE_MAX)
}

pub fn compose(desktop: f32, relative: Option<f32>, fit: f32) -> f32 {
    let wanted = desktop * relative.unwrap_or(1.0);
    wanted.min(largest_option_up_to(fit)).clamp(ABSOLUTE_MIN, ABSOLUTE_MAX)
}

pub fn largest_option_up_to(fit: f32) -> f32 {
    ABSOLUTE_GRID.into_iter().rev().find(|scale| *scale <= fit + 0.01).unwrap_or(1.0)
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn detected() -> f32 {
    desktop::detected()
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
pub fn detected() -> f32 {
    1.0
}

#[cfg(all(unix, not(target_os = "macos")))]
mod desktop {
    use std::sync::OnceLock;

    pub fn detected() -> f32 {
        static DETECTED: OnceLock<f32> = OnceLock::new();
        *DETECTED.get_or_init(|| {
            if !on_x11() {
                return 1.0;
            }
            desktop_scales().into_iter().fold(1.0, f32::max).clamp(1.0, super::ABSOLUTE_MAX)
        })
    }

    fn on_x11() -> bool {
        let forced_x11 = std::env::var("WINIT_UNIX_BACKEND").is_ok_and(|backend| backend == "x11");
        std::env::var_os("DISPLAY").is_some() && (forced_x11 || std::env::var_os("WAYLAND_DISPLAY").is_none())
    }

    fn desktop_scales() -> Vec<f32> {
        let mut scales = Vec::new();
        scales.extend(std::env::var("GDK_SCALE").ok().and_then(|scale| plain_scale(&scale)));
        if let Some(xml) = xfconf_file() {
            scales.extend(property_value(&xml, "WindowScalingFactor").and_then(in_range));
        }
        scales
    }

    fn xfconf_file() -> Option<String> {
        let home = std::env::var_os("HOME")?;
        let path = std::path::Path::new(&home).join(".config/xfce4/xfconf/xfce-perchannel-xml/xsettings.xml");
        std::fs::read_to_string(path).ok()
    }

    fn property_value(xml: &str, name: &str) -> Option<f32> {
        let at = xml.find(&format!("name=\"{name}\""))?;
        let value = xml[at..].split_once("value=\"")?.1;
        value.split('"').next()?.parse().ok()
    }

    fn plain_scale(text: &str) -> Option<f32> {
        text.trim().parse::<f32>().ok().and_then(in_range)
    }

    fn in_range(scale: f32) -> Option<f32> {
        (1.0..=4.0).contains(&scale).then_some(scale)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn reads_plain_desktop_scales() {
            assert_eq!(plain_scale("2\n"), Some(2.0));
            assert_eq!(plain_scale("0.5"), None);
            assert_eq!(in_range(4.5), None);
        }

        #[test]
        fn reads_the_xfce_settings_file() {
            let xml = r#"<channel name="xsettings"><property name="Gdk" type="empty">
            <property name="WindowScalingFactor" type="int" value="2"/></property></channel>"#;
            assert_eq!(property_value(xml, "WindowScalingFactor").and_then(in_range), Some(2.0));
            assert_eq!(property_value(xml, "Nope"), None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_sizes_multiply_the_desktop_scale() {
        assert_eq!(compose(2.0, None, 3.6), 2.0);
        assert_eq!(compose(2.0, Some(1.0), 3.6), 2.0);
        assert_eq!(compose(2.0, Some(0.75), 3.6), 1.5);
        assert_eq!(compose(2.0, Some(1.5), 3.6), 3.0);
        assert_eq!(compose(1.0, Some(1.5), 3.6), 1.5);
        assert_eq!(compose(1.0, Some(2.0), 3.6), 2.0);
        assert_eq!(compose(1.0, Some(0.75), 3.6), 0.75);
    }

    #[test]
    fn never_allows_a_scale_where_the_window_cannot_fit() {
        assert_eq!(largest_fit((3840.0, 2160.0), 1.0), 3.0);
        assert!((largest_fit((1920.0, 1080.0), 1.0) - 1.8).abs() < 0.01);
        assert!((largest_fit((1080.0, 1920.0), 1.0) - 1080.0f32 / 940.0).abs() < 0.01);
        assert_eq!(largest_fit((1024.0, 768.0), 1.0), (1024.0f32 / 940.0).min(768.0 / 600.0));
        assert_eq!(largest_fit((800.0, 500.0), 1.0), 1.0);
        assert_eq!(compose(2.0, Some(2.0), 1.8), 1.5);
    }

    #[test]
    fn clamped_scales_snap_to_an_offered_size() {
        assert_eq!(largest_option_up_to(3.6), 3.0);
        assert_eq!(largest_option_up_to(2.0), 2.0);
        assert_eq!(largest_option_up_to(1.8), 1.5);
        assert_eq!(largest_option_up_to(1.26), 1.25);
        assert_eq!(largest_option_up_to(0.9), 1.0);
    }
}
