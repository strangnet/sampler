//! The Sampler window: Recording, Selection editing, playback and saving.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{
    Bounds, Context, CursorStyle, DispatchPhase, Div, FocusHandle, FontWeight, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseExitEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    SharedString, Stateful, Task, WeakEntity, Window, actions, canvas, div, prelude::*, px, rgb,
};

use crate::audio::{
    AudioBufferState, AudioPlayer, AudioRecorder, AudioSource, CHANNELS, Capture, Playback,
    RunningAppInfo, SAMPLE_RATE, extract_slice, quick_save_dir, quick_save_wav,
    save_wav_with_options, slice_frame_range,
};
use crate::controls::{ButtonTone, format_duration, level_meter, styled_button};
use crate::selection::{
    DragKind, Handle, Selection, SelectionDrag, SelectionHit, WaveformPos, hit_test,
};
use crate::source_picker::{reconcile_selected_source, render_source_dropdown};
use crate::waveform::{WAVEFORM_PAD_X, waveform_canvas};

actions!(sampler, [Quit, CloseWindow]);

#[derive(Clone, Copy)]
enum Action {
    Record,
    Play,
    Loop,
    Clear,
    Normalize,
    QuickSave,
    Save,
}

type ButtonSpec = (&'static str, &'static str, bool, ButtonTone, Action);

/// Maps a window x coordinate to an unclamped position on the drawn waveform.
fn pixel_x_to_waveform_pos(bounds: Bounds<Pixels>, x_px: f32) -> WaveformPos {
    let start_x = f32::from(bounds.origin.x) + WAVEFORM_PAD_X;
    let width = (f32::from(bounds.size.width) - WAVEFORM_PAD_X * 2.0).max(1.0);
    WaveformPos {
        x: (x_px - start_x) / width,
        width,
    }
}

/// A paint-only element for window-wide pointer concerns: pins the cursor
/// while a Selection drag is in progress (so buttons under the pointer don't
/// override it) and clears the waveform hover when the pointer leaves the
/// window.
fn pointer_overlay(
    app: WeakEntity<SamplerApp>,
    drag_cursor: Option<CursorStyle>,
) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            if let Some(style) = drag_cursor {
                window.set_window_cursor_style(style);
            }
            window.on_mouse_event(move |_: &MouseExitEvent, phase, _window, cx| {
                if phase == DispatchPhase::Bubble {
                    let _ = app.update(cx, |t, cx| t.track_pointer(None, cx));
                }
            });
        },
    )
    .absolute()
    .size_0()
}

fn file_name_or_default(path: &Path) -> &str {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("sample.wav")
}

