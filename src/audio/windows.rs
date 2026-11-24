//! Capture through WASAPI process loopback (Windows 11; see ADR-0002).
//!
//! An application Source is captured with its process tree included, so
//! audio from child helper processes comes with it. All System Audio
//! excludes Sampler's own process tree, as the macOS backend does. COM work
//! runs on a capture thread in the multithreaded apartment, never on gpui's
//! UI thread.

use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

use wasapi::{
    AudioCaptureClient, AudioClient, Direction, Handle, SampleType, StreamMode, WasapiError,
    WaveFormat,
};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GetWindow, GetWindowTextLengthW, GetWindowThreadProcessId,
    IsWindowVisible,
};
use windows::core::{BOOL, PWSTR};

use super::{AudioBufferState, AudioSource, CHANNELS, Capture, RunningAppInfo, SAMPLE_RATE};

/// How long Record waits for the capture thread to activate before giving up.
const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(2);
/// How long the capture loop waits for a packet before checking for Stop.
const PACKET_WAIT_MS: u32 = 100;
const BYTES_PER_SAMPLE: usize = 4;

/// Apps with a visible top-level window, named by their exe file.
fn list_running_apps() -> Vec<RunningAppInfo> {
    let self_pid = std::process::id();
    let mut seen = HashSet::new();
    let mut apps: Vec<RunningAppInfo> = windowed_process_ids()
        .into_iter()
        .filter(|&pid| pid != self_pid && seen.insert(pid))
        .filter_map(|pid| {
            let path = exe_path(pid)?;
            let name = Path::new(&path).file_stem()?.to_string_lossy().into_owned();
            Some(RunningAppInfo {
                pid: pid as i32,
                name,
                bundle_id: path,
            })
        })
        .collect();

    apps.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.pid.cmp(&b.pid))
    });
    apps
}

