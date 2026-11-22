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
    use rstest::rstest;

    const W: f32 = 1000.0;
    const SEL: Option<Selection> = Some((0.2, 0.6));

    fn at(x: f32) -> WaveformPos {
        WaveformPos { x, width: W }
    }

    /// Pixel `n` on a 1024 px waveform: every whole pixel is exact in f32,
    /// so tests can land exactly on a threshold.
    fn x_px(n: f32) -> f32 {
        n / 1024.0
    }

    fn at_px(n: f32) -> WaveformPos {
        WaveformPos {
            x: x_px(n),
            width: 1024.0,
        }
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

    #[rstest]
    #[case::on_the_start_handle(0.2, SelectionHit::Handle(Handle::Start))]
    #[case::five_px_before_the_start_handle(0.195, SelectionHit::Handle(Handle::Start))]
    #[case::five_px_after_the_start_handle(0.205, SelectionHit::Handle(Handle::Start))]
    #[case::five_px_before_the_end_handle(0.595, SelectionHit::Handle(Handle::End))]
    #[case::five_px_after_the_end_handle(0.605, SelectionHit::Handle(Handle::End))]
    #[case::seven_px_before_the_start_handle(0.193, SelectionHit::Outside)]
    #[case::between_the_handles(0.4, SelectionHit::Inside)]
    #[case::past_the_end_handle(0.9, SelectionHit::Outside)]
    fn a_press_is_classified_by_where_it_lands(#[case] x: f32, #[case] expected: SelectionHit) {
        // When
        let hit = hit_test(SEL, at(x));

        // Then
        assert_eq!(hit, expected);
    }

    #[test]
    fn without_a_selection_every_press_is_outside() {
        // When
        let hit = hit_test(None, at(0.4));

        // Then
        assert_eq!(hit, SelectionHit::Outside);
    }

    #[test]
    fn a_press_exactly_six_pixels_from_a_handle_grabs_it() {
        // Given
        let selection = Some((x_px(256.0), x_px(512.0)));

        // When
        let hit = hit_test(selection, at_px(262.0));

        // Then
        assert_eq!(hit, SelectionHit::Handle(Handle::Start));
    }

    // 10 px wide: both grab zones cover the whole inside, so it cannot be moved.
    #[rstest]
    #[case::inside_near_the_start(0.503, Handle::Start)]
    #[case::inside_near_the_end(0.507, Handle::End)]
    #[case::just_before_the_start(0.498, Handle::Start)]
    #[case::just_past_the_end(0.512, Handle::End)]
    fn a_narrow_selection_is_grabbed_by_its_nearer_handle(
        #[case] x: f32,
        #[case] expected: Handle,
    ) {
        // Given
        let selection = Some((0.5, 0.51));

        // When
        let hit = hit_test(selection, at(x));

        // Then
        assert_eq!(hit, SelectionHit::Handle(expected));
    }

    #[test]
    fn a_press_midway_between_close_handles_grabs_the_start_handle() {
        // Given
        let selection = Some((x_px(500.0), x_px(510.0)));

        // When
        let hit = hit_test(selection, at_px(505.0));

        // Then
        assert_eq!(hit, SelectionHit::Handle(Handle::Start));
    }

    #[rstest]
    #[case::the_start_handle(SEL, 0.2, DragKind::Resize)]
    #[case::the_end_handle(SEL, 0.6, DragKind::Resize)]
    #[case::inside_the_selection(SEL, 0.4, DragKind::Move)]
    #[case::outside_the_selection(SEL, 0.9, DragKind::Create)]
    #[case::with_no_selection(None, 0.4, DragKind::Create)]
    fn the_drag_kind_follows_what_was_pressed(
        #[case] selection: Option<Selection>,
        #[case] press: f32,
        #[case] expected: DragKind,
    ) {
        // When
        let drag = SelectionDrag::begin(selection, at(press));

        // Then
        assert_eq!(drag.kind(), expected);
    }

    #[test]
    fn dragging_a_handle_resizes_the_selection() {
        // Given
        let mut drag = SelectionDrag::begin(SEL, at(0.6));

        // When
        let selection = drag.update(0.7);

        // Then
        assert_sel(selection, (0.2, 0.7));
    }

    #[rstest]
    #[case::the_end_handle_past_the_start(0.6, 0.1, (0.1, 0.2))]
    #[case::the_start_handle_past_the_end(0.2, 0.8, (0.6, 0.8))]
    fn a_handle_dragged_past_the_other_flips_them(
        #[case] press: f32,
        #[case] release: f32,
        #[case] expected: Selection,
    ) {
        // Given
        let drag = SelectionDrag::begin(SEL, at(press));

        // When
        let selection = drag.finish(release);

        // Then
        assert_sel(selection, expected);
    }

    #[rstest]
    #[case::from_its_initial_side(0.6, Handle::End)]
    #[case::across_the_other_handle(0.1, Handle::Start)]
    #[case::past_the_end_of_the_recording(1.5, Handle::End)]
    fn the_dragged_handle_follows_the_pointer(#[case] pointer: f32, #[case] expected: Handle) {
        // Given
        let mut drag = SelectionDrag::begin(SEL, at(0.6));

        // When
        drag.update(pointer);

        // Then
        assert_eq!(drag.dragged_handle(), Some(expected));
    }

    #[rstest]
    #[case::when_moving(0.4)]
    #[case::when_creating(0.9)]
    fn only_a_resize_has_a_dragged_handle(#[case] press: f32) {
        // When
        let drag = SelectionDrag::begin(SEL, at(press));

        // Then
        assert_eq!(drag.dragged_handle(), None);
    }

    #[rstest]
    #[case::towards_the_end(0.705, (0.2, 0.7))]
    #[case::across_the_start_handle(0.105, (0.1, 0.2))]
    fn a_handle_grabbed_off_centre_keeps_its_offset_from_the_pointer(
        #[case] pointer: f32,
        #[case] expected: Selection,
    ) {
        // Given: pressed 5 px right of the end handle
        let mut drag = SelectionDrag::begin(SEL, at(0.605));

        // When
        let selection = drag.update(pointer);

        // Then
        assert_sel(selection, expected);
    }

    #[rstest]
    #[case::past_the_end(1.4, (0.2, 1.0))]
    #[case::past_the_start(-0.3, (0.0, 0.2))]
    fn resizing_past_either_end_of_the_recording_stops_at_the_end(
        #[case] pointer: f32,
        #[case] expected: Selection,
    ) {
        // Given
        let mut drag = SelectionDrag::begin(SEL, at(0.6));

        // When
        let selection = drag.update(pointer);

        // Then
        assert_sel(selection, expected);
    }

    #[test]
    fn a_resize_that_ends_under_four_pixels_wide_removes_the_selection() {
        // Given
        let drag = SelectionDrag::begin(SEL, at(0.6));

        // When
        let selection = drag.finish(0.203);

        // Then
        assert_eq!(selection, None);
    }

    #[test]
    fn a_resize_that_ends_exactly_four_pixels_wide_keeps_the_selection() {
        // Given
        let drag = SelectionDrag::begin(Some((x_px(256.0), x_px(512.0))), at_px(512.0));

        // When
        let selection = drag.finish(x_px(260.0));

        // Then
        assert_eq!(selection, Some((x_px(256.0), x_px(260.0))));
    }

    #[test]
    fn dragging_inside_moves_the_selection_at_the_same_length() {
        // Given
        let drag = SelectionDrag::begin(Some((0.2, 0.4)), at(0.3));

        // When
        let selection = drag.finish(0.4);

        // Then
        assert_sel(selection, (0.3, 0.5));
    }

    #[rstest]
    #[case::at_the_end(0.95, (0.8, 1.0))]
    #[case::at_the_start(-0.5, (0.0, 0.2))]
    fn a_moved_selection_stops_at_the_recording_ends(
        #[case] pointer: f32,
        #[case] expected: Selection,
    ) {
        // Given
        let mut drag = SelectionDrag::begin(Some((0.2, 0.4)), at(0.3));

        // When
        let selection = drag.update(pointer);

        // Then
        assert_sel(selection, expected);
    }

    #[rstest]
    #[case::on_the_end_handle(0.6, 0.602, (0.2, 0.602))]
    #[case::inside_the_selection(0.4, 0.402, (0.202, 0.602))]
    fn a_small_nudge_on_the_selection_applies(
        #[case] press: f32,
        #[case] release: f32,
        #[case] expected: Selection,
    ) {
        // Given
        let drag = SelectionDrag::begin(SEL, at(press));

        // When
        let selection = drag.finish(release);

        // Then
        assert_sel(selection, expected);
    }

    #[rstest]
    #[case::on_the_start_handle(SEL, 0.2)]
    #[case::inside_the_selection(SEL, 0.4)]
    #[case::on_the_end_handle(SEL, 0.6)]
    #[case::outside_the_selection(SEL, 0.9)]
    #[case::with_no_selection(None, 0.5)]
    fn a_still_press_clears_the_selection(
        #[case] selection: Option<Selection>,
        #[case] press: f32,
    ) {
        // Given
        let drag = SelectionDrag::begin(selection, at(press));

        // When
        let selection = drag.finish(press);

        // Then
        assert_eq!(selection, None);
    }

    #[test]
    fn a_short_drag_outside_the_selection_counts_as_a_click() {
        // Given
        let drag = SelectionDrag::begin(SEL, at(0.9));

        // When
        let selection = drag.finish(0.903);

        // Then
        assert_eq!(selection, None);
    }

    #[test]
    fn exactly_four_pixels_of_travel_creates_a_selection() {
        // Given
        let drag = SelectionDrag::begin(None, at_px(500.0));

        // When
        let selection = drag.finish(x_px(504.0));

        // Then
        assert_eq!(selection, Some((x_px(500.0), x_px(504.0))));
    }

    #[test]
    fn dragging_away_and_back_near_the_press_point_is_not_a_click() {
        // Given
        let mut drag = SelectionDrag::begin(Some((0.2, 0.4)), at(0.3));
        drag.update(0.5);

        // When
        let selection = drag.finish(0.302);

        // Then
        assert_sel(selection, (0.202, 0.402));
    }

    #[rstest]
    #[case::rightwards(0.9, (0.8, 0.9))]
    #[case::leftwards(0.7, (0.7, 0.8))]
    fn dragging_outside_creates_a_new_selection_from_the_press_point(
        #[case] release: f32,
        #[case] expected: Selection,
    ) {
        // Given
        let drag = SelectionDrag::begin(Some((0.2, 0.4)), at(0.8));

        // When
        let selection = drag.finish(release);

        // Then
        assert_sel(selection, expected);
    }

    #[test]
    fn creating_from_the_padding_before_the_recording_starts_at_zero() {
        // Given: 3 px into the left padding, before the Recording starts
        let drag = SelectionDrag::begin(None, at(-0.003));

        // When
        let selection = drag.finish(0.1);

        // Then
        assert_sel(selection, (0.0, 0.1));
    }
}
