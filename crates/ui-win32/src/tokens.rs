use crate::Theme;
use windows::Win32::Foundation::RECT;

pub type Rgba = [f32; 4];
const fn rgba(red: u8, green: u8, blue: u8, alpha: f32) -> Rgba {
    [
        red as f32 / 255.,
        green as f32 / 255.,
        blue as f32 / 255.,
        alpha,
    ]
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemePalette {
    pub bg_overlay: Rgba,
    pub border_strong: Rgba,
    pub border_subtle: Rgba,
    pub text_primary: Rgba,
    pub text_secondary: Rgba,
    pub text_muted: Rgba,
    pub accent_codex: Rgba,
    pub accent_ag: Rgba,
    pub ok: Rgba,
    pub warn: Rgba,
    pub danger: Rgba,
    pub track: Rgba,
}
#[allow(non_upper_case_globals)]
impl ThemePalette {
    pub const ObsidianSlate: Self = Self {
        bg_overlay: rgba(17, 22, 31, 0.90),
        border_strong: rgba(255, 255, 255, 0.15),
        border_subtle: rgba(255, 255, 255, 0.08),
        text_primary: rgba(241, 245, 249, 1.),
        text_secondary: rgba(148, 163, 184, 1.),
        text_muted: rgba(100, 116, 139, 1.),
        accent_codex: rgba(129, 140, 248, 1.),
        accent_ag: rgba(56, 189, 248, 1.),
        ok: rgba(52, 211, 153, 1.),
        warn: rgba(251, 191, 36, 1.),
        danger: rgba(251, 113, 133, 1.),
        track: rgba(148, 163, 184, 0.16),
    };
    pub const PorcelainFrost: Self = Self {
        bg_overlay: rgba(255, 255, 255, 0.92),
        border_strong: rgba(15, 23, 42, 0.16),
        border_subtle: rgba(15, 23, 42, 0.08),
        text_primary: rgba(15, 23, 42, 1.),
        text_secondary: rgba(71, 85, 105, 1.),
        text_muted: rgba(148, 163, 184, 1.),
        accent_codex: rgba(79, 70, 229, 1.),
        accent_ag: rgba(2, 132, 199, 1.),
        ok: rgba(5, 150, 105, 1.),
        warn: rgba(217, 119, 6, 1.),
        danger: rgba(225, 29, 72, 1.),
        track: rgba(15, 23, 42, 0.08),
    };
    pub const fn for_theme(theme: Theme) -> Self {
        match theme {
            Theme::Dark => Self::ObsidianSlate,
            Theme::Light => Self::PorcelainFrost,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayMode {
    CompactCapsule,
    ExpandedCard,
}
impl OverlayMode {
    pub const fn size_dip(self) -> (i32, i32) {
        match self {
            Self::CompactCapsule => (244, 46),
            Self::ExpandedCard => (332, 236),
        }
    }
    pub const fn surface_dip(self) -> (i32, i32) {
        match self {
            Self::CompactCapsule => (236, 44),
            Self::ExpandedCard => (328, 236),
        }
    }
    pub fn toggled(self) -> Self {
        match self {
            Self::CompactCapsule => Self::ExpandedCard,
            Self::ExpandedCard => Self::CompactCapsule,
        }
    }
}
pub struct OverlayGeometry;
impl OverlayGeometry {
    pub fn dip_to_px(dip: i32, dpi: u32) -> i32 {
        (f64::from(dip) * f64::from(dpi) / 96.).round() as i32
    }
    pub fn snap_to_work_area(rect: RECT, work_area: RECT, snap_distance_px: i32) -> RECT {
        snap_to_work_area(rect, work_area, snap_distance_px)
    }
    pub fn clamp_to_visible_monitors(
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        work_areas: &[RECT],
    ) -> (i32, i32) {
        clamp_to_visible_monitors(x, y, w, h, work_areas)
    }
}
pub fn snap_to_work_area(rect: RECT, work_area: RECT, snap_distance_px: i32) -> RECT {
    let delta = |start: i32, end: i32, low: i32, high: i32| {
        [
            i64::from(low) - i64::from(start),
            i64::from(high) - i64::from(end),
        ]
        .into_iter()
        .min_by_key(|d| d.abs())
        .filter(|d| d.abs() <= i64::from(snap_distance_px))
        .unwrap_or(0)
    };
    let dx = delta(rect.left, rect.right, work_area.left, work_area.right);
    let dy = delta(rect.top, rect.bottom, work_area.top, work_area.bottom);
    let shift = |v: i32, d: i64| (i64::from(v) + d).clamp(i32::MIN.into(), i32::MAX.into()) as i32;
    RECT {
        left: shift(rect.left, dx),
        right: shift(rect.right, dx),
        top: shift(rect.top, dy),
        bottom: shift(rect.bottom, dy),
    }
}
pub fn clamp_to_visible_monitors(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    work_areas: &[RECT],
) -> (i32, i32) {
    work_areas
        .iter()
        .filter(|area| area.right > area.left && area.bottom > area.top)
        .map(|area| {
            (
                x.clamp(
                    area.left,
                    area.right.saturating_sub(w.max(1)).max(area.left),
                ),
                y.clamp(area.top, area.bottom.saturating_sub(h.max(1)).max(area.top)),
            )
        })
        .min_by_key(|&(left, top)| {
            (i128::from(left) - i128::from(x)).pow(2) + (i128::from(top) - i128::from(y)).pow(2)
        })
        .unwrap_or((x, y))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn palette_and_scale() {
        assert_eq!(
            ThemePalette::for_theme(Theme::Dark).warn,
            rgba(251, 191, 36, 1.)
        );
        assert_eq!(
            ThemePalette::for_theme(Theme::Light).border_strong,
            rgba(15, 23, 42, 0.16)
        );
        assert_eq!(OverlayGeometry::dip_to_px(46, 144), 69);
    }
    #[test]
    fn displaced_screens() {
        let left = RECT {
            left: -1920,
            top: -100,
            right: 0,
            bottom: 980,
        };
        let right = RECT {
            left: 100,
            top: 0,
            right: 2020,
            bottom: 1080,
        };
        let near = RECT {
            left: -1917,
            top: 2,
            right: -1673,
            bottom: 48,
        };
        let snapped = snap_to_work_area(near, left, 8);
        assert_eq!(
            (snapped.left, snapped.right, snapped.top),
            (-1920, -1676, 2)
        );
        assert_eq!(
            clamp_to_visible_monitors(-1800, 0, 244, 46, &[left, right]),
            (-1800, 0)
        );
        assert_eq!(
            clamp_to_visible_monitors(40, 0, 244, 46, &[left, right]),
            (100, 0)
        );
        assert_eq!(
            clamp_to_visible_monitors(-3000, -500, 3000, 2000, &[left]),
            (-1920, -100)
        );
    }
}
