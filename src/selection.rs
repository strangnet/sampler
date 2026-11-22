//! Pure Selection editing logic: hit-testing the waveform and the drag
//! interactions that create, resize, and move a Selection.
//!
//! Positions are normalized to the Recording (0.0 = start, 1.0 = end).

/// A Selection as normalized `(start, end)` with `start <= end`.
pub type Selection = (f32, f32);

/// A pointer position on the waveform: normalized `x` (unclamped, so it can
/// lie past either end of the Recording) and the drawn waveform `width` in
/// logical pixels, used to apply the pixel-based thresholds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaveformPos {
    pub x: f32,
    pub width: f32,
}

/// One of the two Selection handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    Start,
    End,
}

/// What lies under the pointer relative to the Selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionHit {
    Handle(Handle),
    Inside,
    Outside,
}

/// How close (in pixels) the pointer must be to a Selection handle to grab it.
pub const HANDLE_GRAB_PX: f32 = 6.0;

/// Pointer travel (and Selection width) in pixels below which a drag that
/// creates a Selection counts as a click and leaves no Selection.
pub const MIN_SELECTION_PX: f32 = 4.0;

/// Pointer travel in pixels below which a press on an existing Selection
/// counts as a click rather than a nudge.
const STILL_PRESS_PX: f32 = 0.5;

pub fn hit_test(selection: Option<Selection>, at: WaveformPos) -> SelectionHit {
    let Some((start, end)) = selection else {
        return SelectionHit::Outside;
    };
    let to_start = (at.x - start).abs() * at.width;
    let to_end = (at.x - end).abs() * at.width;
    if to_start <= HANDLE_GRAB_PX || to_end <= HANDLE_GRAB_PX {
        SelectionHit::Handle(if to_start <= to_end {
            Handle::Start
        } else {
            Handle::End
        })
    } else if at.x > start && at.x < end {
        SelectionHit::Inside
    } else {
        SelectionHit::Outside
    }
}

/// The kind of interaction a drag performs, decided by what was pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragKind {
    Create,
    Resize,
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DragMode {
    /// Shape a span between a fixed anchor and the pointer (plus the offset
    /// at which a handle was grabbed, so it doesn't jump to the pointer).
    /// For a resize the anchor is the opposite handle, so crossing it flips
    /// the handles.
    Span {
        anchor: f32,
        grab_offset: f32,
        kind: DragKind,
    },
    /// Slide the original Selection by the pointer's offset from the press.
    Move { original: Selection },
}

/// An in-progress pointer drag on the waveform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectionDrag {
    mode: DragMode,
    press: WaveformPos,
    /// Furthest the pointer has been from the press point, in pixels.
    max_travel_px: f32,
    /// Most recent pointer position.
    last_x: f32,
}

impl SelectionDrag {
    pub fn begin(selection: Option<Selection>, press: WaveformPos) -> Self {
        let mode = match (selection, hit_test(selection, press)) {
            (Some((start, end)), SelectionHit::Handle(handle)) => {
                let (grabbed, anchor) = match handle {
                    Handle::Start => (start, end),
                    Handle::End => (end, start),
                };
                DragMode::Span {
                    anchor,
                    grab_offset: grabbed - press.x,
                    kind: DragKind::Resize,
                }
            }
            (Some(original), SelectionHit::Inside) => DragMode::Move { original },
            _ => DragMode::Span {
                anchor: press.x.clamp(0.0, 1.0),
                grab_offset: 0.0,
                kind: DragKind::Create,
            },
        };
        Self {
            mode,
            press,
            max_travel_px: 0.0,
            last_x: press.x,
        }
    }

    pub fn kind(&self) -> DragKind {
        match self.mode {
            DragMode::Span { kind, .. } => kind,
            DragMode::Move { .. } => DragKind::Move,
        }
    }

    /// The handle being dragged during a resize (which side of the anchor it
    /// is on, so it follows flips); `None` when creating or moving.
    pub fn dragged_handle(&self) -> Option<Handle> {
        match self.mode {
            DragMode::Span {
                anchor,
                grab_offset,
                kind: DragKind::Resize,
            } => Some(if self.last_x + grab_offset < anchor {
                Handle::Start
            } else {
                Handle::End
            }),
            _ => None,
        }
    }

