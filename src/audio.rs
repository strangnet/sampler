use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::AudioRecorder;
#[cfg(not(target_os = "macos"))]
mod unsupported;
#[cfg(not(target_os = "macos"))]
pub use unsupported::AudioRecorder;

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

/// What the app needs from audio capture (ScreenCaptureKit in production).
pub trait Capture {
    /// Running applications that can be chosen as a source.
    fn running_apps(&self) -> Result<Vec<RunningAppInfo>, String>;
    /// Clears `recording` and starts capturing `source` into it.
    fn start(
        &mut self,
        recording: Arc<Mutex<AudioBufferState>>,
        source: &AudioSource,
    ) -> Result<(), String>;
    fn stop(&mut self) -> Result<(), String>;
    fn is_recording(&self) -> bool;
}

/// What the app needs from audio output (rodio in production).
pub trait Playback {
    /// Plays `samples`, the part of the Recording spanning `clip_range`
    /// (normalized), repeating while `loop_enabled` is set.
    fn play(
        &self,
        samples: Vec<f32>,
        clip_range: (f32, f32),
        loop_enabled: Arc<AtomicBool>,
    ) -> Result<(), String>;
    fn stop(&self);
    fn is_playing(&self) -> bool;
    /// Normalized position in the Recording while playing.
    fn playhead_position(&self) -> Option<f32>;
    /// The last playback error, cleared once taken.
    fn take_error(&self) -> Option<String>;
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
    /// Last capture error reported by the capture backend, if any.
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

    /// Appends captured interleaved stereo samples and updates the
    /// waveform peaks and live level.
    pub fn push_interleaved_stereo(&mut self, interleaved: &[f32]) {
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
}

impl Playback for AudioPlayer {
    fn is_playing(&self) -> bool {
        self.is_playing.load(Ordering::Relaxed)
    }

    fn playhead_position(&self) -> Option<f32> {
        if !self.is_playing() {
            return None;
        }
        let pos = f32::from_bits(self.playhead_bits.load(Ordering::Relaxed));
        pos.is_finite().then_some(pos)
    }

    fn take_error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|mut guard| guard.take())
    }

    fn stop(&self) {
        if let Ok(mut guard) = self.current_stop.lock()
            && let Some(flag) = guard.take()
        {
            flag.store(true, Ordering::Relaxed);
        }
        self.is_playing.store(false, Ordering::Relaxed);
        self.playhead_bits
            .store(f32::NAN.to_bits(), Ordering::Relaxed);
    }

    fn play(
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

    let file = std::fs::File::create(path).map_err(|e| format!("Failed to create WAV: {e}"))?;
    write_wav(std::io::BufWriter::new(file), samples, normalize)
}

