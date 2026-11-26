//! Where the popup appears for each popup style.
//!
//! All inputs and outputs are physical pixels; `scale` is the monitor's DPI
//! scale (1.0 at 96 DPI) used to convert the logical design sizes.

use crate::settings::PopupStyle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }
    pub const fn right(&self) -> i32 {
        self.x + self.w
    }
    pub const fn bottom(&self) -> i32 {
        self.y + self.h
    }
}

/// Logical design sizes, matching the prototypes.
pub const BEAM_WIDTH: f32 = 580.0;
pub const BEAM_HEIGHT: f32 = 420.0;
pub const DRAWER_WIDTH: f32 = 330.0;
/// Gap between the popup and screen edges, the taskbar or the caret.
pub const MARGIN: f32 = 12.0;

/// Which edge of the monitor the taskbar sits on, inferred from the gap
/// between the full monitor rectangle and its work area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskbarEdge {
    Bottom,
    Top,
    Left,
    Right,
}

pub fn taskbar_edge(monitor: Rect, work: Rect) -> TaskbarEdge {
    let gaps = [
        (work.y - monitor.y, TaskbarEdge::Top),
        (monitor.bottom() - work.bottom(), TaskbarEdge::Bottom),
        (work.x - monitor.x, TaskbarEdge::Left),
        (monitor.right() - work.right(), TaskbarEdge::Right),
    ];
    gaps.iter()
        .filter(|(g, _)| *g > 0)
        .max_by_key(|(g, _)| *g)
        .map(|(_, e)| *e)
        // Auto-hidden taskbar: no gap; Windows defaults to the bottom.
        .unwrap_or(TaskbarEdge::Bottom)
}

fn px(v: f32, scale: f32) -> i32 {
    (v * scale).round() as i32
}

/// Keep `r` inside `area`, shrinking it if it is larger than the area.
fn clamp_into(mut r: Rect, area: Rect) -> Rect {
    r.w = r.w.min(area.w);
    r.h = r.h.min(area.h);
    r.x = r.x.clamp(area.x, area.right() - r.w);
    r.y = r.y.clamp(area.y, area.bottom() - r.h);
    r
}

