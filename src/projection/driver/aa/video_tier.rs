//! Which video size and density the phone is asked to render at.

/// The Android Auto video tier and density to ask the phone for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoTier {
    pub width: u32,
    pub height: u32,
    pub dpi: u32,
}

// The slider's range, and the range a density is kept within when it's picked automatically
const MIN_DPI: u32 = 96;
const MAX_DPI: u32 = 213;

/// The tier for a `ao_resolution` setting: "480p", "720p", "1080p", or "auto" (anything else
/// too) to match the screen. Auto never goes above 1080p, the biggest the decoders here are
/// expected to handle.
fn tier_size(resolution: &str, screen: Option<(u32, u32)>) -> (u32, u32) {
    match resolution {
        "480p" => (800, 480),
        "720p" => (1280, 720),
        "1080p" => (1920, 1080),
        _ => match screen.map(|(w, h)| w.min(h)) {
            Some(short) if short >= 1080 => (1920, 1080),
            Some(short) if short >= 720 => (1280, 720),
            Some(_) => (800, 480),
            // Window size not reported yet
            None => (1920, 1080),
        },
    }
}

/// The density that suits `tier_height` rendered for a screen `screen_height` pixels tall.
///
/// The phone lays its UI out in dp (`px * 160 / dpi`), so the density sets how much UI fits: a
/// taller tier needs more dpi to keep the same amount on screen. The amount that suits a screen
/// grows with its resolution, but slower than the pixel count (bigger panels are usually only
/// somewhat larger, or denser), hence the square root. The reference is 720 px at 160 dpi:
///
///   dpi = 160 * tier_height / sqrt(720 * screen_height)
///
/// i.e. 160 for a 720p tier on a 720p screen, about 196 for 1080p on 1080p, about 131 for 480p
/// on 480p, and lower when a screen is bigger than its tier.
fn auto_dpi(tier_height: u32, screen_height: u32) -> u32 {
    let dpi = 160.0 * f64::from(tier_height) / (720.0 * f64::from(screen_height)).sqrt();
    (dpi.round() as u32).clamp(MIN_DPI, MAX_DPI)
}

/// `screen` is the main window's size in pixels, once QML has reported it.
pub fn resolve(
    resolution: &str,
    dpi_auto: bool,
    manual_dpi: f64,
    screen: Option<(u32, u32)>,
) -> VideoTier {
    let (width, height) = tier_size(resolution, screen);
    let dpi = if dpi_auto {
        // Without a screen size, assume the tier fills it
        let screen_height = screen.map_or(height, |(w, h)| w.min(h));
        auto_dpi(height, screen_height)
    } else {
        manual_dpi as u32
    };
    VideoTier { width, height, dpi }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_resolution_follows_the_screen() {
        assert_eq!(tier_size("auto", Some((1920, 1080))), (1920, 1080));
        assert_eq!(tier_size("auto", Some((3840, 2160))), (1920, 1080));
        assert_eq!(tier_size("auto", Some((1366, 768))), (1280, 720));
        assert_eq!(tier_size("auto", Some((1280, 720))), (1280, 720));
        assert_eq!(tier_size("auto", Some((1024, 600))), (800, 480));
        assert_eq!(tier_size("auto", Some((800, 480))), (800, 480));
        // A portrait window counts by its short side too
        assert_eq!(tier_size("auto", Some((768, 1366))), (1280, 720));
        assert_eq!(tier_size("auto", None), (1920, 1080));
    }

    #[test]
    fn manual_resolution_ignores_the_screen() {
        assert_eq!(tier_size("720p", Some((3840, 2160))), (1280, 720));
        assert_eq!(tier_size("1080p", Some((800, 480))), (1920, 1080));
        assert_eq!(tier_size("480p", None), (800, 480));
    }

    #[test]
    fn auto_dpi_matches_the_reference_points() {
        assert_eq!(auto_dpi(720, 720), 160);
        assert_eq!(auto_dpi(1080, 1080), 196);
        assert_eq!(auto_dpi(480, 480), 131);
        // Surface RT: 1366x768 screen, 720p tier
        assert_eq!(auto_dpi(720, 768), 155);
        // A tier below the screen's size gets less dpi, above it more
        assert!(auto_dpi(720, 1080) < auto_dpi(720, 720));
        assert!(auto_dpi(1080, 720) > auto_dpi(720, 720));
    }

    #[test]
    fn auto_dpi_stays_in_the_slider_range() {
        assert_eq!(auto_dpi(1080, 300), MAX_DPI);
        assert_eq!(auto_dpi(480, 4000), MIN_DPI);
    }

    #[test]
    fn resolve_combines_them() {
        let tier = resolve("auto", true, 160.0, Some((1366, 768)));
        assert_eq!(tier, VideoTier { width: 1280, height: 720, dpi: 155 });
        // Manual dpi is taken as is
        assert_eq!(resolve("auto", false, 180.0, Some((1366, 768))).dpi, 180);
        // No screen size yet: the tier is assumed to fill it
        assert_eq!(resolve("720p", true, 160.0, None).dpi, 160);
    }
}
