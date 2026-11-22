//! Layer 3: protocol drivers. `GpuiDriver` hosts the real `SamplerApp` in a
//! gpui test window (ADR-0001) and talks to it with simulated input.

use std::path::{Path, PathBuf};

use gpui::{
    Entity, Modifiers, MouseButton, Pixels, Point, TestAppContext, VisualTestContext, point, px,
};
use sampler::WAVEFORM_PAD_X;
use sampler::app::{Backends, SamplerApp};
use tempfile::TempDir;

use super::dsl::Driver;
use super::stubs::{StubCapture, StubPlayback, ramp_span};

/// How far a span may be from what a scenario expects. Selections are made by
/// pointer, so they land within a pixel of the intended second.
const TOLERANCE_SECS: f64 = 0.01;

#[track_caller]
fn assert_span(what: &str, actual: (f64, f64), expected: (f64, f64)) {
    let close = (actual.0 - expected.0).abs() < TOLERANCE_SECS
        && (actual.1 - expected.1).abs() < TOLERANCE_SECS;
    assert!(
        close,
        "{what} spans {:.3}–{:.3} s, expected {:.3}–{:.3} s",
        actual.0, actual.1, expected.0, expected.1
    );
}

/// A saved WAV's samples, scaled back to -1.0..=1.0.
fn read_wav(path: &Path) -> Vec<f32> {
    hound::WavReader::open(path)
        .unwrap_or_else(|e| panic!("{} is not a readable WAV: {e}", path.display()))
        .into_samples::<i16>()
        .map(|s| f32::from(s.expect("WAV sample decodes")) / f32::from(i16::MAX))
        .collect()
}

/// Picks the driver named by `ACCEPTANCE_DRIVER` (default: `gpui`).
pub fn from_env(cx: &mut TestAppContext) -> Box<dyn Driver + '_> {
    match std::env::var("ACCEPTANCE_DRIVER").as_deref() {
        Err(_) | Ok("gpui") => Box::new(GpuiDriver::open(cx)),
        Ok(other) => panic!("unknown ACCEPTANCE_DRIVER {other:?}"),
    }
}

pub struct GpuiDriver<'a> {
    cx: &'a mut VisualTestContext,
    app: Entity<SamplerApp>,
    capture: StubCapture,
    playback: StubPlayback,
    save_folder: TempDir,
    /// Where Quick Save should write, inside `save_folder`.
    quick_save_dir: PathBuf,
}

impl<'a> GpuiDriver<'a> {
    /// Opens a Sampler window wired to fresh stubs and an empty save folder.
    pub fn open(cx: &'a mut TestAppContext) -> Self {
        let capture = StubCapture::default();
        let playback = StubPlayback::default();
        let save_folder = tempfile::tempdir().expect("temp save folder");
        let quick_save_dir = save_folder.path().join("Sampler");
        let backends = Backends {
            capture: Box::new(capture.clone()),
            playback: Box::new(playback.clone()),
            quick_save_dir: quick_save_dir.clone(),
            save_dialog_dir: save_folder.path().to_path_buf(),
        };
        let (app, cx) = cx.add_window_view(|_, cx| SamplerApp::new(backends, cx));
        Self {
            cx,
            app,
            capture,
            playback,
            save_folder,
            quick_save_dir,
        }
    }
}

