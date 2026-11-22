use gpui::TestAppContext;

use super::drivers;
use super::dsl::Dsl;

#[gpui::test]
fn quick_save_with_no_selection_writes_the_whole_recording(cx: &mut TestAppContext) {
    let mut dsl = Dsl::new(drivers::from_env(cx));

    // Given
    dsl.recording().records(3.0);

    // When
    dsl.saving().quick_saves();

    // Then
    dsl.saving().confirm_saved(0.0, 3.0);
}

#[gpui::test]
fn quick_save_with_a_selection_writes_only_the_selection(cx: &mut TestAppContext) {
    let mut dsl = Dsl::new(drivers::from_env(cx));

    // Given
    dsl.recording().records(4.0);
    dsl.selection().selects(1.0, 2.5);

    // When
    dsl.saving().quick_saves();

    // Then
    dsl.saving().confirm_saved(1.0, 2.5);
}

#[gpui::test]
fn a_normalised_export_peaks_at_minus_0_1_dbfs(cx: &mut TestAppContext) {
    let mut dsl = Dsl::new(drivers::from_env(cx));

    // Given
    dsl.recording().records(2.0);
    dsl.saving().turns_normalise_on();

    // When
    dsl.saving().quick_saves();

    // Then
    dsl.saving().confirm_peak_dbfs(-0.1);
}

#[gpui::test]
fn save_wav_writes_to_the_chosen_file_and_adds_wav(cx: &mut TestAppContext) {
    let mut dsl = Dsl::new(drivers::from_env(cx));

    // Given
    dsl.recording().records(2.0);

    // When
    dsl.saving().saves_as("kick");

    // Then
    dsl.saving().confirm_saved_file("kick.wav");
    dsl.status().confirm_says("Saved WAV to kick.wav");
}

#[gpui::test]
fn cancelling_the_save_dialog_saves_nothing(cx: &mut TestAppContext) {
    let mut dsl = Dsl::new(drivers::from_env(cx));

    // Given
    dsl.recording().records(2.0);

    // When
    dsl.saving().cancels_save();

    // Then
    dsl.saving().confirm_nothing_saved();
    dsl.status().confirm_says("Save cancelled");
}

#[gpui::test]
fn there_is_nothing_to_save_before_recording(cx: &mut TestAppContext) {
    let mut dsl = Dsl::new(drivers::from_env(cx));

    // When
    dsl.saving().quick_saves();

    // Then
    dsl.saving().confirm_nothing_saved();
}
