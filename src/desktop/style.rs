//! Visual design for the desktop palette: a compact panel that follows the
//! system light or dark appearance and, where available, the system accent.

use iced::widget::{button, container, text_input};
use iced::{Background, Border, Color, Shadow, Theme, Vector, color, theme};

const RADIUS: f32 = 10.0;

/// Repasta's own accent, used when the system does not choose one.
const AMBER_DARK: Color = color!(0xf2a65a);
const AMBER_LIGHT: Color = color!(0xc8711f);

/// The resolved colors for one appearance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Colors {
    pub dark: bool,
    pub background: Color,
    pub surface: Color,
    pub surface_raised: Color,
    pub outline: Color,
    pub text: Color,
    pub muted: Color,
    pub faint: Color,
    pub accent: Color,
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
}

impl Default for Colors {
    fn default() -> Self {
        Self::new(theme::Mode::Dark, None)
    }
}

impl Colors {
    /// Colors for the system `mode`. `accent` is the system accent color, or
    /// `None` to use Repasta's amber.
    pub fn new(mode: theme::Mode, accent: Option<Color>) -> Self {
        if mode == theme::Mode::Light {
            Self {
                dark: false,
                background: color!(0xf4f4f6),
                surface: color!(0xffffff),
                surface_raised: color!(0xf0f0f3),
                outline: color!(0xdcdce2),
                text: color!(0x1d1d21),
                muted: color!(0x63646d),
                faint: color!(0x9a9ba3),
                accent: accent.unwrap_or(AMBER_LIGHT),
                success: color!(0x1b8a57),
                warning: color!(0xa86a12),
                danger: color!(0xc8372d),
            }
        } else {
            Self {
                dark: true,
                background: color!(0x141519),
                surface: color!(0x1d1e24),
                surface_raised: color!(0x25262e),
                outline: color!(0x2f313a),
                text: color!(0xececf1),
                muted: color!(0x8a8c97),
                faint: color!(0x5d5f6a),
                accent: accent.unwrap_or(AMBER_DARK),
                success: color!(0x5bd49a),
                warning: color!(0xf2c94c),
                danger: color!(0xf2685a),
            }
        }
    }

    pub fn theme(&self) -> Theme {
        Theme::custom(
            if self.dark {
                "Repasta Dark"
            } else {
                "Repasta Light"
            },
            theme::Palette {
                background: self.background,
                text: self.text,
                primary: self.accent,
                success: self.success,
                warning: self.warning,
                danger: self.danger,
            },
        )
    }

    /// Tint strength for accent-colored fills; light backgrounds need less.
    fn tint(&self, dark: f32, light: f32) -> f32 {
        if self.dark { dark } else { light }
    }
}

fn alpha(color: Color, a: f32) -> Color {
    Color { a, ..color }
}

fn border(color: Color, radius: f32) -> Border {
    Border {
        color,
        width: 1.0,
        radius: radius.into(),
    }
}

pub fn footer(c: Colors) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(c.surface)),
        border: Border {
            color: c.outline,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    }
}

pub fn keycap(c: Colors) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(c.surface_raised)),
        border: border(c.outline, 5.0),
        text_color: Some(c.muted),
        ..container::Style::default()
    }
}

pub fn empty_state(c: Colors) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        border: border(c.outline, RADIUS),
        text_color: Some(c.muted),
        ..container::Style::default()
    }
}

pub fn search(c: Colors) -> impl Fn(&Theme, text_input::Status) -> text_input::Style {
    move |_, status| {
        let edge = match status {
            text_input::Status::Focused { .. } => alpha(c.accent, 0.7),
            text_input::Status::Hovered => c.faint,
            _ => c.outline,
        };
        text_input::Style {
            background: Background::Color(c.surface),
            border: border(edge, RADIUS),
            icon: c.muted,
            placeholder: c.faint,
            value: c.text,
            selection: alpha(c.accent, c.tint(0.35, 0.25)),
        }
    }
}

pub fn clip_card(c: Colors, selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let (background, edge) = match (selected, hovered) {
            (true, _) => (alpha(c.accent, c.tint(0.12, 0.08)), alpha(c.accent, 0.65)),
            (false, true) => (c.surface_raised, c.faint),
            (false, false) => (c.surface, c.outline),
        };
        let shadow = if selected {
            Shadow {
                color: alpha(c.accent, c.tint(0.18, 0.14)),
                offset: Vector::new(0.0, 2.0),
                blur_radius: 12.0,
            }
        } else if c.dark {
            Shadow::default()
        } else {
            // Lift cards slightly off the light background.
            Shadow {
                color: Color::from_rgba8(0, 0, 0, 0.05),
                offset: Vector::new(0.0, 1.0),
                blur_radius: 3.0,
            }
        };
        button::Style {
            background: Some(Background::Color(background)),
            text_color: c.text,
            border: border(edge, RADIUS),
            shadow,
            snap: false,
        }
    }
}

pub fn ghost(c: Colors) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let (background, text_color) = match status {
            button::Status::Hovered => (c.surface_raised, c.text),
            button::Status::Pressed => (c.outline, c.text),
            button::Status::Disabled => (Color::TRANSPARENT, c.faint),
            button::Status::Active => (Color::TRANSPARENT, c.muted),
        };
        button::Style {
            background: Some(Background::Color(background)),
            text_color,
            border: Border {
                radius: 7.0.into(),
                ..Border::default()
            },
            shadow: Shadow::default(),
            snap: false,
        }
    }
}

/// A color clip's swatch, outlined so light and dark colors stay visible.
pub fn swatch(c: Colors, fill: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(fill)),
        border: border(c.outline, 7.0),
        ..container::Style::default()
    }
}

/// A rounded status pill; `active` uses the accent, otherwise a neutral outline.
pub fn pill(c: Colors, active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let (background, edge, text_color) = if active {
            let fill = if hovered {
                c.tint(0.24, 0.18)
            } else {
                c.tint(0.16, 0.12)
            };
            (alpha(c.accent, fill), alpha(c.accent, 0.5), c.accent)
        } else {
            (
                if hovered { c.surface_raised } else { c.surface },
                c.outline,
                c.muted,
            )
        };
        button::Style {
            background: Some(Background::Color(background)),
            text_color,
            border: border(edge, 999.0),
            shadow: Shadow::default(),
            snap: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AMBER_DARK, AMBER_LIGHT, Colors};
    use iced::{color, theme::Mode};

    #[test]
    fn colors_follow_system_mode_and_accent() {
        let blue = color!(0x0a84ff);
        let light = Colors::new(Mode::Light, Some(blue));
        assert!(!light.dark);
        assert_eq!(light.accent, blue);
        assert_eq!(Colors::new(Mode::Light, None).accent, AMBER_LIGHT);

        // Unknown modes keep the dark design.
        let dark = Colors::new(Mode::None, None);
        assert!(dark.dark);
        assert_eq!(dark.accent, AMBER_DARK);
    }
}
