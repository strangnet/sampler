use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gpui::{Bounds, Pixels, Styled, Window, canvas, fill, point, px, rgb, rgba, size};

use crate::audio::AudioBufferState;
use crate::selection::{Handle, Selection};

/// Horizontal padding inside the waveform canvas in logical pixels.
pub const WAVEFORM_PAD_X: f32 = 4.0;

fn draw_x_range(origin_x: f32, width: f32) -> (f32, f32) {
    let start_x = origin_x + WAVEFORM_PAD_X;
    let draw_width = (width - WAVEFORM_PAD_X * 2.0).max(1.0);
    (start_x, draw_width)
}

fn selection_x_bounds(start_x: f32, draw_width: f32, a: f32, b: f32) -> (f32, f32) {
    let lo = a.min(b).clamp(0.0, 1.0);
    let hi = a.max(b).clamp(0.0, 1.0);
    (start_x + lo * draw_width, start_x + hi * draw_width)
}

fn playhead_x(origin_x: f32, width: f32, start_x: f32, draw_width: f32, pos: f32) -> f32 {
    let px_pos = start_x + pos.clamp(0.0, 1.0) * draw_width;
    (px_pos - 1.0).clamp(origin_x, origin_x + width - 2.0)
}

fn paint_grid(window: &mut Window, origin_x: f32, origin_y: f32, width: f32, height: f32) {
    let center_y = origin_y + height * 0.5;

    // Subtle vertical grid lines (25%, 50%, 75%)
    let grid_color = rgb(0x232632);
    for frac in [0.25_f32, 0.5, 0.75] {
        let gx = origin_x + width * frac;
        window.paint_quad(fill(
            Bounds {
                origin: point(px(gx), px(origin_y)),
                size: size(px(1.0), px(height)),
            },
            grid_color,
        ));
    }

    // Center zero-crossing line
    window.paint_quad(fill(
        Bounds {
            origin: point(px(origin_x), px(center_y - 0.5)),
            size: size(px(width), px(1.0)),
        },
        rgb(0x2e3242),
    ));
}

fn paint_peaks(
    window: &mut Window,
    peaks: &[(f32, f32)],
    is_recording: bool,
    origin_y: f32,
    height: f32,
    start_x: f32,
    draw_width: f32,
) {
    if peaks.is_empty() {
        return;
    }

    let center_y = origin_y + height * 0.5;
    let bar_width = 2.0_f32;
    let bar_gap = 1.0_f32;
    let step = bar_width + bar_gap;
    let num_bars = (draw_width / step).floor().max(1.0) as usize;
    let half_height = (height * 0.5 - 12.0).max(4.0);

    let bar_color = if is_recording {
        rgb(0xf43f5e)
    } else {
        rgb(0x38bdf8)
    };

    let total_peaks = peaks.len();

    for bar_idx in 0..num_bars {
        let p_start = (bar_idx * total_peaks) / num_bars;
        let p_end = (((bar_idx + 1) * total_peaks) / num_bars)
            .max(p_start + 1)
            .min(total_peaks);

        let mut min_val = 0.0_f32;
        let mut max_val = 0.0_f32;
        for &(lo, hi) in &peaks[p_start..p_end] {
            min_val = min_val.min(lo);
            max_val = max_val.max(hi);
        }

        // Map [-1.0, 1.0] to screen Y (Y grows downward)
        let top_y = center_y - max_val.clamp(-1.0, 1.0) * half_height;
        let bottom_y = center_y - min_val.clamp(-1.0, 1.0) * half_height;
        let bar_h = (bottom_y - top_y).max(2.0);
        let y = if bottom_y - top_y < 2.0 {
            center_y - 1.0
        } else {
            top_y
        };

        let x = start_x + (bar_idx as f32) * step;
        window.paint_quad(fill(
            Bounds {
                origin: point(px(x), px(y)),
                size: size(px(bar_width), px(bar_h)),
            },
            bar_color,
        ));
    }
}

fn paint_selection_overlay(
    window: &mut Window,
    origin_y: f32,
    height: f32,
    start_x: f32,
    draw_width: f32,
    (a, b): Selection,
    highlighted_handle: Option<Handle>,
) {
    if a.is_nan() || b.is_nan() {
        return;
    }
    let (sel_x0, sel_x1) = selection_x_bounds(start_x, draw_width, a, b);

    if sel_x1 > sel_x0 {
        window.paint_quad(fill(
            Bounds {
                origin: point(px(sel_x0), px(origin_y)),
                size: size(px(sel_x1 - sel_x0), px(height)),
            },
            rgba(0x38bdf833),
        ));
    }

    for (handle, handle_edge_x) in [(Handle::Start, sel_x0), (Handle::End, sel_x1)] {
        let (handle_width, color) = if highlighted_handle == Some(handle) {
            (4.0, rgb(0xe0f2fe))
        } else {
            (2.0, rgb(0x7dd3fc))
        };
        let handle_x = match handle {
            Handle::Start => handle_edge_x,
            Handle::End => (handle_edge_x - handle_width).max(sel_x0),
        };
        window.paint_quad(fill(
            Bounds {
                origin: point(px(handle_x), px(origin_y)),
                size: size(px(handle_width), px(height)),
            },
            color,
        ));
    }
}

