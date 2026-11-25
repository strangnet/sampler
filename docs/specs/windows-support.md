# Spec: Windows support, V1

Status: approved
Source: conversation (shape session, Q1–Q26, three doubt rounds); ADR-0002

## Problem

Sampler only runs on macOS. Its author wants a proof of concept of how well Windows can be supported. On their own Windows 11 PC they want to pick a Source, record it, trim a Selection and save a WAV sample, as they can on macOS. Success means one codebase that builds and passes its checks on both platforms, plus a real Recording made on Windows hardware that plays back correctly. The macOS app must not lose anything: its behaviour and acceptance suite stay intact.

## Solution

Sampler builds and runs on Windows 11 with the same window, controls and behaviour as on macOS.

- **Sources:** the Source list shows All System Audio and the running apps, and Record captures the chosen Source.
- **Unchanged features:** Selection, Play, Loop, Save, Quick Save and Normalize work as before.
- **Windows conventions:** Quick Save writes to the user's Music folder, and the quit and close-window shortcuts use Ctrl on Windows and Cmd on macOS.
- **App matching, both platforms:** after Refresh or at Record, Sampler finds an app again only when the match is certain. When it isn't, Sampler says so instead of recording the wrong app.

## Scope

V1 is parity on one user's Windows 11 PC, built from source. It also makes app matching stricter on macOS (ADR-0002).

The riskiest assumptions are that gpui 0.2.2 runs this app on Windows and that process loopback records a real app. The first slice therefore tests both with a walking skeleton:

- dependencies gated by target;
- CI running on both platforms;
- the app opening on Windows;
- All System Audio and one windowed app recorded to a WAV through the real backend;
- the macOS silence check.

Later slices add the matching rules, the full Source list, silence handling, the OS gate, and the platform folders and shortcuts.

## Behaviour

### As the user, I want to choose any running app as a Source on Windows, so that I can record just that app

- Given an app with a visible window that has never played audio, when the user opens the Source list, then the app is listed.
- Given an app with a window whose audio plays from a child helper process (Chrome), when the user opens the Source list, then the app is listed once. Recording it captures the helper's audio.
- Given a background process with an audio session and no listed ancestor, when the user opens the Source list, then it is listed by its exe name.
- Given a process playing to an output device that isn't the default, when the user opens the Source list, then it is listed.
- Given a packaged app whose window is hosted by the system frame host, when the user opens the Source list, then it isn't listed until it has its own audio session. After it has played and the user presses Refresh, it is listed.
- Sampler itself and the system-sounds session are never listed.
- Given two listed apps are playing, when the user records one of them, then the Recording contains that app's audio and not the other's.

### As the user, I want All System Audio on Windows to capture every app, so that I can sample whatever is playing

- Given two apps are playing, when the user records All System Audio, then the Recording contains both.

### As the user, I want a Recording to last as long as I recorded, so that what I hear matches what I selected

A Recording's time starts when Record succeeds and ends at Stop.

- Given a chosen app plays a clip with a 3 s silent gap, when the user records it, then the gap in the Recording is 3 s, within 50 ms.
- Given audio plays without a break, when the user records it, then no zeros are inserted into it.
- Given the chosen app makes no sound, when the user records for 5 s, then the Recording is 5 s of silence, within 50 ms.
- Given the chosen app starts playing 2 s after Record, then the Recording starts with 2 s of silence, within 50 ms.

### As the user, I want Sampler to find my app again only when it is sure, so that I never record the wrong app (both platforms)

**When the match is certain:**

- Given the user chose an app and it quit and relaunched, when the user presses Refresh, then the relaunched app is chosen.
- Given the user chose an app and it quit and relaunched, when the user presses Record without Refresh, then the relaunched app is recorded.
- Given two instances of the same app are running and the chosen one is still running, when the user presses Refresh, then the chosen instance stays chosen.

**When it isn't:**

