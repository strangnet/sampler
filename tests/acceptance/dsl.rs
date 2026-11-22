//! Layer 2: one method per business action or check, named in the scenarios'
//! words. Only parameter handling, bridging state and delegation to the
//! driver live here: no assertions, no gpui.

/// Protocol driver contract. Check methods panic with a message naming the
/// actual and expected values, which fails the test.
pub trait Driver {
    fn launch_app(&mut self, name: &str);
    fn quit_app(&mut self, name: &str);
    fn refresh_sources(&mut self);
    fn choose_source(&mut self, label: &str);
    fn record(&mut self, secs: f64);
    fn clear_recording(&mut self);
    /// Starts recording and captures `secs` of audio, without stopping.
    fn start_recording_for(&mut self, secs: f64);
    /// Drags across the waveform from `from` to `to` seconds.
    fn drag_across_waveform(&mut self, from: f64, to: f64);
    /// Drags the Selection's end handle to `to` seconds.
    fn drag_selection_end_to(&mut self, to: f64);
    /// Drags from inside the Selection by `secs` (negative: leftwards).
    fn drag_selection_by(&mut self, secs: f64);
    fn press_escape(&mut self);
    fn press_play(&mut self);
    fn press_space(&mut self);
    fn press_loop(&mut self);
    fn press_normalize(&mut self);
    fn quick_save(&mut self);
    /// Saves with Save WAV…, choosing `name` in the save dialog's folder.
    fn save_as(&mut self, name: &str);
    fn cancel_save(&mut self);

    fn confirm_chosen_source(&mut self, label: &str);
    fn confirm_recorded_from(&mut self, label: &str);
    fn confirm_recording_length(&mut self, secs: f64);
    fn confirm_recording_empty(&mut self);
    fn confirm_selection(&mut self, from: f64, to: f64);
    fn confirm_no_selection(&mut self);
    fn confirm_playing(&mut self, from: f64, to: f64);
    fn confirm_looping(&mut self);
    fn confirm_stopped(&mut self);
    /// The one saved WAV holds the Recording from `from` to `to` seconds.
    fn confirm_saved(&mut self, from: f64, to: f64);
    fn confirm_saved_peak_dbfs(&mut self, dbfs: f64);
    fn confirm_saved_file(&mut self, name: &str);
    fn confirm_nothing_saved(&mut self);
    fn confirm_play_unavailable(&mut self);
    fn confirm_save_unavailable(&mut self);
    fn confirm_status(&mut self, text: &str);
}

/// What scenarios talk to. Create one per test: it opens the test's own
/// Sampler window.
pub struct Dsl<'a> {
    driver: Box<dyn Driver + 'a>,
}

impl<'a> Dsl<'a> {
    pub fn new(driver: Box<dyn Driver + 'a>) -> Self {
        Self { driver }
    }

    pub fn sources(&mut self) -> Sources<'_, 'a> {
        Sources(self)
    }

    pub fn recording(&mut self) -> Recording<'_, 'a> {
        Recording(self)
    }

    pub fn selection(&mut self) -> Selection<'_, 'a> {
        Selection(self)
    }

    pub fn playback(&mut self) -> Playback<'_, 'a> {
        Playback(self)
    }

    pub fn saving(&mut self) -> Saving<'_, 'a> {
        Saving(self)
    }

    pub fn status(&mut self) -> Status<'_, 'a> {
        Status(self)
    }
}

/// Audio sources: running applications, or all system audio.
pub struct Sources<'d, 'a>(&'d mut Dsl<'a>);

impl Sources<'_, '_> {
    pub fn app_is_running(self, name: &str) {
        self.0.driver.launch_app(name);
    }

    pub fn chooses(self, label: &str) {
        self.0.driver.choose_source(label);
    }

    pub fn app_quits(self, name: &str) {
        self.0.driver.quit_app(name);
    }

