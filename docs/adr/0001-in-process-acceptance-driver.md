# ADR-0001: Acceptance tests drive the app in-process through gpui

Status: accepted (2026-11-22)

## Context

The `acceptance-testing` skill runs scenarios against the deployed system, started outside the suite and reached only through its public boundary (an HTTP API, a browser, a CLI). Sampler is a macOS desktop app with none of these. Its only boundary is the window.

A black-box driver would launch the binary and drive it through macOS Accessibility. That needs Accessibility and Screen Recording permissions, which CI runners don't grant. gpui 0.2's accessibility tree is too thin to find controls reliably, and real capture and playback make results depend on what the machine is playing.

## Decision

Acceptance tests host the real `SamplerApp` in a gpui `VisualTestContext` window, one per test:

- **Input** goes through real gpui events: clicks at the bounds of elements tagged with `debug_selector`, mouse drags on the waveform, keystrokes, and gpui's simulated Save dialog.
- **Checks** read what the screen shows through read-only accessors on `SamplerApp`, read saved WAVs back from disk, and inspect the playback stub.
- **Stubs** replace only the operating-system boundaries, through `Backends` (#3): ScreenCaptureKit capture, rodio playback, and the `$HOME` save folders.

## Consequences

- The suite runs on CI in milliseconds and needs no permissions.
- It is grey-box. It doesn't cover the ScreenCaptureKit buffer conversion, real audio output, or pixels actually drawn. Those still need a manual check (`cargo run`) when that code changes.
- Production code carries `debug_selector` tags (no-ops in normal builds) and a few read-only accessors that exist for tests.
- Each test owns its whole system, so the skill's identifier aliasing isn't needed.
- If gpui gains a usable accessibility tree, a black-box driver can sit behind the same driver trait without changing the scenarios or the DSL.