- Given the user chose an app that quit, when the user presses Record, then nothing is recorded and the status says the app is no longer running.
- Given the user chose an app that quit and its process ID now belongs to a different app, when the user presses Refresh, then the Source falls back to All System Audio and the status says so.
- Given the same reused process ID, when the user presses Record without Refresh, then nothing is recorded and the status says the app is no longer running.
- Given two instances of the same app were running and the chosen instance quit, when the user presses Refresh, then the Source falls back to All System Audio and the status says so.
- Given the chosen app has no app identity (macOS: no bundle ID) and it quit, then it is never matched to another app.
- Given the chosen app on Windows has no AUMID and no visible window (for example a script run by `python.exe`), and it quit and relaunched, when the user presses Refresh, then the Source falls back to All System Audio and the status says so. Its exe path isn't a certain identity.

### As the user, I want a clear message when my Windows version can't record, so that I know why nothing happens

- Given Windows below build 22000, when the user presses Record, then nothing is recorded and the status says Windows 11 is required.
- Given Windows below build 22000, when the user opens the Source list, then the apps are still listed.

### As the user, I want Record to fail cleanly if Windows doesn't respond, so that I can carry on

- Given capture activation doesn't finish, when the user presses Record, then within 2 s Sampler isn't recording, the status says recording couldn't start, and the existing Recording is unchanged.
- Given an earlier activation timed out and finishes later, then the Recording is unchanged and Sampler still isn't recording.
- Given an earlier activation is still stuck, when the user presses Refresh or Record again, then each answers on its own and doesn't wait behind the stuck one.

### As the user, I want Windows conventions for shortcuts and folders, so that Sampler feels native

- Ctrl+Q quits and Ctrl+W closes the window on Windows. Cmd+Q and Cmd+W still do on macOS.
- Quick Save writes to `Sampler` inside the user's Music folder: the Windows known folder, or `~/Music` on macOS.
- The Save dialog opens in the Music folder, or in the home folder when there is no Music folder.
- After Quick Save, the status names the folder: `~/Music/Sampler` on macOS, the full path on Windows.

### As the developer, I want the checks to pass on both platforms, so that neither platform breaks unnoticed

- Given a change, when CI runs the check script, then formatting, lints (warnings are errors) and all tests pass on both macOS and Windows.
- The whole acceptance suite runs on Windows.

## State model

Tier: lightweight

Capture on Windows (per Record attempt):

| From | To | When |
|---|---|---|
| Idle | Activating | The user presses Record, the OS gate passed, and the app resolved the Source to a live process (or to All System Audio). |
| Activating | Recording | The activation finished within the timeout, and the process still has the resolved app identity. |
| Activating | Idle | The activation failed or timed out, or the process identity didn't match. The status says why, and the attempt is abandoned. |
| Recording | Idle | The user presses Stop, or the stream reports an error. |
| Abandoned attempt | Released | A late activation is released with no effect. |

Invariants:
- **INV-1**: An abandoned attempt never writes to the Recording, and never makes Sampler report that it is recording.
- **INV-2**: A Source resolves to an app only in two cases. Either a live process whose app identity still matches (checked through the process handle that capture uses, on Windows), or a certain app identity that exactly one running app has. An identity is certain when it is a non-empty bundle ID, an AUMID, or the exe path of an app that owns a visible window.
- **INV-3**: Recording time starts when Record succeeds. Each packet is placed at its timestamp relative to that start. A gap longer than the jitter tolerance is filled with zeros, and silence is never filled twice. The Recording's length equals Stop minus start, within 50 ms.
- **INV-4**: Recording never starts on Windows below build 22000.

Impossible transitions:
- Abandoned → Recording.
- Idle → Recording without passing through Activating.

## Decisions

**Capture backend**
- **Windows capture follows ADR-0002**: WASAPI process loopback through the `wasapi` crate, with the `windows` crate for window, process, AUMID and OS-build queries. One app is recorded with its process tree included. All System Audio excludes Sampler's own process tree. Both capture 48 kHz stereo f32 with automatic conversion.
- **Platform backends** are chosen at compile time by target OS, behind the existing `Capture` trait and `Backends`. ScreenCaptureKit is a macOS-only dependency; `wasapi` and `windows` are Windows-only.

