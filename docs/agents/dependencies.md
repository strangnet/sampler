# Dependency docs

Look up library APIs with the Context7 MCP (`query-docs`), passing the ID below directly instead of resolving it. Context7 serves docs; it doesn't serve undocumented behaviour, so for gpui also read `docs/agents/gpui.md`.

| Crate | Locked | Context7 ID | Version match |
|---|---|---|---|
| gpui | 0.2.2 | `/websites/rs_gpui_gpui` | docs.rs latest (0.3.x): may drift from 0.2.2 |
| screencapturekit | 1.5.4 | `/websites/doom-fish_github_io_screencapturekit-rs` | unversioned, tracks current |
| rodio | 0.20.1 | `/rustaudio/rodio/v0.21.1` | nearest; no 0.20 index |
| hound | 3.5.1 | `/ruuda/hound/v3.5.1` | exact |

## Confirm against the crate source when

- the version doesn't match exactly (gpui, rodio), and the answer drives code you write, or
- the question is about runtime behaviour (event dispatch order, which phase an API may run in, threading) rather than a signature.

The exact source lives at `~/.cargo/registry/src/index.crates.io-*/<crate>-<locked version>/src`.

When a crate in `Cargo.toml` is added or bumped, update its row here.