impl GpuiDriver<'_> {
    /// Clicks the element tagged `selector`, then lets the app settle.
    fn click(&mut self, selector: &'static str) {
        let bounds = self
            .cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("nothing tagged {selector:?} is on screen"));
        self.cx.simulate_click(bounds.center(), Modifiers::none());
        self.cx.run_until_parked();
    }

    /// The on-screen point `secs` into the Recording, mid-height on the
    /// waveform.
    fn waveform_point(&mut self, secs: f64) -> Point<Pixels> {
        let bounds = self
            .cx
            .debug_bounds("waveform")
            .expect("the waveform is on screen");
        let length = self.read(|app| app.recording_secs());
        let drawn_width = f32::from(bounds.size.width) - 2.0 * WAVEFORM_PAD_X;
        let x = f32::from(bounds.origin.x) + WAVEFORM_PAD_X + (secs / length) as f32 * drawn_width;
        point(px(x), bounds.center().y)
    }

    fn drag(&mut self, from: Point<Pixels>, to: Point<Pixels>) {
        let none = Modifiers::none();
        self.cx.simulate_mouse_down(from, MouseButton::Left, none);
        self.cx.simulate_mouse_move(to, MouseButton::Left, none);
        self.cx.simulate_mouse_up(to, MouseButton::Left, none);
    }

    /// The Selection in seconds of the Recording.
    fn selection_secs(&mut self) -> Option<(f64, f64)> {
        let (selection, length) = self.read(|app| (app.selection(), app.recording_secs()));
        selection.map(|(a, b)| (f64::from(a) * length, f64::from(b) * length))
    }

    /// The one WAV saved so far, which must be in the quick-save folder.
    fn quick_saved_wav(&self) -> Vec<f32> {
        match self.saved_files().as_slice() {
            [path] if path.parent() == Some(self.quick_save_dir.as_path()) => read_wav(path),
            files => panic!(
                "expected one WAV in {}, found {files:?}",
                self.quick_save_dir.display()
            ),
        }
    }

    /// Every WAV file under the test's save folder.
    fn saved_files(&self) -> Vec<PathBuf> {
        fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, found);
                } else if path.extension().is_some_and(|ext| ext == "wav") {
                    found.push(path);
                }
            }
        }
        let mut found = Vec::new();
        walk(self.save_folder.path(), &mut found);
        found.sort();
        found
    }

    /// Reads what the app currently shows.
    fn read<R>(&self, f: impl FnOnce(&SamplerApp) -> R) -> R {
        self.app.read_with(&*self.cx, |app, _| f(app))
    }
}