**App matching**
- **App identity**: `RunningAppInfo`'s `bundle_id` becomes a platform-neutral `app_id`. On macOS it holds the bundle ID, which can be empty. On Windows it holds the AUMID, or else the exe path. An exe path is a certain identity only for an app that owns a visible window, because host executables (`python.exe`, `javaw.exe`, `msedgewebview2.exe`) share one path across unrelated apps (INV-2).
- **Matching moves into the app**, so the acceptance seam reaches it. One shared rule (INV-2) runs in the app on both platforms, at Refresh and at Record. Record takes a fresh listing, resolves the Source and passes the resolved app to `start`.
- **The backend's `start` only re-checks the identity.** It confirms that the process it is about to capture still has the resolved app identity. On Windows it opens the process handle, reads the identity through it, and holds the handle through activation. On macOS it checks the PID and bundle ID against ScreenCaptureKit's live list. A mismatch fails with "no longer running".

**OS gate**
- The OS gate is decided once at startup from the build number. It reads `RtlGetVersion`, not `GetVersionEx`, which under-reports without a manifest. The result reaches the app through `Backends` as "recording unavailable: <reason>", so the acceptance seam can set it.

**Threading**
- **Activation threading**: each Record activates on its own thread in the COM multithreaded apartment, and listing also runs off the UI thread. `Capture` stays synchronous: the UI thread waits at most 2 s for either and then gives up. Accepted trade-off: if activation hangs, the window freezes for up to 2 s before the status says recording couldn't start. A non-blocking `Capture` (start, then poll) would avoid that, but would change ADR-0001's stub boundary and every Record step in the DSL. That isn't worth it for an unconfirmed failure path in a proof of concept.

**Silence and timing**
- **Jitter tolerance**: a gap of 10 ms or less between the written audio and a packet's timestamp counts as continuous, and the packet is appended directly. A packet that overlaps the written audio is also appended directly, so device drift doesn't cause clicks. Only larger gaps are filled with zeros, and so is the time to "now" while no packets arrive.

**Pure cores**
- The Source-list builder, the Recording-time placer, the matching rule and the OS gate are pure functions over plain values. They compile and are tested on every platform. The Windows adapter only turns OS calls into their inputs.

**Source list on Windows**
- Listed: processes owning a visible top-level window, plus processes with an audio session on any active render endpoint whose ancestors aren't already listed.
- Not listed: Sampler, the system-sounds session, and windows owned by the system frame host.
- Naming: by the window's app name, else by exe file name.
- Each row is identified by process ID, so two instances of one app are distinct rows.

**Platform conventions and CI**
- **Folders** come from the platform's known folders (the `dirs` crate) instead of `$HOME`.
- **Shortcuts** use gpui's `secondary` modifier: Cmd on macOS, Ctrl elsewhere.
- **CI** becomes a macOS + Windows matrix running the same script under bash. Shell scripts are pinned to LF line endings.
- **Delivery** is built from source on the user's PC. Release builds need Windows SDK 10.0.26100 for gpui's shader compiler (or `GPUI_FXC_PATH`). The README's requirements say so.

## Testing

Work type: domain

Seams:

**`Backends` / `Capture` (ADR-0001): acceptance tests through the DSL, run on macOS and Windows CI.** This seam covers:
- the matching stories, both at Refresh and at Record, because matching now runs in the app;
- the OS-gate stories, through the `Backends` availability;
- the Record-failure statuses.

The capture stub and DSL gain:
- each instance identified by process ID, with a per-instance quit;
- an app that relaunches with a new process ID, without pressing Refresh;
- a process ID reused by a different app;
- a second instance of an app;
- choosing a row by instance;
- the identity re-check in `start` failing;
- `start` timing out.

**Pure cores: unit tests on every platform.** Nothing is doubled; inputs are plain values.
- The Source-list builder takes a process snapshot.
- The matching rule takes a chosen app and a listing.
- The Recording-time placer takes timestamped packets and "now". Its tests include jittered and drifting timestamps, overlaps, and a late first packet.
- The OS gate takes a build number.
- The Quick Save status formatter.

**Real WASAPI and COM adapter:** not reachable in CI, because hosted runners have no audio endpoint. The runbooks below cover it.

Prior art: `a_closed_source_falls_back_to_all_system_audio` and the stub-driven acceptance tests; the unit tests of `reconcile_selected_source`.

