//! Stand-ins for the operating-system boundaries (ADR-0001): ScreenCaptureKit
//! capture and rodio playback. Each is a cheap handle to shared state, so the
//! app owns one clone and the driver keeps another to script and inspect it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use sampler::audio::{
    AudioBufferState, AudioSource, Capture, Playback, RunningAppInfo, SAMPLE_RATE,
};

/// How much the stub audio's level rises per second of Recording, so any
/// sample's level tells which moment of the Recording it came from.
pub const RAMP_PER_SEC: f32 = 0.1;

/// Interleaved stereo ramp audio covering `from_secs..to_secs`.
pub fn ramp(from_secs: f64, to_secs: f64) -> Vec<f32> {
    let first = (from_secs * SAMPLE_RATE as f64).round() as usize;
    let last = (to_secs * SAMPLE_RATE as f64).round() as usize;
    (first..last)
        .flat_map(|frame| {
            let level = frame as f32 / SAMPLE_RATE as f32 * RAMP_PER_SEC;
            [level, level]
        })
        .collect()
}

/// The seconds of the Recording that ramp audio spans, read back from its
/// first sample's level and its length.
pub fn ramp_span(interleaved: &[f32]) -> (f64, f64) {
    let first_level = interleaved.first().copied().unwrap_or(0.0);
    let start = f64::from(first_level / RAMP_PER_SEC);
    let frames = interleaved.len() / 2;
    (start, start + frames as f64 / SAMPLE_RATE as f64)
}

#[derive(Default)]
struct CaptureState {
    running_apps: Vec<RunningAppInfo>,
    next_pid: i32,
    recording: Option<Arc<Mutex<AudioBufferState>>>,
    source: Option<AudioSource>,
}

#[derive(Clone, Default)]
pub struct StubCapture(Arc<Mutex<CaptureState>>);

impl StubCapture {
    fn state(&self) -> MutexGuard<'_, CaptureState> {
        self.0.lock().expect("capture stub lock")
    }

    pub fn app_launches(&self, name: &str) {
        let mut state = self.state();
        state.next_pid += 1;
        let pid = 1000 + state.next_pid;
        state.running_apps.push(RunningAppInfo {
            pid,
            name: name.to_string(),
            bundle_id: format!("test.{}", name.to_lowercase()),
        });
    }

    pub fn app_quits(&self, name: &str) {
        self.state().running_apps.retain(|app| app.name != name);
    }

    /// Delivers `secs` more ramp audio into the Recording being captured.
    pub fn deliver(&self, secs: f64) {
        let state = self.state();
        let recording = state
            .recording
            .as_ref()
            .expect("audio delivered while the app isn't recording");
        let mut buffer = recording.lock().expect("recording lock");
        let start = buffer.duration_secs();
        buffer.push_interleaved_stereo(&ramp(start, start + secs));
    }

    pub fn source(&self) -> Option<AudioSource> {
        self.state().source.clone()
    }
}

impl Capture for StubCapture {
    fn running_apps(&self) -> Result<Vec<RunningAppInfo>, String> {
        Ok(self.state().running_apps.clone())
    }

    fn start(
        &mut self,
        recording: Arc<Mutex<AudioBufferState>>,
        source: &AudioSource,
    ) -> Result<(), String> {
        let mut state = self.state();
        if state.recording.is_none() {
            recording.lock().expect("recording lock").clear();
            state.recording = Some(recording);
            state.source = Some(source.clone());
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), String> {
        self.state().recording = None;
        Ok(())
    }

    fn is_recording(&self) -> bool {
        self.state().recording.is_some()
    }
}

/// What the app last asked the playback stub to play.
#[derive(Clone)]
pub struct PlayRequest {
    pub samples: Vec<f32>,
    pub clip_range: (f32, f32),
    pub loop_enabled: Arc<AtomicBool>,
}

impl PlayRequest {
    pub fn loops(&self) -> bool {
        self.loop_enabled.load(Ordering::Relaxed)
    }
}

#[derive(Default)]
struct PlaybackState {
    playing: bool,
    last_request: Option<PlayRequest>,
}

#[derive(Clone, Default)]
pub struct StubPlayback(Arc<Mutex<PlaybackState>>);

impl StubPlayback {
    fn state(&self) -> MutexGuard<'_, PlaybackState> {
        self.0.lock().expect("playback stub lock")
    }

    /// The last request, while still playing.
    pub fn now_playing(&self) -> Option<PlayRequest> {
        let state = self.state();
        state.playing.then(|| state.last_request.clone()).flatten()
    }
}

impl Playback for StubPlayback {
    fn play(
        &self,
        samples: Vec<f32>,
        clip_range: (f32, f32),
        loop_enabled: Arc<AtomicBool>,
    ) -> Result<(), String> {
        let mut state = self.state();
        state.playing = true;
        state.last_request = Some(PlayRequest {
            samples,
            clip_range,
            loop_enabled,
        });
        Ok(())
    }

    fn stop(&self) {
        self.state().playing = false;
    }

    fn is_playing(&self) -> bool {
        self.state().playing
    }

    fn playhead_position(&self) -> Option<f32> {
        None
    }

    fn take_error(&self) -> Option<String> {
        None
    }
}