impl Driver for GpuiDriver<'_> {
    /// The app only lists apps it has seen, so launching one means
    /// refreshing the source list afterwards.
    fn launch_app(&mut self, name: &str) {
        self.capture.app_launches(name);
        self.click("refresh-sources-btn");
    }

    fn quit_app(&mut self, name: &str) {
        self.capture.app_quits(name);
    }

    fn refresh_sources(&mut self) {
        self.click("refresh-sources-btn");
    }

    fn choose_source(&mut self, label: &str) {
        self.click("source-picker-btn");
        // gpui looks elements up by `&'static str`; one small leak per test.
        self.click(Box::leak(format!("source:{label}").into_boxed_str()));
    }

    fn record(&mut self, secs: f64) {
        self.start_recording_for(secs);
        self.click("record-btn");
    }

    fn clear_recording(&mut self) {
        self.click("clear-btn");
    }

    fn start_recording_for(&mut self, secs: f64) {
        self.click("record-btn");
        self.capture.deliver(secs);
        self.cx.run_until_parked();
    }

    fn drag_across_waveform(&mut self, from: f64, to: f64) {
        let (start, end) = (self.waveform_point(from), self.waveform_point(to));
        self.drag(start, end);
    }

    fn drag_selection_end_to(&mut self, to: f64) {
        let (_, end) = self.selection_secs().expect("a Selection to resize");
        let (grab, release) = (self.waveform_point(end), self.waveform_point(to));
        self.drag(grab, release);
    }

    fn drag_selection_by(&mut self, secs: f64) {
        let (start, end) = self.selection_secs().expect("a Selection to move");
        let middle = (start + end) / 2.0;
        let (grab, release) = (
            self.waveform_point(middle),
            self.waveform_point(middle + secs),
        );
        self.drag(grab, release);
    }

    fn press_escape(&mut self) {
        self.cx.simulate_keystrokes("escape");
    }

    fn press_play(&mut self) {
        self.click("play-btn");
    }

    fn press_space(&mut self) {
        self.cx.simulate_keystrokes("space");
    }

    fn press_loop(&mut self) {
        self.click("loop-btn");
    }

    fn press_normalize(&mut self) {
        self.click("normalize-btn");
    }

    fn quick_save(&mut self) {
        self.click("quick-save-btn");
    }

    fn save_as(&mut self, name: &str) {
        self.click("save-btn");
        let name = name.to_string();
        self.cx
            .simulate_new_path_selection(move |folder| Some(folder.join(name)));
        self.cx.run_until_parked();
    }

    fn cancel_save(&mut self) {
        self.click("save-btn");
        self.cx.simulate_new_path_selection(|_| None);
        self.cx.run_until_parked();
    }

    fn confirm_chosen_source(&mut self, label: &str) {
        let actual = self.read(|app| app.source_label());
        assert_eq!(actual, label, "chosen source");
    }

    fn confirm_recorded_from(&mut self, label: &str) {
        let source = self.capture.source().map(|source| source.label());
        assert_eq!(source.as_deref(), Some(label), "source captured from");
    }

    fn confirm_recording_length(&mut self, secs: f64) {
        let actual = self.read(|app| app.recording_secs());
        assert!(
            (actual - secs).abs() < 1e-6,
            "the Recording is {actual} s long, expected {secs} s"
        );
    }

    fn confirm_recording_empty(&mut self) {
        let actual = self.read(|app| app.recording_secs());
        assert_eq!(
            actual, 0.0,
            "the Recording is {actual} s long, expected empty"
        );
    }

    fn confirm_selection(&mut self, from: f64, to: f64) {
        let actual = self
            .selection_secs()
            .expect("expected a Selection, found none");
        assert_span("the Selection", actual, (from, to));
    }

    fn confirm_playing(&mut self, from: f64, to: f64) {
        let request = self.playback.now_playing().expect("nothing is playing");
        assert_span("what's playing", ramp_span(&request.samples), (from, to));
        let length = self.read(|app| app.recording_secs());
        let (a, b) = request.clip_range;
        let playhead_range = (f64::from(a) * length, f64::from(b) * length);
        assert_span("the playhead's range", playhead_range, (from, to));
    }

    fn confirm_looping(&mut self) {
        let request = self.playback.now_playing().expect("nothing is playing");
        assert!(request.loops(), "playback isn't looping");
    }

    fn confirm_playing_once(&mut self) {
        let request = self.playback.now_playing().expect("nothing is playing");
        assert!(
            !request.loops(),
            "playback is looping without Loop turned on"
        );
    }

    fn confirm_stopped(&mut self) {
        let playing = self
            .playback
            .now_playing()
            .map(|request| ramp_span(&request.samples));
        assert_eq!(playing, None, "still playing");
    }

    fn confirm_quick_saved(&mut self, from: f64, to: f64) {
        let wav = self.quick_saved_wav();
        assert_span("the quick-saved WAV", ramp_span(&wav), (from, to));
    }

    fn confirm_quick_saved_peak_dbfs(&mut self, dbfs: f64) {
        let peak = self
            .quick_saved_wav()
            .iter()
            .fold(0.0_f32, |peak, s| peak.max(s.abs()));
        let actual = 20.0 * f64::from(peak).log10();
        assert!(
            (actual - dbfs).abs() < 0.01,
            "the quick-saved WAV peaks at {actual:.3} dBFS, expected {dbfs} dBFS"
        );
    }

    fn confirm_file_holds(&mut self, name: &str, from: f64, to: f64) {
        let expected = self.save_folder.path().join(name);
        assert_eq!(self.saved_files(), vec![expected.clone()], "saved files");
        assert_span(name, ramp_span(&read_wav(&expected)), (from, to));
    }

    fn confirm_nothing_saved(&mut self) {
        assert_eq!(
            self.saved_files(),
            Vec::<PathBuf>::new(),
            "files were saved"
        );
    }

    fn confirm_no_selection(&mut self) {
        let actual = self.read(|app| app.selection());
        assert_eq!(actual, None, "expected no Selection");
    }

    /// Unavailable means pressing Play changes nothing.
    fn confirm_play_unavailable(&mut self) {
        let status = self.read(|app| app.status().to_string());
        self.click("play-btn");
        assert!(
            self.playback.now_playing().is_none(),
            "Play started playback"
        );
        let after = self.read(|app| app.status().to_string());
        assert_eq!(after, status, "pressing Play changed the status");
    }

    /// Unavailable means pressing Quick Save and Save WAV… changes nothing.
    fn confirm_save_unavailable(&mut self) {
        let status = self.read(|app| app.status().to_string());
        self.click("quick-save-btn");
        self.click("save-btn");
        assert!(
            !self.cx.did_prompt_for_new_path(),
            "Save WAV… opened the save dialog"
        );
        assert_eq!(
            self.saved_files(),
            Vec::<PathBuf>::new(),
            "files were saved"
        );
        let after = self.read(|app| app.status().to_string());
        assert_eq!(after, status, "pressing Save changed the status");
    }

    fn confirm_status(&mut self, text: &str) {
        let actual = self.read(|app| app.status().to_string());
        assert_eq!(actual, text, "status line");
    }
}
