use gpui::TestAppContext;

use super::drivers;
use super::dsl::Dsl;

#[gpui::test]
fn recording_captures_audio_from_the_chosen_source(cx: &mut TestAppContext) {
    let mut dsl = Dsl::new(drivers::from_env(cx));

    // Given
    dsl.sources().app_is_running("Music");

    // When
    dsl.sources().chooses("Music");
    dsl.recording().records(2.0);

    // Then
    dsl.sources().confirm_recorded_from("Music");
    dsl.recording().confirm_length(2.0);
    dsl.status().confirm_says("Recorded 2.00s from Music");
}

#[gpui::test]
fn a_closed_source_falls_back_to_all_system_audio(cx: &mut TestAppContext) {
    let mut dsl = Dsl::new(drivers::from_env(cx));

    // Given
    dsl.sources().app_is_running("Music");
    dsl.sources().chooses("Music");

    // When
    dsl.sources().app_quits("Music");
    dsl.sources().refreshes();

    // Then
    dsl.sources().confirm_chosen("All System Audio");
}

#[gpui::test]
fn recording_again_starts_a_fresh_recording(cx: &mut TestAppContext) {
    let mut dsl = Dsl::new(drivers::from_env(cx));

    // Given
    dsl.recording().records(2.0);
    dsl.selection().selects(0.5, 1.5);

    // When
    dsl.recording().records(1.0);

    // Then
    dsl.recording().confirm_length(1.0);
    dsl.selection().confirm_none();
}

#[gpui::test]
fn clear_empties_the_recording(cx: &mut TestAppContext) {
    let mut dsl = Dsl::new(drivers::from_env(cx));

    // Given
    dsl.recording().records(2.0);

    // When
    dsl.recording().clears();

    // Then
    dsl.recording().confirm_empty();
    dsl.playback().confirm_unavailable();
    dsl.saving().confirm_unavailable();
}
