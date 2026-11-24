//! Capture through ScreenCaptureKit (macOS).

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use screencapturekit::prelude::*;
use screencapturekit::stream::delegate_trait::StreamCallbacks;

use super::{AudioBufferState, AudioSource, CHANNELS, Capture, RunningAppInfo, SAMPLE_RATE};

pub fn list_running_apps() -> Result<Vec<RunningAppInfo>, String> {
    let content = SCShareableContent::get().map_err(|e| {
        format!("Failed to access screen/audio content (check Screen Recording permission): {e}")
    })?;

    let self_pid = std::process::id() as i32;
    let mut seen_pids = HashSet::new();
    let mut apps = Vec::new();

    for app in content.applications() {
        let pid = app.process_id();
        if pid == self_pid || !seen_pids.insert(pid) {
            continue;
        }

        let name = app.application_name();
        if name.trim().is_empty() {
            continue;
        }

        apps.push(RunningAppInfo {
            pid,
            name,
            bundle_id: app.bundle_identifier(),
        });
    }

    apps.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.pid.cmp(&b.pid))
    });

    Ok(apps)
}

fn bytes_to_f32_cow(bytes: &[u8]) -> Cow<'_, [f32]> {
    let aligned_len = bytes.len() - (bytes.len() % 4);
    let trimmed = &bytes[..aligned_len];
    if trimmed.is_empty() {
        return Cow::Borrowed(&[]);
    }
    match bytemuck::try_cast_slice(trimmed) {
        Ok(floats) => Cow::Borrowed(floats),
        Err(_) => Cow::Owned(
            trimmed
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
                .collect(),
        ),
    }
}

struct AudioOutputHandler {
    shared: Arc<Mutex<AudioBufferState>>,
}

impl SCStreamOutputTrait for AudioOutputHandler {
    fn did_output_sample_buffer(&self, sample: CMSampleBuffer, of_type: SCStreamOutputType) {
        if of_type != SCStreamOutputType::Audio {
            return;
        }

        let Some(abl) = sample.audio_buffer_list() else {
            return;
        };

        let buffers: Vec<_> = abl.iter().collect();
        if buffers.is_empty() {
            return;
        }

        let mut interleaved = Vec::new();

        if buffers.len() >= 2 {
            // Non-interleaved (planar) stereo: buffer 0 = Left, buffer 1 = Right
            let left = bytes_to_f32_cow(buffers[0].data());
            let right = bytes_to_f32_cow(buffers[1].data());
            let frames = left.len().min(right.len());
            if frames > 0 {
                interleaved.reserve(frames * 2);
                for i in 0..frames {
                    interleaved.push(left[i]);
                    interleaved.push(right[i]);
                }
            }
        } else {
            let floats = bytes_to_f32_cow(buffers[0].data());
            if !floats.is_empty() {
                if buffers[0].number_channels == 1 {
                    interleaved.reserve(floats.len() * 2);
                    for &s in floats.iter() {
                        interleaved.push(s);
                        interleaved.push(s);
                    }
                } else {
                    interleaved.extend_from_slice(&floats);
                }
            }
        }

        if !interleaved.is_empty()
            && let Ok(mut state) = self.shared.lock()
        {
            state.push_interleaved_stereo(&interleaved);
        }
    }
}

pub struct AudioRecorder {
    stream: Option<SCStream>,
}

impl Default for AudioRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioRecorder {
    pub fn new() -> Self {
        Self { stream: None }
    }
}

impl Capture for AudioRecorder {
    fn running_apps(&self) -> Result<Vec<RunningAppInfo>, String> {
        list_running_apps()
    }

    fn is_recording(&self) -> bool {
        self.stream.is_some()
    }

