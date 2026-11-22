use gpui::TestAppContext;

use super::drivers;
use super::dsl::Dsl;

#[gpui::test]
fn dragging_across_the_waveform_creates_a_selection(cx: &mut TestAppContext) {
    let mut dsl = Dsl::new(drivers::from_env(cx));

    // Given
    dsl.recording().records(4.0);

    // When
    dsl.selection().selects(1.0, 3.0);

    // Then
    dsl.selection().confirm_spans(1.0, 3.0);
}

#[gpui::test]
fn dragging_a_selection_handle_resizes_it(cx: &mut TestAppContext) {
    let mut dsl = Dsl::new(drivers::from_env(cx));

    // Given
    dsl.recording().records(4.0);
    dsl.selection().selects(1.0, 3.0);

    // When
    dsl.selection().drags_end_handle_to(3.5);

    // Then
    dsl.selection().confirm_spans(1.0, 3.5);
}

#[gpui::test]
fn dragging_inside_a_selection_moves_it_at_the_same_length(cx: &mut TestAppContext) {
    let mut dsl = Dsl::new(drivers::from_env(cx));

    // Given
    dsl.recording().records(4.0);
    dsl.selection().selects(1.0, 2.0);

    // When
    dsl.selection().drags_it_by(1.5);

    // Then
    dsl.selection().confirm_spans(2.5, 3.5);
}

#[gpui::test]
fn escape_clears_the_selection(cx: &mut TestAppContext) {
    let mut dsl = Dsl::new(drivers::from_env(cx));

    // Given
    dsl.recording().records(4.0);
    dsl.selection().selects(1.0, 2.0);

    // When
    dsl.selection().presses_escape();

    // Then
    dsl.selection().confirm_none();
}

#[gpui::test]
fn the_selection_cannot_be_changed_while_recording(cx: &mut TestAppContext) {
    let mut dsl = Dsl::new(drivers::from_env(cx));

    // Given
    dsl.recording().in_progress_for(2.0);

    // When
    dsl.selection().selects(0.5, 1.5);

    // Then
    dsl.selection().confirm_none();
}