Checks other than tests:

**Inspection (CI):** both matrix jobs green. This covers the developer story, and the lint gating of macOS-only code on Windows.

**Inspection of the adapter, plus unit tests of attempt bookkeeping with a fake activation:** covers INV-1, the timeout stories, and the handle-based identity re-check.

**Runbook on the user's Windows 11 PC.** Pass means every step does what its story says and every saved WAV plays correctly in another player. Gaps are measured in an audio editor, not with a stopwatch.
1. Arm Record on a windowed app (for example Spotify), then press Play in it.
2. Record a Chrome tab while a second app also plays, and check that only the tab is in the Recording.
3. Record All System Audio while two apps play, and check that both are in it.
4. Record a reference clip with a 3 s silent gap, start it 2 s after Record, and measure the leading silence and the gap.
5. Record a silent app for 5 s.
6. Quit and relaunch the app, then press Record without Refresh.
7. Press Ctrl+Q and Ctrl+W.
8. Quick Save, then check the folder. Open Save, then check its starting folder.

**Runbook on the user's Mac:**
1. Record a silent app for 5 s and record the reference clip. This covers the parity assumption.
2. Press Cmd+Q and Cmd+W.
3. Open Save, then check its starting folder.

## Boundaries

- Always: keep the Windows adapter thin. Any decision belongs in a pure core with tests.
- Always: run the runbook when the adapter changes, and record the result in the PR.
- Ask first: adding a dependency beyond `wasapi`, `windows` and `dirs`.
- Ask first: any change to macOS behaviour other than the matching rule.
- Never: call WASAPI or COM from gpui's UI thread.
- Never: let a test open a real audio device.
- Never: weaken the CI checks on either platform to get the other one green.

## Assumptions

- [x] **gpui 0.2.2 builds this app on `windows-latest` and runs it on Windows 11.** Test: the first slice's CI job, and `cargo run` on the user's PC. Confirmed in #19: CI builds on `windows-latest`, and the app launched on the user's Windows 11 PC.
- [x] **Process loopback with the tree included records a real app as 48 kHz f32, and excluding Sampler's tree records every other app.** Test: runbook steps 1–3. Confirmed in #19: steps 1–3 passed on the user's Windows 11 PC.
- [x] **Loopback activates on a process that hasn't opened audio yet, and delivers once it plays.** Test: runbook step 1. Confirmed in #19: Record armed before Play recorded the app.
- [x] **A macOS Recording advances in real time during silence**, so padding on Windows is parity. Test: Mac runbook step 1. Confirmed in #19: recording a silent Finder for 5 s gave a 5 s Recording.
- [x] **No consent prompt blocks activation.** If one appears, the timeout turns it into a status message. Test: runbook step 1 on a fresh install. Confirmed in #19: the first run on the user's PC activated with no prompt.
- [x] **2 s is long enough for activation on a healthy machine.** Test: the runbook. If real activations are slow, a non-blocking `Capture` becomes its own spec. Confirmed in #19: every activation in steps 1–3 succeeded within the timeout.
- [ ] **10 ms jitter tolerance is enough to avoid clicks.** Test: runbook steps 1–4.
- [x] **The whole acceptance suite passes on the hosted Windows runner.** Test: the first slice's CI job. Confirmed in #19: all 20 acceptance tests pass on `windows-latest`.

## Out of scope

- **Installer, code signing, SmartScreen:** the user is the only user for now.
- **Windows 10 and Windows Server:** process loopback isn't available on Windows 10, and Server would be a second target to test.
- **Following a change of default output device during a Recording:** unconfirmed in the docs, and an edge case for a proof of concept.
- **Listing packaged apps before they play:** mapping a frame-host window to its app process isn't documented.
- **Tests against real audio devices in CI:** hosted runners have no audio endpoint.
- **A Windows counterpart to the macOS app bundle script:** built from source for now.
- **Windows on ARM:** not the user's machine.
- **A non-blocking `Capture` with a "Starting…" state:** it would change ADR-0001's stub boundary and the DSL. A 2 s freeze on a hung activation is accepted instead.

## Open questions