/// Encodes interleaved stereo samples as a 16-bit PCM WAV into `writer`.
fn write_wav<W: std::io::Write + std::io::Seek>(
    writer: W,
    samples: &[f32],
    normalize: bool,
) -> Result<(), String> {
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
        hound::WavWriter::new(writer, spec).map_err(|e| format!("Failed to create WAV: {e}"))?;

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

/// Quick-saves into `dir`, named after the current time.
pub fn quick_save_wav(dir: &Path, samples: &[f32], normalize: bool) -> Result<PathBuf, String> {
    let unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    quick_save_wav_in_dir(dir, unix_secs, samples, normalize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn encode(samples: &[f32], normalize: bool) -> hound::WavReader<std::io::Cursor<Vec<u8>>> {
        let mut wav = std::io::Cursor::new(Vec::new());
        write_wav(&mut wav, samples, normalize).expect("writing to memory succeeds");
        wav.set_position(0);
        hound::WavReader::new(wav).expect("output is a valid WAV")
    }

    #[track_caller]
    fn assert_close(actual: &[f32], expected: &[f32]) {
        let close = actual.len() == expected.len()
            && actual
                .iter()
                .zip(expected)
                .all(|(a, e)| (a - e).abs() < 1e-6);
        assert!(close, "{actual:?} is not close to {expected:?}");
    }

    /// Interleaved stereo frames where frame `i` is `(i, -i)`.
    fn frames(range: std::ops::Range<usize>) -> Vec<f32> {
        range.flat_map(|i| [i as f32, -(i as f32)]).collect()
    }

    fn decode(wav: hound::WavReader<std::io::Cursor<Vec<u8>>>) -> Vec<i16> {
        wav.into_samples()
            .collect::<Result<_, _>>()
            .expect("every sample decodes")
    }

    #[test]
    fn every_256_frames_add_one_waveform_peak() {
        // Given
        let mut state = AudioBufferState::default();
        let mut recording = [0.5_f32, 0.5].repeat(256);
        recording.extend([-0.25_f32, -0.25].repeat(256));

        // When
        state.push_interleaved_stereo(&recording);

        // Then
        assert_eq!(state.peaks, vec![(0.5, 0.5), (-0.25, -0.25)]);
    }

    #[test]
    fn the_duration_counts_stereo_frames_at_48_khz() {
        // Given
        let mut state = AudioBufferState::default();

        // When
        state.push_interleaved_stereo(&[0.0; 2 * 4_800]);

        // Then
        assert_eq!(state.frame_count(), 4_800);
        assert_eq!(state.duration_secs(), 0.1);
    }

    #[test]
    fn wav_export_is_48_khz_stereo_16_bit_integer() {
        // Given
        let samples = [0.0_f32, 0.0];

        // When
        let wav = encode(&samples, false);

        // Then
        assert_eq!(
            wav.spec(),
            hound::WavSpec {
                channels: 2,
                sample_rate: 48_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            }
        );
    }

    #[rstest]
    #[case::no_selection_is_the_whole_recording(None, 0..10)]
    #[case::a_selection_covers_its_frames(Some((0.2, 0.5)), 2..5)]
    #[case::a_reversed_selection_is_normalised(Some((0.8, 0.2)), 2..8)]
    #[case::rounds_to_the_nearest_frame(Some((0.25, 0.75)), 3..8)]
    #[case::a_narrow_selection_keeps_one_frame(Some((0.0, 0.01)), 0..1)]
    #[case::the_very_end_still_holds_the_last_frame(Some((0.96, 1.0)), 9..10)]
    #[case::a_zero_width_selection_is_the_whole_recording(Some((0.5, 0.5)), 0..10)]
    #[case::an_infinite_end_is_the_whole_recording(Some((0.2, f32::INFINITY)), 0..10)]
    fn a_selection_maps_to_whole_frames(
        #[case] selection: Option<(f32, f32)>,
        #[case] expected: std::ops::Range<usize>,
    ) {
        // Given
        let recording = frames(0..10);

        // When
        let slice = extract_slice(&recording, selection);
        let range = slice_frame_range(10, selection);

        // Then
        assert_eq!(slice, frames(expected.clone()));
        assert_eq!(
            range,
            (expected.start as f32 / 10.0, expected.end as f32 / 10.0)
        );
    }

    #[test]
    fn a_trailing_half_frame_is_dropped() {
        // Given
        let recording = [0.1_f32, -0.1, 0.2, -0.2, 0.3];

        // When
        let slice = extract_slice(&recording, None);

        // Then
        assert_eq!(slice, vec![0.1, -0.1, 0.2, -0.2]);
    }

    #[rstest]
    #[case::no_samples(&[], None)]
    #[case::half_a_frame(&[0.5], Some((0.0, 1.0)))]
    fn a_recording_without_a_whole_frame_slices_to_nothing(
        #[case] recording: &[f32],
        #[case] selection: Option<(f32, f32)>,
    ) {
        // When
        let slice = extract_slice(recording, selection);

        // Then
        assert_eq!(slice, Vec::<f32>::new());
    }

    #[test]
    fn normalising_scales_the_peak_to_minus_0_1_dbfs() {
        // Given
        let mut samples = [0.1_f32, -0.25, 0.05, -0.125];

        // When
        normalize_samples(&mut samples);

        // Then
        assert_close(&samples, &[0.395_4, -0.9885, 0.197_7, -0.494_25]);
    }

    #[test]
    fn normalising_leaves_near_silence_untouched() {
        // Given
        let mut samples = [0.0_f32, 0.0, -0.0, 1e-7];

        // When
        normalize_samples(&mut samples);

        // Then
        assert_eq!(samples, [0.0, 0.0, -0.0, 1e-7]);
    }

    #[rstest]
    #[case::the_unix_epoch(0, "sample-19700101-000000.wav")]
    #[case::a_leap_day(1_709_164_800, "sample-20240229-000000.wav")]
    #[case::a_time_of_day(QUICK_SAVE_SECS, "sample-20261121-010634.wav")]
    fn a_sample_file_is_named_after_its_utc_time(#[case] unix_secs: u64, #[case] expected: &str) {
        // When
        let name = format_timestamp_filename(unix_secs);

        // Then
        assert_eq!(name, expected);
    }

    #[test]
    fn a_normalised_export_peaks_just_below_full_scale() {
        // Given
        let samples = [0.0_f32, 0.1, -0.2, 0.05, -0.1, 0.2];

        // When
        let decoded = decode(encode(&samples, true));

        // Then
        let peak = decoded.iter().map(|s| s.unsigned_abs()).max();
        assert_eq!(peak, Some(32_390), "-0.1 dBFS of i16::MAX");
    }

    #[rstest]
    #[case::the_default_is_all_system_audio(AudioSource::default(), "All System Audio")]
    #[case::an_application_by_its_name(
        AudioSource::Application(RunningAppInfo {
            pid: 42,
            name: "Music".to_string(),
            bundle_id: "com.apple.Music".to_string(),
        }),
        "Music"
    )]
    fn a_source_is_labelled(#[case] source: AudioSource, #[case] expected: &str) {
        // When
        let label = source.label();

        // Then
        assert_eq!(label, expected);
    }

    #[test]
    fn a_new_player_is_idle_with_no_playhead() {
        // When
        let player = AudioPlayer::new();

        // Then
        assert!(!player.is_playing(), "a new player is playing");
        assert_eq!(player.playhead_position(), None);
        assert_eq!(player.take_error(), None);
    }

    #[test]
    fn playing_an_empty_recording_fails() {
        // Given
        let player = AudioPlayer::new();

        // When
        let result = player.play(Vec::new(), (0.0, 1.0), Arc::new(AtomicBool::new(false)));

        // Then
        assert_eq!(result, Err("No audio recorded to play".to_string()));
    }

    #[test]
    fn ending_a_recording_commits_the_partial_peak_and_silences_the_meter() {
        // Given
        let mut state = AudioBufferState::default();
        let mut recording = [0.25_f32, 0.25].repeat(10);
        recording.extend([-0.5_f32, -0.5].repeat(10));
        state.push_interleaved_stereo(&recording);

        // When
        state.flush_partial_peak();

        // Then
        assert_eq!(state.peaks, vec![(-0.5, 0.25)]);
        assert_eq!(state.live_level, 0.0);
    }

    #[test]
    fn ending_a_recording_twice_adds_no_empty_peak() {
        // Given
        let mut state = AudioBufferState::default();
        state.push_interleaved_stereo(&[0.25, 0.25]);
        state.flush_partial_peak();

        // When
        state.flush_partial_peak();

        // Then
        assert_eq!(state.peaks, vec![(0.25, 0.25)]);
    }

    const QUICK_SAVE_SECS: u64 = 1_795_223_194; // 2026-11-21 01:06:34 UTC

    #[test]
    fn a_quick_save_is_named_after_its_time_in_a_newly_created_folder() {
        // Given
        let tmp = tempfile::tempdir().expect("temp dir");
        let folder = tmp.path().join("Sampler");

        // When
        let saved = quick_save_wav_in_dir(&folder, QUICK_SAVE_SECS, &[0.1, -0.1], false)
            .expect("quick save succeeds");

        // Then
        assert_eq!(saved, folder.join("sample-20261121-010634.wav"));
        assert!(saved.is_file(), "{} was not written", saved.display());
    }

    #[test]
    fn quick_saves_in_the_same_second_get_numbered_suffixes() {
        // Given
        let tmp = tempfile::tempdir().expect("temp dir");
        let folder = tmp.path();
        quick_save_wav_in_dir(folder, QUICK_SAVE_SECS, &[0.1, -0.1], false).expect("first save");

        // When
        let second = quick_save_wav_in_dir(folder, QUICK_SAVE_SECS, &[0.1, -0.1], false)
            .expect("second save");
        let third = quick_save_wav_in_dir(folder, QUICK_SAVE_SECS, &[0.1, -0.1], false)
            .expect("third save");

        // Then
        assert_eq!(second, folder.join("sample-20261121-010634-1.wav"));
        assert_eq!(third, folder.join("sample-20261121-010634-2.wav"));
    }

    /// `AudioPlayer` has no seam that makes rodio fail, so these tests plant
    /// the error its playback thread would record.
    fn player_with_error(message: &str) -> AudioPlayer {
        let player = AudioPlayer::new();
        *player.error.lock().expect("error lock") = Some(message.to_string());
        player
    }

    #[test]
    fn a_playback_error_is_reported_once() {
        // Given
        let player = player_with_error("Audio playback error: simulated");

        // When
        let first = player.take_error();
        let second = player.take_error();

        // Then
        assert_eq!(first.as_deref(), Some("Audio playback error: simulated"));
        assert_eq!(second, None);
    }

    #[test]
    fn starting_playback_clears_a_stale_error() {
        // Given
        let player = player_with_error("Audio playback error: stale");

        // When
        let _ = player.play(Vec::new(), (0.0, 1.0), Arc::new(AtomicBool::new(false)));

        // Then
        assert_eq!(player.take_error(), None);
    }

    #[test]
    fn wav_export_writes_each_sample_as_16_bit_pcm() {
        // Given
        let samples = [0.0_f32, 0.5, -0.5, 1.0, -1.0, 0.25];

        // When
        let decoded = decode(encode(&samples, false));

        // Then
        assert_eq!(decoded, vec![0, 16383, -16383, 32767, -32767, 8191]);
    }

    #[test]
    fn saving_an_empty_recording_is_refused() {
        // Given
        let tmp = tempfile::tempdir().expect("temp dir");
        let path = tmp.path().join("empty.wav");

        // When
        let result = save_wav_with_options(&path, &[], false);

        // Then
        assert_eq!(result, Err("No recorded audio to save".to_string()));
        assert!(!path.exists(), "an empty WAV was written");
    }

    #[test]
    fn quick_saving_an_empty_recording_is_refused_without_creating_the_folder() {
        // Given
        let tmp = tempfile::tempdir().expect("temp dir");
        let folder = tmp.path().join("Sampler");

        // When
        let result = quick_save_wav_in_dir(&folder, QUICK_SAVE_SECS, &[], false);

        // Then
        assert_eq!(result, Err("No recorded audio to save".to_string()));
        assert!(!folder.exists(), "the quick-save folder was created");
    }

    #[test]
    fn normalising_ignores_non_finite_samples() {
        // Given
        let mut samples = [0.25_f32, f32::INFINITY, -0.5];

        // When
        normalize_samples(&mut samples);

        // Then
        assert_eq!(samples, [0.494_25, f32::INFINITY, -0.9885]);
    }

    #[test]
    fn waveform_peaks_follow_the_mix_of_both_channels() {
        // Given
        let mut state = AudioBufferState::default();
        let opposite_channels = [0.5_f32, -0.5].repeat(256);

        // When
        state.push_interleaved_stereo(&opposite_channels);

        // Then
        assert_eq!(state.peaks, vec![(0.0, 0.0)]);
    }

    #[test]
    fn the_live_level_moves_most_of_the_way_to_the_loudest_sample() {
        // Given
        let mut state = AudioBufferState::default();

        // When
        state.push_interleaved_stereo(&[0.5, -0.1]);
        let after_one = state.live_level;
        state.push_interleaved_stereo(&[0.5, -0.1]);

        // Then
        assert_close(&[after_one, state.live_level], &[0.3, 0.42]);
    }
}