/// `~/Music`, else `~`, else the working directory.
fn fallback_save_directory() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
        let home_path = PathBuf::from(home);
        let music_dir = home_path.join("Music");
        if music_dir.is_dir() {
            return music_dir;
        }
        if home_path.is_dir() {
            return home_path;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn ensure_wav_extension(path: PathBuf) -> PathBuf {
    if path.extension().is_none() {
        path.with_extension("wav")
    } else {
        path
    }
}

fn quick_save_status(path: &Path) -> String {
    let name = file_name_or_default(path);
    let home = std::env::var_os("HOME");
    let folder = path
        .parent()
        .map(
            |p| match home.as_ref().and_then(|h| p.strip_prefix(h).ok()) {
                Some(rel) => format!("~/{}", rel.display()),
                None => p.display().to_string(),
            },
        )
        .unwrap_or_else(|| "~/Music/Sampler".to_string());
    format!("Saved {name} in {folder}")
}

fn hstack(gap: f32) -> Div {
    div().flex().items_center().gap(px(gap))
}

fn xs_label(color: u32) -> Div {
    div().text_xs().text_color(rgb(color)).whitespace_nowrap()
}

fn pill_badge(px_h: f32, py_v: f32, bg: u32, text: u32) -> Div {
    div()
        .px(px(px_h))
        .py(px(py_v))
        .rounded(px(6.0))
        .bg(rgb(bg))
        .text_color(rgb(text))
        .whitespace_nowrap()
}

fn pill_btn(id: &'static str, enabled: bool, active: bool, text_hex: u32) -> Stateful<Div> {
    let (bg, text) = if enabled {
        (0x1e212d, text_hex)
    } else {
        (0x181a22, 0x6b7280)
    };
    let border = if active && enabled {
        0x38bdf8
    } else {
        0x2e3242
    };
    pill_badge(0.0, 5.0, bg, text)
        .id(id)
        .debug_selector(move || id.to_string())
        .flex()
        .items_center()
        .gap(px(6.0))
        .border_1()
        .border_color(rgb(border))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .when(enabled, |el| {
            el.cursor_pointer().hover(|s| s.bg(rgb(0x272b3a)))
        })
}

/// The app's connections to the operating system, swapped out in tests.
pub struct Backends {
    pub capture: Box<dyn Capture>,
    pub playback: Box<dyn Playback>,
    /// Where Quick Save writes.
    pub quick_save_dir: PathBuf,
    /// Where the Save dialog opens until Quick Save has created
    /// `quick_save_dir`; from then on it opens there.
    pub save_dialog_dir: PathBuf,
}

impl Backends {
    /// ScreenCaptureKit capture, rodio playback and the user's Music folders.
    /// Without `$HOME`, Quick Save falls back to the Save dialog's folder.
    pub fn system() -> Self {
        let save_dialog_dir = fallback_save_directory();
        Self {
            capture: Box::new(AudioRecorder::new()),
            playback: Box::new(AudioPlayer::new()),
            quick_save_dir: quick_save_dir().unwrap_or_else(|_| save_dialog_dir.clone()),
            save_dialog_dir,
        }
    }
}

pub struct SamplerApp {
    shared_audio: Arc<Mutex<AudioBufferState>>,
    recorder: Box<dyn Capture>,
    player: Box<dyn Playback>,
    quick_save_dir: PathBuf,
    save_dialog_dir: PathBuf,
    status_message: SharedString,
    available_apps: Vec<RunningAppInfo>,
    selected_source: AudioSource,
    source_menu_open: bool,
    selection: Option<Selection>,
    drag: Option<SelectionDrag>,
    /// Pointer position while it is over the waveform.
    hover: Option<WaveformPos>,
    waveform_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    loop_enabled: Arc<AtomicBool>,
    normalize_export: bool,
    is_saving: bool,
    focus_handle: FocusHandle,
    was_playing: bool,
    _refresh_task: Option<Task<()>>,
    _save_task: Option<Task<()>>,
}

impl SamplerApp {
    pub fn new(backends: Backends, cx: &mut Context<Self>) -> Self {
        let available_apps = backends.capture.running_apps().unwrap_or_default();
        let mut app = Self {
            shared_audio: Arc::new(Mutex::new(AudioBufferState::default())),
            recorder: backends.capture,
            player: backends.playback,
            quick_save_dir: backends.quick_save_dir,
            save_dialog_dir: backends.save_dialog_dir,
            status_message: "Ready — choose a source and click Record".into(),
            available_apps,
            selected_source: AudioSource::AllSystemAudio,
            source_menu_open: false,
            selection: None,
            drag: None,
            hover: None,
            waveform_bounds: Rc::new(Cell::new(None)),
            loop_enabled: Arc::new(AtomicBool::new(false)),
            normalize_export: false,
            is_saving: false,
            focus_handle: cx.focus_handle(),
            was_playing: false,
            _refresh_task: None,
            _save_task: None,
        };
        app.start_refresh_loop(cx);
        app
    }

    /// The status line shown under the controls.
    pub fn status(&self) -> &str {
        &self.status_message
    }

    /// Length of the Recording in seconds.
    pub fn recording_secs(&self) -> f64 {
        self.audio_snapshot().0
    }

    /// The Selection, normalized to the Recording (0.0 = start, 1.0 = end).
    pub fn selection(&self) -> Option<(f32, f32)> {
        self.selection
    }

    /// The chosen audio source, as labelled in the source picker.
    pub fn source_label(&self) -> String {
        self.selected_source.label()
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    fn save_dialog_start(&self) -> &Path {
        if self.quick_save_dir.is_dir() {
            &self.quick_save_dir
        } else {
            &self.save_dialog_dir
        }
    }

    fn set_status(&mut self, msg: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.status_message = msg.into();
        cx.notify();
    }

    fn audio_snapshot(&self) -> (f64, usize, f32, bool) {
        self.shared_audio
            .lock()
            .map(|s| {
                (
                    s.duration_secs(),
                    s.frame_count(),
                    s.live_level,
                    !s.samples.is_empty(),
                )
            })
            .unwrap_or((0.0, 0, 0.0, false))
    }

    fn selected_slice(&self) -> (Vec<f32>, usize) {
        let (samples, frames) = self
            .shared_audio
            .lock()
            .map(|s| (s.samples.clone(), s.frame_count()))
            .unwrap_or_default();
        (extract_slice(&samples, self.selection), frames)
    }

    fn poll_refresh(&mut self, cx: &mut Context<Self>) {
        let (recording, playing) = (self.recorder.is_recording(), self.player.is_playing());
        let finished = self.was_playing && !playing;
        self.was_playing = playing;

        if finished && self.status_message.as_ref().starts_with("Playing") {
            self.status_message = "Playback finished".into();
        }
        if let Ok(state) = self.shared_audio.lock()
            && let Some(err) = &state.error
        {
            self.status_message = err.clone().into();
        }
        if let Some(err) = self.player.take_error() {
            self.status_message = err.into();
            cx.notify();
        }
        if recording || playing || finished {
            cx.notify();
        }
    }

    fn start_refresh_loop(&mut self, cx: &mut Context<Self>) {
        let task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(33))
                    .await;
                if this.update(cx, |app, cx| app.poll_refresh(cx)).is_err() {
                    break;
                }
            }
        });
        self._refresh_task = Some(task);
    }

    fn waveform_pos(&self, x_px: f32) -> Option<WaveformPos> {
        self.waveform_bounds
            .get()
            .map(|b| pixel_x_to_waveform_pos(b, x_px))
    }

    fn start_waveform_drag(&mut self, x_px: f32, cx: &mut Context<Self>) {
        self.source_menu_open = false;
        if self.audio_snapshot().3 && !self.recorder.is_recording() && !self.is_saving {
            self.drag = self
                .waveform_pos(x_px)
                .map(|press| SelectionDrag::begin(self.selection, press));
        }
        cx.notify();
    }

    fn update_drag(&mut self, x_px: f32, cx: &mut Context<Self>) {
        let Some(at) = self.waveform_pos(x_px) else {
            return;
        };
        if let Some(drag) = &mut self.drag {
            self.selection = drag.update(at.x);
            cx.notify();
        }
    }

    fn finish_drag(&mut self, x_px: f32, cx: &mut Context<Self>) {
        if let Some(drag) = self.drag.take() {
            if let Some(at) = self.waveform_pos(x_px) {
                self.selection = drag.finish(at.x);
            }
            cx.notify();
        }
    }

    /// Tracks the pointer over the waveform so hover cursors and handle
    /// highlights follow it; re-renders only when what's under it changes.
    fn track_pointer(&mut self, position: Option<Point<Pixels>>, cx: &mut Context<Self>) {
        let before = self.hover_hit();
        self.hover = position
            .filter(|p| self.waveform_bounds.get().is_some_and(|b| b.contains(p)))
            .and_then(|p| self.waveform_pos(f32::from(p.x)));
        if self.hover_hit() != before {
            cx.notify();
        }
    }

    fn hover_hit(&self) -> Option<SelectionHit> {
        self.hover.map(|at| hit_test(self.selection, at))
    }

    fn on_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        if ev.keystroke.key == "space" && !ev.keystroke.modifiers.modified() && !ev.is_held {
            self.toggle_playback(cx);
        } else if ev.keystroke.key == "escape" {
            self.source_menu_open = false;
            self.selection = None;
            self.drag = None;
            cx.notify();
        }
    }

    fn toggle_source_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        self.source_menu_open = !self.source_menu_open;
        cx.stop_propagation();
        cx.notify();
    }

    fn select_source(&mut self, source: AudioSource, cx: &mut Context<Self>) {
        if self.recorder.is_recording() || self.is_saving {
            return;
        }
        let label = source.label();
        self.selected_source = source;
        self.source_menu_open = false;
        self.set_status(format!("Source: {label}"), cx);
    }

    fn refresh_sources(&mut self, cx: &mut Context<Self>) {
        if self.recorder.is_recording() || self.is_saving {
            return;
        }
        let msg = match self.recorder.running_apps() {
            Ok(apps) => {
                let count = apps.len();
                let (next_source, was_reset) =
                    reconcile_selected_source(&self.selected_source, &apps);
                self.available_apps = apps;
                self.selected_source = next_source;
                if was_reset {
                    format!(
                        "Refreshed ({count} apps) — previous app closed, reset to All System Audio"
                    )
                } else {
                    format!("Refreshed sources ({count} running apps)")
                }
            }
            Err(err) => err,
        };
        self.set_status(msg, cx);
    }

    fn toggle_record(&mut self, cx: &mut Context<Self>) {
        if self.is_saving {
            return;
        }
        self.source_menu_open = false;
        let label = self.selected_source.label();

        if self.recorder.is_recording() {
            let msg = match self.recorder.stop() {
                Ok(()) => {
                    let dur = self
                        .shared_audio
                        .lock()
                        .map(|mut s| {
                            s.flush_partial_peak();
                            s.duration_secs()
                        })
                        .unwrap_or(0.0);
                    format!("Recorded {dur:.2}s from {label}")
                }
                Err(err) => err,
            };
            return self.set_status(msg, cx);
        }

        self.player.stop();
        self.was_playing = false;
        let msg = match self
            .recorder
            .start(self.shared_audio.clone(), &self.selected_source)
        {
            Ok(()) => {
                self.selection = None;
                self.drag = None;
                format!("Recording {label}…")
            }
            Err(err) => err,
        };
        self.set_status(msg, cx);
    }

    fn toggle_playback(&mut self, cx: &mut Context<Self>) {
        if self.recorder.is_recording() || self.is_saving {
            return;
        }
        self.source_menu_open = false;
        if self.player.is_playing() {
            self.player.stop();
            self.was_playing = false;
            return self.set_status("Playback stopped", cx);
        }

        let (slice, frame_count) = self.selected_slice();
        if slice.is_empty() || frame_count == 0 {
            return self.set_status("Nothing to play — record audio first", cx);
        }

        let clip_range = slice_frame_range(frame_count, self.selection);
        match self
            .player
            .play(slice, clip_range, self.loop_enabled.clone())
        {
            Ok(()) => {
                self.was_playing = true;
                let msg = self
                    .selection
                    .map(|_| "Playing selected range…")
                    .unwrap_or("Playing recorded sample…");
                self.set_status(msg, cx);
            }
            Err(err) => self.set_status(err, cx),
        }
    }

    fn toggle_loop(&mut self, cx: &mut Context<Self>) {
        self.source_menu_open = false;
        let next = !self.loop_enabled.fetch_xor(true, Ordering::Relaxed);
        let msg = if next {
            "Loop playback enabled"
        } else {
            "Loop playback disabled"
        };
        self.set_status(msg, cx);
    }

    fn toggle_normalize(&mut self, cx: &mut Context<Self>) {
        self.source_menu_open = false;
        self.normalize_export = !self.normalize_export;
        let msg = if self.normalize_export {
            "Export normalization enabled (-0.1 dBFS)"
        } else {
            "Export normalization disabled"
        };
        self.set_status(msg, cx);
    }

    fn clear_recording(&mut self, cx: &mut Context<Self>) {
        if self.recorder.is_recording() || self.is_saving {
            return;
        }
        self.source_menu_open = false;
        self.player.stop();
        self.was_playing = false;
        self.selection = None;
        self.drag = None;
        if let Ok(mut state) = self.shared_audio.lock() {
            state.clear();
        }
        self.set_status("Cleared — ready to record", cx);
    }

    fn prepare_export_slice(&mut self, cx: &mut Context<Self>) -> Option<Vec<f32>> {
        self.source_menu_open = false;
        let (slice, _) = self.selected_slice();
        if slice.is_empty() {
            self.set_status("Nothing to save — record audio first", cx);
            return None;
        }
        Some(slice)
    }

    fn start_save_task(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce() -> Result<String, String> + Send + 'static,
    ) {
        self.is_saving = true;
        self.set_status("Saving WAV…", cx);

        let task = cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { work() }).await;
            let _ = this.update(cx, |app, cx| {
                app.is_saving = false;
                let msg = match result {
                    Ok(m) | Err(m) => m,
                };
                app.set_status(msg, cx);
            });
        });
        self._save_task = Some(task);
    }

    fn quick_save_recording(&mut self, cx: &mut Context<Self>) {
        if self.recorder.is_recording() || self.is_saving {
            return;
        }
        let Some(slice) = self.prepare_export_slice(cx) else {
            return;
        };
        let normalize = self.normalize_export;
        let dir = self.quick_save_dir.clone();
        self.start_save_task(cx, move || {
            let path = quick_save_wav(&dir, &slice, normalize)?;
            Ok(quick_save_status(&path))
        });
    }

    fn save_recording(&mut self, cx: &mut Context<Self>) {
        if self.recorder.is_recording() || self.is_saving {
            return;
        }
        let Some(slice) = self.prepare_export_slice(cx) else {
            return;
        };

        let normalize = self.normalize_export;
        let prompt = cx.prompt_for_new_path(self.save_dialog_start(), Some("sample.wav"));
        self.is_saving = true;
        self.set_status("Choose WAV destination…", cx);

        let task = cx.spawn(async move |this, cx| {
            let path = match prompt.await {
                Ok(Ok(Some(path))) => ensure_wav_extension(path),
                Ok(Ok(None)) | Err(_) => {
                    let _ = this.update(cx, |app, cx| {
                        app.is_saving = false;
                        app.set_status("Save cancelled", cx);
                    });
                    return;
                }
                Ok(Err(err)) => {
                    let _ = this.update(cx, |app, cx| {
                        app.is_saving = false;
                        app.set_status(format!("Save dialog failed: {err}"), cx);
                    });
                    return;
                }
            };

            let _ = this.update(cx, |app, cx| {
                app.set_status("Saving WAV…", cx);
            });

            let result = cx
                .background_executor()
                .spawn(async move {
                    save_wav_with_options(&path, &slice, normalize)?;
                    Ok::<String, String>(format!("Saved WAV to {}", file_name_or_default(&path)))
                })
                .await;

            let _ = this.update(cx, |app, cx| {
                app.is_saving = false;
                let msg = match result {
                    Ok(m) | Err(m) => m,
                };
                app.set_status(msg, cx);
            });
        });
        self._save_task = Some(task);
    }

    fn dispatch_action(&mut self, action: Action, cx: &mut Context<Self>) {
        use Action::*;
        match action {
            Record => self.toggle_record(cx),
            Play => self.toggle_playback(cx),
            Loop => self.toggle_loop(cx),
            Clear => self.clear_recording(cx),
            Normalize => self.toggle_normalize(cx),
            QuickSave => self.quick_save_recording(cx),
            Save => self.save_recording(cx),
        }
    }

    fn render_source_controls(&self, can_pick: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.source_menu_open;
        let on_open = cx.listener(|this, _ev, w, cx| this.toggle_source_menu(w, cx));
        let on_refresh = cx.listener(|this, _ev, _w, cx| this.refresh_sources(cx));

        let picker = pill_btn("source-picker-btn", can_pick, open, 0xe5e7eb)
            .px(px(10.0))
            .when(can_pick, |el| el.on_mouse_down(MouseButton::Left, on_open))
            .child(
                div()
                    .max_w(px(180.0))
                    .truncate()
                    .child(self.selected_source.label()),
            )
            .child(div().flex_shrink_0().text_color(rgb(0x9ca3af)).child("▾"));
        let refresh = pill_btn("refresh-sources-btn", can_pick, false, 0xcbd5e1)
            .px(px(9.0))
            .when(can_pick, |el| el.on_click(on_refresh))
            .child("Refresh");

        hstack(6.0)
            .relative()
            .child(picker)
            .child(refresh)
            .when(open && can_pick, |el| {
                el.child(render_source_dropdown(
                    &self.selected_source,
                    &self.available_apps,
                    |this: &mut Self, source, _w, cx| this.select_source(source, cx),
                    cx,
                ))
            })
    }

    fn render_header(
        &self,
        s: (bool, f64, usize, bool),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (is_recording, duration_secs, frame_count, has_audio) = s;
        let can_pick = !is_recording && !self.is_saving;
        let fmt = format!(
            "{:.1} kHz · {} ch · 16-bit WAV",
            SAMPLE_RATE as f32 / 1000.0,
            CHANNELS
        );
        let sel_text = has_audio
            .then_some(self.selection)
            .flatten()
            .and_then(|sel| {
                let (s0, s1) = slice_frame_range(frame_count, Some(sel));
                let frac = (s1 - s0).max(0.0) as f64;
                (frac > 0.0).then(|| format!("Sel {}", format_duration(frac * duration_secs)))
            });
        let (dur_border, dur_text) = if is_recording {
            (0xf43f5e, 0xfda4af)
        } else {
            (0x2a2d3a, 0xe5e7eb)
        };

        let title = div()
            .text_lg()
            .font_weight(FontWeight::BOLD)
            .text_color(rgb(0xffffff))
            .whitespace_nowrap()
            .child("Audio Sampler");
        let fmt_badge = pill_badge(8.0, 4.0, 0x1e202b, 0x9ca3af)
            .text_xs()
            .child(fmt);
        let dur_badge = pill_badge(12.0, 5.0, 0x1a1c25, dur_text)
            .border_1()
            .border_color(rgb(dur_border))
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .child(format_duration(duration_secs));
        let right = hstack(10.0)
            .flex_shrink_0()
            .when_some(sel_text, |el, sel| {
                el.child(
                    pill_badge(8.0, 4.0, 0x0c4a6e, 0x7dd3fc)
                        .border_1()
                        .border_color(rgb(0x0284c7))
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .child(sel),
                )
            })
            .child(xs_label(0x9ca3af).child(format!("{frame_count} frames")))
            .child(dur_badge);

        hstack(12.0)
            .justify_between()
            .child(
                hstack(10.0)
                    .child(title)
                    .child(fmt_badge)
                    .child(self.render_source_controls(can_pick, cx)),
            )
            .child(right)
    }

    fn render_bar(&self, f: (bool, bool, bool, bool), cx: &mut Context<Self>) -> impl IntoElement {
        use Action::*;
        let (is_recording, is_playing, loop_active, can_act) = f;
        let rec_label = if is_recording { "Stop" } else { "Record" };
        let play_label = if is_playing { "Stop Play" } else { "Play" };
        let (rt, lt, nt) = (
            ButtonTone::Danger {
                active: is_recording,
            },
            ButtonTone::Toggle {
                active: loop_active,
            },
            ButtonTone::Toggle {
                active: self.normalize_export,
            },
        );
        let (n, e) = (ButtonTone::Neutral, can_act);

        let left: [ButtonSpec; 4] = [
            ("record-btn", rec_label, !self.is_saving, rt, Record),
            ("play-btn", play_label, e, n, Play),
            ("loop-btn", "Loop", true, lt, Loop),
            ("clear-btn", "Clear", e, n, Clear),
        ];
        let right: [ButtonSpec; 3] = [
            ("normalize-btn", "Normalize", true, nt, Normalize),
            ("quick-save-btn", "Quick Save", e, n, QuickSave),
            ("save-btn", "Save WAV…", e, ButtonTone::Primary, Save),
        ];
        let row = |specs: &[ButtonSpec], cx: &mut Context<Self>| {
            hstack(8.0).children(specs.iter().map(|&(id, label, enabled, tone, action)| {
                styled_button(id, label, enabled, tone)
                    .debug_selector(move || id.to_string())
                    .when(enabled, |el| {
                        el.on_click(
                            cx.listener(move |this, _ev, _w, cx| this.dispatch_action(action, cx)),
                        )
                    })
            }))
        };
        let status = xs_label(0x9ca3af)
            .flex_1()
            .min_w_0()
            .truncate()
            .child(self.status_message.clone());
        let hint = xs_label(0x6b7280)
            .flex_shrink_0()
            .child("Space: Play/Stop · Drag waveform: Trim · Esc: Clear");

        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                hstack(10.0)
                    .justify_between()
                    .child(row(&left, cx))
                    .child(row(&right, cx)),
            )
            .child(hstack(12.0).justify_between().child(status).child(hint))
    }
}

