use gpui::{
    Context, Div, ElementId, FontWeight, IntoElement, SharedString, Stateful, Window, deferred,
    div, prelude::*, px, rgb,
};

use crate::audio::{AudioSource, RunningAppInfo};

/// Reconciles the currently selected audio source against an updated list of running apps.
///
/// Returns the updated `AudioSource` and whether it had to fall back to `AllSystemAudio`.
pub fn reconcile_selected_source(
    current: &AudioSource,
    apps: &[RunningAppInfo],
) -> (AudioSource, bool) {
    match current {
        AudioSource::AllSystemAudio => (AudioSource::AllSystemAudio, false),
        AudioSource::Application(info) => {
            let matched = apps.iter().find(|app| app.pid == info.pid).or_else(|| {
                if info.bundle_id.is_empty() {
                    None
                } else {
                    apps.iter().find(|app| app.bundle_id == info.bundle_id)
                }
            });

            match matched {
                Some(app) => (AudioSource::Application(app.clone()), false),
                None => (AudioSource::AllSystemAudio, true),
            }
        }
    }
}

fn source_dropdown_item(
    id: impl Into<ElementId>,
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
    is_selected: bool,
) -> Stateful<Div> {
    let title = title.into();
    let selector_title = title.clone();
    div()
        .id(id)
        .debug_selector(move || format!("source:{selector_title}"))
        .w_full()
        .min_w_0()
        .px(px(10.0))
        .py(px(7.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .bg(if is_selected {
            rgb(0x0c4a6e)
        } else {
            rgb(0x1a1c26)
        })
        .hover(move |s| {
            s.bg(if is_selected {
                rgb(0x075985)
            } else {
                rgb(0x252938)
            })
        })
        .flex()
        .flex_col()
        .gap(px(1.0))
        .child(
            div()
                .text_sm()
                .font_weight(if is_selected {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::MEDIUM
                })
                .text_color(if is_selected {
                    rgb(0xe0f2fe)
                } else {
                    rgb(0xe5e7eb)
                })
                .truncate()
                .child(title),
        )
        .child(
            div()
                .text_xs()
                .text_color(if is_selected {
                    rgb(0x7dd3fc)
                } else {
                    rgb(0x6b7280)
                })
                .truncate()
                .child(subtitle.into()),
        )
}

/// Renders the floating audio source selector dropdown listing system audio and running apps.
pub fn render_source_dropdown<T: 'static>(
    selected_source: &AudioSource,
    available_apps: &[RunningAppInfo],
    on_select: impl Fn(&mut T, AudioSource, &mut Window, &mut Context<T>) + Clone + 'static,
    cx: &mut Context<T>,
) -> impl IntoElement {
    let is_all = matches!(selected_source, AudioSource::AllSystemAudio);
    let on_select_all = on_select.clone();

    deferred(
        div()
            .absolute()
            .top_full()
            .left_0()
            .mt(px(6.0))
            .w(px(320.0))
            .rounded(px(8.0))
            .bg(rgb(0x1a1c26))
            .border_1()
            .border_color(rgb(0x2f3447))
            .shadow_lg()
            .p(px(4.0))
            .occlude()
            .child(
                div()
                    .id("source-list")
                    .max_h(px(240.0))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        source_dropdown_item(
                            "source-item-all",
                            "All System Audio",
                            "Capture all system desktop audio",
                            is_all,
                        )
                        .on_click(cx.listener(
                            move |this, _ev, window, cx| {
                                on_select_all(this, AudioSource::AllSystemAudio, window, cx);
                            },
                        )),
                    )
                    .when(available_apps.is_empty(), |el| {
                        el.child(
                            div()
                                .px(px(10.0))
                                .py(px(8.0))
                                .text_xs()
                                .text_color(rgb(0x6b7280))
                                .child("No running apps listed — click Refresh"),
                        )
                    })
                    .children(available_apps.iter().enumerate().map(|(idx, app)| {
                        let is_selected = matches!(
                            selected_source,
                            AudioSource::Application(sel) if sel.pid == app.pid
                        );
                        let subtitle = if app.bundle_id.is_empty() {
                            format!("pid {}", app.pid)
                        } else {
                            format!("{} · pid {}", app.bundle_id, app.pid)
                        };
                        let app_info = app.clone();
                        let on_select_app = on_select.clone();

                        source_dropdown_item(
                            ("source-app", idx),
                            app.name.clone(),
                            subtitle,
                            is_selected,
                        )
                        .on_click(cx.listener(
                            move |this, _ev, window, cx| {
                                on_select_app(
                                    this,
                                    AudioSource::Application(app_info.clone()),
                                    window,
                                    cx,
                                );
                            },
                        ))
                    })),
            ),
    )
    .with_priority(10)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn app(pid: i32, name: &str, bundle_id: &str) -> RunningAppInfo {
        RunningAppInfo {
            pid,
            name: name.to_string(),
            bundle_id: bundle_id.to_string(),
        }
    }

    fn running() -> Vec<RunningAppInfo> {
        vec![
            app(101, "Music", "com.apple.Music"),
            app(202, "Safari", "com.apple.Safari"),
            app(303, "Helper", ""),
        ]
    }

    #[rstest]
    #[case::all_system_audio_stays(
        AudioSource::AllSystemAudio,
        (AudioSource::AllSystemAudio, false)
    )]
    #[case::a_running_app_stays(
        AudioSource::Application(app(101, "Music", "com.apple.Music")),
        (AudioSource::Application(app(101, "Music", "com.apple.Music")), false)
    )]
    #[case::a_relaunched_app_is_found_by_bundle_id(
        AudioSource::Application(app(999, "Safari", "com.apple.Safari")),
        (AudioSource::Application(app(202, "Safari", "com.apple.Safari")), false)
    )]
    #[case::a_closed_app_falls_back_to_all_system_audio(
        AudioSource::Application(app(404, "Spotify", "com.spotify.client")),
        (AudioSource::AllSystemAudio, true)
    )]
    #[case::an_empty_bundle_id_never_matches(
        AudioSource::Application(app(999, "OtherHelper", "")),
        (AudioSource::AllSystemAudio, true)
    )]
    fn the_selected_source_is_reconciled_with_the_running_apps(
        #[case] selected: AudioSource,
        #[case] expected: (AudioSource, bool),
    ) {
        // When
        let reconciled = reconcile_selected_source(&selected, &running());

        // Then
        assert_eq!(reconciled, expected);
    }
}
