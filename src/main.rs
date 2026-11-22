use gpui::{
    App, AppContext, Application, Bounds, KeyBinding, WindowBounds, WindowOptions, px, size,
};

use sampler::app::{Backends, CloseWindow, Quit, SamplerApp};

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.activate(true);

        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-w", CloseWindow, None),
        ]);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(960.0), px(520.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                let app = cx.new(|cx| SamplerApp::new(Backends::system(), cx));
                window.focus(app.read(cx).focus_handle());
                app
            },
        )
        .expect("Failed to open Sampler window");
    });
}
