# ADR-0002: Windows capture through WASAPI process loopback

Status: accepted (2026-11-24)

## Context

Sampler captures a Source (All System Audio, or one running application) through ScreenCaptureKit on macOS, behind the `Capture` trait. Windows support has to reach parity from one codebase. Windows offers per-application capture only through WASAPI *process loopback*. Microsoft documents its minimum as build 20348 in most places and 20438 on the `ActivateAudioInterfaceAsync` page; either way, no consumer Windows 10 release (the last is build 19045) qualifies. Classic loopback on a render device captures everything the device plays, including Sampler's own playback, in the device's mix format.

## Decision

The Windows `Capture` backend uses process loopback for both kinds of Source, through the `wasapi` crate, with the `windows` crate for what `wasapi` doesn't wrap (window enumeration, process image paths, AUMIDs, the OS build):

- **One application**: include the target process tree, so audio from child helper processes (Chrome's audio utility, Electron renderers) is captured through the app the user picked.
- **All System Audio**: exclude Sampler's own process tree. This matches the macOS backend, which excludes the current process's audio.
- Both request 48 kHz stereo f32 with `AUTOCONVERTPCM`, the format the Recording already uses.
- **Windows 11 (build 22000) is the minimum**, including for Windows Server builds that technically qualify, to keep one tested target. Record checks the OS build before any activation and, below it, fails with a status message; it never attempts activation there.
- **The source list has one entry per app.** It is the processes that own a visible top-level window, plus processes with an audio session on any active render endpoint whose ancestors aren't already listed. A window owned by `ApplicationFrameHost.exe` (packaged apps) isn't listed; the packaged app appears through its own audio session once it has played.
- **An app is found again only when the match is certain**, on both platforms: a live PID whose `app_id` still matches, else an `app_id` that exactly one running app has, else the app is gone. Refresh falls back to All System Audio; Record fails with "no longer running". On Windows, Record holds a handle to the resolved process from that check through activation, so its PID can't be reused in between.
- **COM work runs off the UI thread.** gpui's UI thread is a single-threaded apartment and activation blocks without a timeout, so each Record activates on its own thread in the multithreaded apartment while the UI waits with a timeout. A timed-out activation is abandoned: if it completes later it is released without touching the Recording, and `is_recording` stays false. A stuck activation can't block later Records, listing or Stop.
- **One clock defines Recording time.** Packets are placed by their QPC timestamps relative to the start of the Recording, and the gap between the end of the written audio and a packet (or, while no packets arrive, the present) is filled with zeros. Silence is therefore never padded twice, and continuous audio is never interrupted by padding.

## Considered options

- **`cpal` loopback** (already a dependency through rodio): system loopback only, no process loopback, and no format conversion on capture streams.
- **Classic loopback for All System Audio**: works on Windows 10, but records Sampler's own playback and delivers whatever mix format the device uses.
- **Raw Win32 through the `windows` crate alone**: we would hand-write the asynchronous activation, capture client and session enumeration that `wasapi` already wraps. We still use `windows` directly for the rest.

## Consequences

- Windows 10 and Windows Server users can't record.
- `RunningAppInfo` identifies an app by a platform-neutral `app_id` (bundle ID on macOS; AUMID, or exe path, on Windows). The stricter matching rule also changes macOS: a reused PID no longer matches, and a bundle ID shared by two running instances no longer picks one of them.
- `screencapturekit` becomes a macOS-only dependency, and `wasapi` and `windows` become Windows-only; CI runs `scripts/check.sh` on both platforms.
- A packaged app can't be chosen until it has played audio once.
- Real capture isn't covered by CI (hosted Windows runners have no audio endpoint); as with ScreenCaptureKit under ADR-0001, it needs a manual check when it changes.
- Unconfirmed, to be tested on real hardware: whether process loopback sends packets during silence (the clock rule is correct either way); whether ScreenCaptureKit advances a macOS Recording during silence, which decides whether padding is parity; whether loopback activates on a PID that hasn't opened audio yet; whether All System Audio follows a change of default output device.
