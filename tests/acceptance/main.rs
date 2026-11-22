//! Acceptance tests: scenarios (layer 1) call the DSL (layer 2), which calls
//! a protocol driver (layer 3). Strategy: docs/acceptance-strategy.md.

mod drivers;
mod dsl;
mod playback;
mod recording;
mod saving;
mod selection;
mod stubs;