/// Compute the popup rectangle.
///
/// * `monitor` – the full bounds of the monitor the popup belongs on.
/// * `work` – that monitor's work area (excludes the taskbar).
/// * `anchor` – the text caret or mouse position, used by `BeamCursor`.
pub fn popup_rect(
    style: PopupStyle,
    monitor: Rect,
    work: Rect,
    anchor: Option<Point>,
    scale: f32,
) -> Rect {
    let m = px(MARGIN, scale);
    match style {
        PopupStyle::BeamCentre => {
            let w = px(BEAM_WIDTH, scale);
            let h = px(BEAM_HEIGHT, scale);
            // Upper third, like a command bar, so results grow downwards.
            let r = Rect::new(work.x + (work.w - w) / 2, work.y + work.h / 5, w, h);
            clamp_into(r, work)
        }
        PopupStyle::BeamCursor => {
            let w = px(BEAM_WIDTH, scale);
            let h = px(BEAM_HEIGHT, scale);
            let Some(a) = anchor else {
                return popup_rect(PopupStyle::BeamCentre, monitor, work, None, scale);
            };
            // Prefer just below the caret, left edge aligned with it;
            // flip above when there is not enough room below.
            let below = a.y + px(24.0, scale);
            let y = if below + h <= work.bottom() {
                below
            } else {
                a.y - m - h
            };
            clamp_into(Rect::new(a.x - px(16.0, scale), y, w, h), work)
        }
        PopupStyle::Drawer => {
            let w = px(DRAWER_WIDTH, scale);
            let h = work.h - 2 * m;
            let x = match taskbar_edge(monitor, work) {
                TaskbarEdge::Left => work.x + m,
                _ => work.right() - w - m,
            };
            clamp_into(Rect::new(x, work.y + m, w, h), work)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MON: Rect = Rect::new(0, 0, 1920, 1080);
    const WORK_BOTTOM_BAR: Rect = Rect::new(0, 0, 1920, 1032);

    #[test]
    fn beam_centre_is_centred_in_upper_third() {
        let r = popup_rect(PopupStyle::BeamCentre, MON, WORK_BOTTOM_BAR, None, 1.0);
        assert_eq!(r.w, 580);
        assert_eq!(r.x, (1920 - 580) / 2);
        assert_eq!(r.y, 1032 / 5);
    }

    #[test]
    fn beam_centre_scales_with_dpi() {
        let r = popup_rect(PopupStyle::BeamCentre, MON, WORK_BOTTOM_BAR, None, 1.5);
        assert_eq!(r.w, 870);
        assert_eq!(r.h, 630);
    }

    #[test]
    fn beam_cursor_sits_below_caret() {
        let a = Point { x: 400, y: 300 };
        let r = popup_rect(PopupStyle::BeamCursor, MON, WORK_BOTTOM_BAR, Some(a), 1.0);
        assert_eq!(r.y, 324);
        assert_eq!(r.x, 384);
    }

    #[test]
    fn beam_cursor_flips_above_near_bottom() {
        let a = Point { x: 400, y: 900 };
        let r = popup_rect(PopupStyle::BeamCursor, MON, WORK_BOTTOM_BAR, Some(a), 1.0);
        assert!(r.bottom() <= a.y);
    }

    #[test]
    fn beam_cursor_stays_on_screen_at_right_edge() {
        let a = Point { x: 1910, y: 100 };
        let r = popup_rect(PopupStyle::BeamCursor, MON, WORK_BOTTOM_BAR, Some(a), 1.0);
        assert_eq!(r.right(), 1920);
    }

    #[test]
    fn beam_cursor_without_anchor_falls_back_to_centre() {
        let a = popup_rect(PopupStyle::BeamCursor, MON, WORK_BOTTOM_BAR, None, 1.0);
        let b = popup_rect(PopupStyle::BeamCentre, MON, WORK_BOTTOM_BAR, None, 1.0);
        assert_eq!(a, b);
    }

    #[test]
    fn drawer_hugs_right_edge_above_bottom_taskbar() {
        let r = popup_rect(PopupStyle::Drawer, MON, WORK_BOTTOM_BAR, None, 1.0);
        assert_eq!(r.right(), 1920 - 12);
        assert_eq!(r.bottom(), 1032 - 12);
        assert_eq!(r.w, 330);
    }

    #[test]
    fn drawer_moves_to_left_taskbar() {
        let work = Rect::new(60, 0, 1860, 1080);
        assert_eq!(taskbar_edge(MON, work), TaskbarEdge::Left);
        let r = popup_rect(PopupStyle::Drawer, MON, work, None, 1.0);
        assert_eq!(r.x, 72);
    }

    #[test]
    fn second_monitor_offsets_are_respected() {
        let mon = Rect::new(1920, -200, 2560, 1440);
        let work = Rect::new(1920, -200, 2560, 1392);
        let r = popup_rect(PopupStyle::Drawer, mon, work, None, 1.0);
        assert_eq!(r.right(), 1920 + 2560 - 12);
        assert_eq!(r.y, -200 + 12);
    }

    #[test]
    fn taskbar_edges_are_detected() {
        assert_eq!(taskbar_edge(MON, WORK_BOTTOM_BAR), TaskbarEdge::Bottom);
        assert_eq!(taskbar_edge(MON, Rect::new(0, 48, 1920, 1032)), TaskbarEdge::Top);
        assert_eq!(taskbar_edge(MON, Rect::new(0, 0, 1860, 1080)), TaskbarEdge::Right);
        assert_eq!(taskbar_edge(MON, MON), TaskbarEdge::Bottom);
    }
}
