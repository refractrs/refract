//! Host-owned inputs are ordinary graph state, exposed read-only to views.
use refract::prelude::*;

pub struct HostInputs {
    pub dark: Read<bool>,
}

#[derive(Clone)]
pub struct InputHandles {
    pub dark: State<bool>,
}

#[derive(Clone, Copy)]
pub(crate) struct Palette {
    pub background: Color,
    pub side: Color,
    pub ink: Color,
    pub muted: Color,
    pub line: Color,
    pub accent: Color,
    pub pale: Color,
    pub card: Color,
    pub focused_card: Color,
    pub on_accent: Color,
    pub primary_start: Color,
    pub primary_end: Color,
    pub avatar_highlight: Color,
    pub shadow: Color,
}

impl Palette {
    pub fn for_dark(dark: bool) -> Self {
        if dark {
            Self {
                background: Color::rgb(16, 24, 38),
                side: Color::rgb(25, 36, 56),
                ink: Color::rgb(226, 235, 249),
                muted: Color::rgb(170, 187, 211),
                line: Color::rgb(56, 73, 100),
                accent: Color::rgb(162, 191, 255),
                pale: Color::rgb(39, 57, 87),
                card: Color::rgb(25, 36, 56),
                focused_card: Color::rgb(31, 45, 68),
                on_accent: Color::rgb(16, 24, 38),
                primary_start: Color::rgb(162, 191, 255),
                primary_end: Color::rgb(124, 159, 230),
                avatar_highlight: Color(162, 191, 255, 45),
                shadow: Color(0, 0, 0, 55),
            }
        } else {
            Self {
                background: crate::BACKGROUND,
                side: Color::rgb(241, 245, 251),
                ink: Color::rgb(24, 40, 63),
                muted: Color::rgb(82, 98, 123),
                line: Color::rgb(203, 213, 227),
                accent: Color::rgb(40, 87, 183),
                pale: Color::rgb(220, 232, 251),
                card: Color::WHITE,
                focused_card: Color::rgb(247, 250, 255),
                on_accent: Color::WHITE,
                primary_start: Color::rgb(49, 84, 166),
                primary_end: Color::rgb(37, 62, 105),
                avatar_highlight: Color(255, 255, 255, 220),
                shadow: Color(24, 40, 63, 18),
            }
        }
    }

    pub fn avatar_gradient(self, radius: f32) -> RadialGradient {
        RadialGradient::new(radius, self.avatar_highlight, Color(117, 159, 224, 36))
            .center(0.25, 0.2)
    }
}

pub(crate) fn palette(cx: &mut ViewContext<'_>) -> Result<Palette, ViewError> {
    let inputs = cx.object::<HostInputs>()?;
    Ok(Palette::for_dark(cx.read(&inputs.dark)))
}
