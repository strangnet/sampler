//! Smoke test: the real `SamplerApp` runs in a gpui test window with its OS
//! boundaries (ScreenCaptureKit, audio output, save folders) replaced.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use sampler::app::{Backends, SamplerApp};
use sampler::audio::{AudioBufferState, AudioSource, Capture, Playback, RunningAppInfo};

#[derive(Default)]
struct FakeCapture {
    recording: bool,
}

impl Capture for FakeCapture {
    fn running_apps(&self) -> Result<Vec<RunningAppInfo>, String> {
        Ok(Vec::new())
    }

    fn start(
        &mut self,
        _recording: Arc<Mutex<AudioBufferState>>,
        _source: &AudioSource,
    ) -> Result<(), String> {
        self.recording = true;
        Ok(())
    }

    fn stop(&mut self) -> Result<(), String> {
        self.recording = false;
        Ok(())
    }

    fn is_recording(&self) -> bool {
        self.recording
    }
}

struct SilentPlayback;

impl Playback for SilentPlayback {
    fn play(
        &self,
        _samples: Vec<f32>,
        _clip_range: (f32, f32),
        _loop_enabled: Arc<AtomicBool>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn stop(&self) {}

    fn is_playing(&self) -> bool {
        false
    }

    fn playhead_position(&self) -> Option<f32> {
        None
    }

    fn take_error(&self) -> Option<String> {
        None
    }
}

#[gpui::test]
fn the_app_opens_ready_to_record(cx: &mut gpui::TestAppContext) {
    // Given
    let save_folder = tempfile::tempdir().expect("temp dir");
    let backends = Backends {
        capture: Box::new(FakeCapture::default()),
        playback: Box::new(SilentPlayback),
        quick_save_dir: save_folder.path().to_path_buf(),
        save_dialog_dir: save_folder.path().to_path_buf(),
    };

    // When
    let (app, cx) = cx.add_window_view(|_window, cx| SamplerApp::new(backends, cx));

    // Then
    let status = app.read_with(cx, |app, _| app.status().to_string());
    assert_eq!(status, "Ready — choose a source and click Record");
}
