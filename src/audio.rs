use std::borrow::Cow;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use screencapturekit::prelude::*;
use screencapturekit::stream::delegate_trait::StreamCallbacks;

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: u16 = 2;
/// Number of stereo frames aggregated into each waveform peak bucket (~5.3 ms at 48 kHz).
const PEAK_BUCKET_FRAMES: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningAppInfo {
    pub pid: i32,
    pub name: String,
    pub bundle_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum AudioSource {
    #[default]
    AllSystemAudio,
    Application(RunningAppInfo),
}

impl AudioSource {
    pub fn label(&self) -> String {
        match self {
            Self::AllSystemAudio => "All System Audio".to_string(),
            Self::Application(info) => info.name.clone(),
        }
    }
}

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

#[derive(Debug)]
pub struct AudioBufferState {
    /// Interleaved stereo f32 PCM samples at `SAMPLE_RATE`.
    pub samples: Vec<f32>,
    /// Downsampled `(min, max)` amplitude pairs for fast waveform rendering.
    pub peaks: Vec<(f32, f32)>,
    bucket_min: f32,
    bucket_max: f32,
    bucket_frames: usize,
    /// Smoothed live amplitude in `0.0..=1.0` for the level meter.
    pub live_level: f32,
    /// Last capture error reported by ScreenCaptureKit, if any.
    pub error: Option<String>,
}

impl Default for AudioBufferState {
    fn default() -> Self {
        Self {
            samples: Vec::new(),
            peaks: Vec::new(),
            bucket_min: 0.0,
            bucket_max: 0.0,
            bucket_frames: 0,
            live_level: 0.0,
            error: None,
        }
    }
}

impl AudioBufferState {
    pub fn clear(&mut self) {
        self.samples.clear();
        self.peaks.clear();
        self.bucket_min = 0.0;
        self.bucket_max = 0.0;
        self.bucket_frames = 0;
        self.live_level = 0.0;
        self.error = None;
    }

    pub fn duration_secs(&self) -> f64 {
        let frames = self.samples.len() / CHANNELS as usize;
        frames as f64 / SAMPLE_RATE as f64
    }

    pub fn frame_count(&self) -> usize {
        self.samples.len() / CHANNELS as usize
    }

    pub fn flush_partial_peak(&mut self) {
        if self.bucket_frames > 0 {
            self.peaks.push((self.bucket_min, self.bucket_max));
            self.bucket_min = 0.0;
            self.bucket_max = 0.0;
            self.bucket_frames = 0;
        }
        self.live_level = 0.0;
    }

    fn push_interleaved_stereo(&mut self, interleaved: &[f32]) {
        if interleaved.is_empty() {
            return;
        }
        self.samples.extend_from_slice(interleaved);

        let mut chunk_peak: f32 = 0.0;
        for frame in interleaved.as_chunks::<{ CHANNELS as usize }>().0 {
            let mono = 0.5 * (frame[0] + frame[1]);
            chunk_peak = chunk_peak.max(frame[0].abs()).max(frame[1].abs());

            if self.bucket_frames == 0 {
                self.bucket_min = mono;
                self.bucket_max = mono;
            } else {
                self.bucket_min = self.bucket_min.min(mono);
                self.bucket_max = self.bucket_max.max(mono);
            }
            self.bucket_frames += 1;

            if self.bucket_frames >= PEAK_BUCKET_FRAMES {
                self.peaks.push((self.bucket_min, self.bucket_max));
                self.bucket_min = 0.0;
                self.bucket_max = 0.0;
                self.bucket_frames = 0;
            }
        }

        // Smooth live meter slightly so it feels responsive without flickering.
        self.live_level = (self.live_level * 0.4 + chunk_peak * 0.6).clamp(0.0, 1.0);
    }
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

    pub fn is_recording(&self) -> bool {
        self.stream.is_some()
    }

    pub fn start(
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

    pub fn stop(&mut self) -> Result<(), String> {
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

fn slice_frame_bounds(total_frames: usize, selection: Option<(f32, f32)>) -> (usize, usize) {
    if total_frames == 0 {
        return (0, 0);
    }

    let Some((a, b)) = selection else {
        return (0, total_frames);
    };

    if !a.is_finite() || !b.is_finite() {
        return (0, total_frames);
    }

    let lo = a.min(b).clamp(0.0, 1.0);
    let hi = a.max(b).clamp(0.0, 1.0);
    if (hi - lo) <= 1e-6 {
        return (0, total_frames);
    }

    let mut start_frame = (lo * total_frames as f32).round() as usize;
    let mut end_frame = (hi * total_frames as f32).round() as usize;

    if start_frame >= total_frames {
        start_frame = total_frames - 1;
    }
    if end_frame <= start_frame {
        end_frame = (start_frame + 1).min(total_frames);
    }
    if end_frame > total_frames {
        end_frame = total_frames;
    }

    (start_frame, end_frame)
}

pub fn slice_frame_range(total_frames: usize, selection: Option<(f32, f32)>) -> (f32, f32) {
    if total_frames == 0 {
        return (0.0, 1.0);
    }
    let (start_frame, end_frame) = slice_frame_bounds(total_frames, selection);
    (
        start_frame as f32 / total_frames as f32,
        end_frame as f32 / total_frames as f32,
    )
}

pub fn extract_slice(samples: &[f32], selection: Option<(f32, f32)>) -> Vec<f32> {
    let ch = CHANNELS as usize;
    let total_frames = samples.len() / ch;
    if total_frames == 0 {
        return Vec::new();
    }

    let (start_frame, end_frame) = slice_frame_bounds(total_frames, selection);
    samples[start_frame * ch..end_frame * ch].to_vec()
}

pub fn normalize_samples(samples: &mut [f32]) {
    const TARGET_PEAK: f32 = 0.9885; // -0.1 dBFS
    let max_abs = samples
        .iter()
        .copied()
        .filter(|s| s.is_finite())
        .map(f32::abs)
        .fold(0.0_f32, f32::max);

    if max_abs > 1e-6 {
        let gain = TARGET_PEAK / max_abs;
        for sample in samples.iter_mut() {
            if sample.is_finite() {
                *sample *= gain;
            }
        }
    }
}

pub struct AudioPlayer {
    current_stop: Mutex<Option<Arc<AtomicBool>>>,
    is_playing: Arc<AtomicBool>,
    playhead_bits: Arc<AtomicU32>,
    error: Arc<Mutex<Option<String>>>,
}

impl Default for AudioPlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioPlayer {
    pub fn new() -> Self {
        Self {
            current_stop: Mutex::new(None),
            is_playing: Arc::new(AtomicBool::new(false)),
            playhead_bits: Arc::new(AtomicU32::new(f32::NAN.to_bits())),
            error: Arc::new(Mutex::new(None)),
        }
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing.load(Ordering::Relaxed)
    }

    pub fn playhead_position(&self) -> Option<f32> {
        if !self.is_playing() {
            return None;
        }
        let pos = f32::from_bits(self.playhead_bits.load(Ordering::Relaxed));
        pos.is_finite().then_some(pos)
    }

    pub fn take_error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|mut guard| guard.take())
    }

    pub fn stop(&self) {
        if let Ok(mut guard) = self.current_stop.lock()
            && let Some(flag) = guard.take()
        {
            flag.store(true, Ordering::Relaxed);
        }
        self.is_playing.store(false, Ordering::Relaxed);
        self.playhead_bits
            .store(f32::NAN.to_bits(), Ordering::Relaxed);
    }

    pub fn play(
        &self,
        samples: Vec<f32>,
        clip_range: (f32, f32),
        loop_enabled: Arc<AtomicBool>,
    ) -> Result<(), String> {
        if let Ok(mut guard) = self.error.lock() {
            *guard = None;
        }
        if samples.len() < CHANNELS as usize {
            return Err("No audio recorded to play".to_string());
        }

        // Signal any previous playback thread to stop.
        self.stop();

        let (range_start, range_end) = if clip_range.0.is_finite() && clip_range.1.is_finite() {
            (
                clip_range.0.min(clip_range.1).clamp(0.0, 1.0),
                clip_range.0.max(clip_range.1).clamp(0.0, 1.0),
            )
        } else {
            (0.0, 1.0)
        };

        let thread_stop = Arc::new(AtomicBool::new(false));
        if let Ok(mut guard) = self.current_stop.lock() {
            *guard = Some(thread_stop.clone());
        }

        let is_playing = self.is_playing.clone();
        let playhead_bits = self.playhead_bits.clone();
        let error = self.error.clone();
        playhead_bits.store(range_start.to_bits(), Ordering::Relaxed);
        is_playing.store(true, Ordering::Relaxed);

        std::thread::spawn(move || {
            let slice_duration_secs =
                (samples.len() / CHANNELS as usize) as f64 / SAMPLE_RATE as f64;

            let playback_init = rodio::OutputStream::try_default()
                .map_err(|e| format!("Audio playback error: {e}"))
                .and_then(|(stream, handle)| {
                    rodio::Sink::try_new(&handle)
                        .map(|sink| (sink, handle, stream))
                        .map_err(|e| format!("Audio playback error: {e}"))
                });

            match playback_init {
                Ok((sink, _handle, _stream)) => {
                    let source =
                        rodio::buffer::SamplesBuffer::new(CHANNELS, SAMPLE_RATE, samples.clone());
                    sink.append(source);
                    let mut pass_start = Instant::now();

                    while !thread_stop.load(Ordering::Relaxed) {
                        if sink.empty() {
                            if loop_enabled.load(Ordering::Relaxed) {
                                let next_source = rodio::buffer::SamplesBuffer::new(
                                    CHANNELS,
                                    SAMPLE_RATE,
                                    samples.clone(),
                                );
                                sink.append(next_source);
                                pass_start = Instant::now();
                            } else {
                                break;
                            }
                        }

                        let elapsed = pass_start.elapsed().as_secs_f64();
                        let progress = if slice_duration_secs > 0.0 {
                            (elapsed / slice_duration_secs) as f32
                        } else {
                            0.0
                        };
                        let pos = (range_start + progress * (range_end - range_start))
                            .clamp(range_start, range_end);
                        playhead_bits.store(pos.to_bits(), Ordering::Relaxed);

                        std::thread::sleep(Duration::from_millis(16));
                    }
                    sink.stop();
                }
                Err(msg) => {
                    if let Ok(mut guard) = error.lock() {
                        *guard = Some(msg);
                    }
                }
            }

            if !thread_stop.load(Ordering::Relaxed) {
                is_playing.store(false, Ordering::Relaxed);
                playhead_bits.store(f32::NAN.to_bits(), Ordering::Relaxed);
            }
        });

        Ok(())
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m as u32, d as u32)
}

pub fn format_timestamp_filename(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let secs_of_day = unix_secs % 86_400;
    let hour = secs_of_day / 3_600;
    let minute = (secs_of_day % 3_600) / 60;
    let second = secs_of_day % 60;
    let (year, month, day) = civil_from_days(days);
    format!("sample-{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}.wav")
}

pub fn quick_save_dir() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .ok_or_else(|| "HOME environment variable is not set".to_string())?;
    Ok(PathBuf::from(home).join("Music").join("Sampler"))
}

pub fn save_wav_with_options(path: &Path, samples: &[f32], normalize: bool) -> Result<(), String> {
    if samples.is_empty() {
        return Err("No recorded audio to save".to_string());
    }

    let normalized_buf;
    let output_samples = if normalize {
        normalized_buf = {
            let mut buf = samples.to_vec();
            normalize_samples(&mut buf);
            buf
        };
        normalized_buf.as_slice()
    } else {
        samples
    };

    let spec = hound::WavSpec {
        channels: CHANNELS,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer =
        hound::WavWriter::create(path, spec).map_err(|e| format!("Failed to create WAV: {e}"))?;

    for &sample in output_samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let pcm_i16 = (clamped * i16::MAX as f32) as i16;
        writer
            .write_sample(pcm_i16)
            .map_err(|e| format!("Failed writing WAV sample: {e}"))?;
    }

    writer
        .finalize()
        .map_err(|e| format!("Failed finalizing WAV file: {e}"))?;

    Ok(())
}

pub fn quick_save_wav_in_dir(
    dir: &Path,
    unix_secs: u64,
    samples: &[f32],
    normalize: bool,
) -> Result<PathBuf, String> {
    if samples.is_empty() {
        return Err("No recorded audio to save".to_string());
    }

    std::fs::create_dir_all(dir).map_err(|e| {
        format!(
            "Failed to create quick-save directory '{}': {e}",
            dir.display()
        )
    })?;

    let filename = format_timestamp_filename(unix_secs);
    let stem = filename.strip_suffix(".wav").unwrap_or(&filename);

    let mut candidate = dir.join(&filename);
    let mut suffix = 1_usize;
    while candidate.exists() {
        candidate = dir.join(format!("{stem}-{suffix}.wav"));
        suffix += 1;
    }

    save_wav_with_options(&candidate, samples, normalize)?;
    Ok(candidate)
}

pub fn quick_save_wav(samples: &[f32], normalize: bool) -> Result<PathBuf, String> {
    if samples.is_empty() {
        return Err("No recorded audio to save".to_string());
    }

    let dir = quick_save_dir()?;
    let unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    quick_save_wav_in_dir(&dir, unix_secs, samples, normalize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaks_and_duration_track_interleaved_stereo() {
        let mut state = AudioBufferState::default();
        // 512 stereo frames (2 peak buckets of 256 frames each)
        let mut samples = Vec::with_capacity(512 * 2);
        for i in 0..512 {
            let v = if i < 256 { 0.5_f32 } else { -0.25_f32 };
            samples.push(v);
            samples.push(v);
        }
        state.push_interleaved_stereo(&samples);

        assert_eq!(state.frame_count(), 512);
        assert_eq!(state.peaks.len(), 2);
        assert!((state.peaks[0].0 - 0.5).abs() < 1e-6);
        assert!((state.peaks[0].1 - 0.5).abs() < 1e-6);
        assert!((state.peaks[1].0 - (-0.25)).abs() < 1e-6);
        assert!((state.peaks[1].1 - (-0.25)).abs() < 1e-6);
        assert!((state.duration_secs() - (512.0 / 48_000.0)).abs() < 1e-9);
    }

    #[test]
    fn save_wav_writes_valid_16bit_stereo_file() {
        let dir = std::env::temp_dir();
        let path = dir.join("sampler_test_output.wav");
        let samples = vec![0.0_f32, 0.5, -0.5, 1.0, -1.0, 0.25];

        save_wav_with_options(&path, &samples, false)
            .expect("save_wav_with_options should succeed");

        let reader = hound::WavReader::open(&path).expect("WavReader should open saved file");
        let spec = reader.spec();
        assert_eq!(spec.channels, 2);
        assert_eq!(spec.sample_rate, 48_000);
        assert_eq!(spec.bits_per_sample, 16);
        assert_eq!(spec.sample_format, hound::SampleFormat::Int);

        let read_samples: Vec<i16> = reader.into_samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(read_samples.len(), samples.len());
        assert_eq!(read_samples[3], i16::MAX);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn extract_slice_handles_full_subrange_reversed_empty_and_alignment() {
        // 10 stereo frames: frame i has values (i as f32, -(i as f32))
        let mut samples = Vec::with_capacity(20);
        for i in 0..10 {
            samples.push(i as f32);
            samples.push(-(i as f32));
        }

        // None returns full stereo clip
        assert_eq!(extract_slice(&samples, None), samples);
        assert_eq!(slice_frame_range(10, None), (0.0, 1.0));

        // Sub-range (0.2, 0.5) -> frames 2..5
        let sub = extract_slice(&samples, Some((0.2, 0.5)));
        assert_eq!(sub, vec![2.0, -2.0, 3.0, -3.0, 4.0, -4.0]);
        assert_eq!(slice_frame_range(10, Some((0.2, 0.5))), (0.2, 0.5));

        // Reversed (0.8, 0.2) -> frames 2..8
        let rev = extract_slice(&samples, Some((0.8, 0.2)));
        assert_eq!(rev.len(), 12);
        assert_eq!(rev[0], 2.0);
        assert_eq!(rev[11], -7.0);
        assert_eq!(slice_frame_range(10, Some((0.8, 0.2))), (0.2, 0.8));

        // Narrow non-degenerate range guarantees at least 1 stereo frame
        let tiny = extract_slice(&samples, Some((0.0, 0.01)));
        assert_eq!(tiny, vec![0.0, 0.0]);
        assert_eq!(slice_frame_range(10, Some((0.0, 0.01))), (0.0, 0.1));

        // Degenerate range falls back to full clip
        assert_eq!(extract_slice(&samples, Some((0.5, 0.5))), samples);
        assert_eq!(slice_frame_range(10, Some((0.5, 0.5))), (0.0, 1.0));

        // Stereo frame alignment drops trailing odd sample
        let unaligned = vec![0.1, -0.1, 0.2, -0.2, 0.3];
        assert_eq!(extract_slice(&unaligned, None), vec![0.1, -0.1, 0.2, -0.2]);

        // Empty input returns empty vec
        assert!(extract_slice(&[], None).is_empty());
        assert!(extract_slice(&[0.5], Some((0.0, 1.0))).is_empty());
    }

    #[test]
    fn normalize_samples_scales_peak_and_preserves_silence() {
        let mut signal = vec![0.1_f32, -0.25, 0.05, -0.125];
        normalize_samples(&mut signal);
        let peak = signal.iter().map(|s| s.abs()).fold(0.0_f32, f32::max);
        assert!((peak - 0.9885).abs() < 1e-5);
        assert!((signal[0] - (0.1 * 0.9885 / 0.25)).abs() < 1e-5);
        assert!((signal[1] + 0.9885).abs() < 1e-5);

        let mut silence = vec![0.0_f32, 0.0, -0.0, 1e-7];
        let original = silence.clone();
        normalize_samples(&mut silence);
        assert_eq!(silence, original);
    }

    #[test]
    fn format_timestamp_filename_formats_utc_correctly() {
        assert_eq!(format_timestamp_filename(0), "sample-19700101-000000.wav");
        assert_eq!(
            format_timestamp_filename(1_709_164_800),
            "sample-20240229-000000.wav"
        );
        assert_eq!(
            format_timestamp_filename(1_795_223_194),
            "sample-20261121-010634.wav"
        );
    }

    #[test]
    fn save_wav_with_options_normalizes_without_mutating_input() {
        let dir = std::env::temp_dir();
        let path = dir.join("sampler_test_normalized_output.wav");
        let samples = vec![0.0_f32, 0.1, -0.2, 0.05, -0.1, 0.2];
        let snapshot = samples.clone();

        save_wav_with_options(&path, &samples, true).expect("save_wav_with_options should succeed");

        assert_eq!(samples, snapshot);

        let reader = hound::WavReader::open(&path).expect("WavReader should open saved file");
        let read_samples: Vec<i16> = reader.into_samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(read_samples.len(), samples.len());

        let expected_peak = (0.9885_f32 * i16::MAX as f32) as i16;
        let max_pcm = read_samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!((max_pcm as i32 - expected_peak as i32).abs() <= 2);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn audio_source_label_and_player_idle_state() {
        assert_eq!(AudioSource::default().label(), "All System Audio");
        let app_source = AudioSource::Application(RunningAppInfo {
            pid: 42,
            name: "Music".to_string(),
            bundle_id: "com.apple.Music".to_string(),
        });
        assert_eq!(app_source.label(), "Music");

        let player = AudioPlayer::new();
        assert!(!player.is_playing());
        assert_eq!(player.playhead_position(), None);
        assert_eq!(player.take_error(), None);

        let loop_flag = Arc::new(AtomicBool::new(false));
        assert!(player.play(Vec::new(), (0.0, 1.0), loop_flag).is_err());
    }

    #[test]
    fn flush_partial_peak_commits_sub_bucket_and_resets_live_level() {
        let mut state = AudioBufferState::default();
        let mut samples = Vec::with_capacity(20 * 2);
        for i in 0..20 {
            let v = if i < 10 { 0.25_f32 } else { -0.5_f32 };
            samples.push(v);
            samples.push(v);
        }
        state.push_interleaved_stereo(&samples);

        assert_eq!(state.frame_count(), 20);
        assert!(state.peaks.is_empty());
        assert!(state.live_level > 0.0);

        state.flush_partial_peak();
        assert_eq!(state.peaks.len(), 1);
        assert!((state.peaks[0].0 - (-0.5)).abs() < 1e-6);
        assert!((state.peaks[0].1 - 0.25).abs() < 1e-6);
        assert_eq!(state.live_level, 0.0);

        // Calling again when no partial bucket remains should not push an empty bucket.
        state.flush_partial_peak();
        assert_eq!(state.peaks.len(), 1);
        assert_eq!(state.live_level, 0.0);
    }

    #[test]
    fn bytes_to_f32_cow_decodes_aligned_and_unaligned_slices() {
        let expected = [0.0_f32, -0.5, 0.25, 1.0, -0.875];
        let payload: Vec<u8> = expected.iter().flat_map(|f| f.to_ne_bytes()).collect();

        let mut aligned_words = vec![0_u32; expected.len() + 1];
        let aligned_bytes: &mut [u8] = bytemuck::cast_slice_mut(&mut aligned_words);
        aligned_bytes[..payload.len()].copy_from_slice(&payload);
        aligned_bytes[payload.len()] = 0xAA;
        aligned_bytes[payload.len() + 1] = 0xBB;

        let aligned_cow = bytes_to_f32_cow(&aligned_bytes[..payload.len() + 2]);
        assert!(matches!(aligned_cow, Cow::Borrowed(_)));
        assert_eq!(aligned_cow.as_ref(), &expected);

        let mut unaligned_words = vec![0_u32; expected.len() + 2];
        let unaligned_bytes: &mut [u8] = bytemuck::cast_slice_mut(&mut unaligned_words);
        unaligned_bytes[1..1 + payload.len()].copy_from_slice(&payload);
        unaligned_bytes[1 + payload.len()] = 0xCC;
        unaligned_bytes[1 + payload.len() + 1] = 0xDD;
        unaligned_bytes[1 + payload.len() + 2] = 0xEE;

        let unaligned_cow = bytes_to_f32_cow(&unaligned_bytes[1..1 + payload.len() + 3]);
        assert!(matches!(unaligned_cow, Cow::Owned(_)));
        assert_eq!(unaligned_cow.as_ref(), &expected);

        assert!(bytes_to_f32_cow(&unaligned_bytes[1..4]).is_empty());
        assert!(bytes_to_f32_cow(&[]).is_empty());
    }

    #[test]
    fn quick_save_wav_in_dir_creates_dir_and_handles_collisions() {
        struct TempDirGuard(PathBuf);
        impl Drop for TempDirGuard {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let base_dir = std::env::temp_dir().join(format!(
            "sampler_quick_save_test_{}_{}",
            std::process::id(),
            unique
        ));
        let _ = std::fs::remove_dir_all(&base_dir);
        let _guard = TempDirGuard(base_dir.clone());

        let nested_dir = base_dir.join("nested");
        assert!(!nested_dir.exists());

        let samples = vec![0.1_f32, -0.1, 0.2, -0.2];
        let unix_secs = 1_795_223_194;

        let p0 = quick_save_wav_in_dir(&nested_dir, unix_secs, &samples, false)
            .expect("first quick_save_wav_in_dir should succeed");
        assert!(nested_dir.exists());
        assert_eq!(p0, nested_dir.join("sample-20261121-010634.wav"));
        assert!(p0.exists());

        let p1 = quick_save_wav_in_dir(&nested_dir, unix_secs, &samples, false)
            .expect("second quick_save_wav_in_dir should append -1");
        assert_eq!(p1, nested_dir.join("sample-20261121-010634-1.wav"));
        assert!(p1.exists());

        let p2 = quick_save_wav_in_dir(&nested_dir, unix_secs, &samples, true)
            .expect("third quick_save_wav_in_dir should append -2");
        assert_eq!(p2, nested_dir.join("sample-20261121-010634-2.wav"));
        assert!(p2.exists());

        let reader = hound::WavReader::open(&p0).expect("saved quick-save WAV should be readable");
        assert_eq!(reader.spec().channels, 2);
        assert_eq!(reader.spec().sample_rate, 48_000);
    }

    #[test]
    fn audio_player_take_error_is_none_when_idle() {
        let player = AudioPlayer::new();
        assert_eq!(player.take_error(), None);

        if let Ok(mut guard) = player.error.lock() {
            *guard = Some("Audio playback error: simulated".to_string());
        }
        assert_eq!(
            player.take_error(),
            Some("Audio playback error: simulated".to_string())
        );
        assert_eq!(player.take_error(), None);

        if let Ok(mut guard) = player.error.lock() {
            *guard = Some("Audio playback error: stale".to_string());
        }
        let loop_flag = Arc::new(AtomicBool::new(false));
        assert!(player.play(Vec::new(), (0.0, 1.0), loop_flag).is_err());
        assert_eq!(player.take_error(), None);
    }
}