impl Render for SamplerApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_recording = self.recorder.is_recording();
        let is_playing = self.player.is_playing();
        let loop_active = self.loop_enabled.load(Ordering::Relaxed);
        let (duration_secs, frame_count, live_level, has_audio) = self.audio_snapshot();
        let meter_pct = if is_recording {
            (live_level * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };
        let can_act = has_audio && !is_recording && !self.is_saving;
        let wave_border = if is_recording { 0x4c1d2b } else { 0x262936 };
        let hover_hit = can_act.then(|| self.hover_hit()).flatten();
        let drag_kind = self.drag.map(|d| d.kind());
        let cursor = match (drag_kind, hover_hit) {
            (Some(DragKind::Resize), _) | (None, Some(SelectionHit::Handle(_))) => {
                CursorStyle::ResizeLeftRight
            }
            (Some(DragKind::Move), _) => CursorStyle::ClosedHand,
            (None, Some(SelectionHit::Inside)) => CursorStyle::OpenHand,
            _ => CursorStyle::Crosshair,
        };
        let highlighted_handle: Option<Handle> = match (self.drag, hover_hit) {
            (Some(drag), _) => drag.dragged_handle(),
            (None, Some(SelectionHit::Handle(handle))) => Some(handle),
            _ => None,
        };
        let drag_cursor = self.drag.is_some().then_some(cursor);

        let on_up = |t: &mut Self, ev: &MouseUpEvent, _w: &mut Window, cx: &mut Context<Self>| {
            t.finish_drag(f32::from(ev.position.x), cx);
        };
        let waveform_panel = div()
            .relative()
            .flex_1()
            .w_full()
            .rounded(px(10.0))
            .bg(rgb(0x161821))
            .border_1()
            .border_color(rgb(wave_border))
            .overflow_hidden()
            .when(can_act, |el| el.cursor(cursor))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|t, ev: &MouseDownEvent, _w, cx| {
                    t.start_waveform_drag(f32::from(ev.position.x), cx)
                }),
            )
            .child(
                div()
                    .size_full()
                    .debug_selector(|| "waveform".to_string())
                    .child(waveform_canvas(
                        self.shared_audio.clone(),
                        is_recording,
                        self.selection,
                        highlighted_handle,
                        self.player.playhead_position(),
                        self.waveform_bounds.clone(),
                    )),
            )
            .when(!has_audio && !is_recording, |el| {
                el.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_sm()
                        .text_color(rgb(0x6b7280))
                        .child("Click Record to capture system or application audio"),
                )
            });

        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .track_focus(&self.focus_handle)
            .on_action(|_: &CloseWindow, window, _| {
                window.remove_window();
            })
            .bg(rgb(0x111217))
            .text_color(rgb(0xe5e7eb))
            .p(px(20.0))
            .gap(px(14.0))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|t, _ev: &MouseDownEvent, _w, cx| {
                    if t.source_menu_open {
                        t.source_menu_open = false;
                        cx.notify();
                    }
                }),
            )
            .on_mouse_move(cx.listener(|t, ev: &MouseMoveEvent, _w, cx| {
                t.track_pointer(Some(ev.position), cx);
                t.update_drag(f32::from(ev.position.x), cx)
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(on_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(on_up))
            .on_key_down(cx.listener(|t, ev: &KeyDownEvent, _w, cx| t.on_key(ev, cx)))
            .child(self.render_header((is_recording, duration_secs, frame_count, has_audio), cx))
            .child(pointer_overlay(cx.entity().downgrade(), drag_cursor))
            .child(waveform_panel)
            .child(level_meter(meter_pct))
            .child(self.render_bar((is_recording, is_playing, loop_active, can_act), cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, size};
    use rstest::rstest;

    /// A 208 px waveform at x = 20: 200 px drawn between 4 px of padding.
    fn waveform_bounds() -> Bounds<Pixels> {
        Bounds {
            origin: point(px(20.0), px(80.0)),
            size: size(px(208.0), px(200.0)),
        }
    }

    #[rstest]
    #[case::the_start_of_the_recording(24.0, 0.0)]
    #[case::the_middle(124.0, 0.5)]
    #[case::the_end_of_the_recording(224.0, 1.0)]
    #[case::left_of_the_recording_unclamped(4.0, -0.1)]
    #[case::right_of_the_recording_unclamped(264.0, 1.2)]
    fn a_window_x_maps_onto_the_drawn_waveform(#[case] window_x: f32, #[case] expected: f32) {
        // When
        let pos = pixel_x_to_waveform_pos(waveform_bounds(), window_x);

        // Then
        assert!((pos.x - expected).abs() < 1e-6, "{} != {expected}", pos.x);
    }

    #[test]
    fn the_drawn_waveform_excludes_the_padding() {
        // When
        let pos = pixel_x_to_waveform_pos(waveform_bounds(), 24.0);

        // Then
        assert_eq!(pos.width, 200.0);
    }

    #[test]
    fn a_waveform_narrower_than_its_padding_counts_as_one_pixel_wide() {
        // Given
        let bounds = Bounds {
            origin: point(px(20.0), px(80.0)),
            size: size(px(6.0), px(200.0)),
        };

        // When
        let pos = pixel_x_to_waveform_pos(bounds, 24.0);

        // Then
        assert_eq!(pos.width, 1.0);
    }

    #[rstest]
    #[case::appended_when_missing("/tmp/my_sample", "/tmp/my_sample.wav")]
    #[case::kept_when_already_wav("/tmp/my_sample.wav", "/tmp/my_sample.wav")]
    #[case::kept_when_another_format("/tmp/my_sample.aiff", "/tmp/my_sample.aiff")]
    fn a_save_path_gets_a_wav_extension_only_when_it_has_none(
        #[case] chosen: &str,
        #[case] expected: &str,
    ) {
        // When
        let path = ensure_wav_extension(PathBuf::from(chosen));

        // Then
        assert_eq!(path, PathBuf::from(expected));
    }
}
