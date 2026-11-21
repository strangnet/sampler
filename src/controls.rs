use gpui::{Div, ElementId, FontWeight, Stateful, div, prelude::*, px, relative, rgb};

/// Visual tone for [`styled_button`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonTone {
    Neutral,
    Danger { active: bool },
    Primary,
    Toggle { active: bool },
}

/// Builds a styled, interactive button element with enabled/disabled and tone-specific appearance.
pub fn styled_button(
    id: impl Into<ElementId>,
    label: &'static str,
    enabled: bool,
    tone: ButtonTone,
) -> Stateful<Div> {
    let (bg, hover_bg, border, text, weight) = if !enabled {
        (
            rgb(0x1a1c24),
            rgb(0x1a1c24),
            rgb(0x252836),
            rgb(0x525866),
            FontWeight::MEDIUM,
        )
    } else {
        match tone {
            ButtonTone::Danger { active: true } => (
                rgb(0xdc2626),
                rgb(0xb91c1c),
                rgb(0xef4444),
                rgb(0xffffff),
                FontWeight::SEMIBOLD,
            ),
            ButtonTone::Danger { active: false } => (
                rgb(0xef4444),
                rgb(0xdc2626),
                rgb(0xf87171),
                rgb(0xffffff),
                FontWeight::SEMIBOLD,
            ),
            ButtonTone::Primary => (
                rgb(0x0284c7),
                rgb(0x0369a1),
                rgb(0x38bdf8),
                rgb(0xffffff),
                FontWeight::SEMIBOLD,
            ),
            ButtonTone::Toggle { active: true } => (
                rgb(0x0c4a6e),
                rgb(0x075985),
                rgb(0x38bdf8),
                rgb(0xe0f2fe),
                FontWeight::SEMIBOLD,
            ),
            ButtonTone::Toggle { active: false } | ButtonTone::Neutral => (
                rgb(0x252836),
                rgb(0x323648),
                rgb(0x323648),
                rgb(0xe5e7eb),
                FontWeight::MEDIUM,
            ),
        }
    };

    div()
        .id(id)
        .px(px(12.0))
        .py(px(8.0))
        .rounded(px(8.0))
        .bg(bg)
        .border_1()
        .border_color(border)
        .text_sm()
        .font_weight(weight)
        .text_color(text)
        .whitespace_nowrap()
        .when(enabled, |el| {
            el.cursor_pointer().hover(move |s| s.bg(hover_bg))
        })
        .child(label)
}

/// Formats a duration in seconds as `MM:SS.t` (clamped at zero for negative inputs).
pub fn format_duration(secs: f64) -> String {
    let total_tenths = (secs.max(0.0) * 10.0).floor() as u64;
    let tenths = total_tenths % 10;
    let total_secs = total_tenths / 10;
    let seconds = total_secs % 60;
    let minutes = total_secs / 60;
    format!("{:02}:{:02}.{}", minutes, seconds, tenths)
}

/// Renders the 4px horizontal live input level meter bar for a 0..=100 percentage.
pub fn level_meter(meter_pct: f32) -> impl gpui::IntoElement {
    let pct = if meter_pct.is_nan() {
        0.0
    } else {
        meter_pct.clamp(0.0, 100.0)
    };
    div()
        .w_full()
        .h(px(4.0))
        .rounded(px(2.0))
        .bg(rgb(0x1e202b))
        .overflow_hidden()
        .child(div().h_full().w(relative(pct / 100.0)).bg(if pct > 85.0 {
            rgb(0xef4444)
        } else {
            rgb(0x22c55e)
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_formats_minutes_seconds_tenths() {
        assert_eq!(format_duration(0.0), "00:00.0");
        assert_eq!(format_duration(-1.0), "00:00.0");
        assert_eq!(format_duration(2.4), "00:02.4");
        assert_eq!(format_duration(65.35), "01:05.3");
    }
}