    pub fn update(&mut self, x: f32) -> Option<Selection> {
        self.last_x = x;
        let travel_px = (x - self.press.x).abs() * self.press.width;
        self.max_travel_px = self.max_travel_px.max(travel_px);
        match self.mode {
            DragMode::Span {
                anchor,
                grab_offset,
                ..
            } => {
                let dragged = (x + grab_offset).clamp(0.0, 1.0);
                Some((anchor.min(dragged), anchor.max(dragged)))
            }
            DragMode::Move {
                original: (start, end),
            } => {
                let len = end - start;
                let new_start = (start + x - self.press.x).clamp(0.0, 1.0 - len);
                Some((new_start, new_start + len))
            }
        }
    }

    /// Ends the drag. A click or a Selection narrower than
    /// [`MIN_SELECTION_PX`] yields no Selection. Creating needs at least
    /// [`MIN_SELECTION_PX`] of travel to count as a drag; on an existing
    /// Selection any movement does, so small nudges apply.
    pub fn finish(mut self, x: f32) -> Option<Selection> {
        let selection = self.update(x);
        let click_threshold_px = match self.kind() {
            DragKind::Create => MIN_SELECTION_PX,
            DragKind::Resize | DragKind::Move => STILL_PRESS_PX,
        };
        if self.max_travel_px < click_threshold_px {
            return None;
        }
        let width = self.press.width;
        selection.filter(|(start, end)| (end - start) * width >= MIN_SELECTION_PX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f32 = 1000.0;

    fn at(x: f32) -> WaveformPos {
        WaveformPos { x, width: W }
    }

    #[track_caller]
    fn assert_sel(actual: Option<Selection>, expected: Selection) {
        let (a0, a1) = actual.expect("expected a Selection");
        assert!(
            (a0 - expected.0).abs() < 1e-5 && (a1 - expected.1).abs() < 1e-5,
            "{:?} != {expected:?}",
            (a0, a1)
        );
    }

    #[test]
    fn hit_test_grabs_handles_within_six_pixels() {
        let sel = Some((0.2, 0.6));
        assert_eq!(hit_test(sel, at(0.2)), SelectionHit::Handle(Handle::Start));
        assert_eq!(
            hit_test(sel, at(0.195)),
            SelectionHit::Handle(Handle::Start)
        );
        assert_eq!(
            hit_test(sel, at(0.205)),
            SelectionHit::Handle(Handle::Start)
        );
        assert_eq!(hit_test(sel, at(0.595)), SelectionHit::Handle(Handle::End));
        assert_eq!(hit_test(sel, at(0.605)), SelectionHit::Handle(Handle::End));
        assert_eq!(hit_test(sel, at(0.193)), SelectionHit::Outside);
        assert_eq!(hit_test(sel, at(0.4)), SelectionHit::Inside);
        assert_eq!(hit_test(sel, at(0.9)), SelectionHit::Outside);
        assert_eq!(hit_test(None, at(0.4)), SelectionHit::Outside);
    }

    #[test]
    fn hit_test_small_selection_picks_nearer_handle_and_cannot_be_moved() {
        // 10 px wide: both grab zones cover the whole inside.
        let sel = Some((0.5, 0.51));
        assert_eq!(
            hit_test(sel, at(0.503)),
            SelectionHit::Handle(Handle::Start)
        );
        assert_eq!(hit_test(sel, at(0.507)), SelectionHit::Handle(Handle::End));
        assert_eq!(
            hit_test(sel, at(0.498)),
            SelectionHit::Handle(Handle::Start)
        );
        assert_eq!(hit_test(sel, at(0.512)), SelectionHit::Handle(Handle::End));
    }

    #[test]
    fn dragging_a_handle_resizes_and_flips_past_the_other_handle() {
        let mut drag = SelectionDrag::begin(Some((0.2, 0.6)), at(0.6));
        assert_eq!(drag.update(0.7), Some((0.2, 0.7)));
        // Past the start handle: the dragged handle becomes the start.
        assert_eq!(drag.update(0.1), Some((0.1, 0.2)));
        assert_eq!(drag.finish(0.1), Some((0.1, 0.2)));

        let drag = SelectionDrag::begin(Some((0.2, 0.6)), at(0.2));
        assert_eq!(drag.finish(0.8), Some((0.6, 0.8)));
    }

    #[test]
    fn dragging_inside_moves_at_same_length_and_stops_at_recording_ends() {
        let mut drag = SelectionDrag::begin(Some((0.2, 0.4)), at(0.3));
        assert_eq!(drag.update(0.35), Some((0.25, 0.45)));
        assert_eq!(drag.update(0.95), Some((0.8, 1.0)));
        assert_eq!(drag.update(-0.5), Some((0.0, 0.2)));
        assert_eq!(drag.finish(0.4), Some((0.3, 0.5)));
    }

    #[test]
    fn a_still_press_clears_the_selection_anywhere() {
        let sel = Some((0.2, 0.6));
        for press in [0.2, 0.4, 0.6, 0.9] {
            let drag = SelectionDrag::begin(sel, at(press));
            assert_eq!(drag.finish(press), None, "press at {press}");
        }
        assert_eq!(SelectionDrag::begin(None, at(0.5)).finish(0.5), None);
    }

    #[test]
    fn a_short_drag_outside_the_selection_counts_as_a_click() {
        let drag = SelectionDrag::begin(Some((0.2, 0.6)), at(0.9));
        assert_eq!(drag.finish(0.903), None);
    }

    #[test]
    fn small_nudges_on_a_handle_or_inside_the_selection_apply() {
        let sel = Some((0.2, 0.6));
        assert_sel(
            SelectionDrag::begin(sel, at(0.6)).finish(0.602),
            (0.2, 0.602),
        );
        assert_sel(
            SelectionDrag::begin(sel, at(0.4)).finish(0.402),
            (0.202, 0.602),
        );
    }

    #[test]
    fn a_resize_that_ends_under_four_pixels_wide_removes_the_selection() {
        let drag = SelectionDrag::begin(Some((0.2, 0.6)), at(0.6));
        assert_eq!(drag.finish(0.203), None);
    }

    #[test]
    fn dragging_outside_creates_a_new_selection() {
        let mut drag = SelectionDrag::begin(Some((0.2, 0.4)), at(0.8));
        assert_eq!(drag.update(0.9), Some((0.8, 0.9)));
        assert_eq!(drag.finish(0.7), Some((0.7, 0.8)));
    }

    #[test]
    fn drag_kind_follows_what_was_pressed() {
        let sel = Some((0.2, 0.6));
        assert_eq!(SelectionDrag::begin(sel, at(0.2)).kind(), DragKind::Resize);
        assert_eq!(SelectionDrag::begin(sel, at(0.6)).kind(), DragKind::Resize);
        assert_eq!(SelectionDrag::begin(sel, at(0.4)).kind(), DragKind::Move);
        assert_eq!(SelectionDrag::begin(sel, at(0.9)).kind(), DragKind::Create);
        assert_eq!(SelectionDrag::begin(None, at(0.4)).kind(), DragKind::Create);
    }

    #[test]
    fn dragging_away_and_back_near_the_press_point_is_not_a_click() {
        let mut drag = SelectionDrag::begin(Some((0.2, 0.4)), at(0.3));
        drag.update(0.5);
        assert_sel(drag.finish(0.302), (0.202, 0.402));
    }

    #[test]
    fn grabbing_a_handle_off_centre_moves_it_by_the_pointer_offset_without_jumping() {
        // Pressed 5 px right of the end handle.
        let mut drag = SelectionDrag::begin(Some((0.2, 0.6)), at(0.605));
        assert_sel(drag.update(0.705), (0.2, 0.7));
        assert_sel(drag.update(0.105), (0.1, 0.2));
    }

    #[test]
    fn dragged_handle_tracks_flips_and_is_none_unless_resizing() {
        let sel = Some((0.2, 0.6));
        let mut drag = SelectionDrag::begin(sel, at(0.6));
        assert_eq!(drag.dragged_handle(), Some(Handle::End));
        drag.update(0.1);
        assert_eq!(drag.dragged_handle(), Some(Handle::Start));
        drag.update(1.5);
        assert_eq!(drag.dragged_handle(), Some(Handle::End));

        assert_eq!(SelectionDrag::begin(sel, at(0.4)).dragged_handle(), None);
        assert_eq!(SelectionDrag::begin(sel, at(0.9)).dragged_handle(), None);
    }
}
