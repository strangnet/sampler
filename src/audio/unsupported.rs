//! Capture on platforms without a backend yet: lists no apps and refuses to record.

use std::sync::{Arc, Mutex};

use super::{AudioBufferState, AudioSource, Capture, RunningAppInfo};

const UNSUPPORTED: &str = "Recording isn't supported on this platform yet";

#[derive(Default)]
pub struct AudioRecorder;

impl AudioRecorder {
    pub fn new() -> Self {
        Self
    }
}

impl Capture for AudioRecorder {
    fn running_apps(&self) -> Result<Vec<RunningAppInfo>, String> {
        Ok(Vec::new())
    }

    fn start(
        &mut self,
        _recording: Arc<Mutex<AudioBufferState>>,
        _source: &AudioSource,
    ) -> Result<(), String> {
        Err(UNSUPPORTED.to_string())
    }

    fn stop(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn is_recording(&self) -> bool {
        false
    }
}
