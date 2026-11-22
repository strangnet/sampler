# gpui notes

gpui (0.2.2) has thin docs; the source is the reference. Read it at:

```
~/.cargo/registry/src/index.crates.io-*/gpui-0.2.2/src
```

Key files: `window.rs` (window-level events, cursor, paint-phase APIs), `elements/div.rs` (element listeners), `platform.rs` (`CursorStyle` variants), `interactive.rs` (event types).

## Gotchas found the hard way

- **Window-wide cursor**: `Window::set_window_cursor_style` overrides every element's `.cursor(...)`, but may only be called during paint. Call it from a `canvas` paint closure (see `pointer_overlay` in `src/main.rs`). An element's `.cursor(...)` alone loses to whatever element is under the pointer, e.g. a button mid-drag.
- **Pointer leaving the window**: arrives as `MouseExitEvent`, reachable only through `window.on_mouse_event` (also paint-only), not through a div listener. Root `on_mouse_move` stops firing once the pointer is outside the window.
- **`on_hover`** requires a stateful element (`.id(...)`), and it reports `false` while a press on that element is pending or a gpui drag is active, so it can't track hover during a drag.
- **A zero-size `canvas`** still runs its paint closure, so `canvas(..).absolute().size_0()` works as a host for paint-phase window calls.
- **Releasing outside the window**: use `on_mouse_up_out` alongside `on_mouse_up` to end a drag.