fn paint_playhead(
    window: &mut Window,
    origin_x: f32,
    origin_y: f32,
    width: f32,
    height: f32,
    start_x: f32,
    draw_width: f32,
    pos: f32,
) {
    if pos.is_nan() {
        return;
    }
    let line_x = playhead_x(origin_x, width, start_x, draw_width, pos);
    window.paint_quad(fill(
        Bounds {
            origin: point(px(line_x), px(origin_y)),
            size: size(px(2.0), px(height)),
        },
        rgb(0xfacc15),
    ));
}

/// Renders the waveform canvas with optional selection overlay and playback playhead.
pub fn waveform_canvas(
    shared_audio: Arc<Mutex<AudioBufferState>>,
    is_recording: bool,
    selection: Option<Selection>,
    highlighted_handle: Option<Handle>,
    playhead: Option<f32>,
    bounds_slot: Rc<Cell<Option<Bounds<Pixels>>>>,
) -> impl gpui::IntoElement {
    canvas(
        move |bounds, _window, _cx| {
            bounds_slot.set(Some(bounds));
            bounds
        },
        move |bounds: Bounds<Pixels>, _prepaint_bounds, window: &mut Window, _cx| {
            let width: f32 = bounds.size.width.into();
            let height: f32 = bounds.size.height.into();
            if width <= 4.0 || height <= 4.0 {
                return;
            }

            let origin_x: f32 = bounds.origin.x.into();
            let origin_y: f32 = bounds.origin.y.into();

            paint_grid(window, origin_x, origin_y, width, height);

            let (start_x, draw_width) = draw_x_range(origin_x, width);
            if let Ok(state) = shared_audio.lock() {
                paint_peaks(
                    window,
                    &state.peaks,
                    is_recording,
                    origin_y,
                    height,
                    start_x,
                    draw_width,
                );
            }

            if !is_recording {
                if let Some(sel) = selection {
                    paint_selection_overlay(
                        window,
                        origin_y,
                        height,
                        start_x,
                        draw_width,
                        sel,
                        highlighted_handle,
                    );
                }

                if let Some(pos) = playhead {
                    paint_playhead(
                        window, origin_x, origin_y, width, height, start_x, draw_width, pos,
                    );
                }
            }
        },
    )
    .size_full()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_x_range_respects_padding_and_minimum_width() {
        let (start_x, draw_width) = draw_x_range(10.0, 208.0);
        assert!((start_x - 14.0).abs() < 1e-6);
        assert!((draw_width - 200.0).abs() < 1e-6);

        let (narrow_start, narrow_width) = draw_x_range(5.0, 6.0);
        assert!((narrow_start - 9.0).abs() < 1e-6);
        assert!((narrow_width - 1.0).abs() < 1e-6);
    }

    #[test]
    fn selection_x_bounds_normalizes_and_clamps() {
        let (x0, x1) = selection_x_bounds(14.0, 200.0, 0.75, 0.25);
        assert!((x0 - 64.0).abs() < 1e-6);
        assert!((x1 - 164.0).abs() < 1e-6);

        let (cx0, cx1) = selection_x_bounds(14.0, 200.0, -0.5, 1.5);
        assert!((cx0 - 14.0).abs() < 1e-6);
        assert!((cx1 - 214.0).abs() < 1e-6);
    }

    #[test]
    fn playhead_x_centers_and_clamps_within_canvas() {
        let mid = playhead_x(10.0, 208.0, 14.0, 200.0, 0.5);
        assert!((mid - 113.0).abs() < 1e-6);

        let left = playhead_x(10.0, 208.0, 14.0, 200.0, -0.5);
        assert!((left - 13.0).abs() < 1e-6);

        let right = playhead_x(10.0, 208.0, 14.0, 200.0, 1.5);
        assert!((right - 213.0).abs() < 1e-6);

        // When draw_width extends past right bound on ultra-narrow canvas, clamps to width - 2.
        let clamped_right = playhead_x(0.0, 5.0, 4.0, 1.0, 1.0);
        assert!((clamped_right - 3.0).abs() < 1e-6);
    }
}