/// Process IDs owning a visible, titled, unowned top-level window.
fn windowed_process_ids() -> Vec<u32> {
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` is the `&mut Vec<u32>` passed to `EnumWindows`
        // below, which outlives the enumeration.
        let pids = unsafe { &mut *(lparam.0 as *mut Vec<u32>) };
        // SAFETY: plain Win32 queries on a window handle EnumWindows gave us.
        unsafe {
            let unowned = GetWindow(hwnd, GW_OWNER).is_err();
            if IsWindowVisible(hwnd).as_bool() && unowned && GetWindowTextLengthW(hwnd) > 0 {
                let mut pid = 0_u32;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if pid != 0 {
                    pids.push(pid);
                }
            }
        }
        true.into()
    }

    let mut pids: Vec<u32> = Vec::new();
    // SAFETY: the callback only touches `pids` through `lparam`, and only
    // during this call.
    let _ = unsafe { EnumWindows(Some(collect), LPARAM(&mut pids as *mut Vec<u32> as isize)) };
    pids
}

/// The full path of a process's executable, if it can be queried.
fn exe_path(pid: u32) -> Option<String> {
    // SAFETY: the handle is closed before returning, and the buffer length
    // passed in matches the buffer.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0_u16; 1024];
        let mut len = buffer.len() as u32;
        let queried = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(process);
        queried.ok()?;
        Some(String::from_utf16_lossy(&buffer[..len as usize]))
    }
}

struct Session {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

pub struct AudioRecorder {
    session: Option<Session>,
}

impl Default for AudioRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioRecorder {
    pub fn new() -> Self {
        Self { session: None }
    }
}

impl Capture for AudioRecorder {
    fn running_apps(&self) -> Result<Vec<RunningAppInfo>, String> {
        Ok(list_running_apps())
    }

    fn is_recording(&self) -> bool {
        self.session.is_some()
    }

    fn start(
        &mut self,
        shared: Arc<Mutex<AudioBufferState>>,
        source: &AudioSource,
    ) -> Result<(), String> {
        if self.session.is_some() {
            return Ok(());
        }

        let (target_pid, include_tree) = match source {
            AudioSource::AllSystemAudio => (std::process::id(), false),
            AudioSource::Application(info) => (info.pid as u32, true),
        };

        if let Ok(mut state) = shared.lock() {
            state.clear();
        }

        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread_stop = stop.clone();
        let thread = std::thread::Builder::new()
            .name("wasapi-capture".to_string())
            .spawn(move || capture_thread(target_pid, include_tree, shared, thread_stop, ready_tx))
            .map_err(|e| format!("Could not start audio capture: {e}"))?;

        match ready_rx.recv_timeout(ACTIVATION_TIMEOUT) {
            Ok(Ok(())) => {
                self.session = Some(Session { stop, thread });
                Ok(())
            }
            Ok(Err(err)) => {
                let _ = thread.join();
                Err(err)
            }
            Err(_) => {
                // The thread may be stuck in activation: leave it behind, and
                // have it exit without touching the Recording if it wakes up.
                stop.store(true, Ordering::Release);
                Err("Recording couldn't start: Windows didn't respond in time".to_string())
            }
        }
    }

    fn stop(&mut self) -> Result<(), String> {
        if let Some(session) = self.session.take() {
            session.stop.store(true, Ordering::Release);
            session
                .thread
                .join()
                .map_err(|_| "The audio capture thread failed".to_string())?;
        }
        Ok(())
    }
}

impl Drop for AudioRecorder {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// An activated, initialised loopback stream.
struct Stream {
    client: AudioClient,
    capture: AudioCaptureClient,
    event: Handle,
}

fn open_stream(target_pid: u32, include_tree: bool) -> Result<Stream, String> {
    let fail = |e: WasapiError| format!("Could not start audio capture: {e}");
    let mut client =
        AudioClient::new_application_loopback_client(target_pid, include_tree).map_err(fail)?;
    let format = WaveFormat::new(
        32,
        32,
        SampleType::Float,
        SAMPLE_RATE as usize,
        CHANNELS as usize,
        None,
    );
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: 0,
    };
    client
        .initialize_client(&format, Direction::Capture, mode, None)
        .map_err(fail)?;
    let event = client.set_get_eventhandle().map_err(fail)?;
    let capture = client.get_audiocaptureclient().map_err(fail)?;
    Ok(Stream {
        client,
        capture,
        event,
    })
}

fn capture_thread(
    target_pid: u32,
    include_tree: bool,
    shared: Arc<Mutex<AudioBufferState>>,
    stop: Arc<AtomicBool>,
    ready: mpsc::Sender<Result<(), String>>,
) {
    // Joining the multithreaded apartment can only fail if this thread is
    // already in another apartment, which a fresh thread never is.
    let _ = wasapi::initialize_mta();

    let stream = match open_stream(target_pid, include_tree) {
        Ok(stream) => stream,
        Err(err) => {
            let _ = ready.send(Err(err));
            return;
        }
    };
    if stop.load(Ordering::Acquire) {
        // Record gave up waiting for us.
        return;
    }
    if let Err(e) = stream.client.start_stream() {
        let _ = ready.send(Err(format!("Could not start audio capture: {e}")));
        return;
    }
    let _ = ready.send(Ok(()));

    if let Err(e) = capture_loop(&stream, &shared, &stop)
        && let Ok(mut state) = shared.lock()
    {
        state.error = Some(format!("Audio stream error: {e}"));
    }
    let _ = stream.client.stop_stream();
}

/// Moves packets into the Recording until Stop. WASAPI sends nothing while
/// the Source is silent, so a wait that times out just means silence.
fn capture_loop(
    stream: &Stream,
    shared: &Mutex<AudioBufferState>,
    stop: &AtomicBool,
) -> Result<(), WasapiError> {
    let mut bytes: VecDeque<u8> = VecDeque::new();
    while !stop.load(Ordering::Acquire) {
        match stream.event.wait_for_event(PACKET_WAIT_MS) {
            Ok(()) => {}
            Err(WasapiError::EventTimeout) => continue,
            Err(e) => return Err(e),
        }

        while stream.capture.get_next_packet_size()?.unwrap_or(0) > 0 {
            stream.capture.read_from_device_to_deque(&mut bytes)?;
        }

        let samples = drain_whole_frames(&mut bytes);
        if !samples.is_empty()
            && let Ok(mut state) = shared.lock()
        {
            state.push_interleaved_stereo(&samples);
        }
    }
    Ok(())
}

/// Takes every whole stereo frame of little-endian f32 samples off the front
/// of `bytes`, leaving any partial frame for the next packet.
fn drain_whole_frames(bytes: &mut VecDeque<u8>) -> Vec<f32> {
    let frame_bytes = BYTES_PER_SAMPLE * CHANNELS as usize;
    let whole = bytes.len() - bytes.len() % frame_bytes;
    let taken: Vec<u8> = bytes.drain(..whole).collect();
    taken
        .as_chunks::<BYTES_PER_SAMPLE>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le_bytes(samples: &[f32]) -> VecDeque<u8> {
        samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    #[test]
    fn whole_frames_are_drained_as_samples() {
        // Given
        let mut bytes = le_bytes(&[0.25, -0.5, 1.0, -1.0]);

        // When
        let samples = drain_whole_frames(&mut bytes);

        // Then
        assert_eq!(samples, vec![0.25, -0.5, 1.0, -1.0]);
        assert!(bytes.is_empty(), "drained bytes were left behind");
    }

    #[test]
    fn a_partial_frame_waits_for_the_next_packet() {
        // Given: one whole frame, then the left sample and one stray byte of the next.
        let mut bytes = le_bytes(&[0.25, -0.5, 0.75]);
        bytes.push_back(0xAA);

        // When
        let samples = drain_whole_frames(&mut bytes);

        // Then
        assert_eq!(samples, vec![0.25, -0.5]);
        assert_eq!(bytes.len(), 5, "the partial frame's bytes were not kept");
    }
}