    pub fn refreshes(self) {
        self.0.driver.refresh_sources();
    }

    pub fn confirm_chosen(self, label: &str) {
        self.0.driver.confirm_chosen_source(label);
    }

    pub fn confirm_recorded_from(self, label: &str) {
        self.0.driver.confirm_recorded_from(label);
    }
}

/// The Recording: all audio captured in one capture session.
pub struct Recording<'d, 'a>(&'d mut Dsl<'a>);

impl Recording<'_, '_> {
    pub fn records(self, secs: f64) {
        self.0.driver.record(secs);
    }

    pub fn in_progress_for(self, secs: f64) {
        self.0.driver.start_recording_for(secs);
    }

    pub fn clears(self) {
        self.0.driver.clear_recording();
    }

    pub fn confirm_length(self, secs: f64) {
        self.0.driver.confirm_recording_length(secs);
    }

    pub fn confirm_empty(self) {
        self.0.driver.confirm_recording_empty();
    }
}

/// The Selection: the span of the Recording that Play and Save act on.
pub struct Selection<'d, 'a>(&'d mut Dsl<'a>);

impl Selection<'_, '_> {
    pub fn selects(self, from: f64, to: f64) {
        self.0.driver.drag_across_waveform(from, to);
    }

    pub fn drags_end_handle_to(self, to: f64) {
        self.0.driver.drag_selection_end_to(to);
    }

    pub fn drags_it_by(self, secs: f64) {
        self.0.driver.drag_selection_by(secs);
    }

    pub fn presses_escape(self) {
        self.0.driver.press_escape();
    }

    pub fn confirm_spans(self, from: f64, to: f64) {
        self.0.driver.confirm_selection(from, to);
    }

    pub fn confirm_none(self) {
        self.0.driver.confirm_no_selection();
    }
}

pub struct Playback<'d, 'a>(&'d mut Dsl<'a>);

impl Playback<'_, '_> {
    pub fn plays(self) {
        self.0.driver.press_play();
    }

    pub fn presses_space(self) {
        self.0.driver.press_space();
    }

    pub fn turns_loop_on(self) {
        self.0.driver.press_loop();
    }

    pub fn confirm_playing(self, from: f64, to: f64) {
        self.0.driver.confirm_playing(from, to);
    }

    pub fn confirm_looping(self) {
        self.0.driver.confirm_looping();
    }

    pub fn confirm_stopped(self) {
        self.0.driver.confirm_stopped();
    }

    pub fn confirm_unavailable(self) {
        self.0.driver.confirm_play_unavailable();
    }
}

/// Saving WAV files: Quick Save and Save WAV….
pub struct Saving<'d, 'a>(&'d mut Dsl<'a>);

impl Saving<'_, '_> {
    pub fn quick_saves(self) {
        self.0.driver.quick_save();
    }

    pub fn turns_normalise_on(self) {
        self.0.driver.press_normalize();
    }

    pub fn saves_as(self, name: &str) {
        self.0.driver.save_as(name);
    }

    pub fn cancels_save(self) {
        self.0.driver.cancel_save();
    }

    pub fn confirm_saved(self, from: f64, to: f64) {
        self.0.driver.confirm_saved(from, to);
    }

    pub fn confirm_peak_dbfs(self, dbfs: f64) {
        self.0.driver.confirm_saved_peak_dbfs(dbfs);
    }

    pub fn confirm_saved_file(self, name: &str) {
        self.0.driver.confirm_saved_file(name);
    }

    pub fn confirm_nothing_saved(self) {
        self.0.driver.confirm_nothing_saved();
    }

    pub fn confirm_unavailable(self) {
        self.0.driver.confirm_save_unavailable();
    }
}

/// The status line under the controls.
pub struct Status<'d, 'a>(&'d mut Dsl<'a>);

impl Status<'_, '_> {
    pub fn confirm_says(self, text: &str) {
        self.0.driver.confirm_status(text);
    }
}
