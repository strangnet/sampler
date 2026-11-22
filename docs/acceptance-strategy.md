# Acceptance strategy: Sampler

Agreed 2026-11-22. How it's driven, and why in-process: [ADR-0001](adr/0001-in-process-acceptance-driver.md).

Gherkin: no. Each scenario is a plain `#[gpui::test]` in `tests/acceptance/` that calls the DSL, with Given/When/Then comments.

## Scenarios

Recording
1. Recording captures audio from the chosen source.
2. A closed source falls back to All System Audio when sources are refreshed.
3. Recording again starts a fresh Recording, with no Selection.
4. Clear empties the Recording, and Play and Save become unavailable.

Playback
5. Play with no Selection plays the whole Recording.
6. Play with a Selection plays only the Selection.
7. Loop repeats playback.
8. Space starts and stops playback.

Selection
9. Dragging across the waveform creates a Selection.
10. Dragging a Selection handle resizes it.
11. Dragging inside a Selection moves it at the same length.
12. Escape clears the Selection.
13. The Selection can't be changed while recording.

Saving
14. Quick Save with no Selection writes the whole Recording as a WAV.
15. Quick Save with a Selection writes only the Selection.
16. A normalised export peaks at −0.1 dBFS.
17. Save WAV… writes the Selection to the chosen file and adds `.wav`.
18. Cancelling the Save dialog saves nothing.
19. There's nothing to save before recording.

## DSL areas

Times are seconds of the Recording.

| Area | Actions | Checks |
|---|---|---|
| `sources` | `app_is_running(name)`, `chooses(label)`, `app_quits(name)`, `refreshes()` | `confirm_chosen(label)`, `confirm_recorded_from(label)` |
| `recording` | `records(secs)`, `in_progress_for(secs)`, `clears()` | `confirm_length(secs)`, `confirm_empty()` |
| `selection` | `selects(from, to)`, `drags_end_handle_to(to)`, `drags_it_by(secs)`, `presses_escape()` | `confirm_spans(from, to)`, `confirm_none()` |
| `playback` | `plays()`, `presses_space()`, `presses_loop()` | `confirm_playing(from, to)`, `confirm_playing_once()`, `confirm_looping()`, `confirm_stopped()`, `confirm_unavailable()` |
| `saving` | `quick_saves()`, `presses_normalise()`, `saves_as(name)`, `cancels_save()` | `confirm_quick_saved(from, to)`, `confirm_quick_saved_peak_dbfs(dbfs)`, `confirm_file_holds(name, from, to)`, `confirm_nothing_saved()`, `confirm_unavailable()` |
| `status` | (none) | `confirm_says(text)` |

Scenario 8 runs as two tests, `space_starts_playback` and `space_stops_playback`, so each has a single When.

## Driver

`GpuiDriver`, selected by `ACCEPTANCE_DRIVER` (default and only value: `gpui`). It hosts `SamplerApp` in a `VisualTestContext` window per test.

- **Input:** clicks at `debug_selector`-tagged elements, mouse drags on the `waveform` element, `simulate_keystrokes`, `simulate_new_path_selection`. After each action the executor runs until idle.
- **Checks:** read-only `SamplerApp` accessors, WAVs read back from the temp folder, and the playback stub. Failures name actual and expected values.

## Isolation

- **Partition:** each test has its own `TestAppContext`, `SamplerApp`, stubs and temp save folder.
- **Aliasing:** none; nothing is shared between tests.
- **Defaults:** stub audio is a ramp rising 0.1 per second, so a slice can be located in time to about 1 ms after 16-bit export.

## Stubs

- **ScreenCaptureKit → `StubCapture`:** a scriptable list of running apps. Recording pushes ramp audio into the app's buffer.
- **rodio → `StubPlayback`:** records the last play request and reports playing until stopped.
- **`$HOME` folders → a per-test temp dir.**
- **Save dialog → gpui's test platform.**
