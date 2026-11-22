use gpui::TestAppContext;

use super::drivers;
use super::dsl::Dsl;

#[gpui::test]
fn play_with_no_selection_plays_the_whole_recording(cx: &mut TestAppContext) {
    let mut dsl = Dsl::new(drivers::from_env(cx));

    // Given
    dsl.recording().records(4.0);

    // When
    dsl.playback().plays();

    // Then
    dsl.playback().confirm_playing(0.0, 4.0);
    dsl.playback().confirm_playing_once();
    dsl.status().confirm_says("Playing recorded sample…");
}

#[gpui::test]
fn play_with_a_selection_plays_only_the_selection(cx: &mut TestAppContext) {
    let mut dsl = Dsl::new(drivers::from_env(cx));

    // Given
    dsl.recording().records(4.0);
    dsl.selection().selects(1.0, 2.0);

    // When
    dsl.playback().plays();

    // Then
    dsl.playback().confirm_playing(1.0, 2.0);
    dsl.status().confirm_says("Playing selected range…");
}

#[gpui::test]
fn loop_repeats_playback(cx: &mut TestAppContext) {
    let mut dsl = Dsl::new(drivers::from_env(cx));

    // Given
    dsl.recording().records(2.0);
    dsl.playback().presses_loop();

    // When
    dsl.playback().plays();

    // Then
    dsl.playback().confirm_looping();
}

#[gpui::test]
fn space_starts_playback(cx: &mut TestAppContext) {
    let mut dsl = Dsl::new(drivers::from_env(cx));

    // Given
    dsl.recording().records(2.0);

    // When
    dsl.playback().presses_space();

    // Then
    dsl.playback().confirm_playing(0.0, 2.0);
}

#[gpui::test]
fn space_stops_playback(cx: &mut TestAppContext) {
    let mut dsl = Dsl::new(drivers::from_env(cx));

    // Given
    dsl.recording().records(2.0);
    dsl.playback().plays();

    // When
    dsl.playback().presses_space();

    // Then
    dsl.playback().confirm_stopped();
    dsl.status().confirm_says("Playback stopped");
}