    fn start(
        &mut self,
        shared: Arc<Mutex<AudioBufferState>>,
        source: &AudioSource,
    ) -> Result<(), String> {
        if self.stream.is_some() {
            return Ok(());
        }

        let content = SCShareableContent::get().map_err(|e| {
            format!(
                "Failed to access screen/audio content (check Screen Recording permission): {e}"
            )
        })?;

        let displays = content.displays();
        let display = displays
            .first()
            .ok_or_else(|| "No display found for ScreenCaptureKit audio capture".to_string())?;

        let filter = match source {
            AudioSource::AllSystemAudio => SCContentFilter::create()
                .with_display(display)
                .with_excluding_windows(&[])
                .build(),
            AudioSource::Application(info) => {
                let apps = content.applications();
                let target = apps
                    .iter()
                    .find(|app| app.process_id() == info.pid)
                    .or_else(|| {
                        if info.bundle_id.is_empty() {
                            None
                        } else {
                            apps.iter()
                                .find(|app| app.bundle_identifier() == info.bundle_id)
                        }
                    })
                    .ok_or_else(|| {
                        format!(
                            "Application '{}' is no longer running. Refresh sources and select an active app.",
                            info.name
                        )
                    })?;

                SCContentFilter::create()
                    .with_display(display)
                    .with_including_applications(&[target], &[])
                    .build()
            }
        };

        let config = SCStreamConfiguration::new()
            .with_width(2)
            .with_height(2)
            .with_captures_audio(true)
            .with_excludes_current_process_audio(true)
            .with_sample_rate(SAMPLE_RATE as i32)
            .with_channel_count(CHANNELS as i32);

        if let Ok(mut state) = shared.lock() {
            state.clear();
        }

        let error_shared = shared.clone();
        let delegate = StreamCallbacks::new().on_error(move |err| {
            if let Ok(mut state) = error_shared.lock() {
                state.error = Some(format!("Audio stream error: {err}"));
            }
        });

        let mut stream = SCStream::new_with_delegate(&filter, &config, delegate);
        stream.add_output_handler(AudioOutputHandler { shared }, SCStreamOutputType::Audio);

        stream
            .start_capture()
            .map_err(|e| format!("Could not start audio capture: {e}"))?;

        self.stream = Some(stream);
        Ok(())
    }

    fn stop(&mut self) -> Result<(), String> {
        if let Some(stream) = self.stream.take() {
            stream
                .stop_capture()
                .map_err(|e| format!("Failed to stop capture cleanly: {e}"))?;
        }
        Ok(())
    }
}

impl Drop for AudioRecorder {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    const DECODED: [f32; 5] = [0.0, -0.5, 0.25, 1.0, -0.875];

    /// A 4-byte-aligned buffer holding `DECODED` as native-endian bytes,
    /// `offset` bytes in and followed by `trailing` stray bytes, plus the
    /// byte range covering them.
    fn capture_buffer(offset: usize, trailing: usize) -> (Vec<u32>, std::ops::Range<usize>) {
        let payload: Vec<u8> = DECODED.iter().flat_map(|f| f.to_ne_bytes()).collect();
        let len = offset + payload.len() + trailing;
        let mut words = vec![0_u32; len.div_ceil(4)];
        let bytes: &mut [u8] = bytemuck::cast_slice_mut(&mut words);
        bytes[offset..offset + payload.len()].copy_from_slice(&payload);
        bytes[offset + payload.len()..len].fill(0xAA);
        (words, offset..len)
    }

    #[test]
    fn aligned_capture_bytes_decode_without_copying() {
        // Given
        let (words, range) = capture_buffer(0, 2);
        let bytes: &[u8] = bytemuck::cast_slice(&words);

        // When
        let decoded = bytes_to_f32_cow(&bytes[range]);

        // Then
        assert!(
            matches!(decoded, Cow::Borrowed(_)),
            "aligned bytes were copied"
        );
        assert_eq!(decoded.as_ref(), &DECODED);
    }

    #[test]
    fn misaligned_capture_bytes_decode_into_a_copy() {
        // Given
        let (words, range) = capture_buffer(1, 3);
        let bytes: &[u8] = bytemuck::cast_slice(&words);

        // When
        let decoded = bytes_to_f32_cow(&bytes[range]);

        // Then
        assert!(
            matches!(decoded, Cow::Owned(_)),
            "misaligned bytes were borrowed"
        );
        assert_eq!(decoded.as_ref(), &DECODED);
    }

    #[rstest]
    #[case::no_bytes(0)]
    #[case::less_than_one_sample(3)]
    fn fewer_than_four_capture_bytes_decode_to_nothing(#[case] len: usize) {
        // Given
        let bytes = [0xAA_u8; 3];

        // When
        let decoded = bytes_to_f32_cow(&bytes[..len]);

        // Then
        assert_eq!(decoded.as_ref(), &[] as &[f32]);
    }
}
