//! Recognizes text clips that read as a link or a color, for display only.
//! The clip stays plain text; nothing is opened or fetched.

use iced::Color;

/// Longest text checked for a link.
const MAX_LINK_BYTES: usize = 2048;

#[derive(Debug, PartialEq)]
pub enum Look {
    Plain,
    /// An http or https URL, with its host.
    Link {
        host: String,
    },
    Color(Color),
}

pub fn look(text: &str) -> Look {
    let text = text.trim();
    if let Some(color) = color(text) {
        return Look::Color(color);
    }
    if let Some(host) = link_host(text) {
        return Look::Link {
            host: host.to_owned(),
        };
    }
    Look::Plain
}

fn link_host(text: &str) -> Option<&str> {
    // Reject spaces and anything the preview would escape, such as controls
    // and bidi overrides.
    let unsafe_char = |c: char| {
        c.is_whitespace() || !matches!(c, '"' | '\'' | '\\') && c.escape_debug().count() > 1
    };
    if text.len() > MAX_LINK_BYTES || text.chars().any(unsafe_char) {
        return None;
    }
    let lower = |prefix: &str| {
        text.get(..prefix.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
            .then(|| &text[prefix.len()..])
    };
    let rest = lower("https://").or_else(|| lower("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    // Drop any user information before the host.
    let host = authority.rsplit('@').next()?;
    (!host.is_empty()).then_some(host)
}

fn color(text: &str) -> Option<Color> {
    if let Some(hex) = text.strip_prefix('#') {
        return hex_color(hex);
    }
    let lower = text.to_ascii_lowercase();
    let (body, alpha) = if let Some(body) = lower.strip_prefix("rgba(") {
        (body, true)
    } else {
        (lower.strip_prefix("rgb(")?, false)
    };
    let parts: Vec<&str> = body.strip_suffix(')')?.split(',').map(str::trim).collect();
    let channel = |part: &str| part.parse::<u8>().ok();
    match (&parts[..], alpha) {
        ([r, g, b], false) => Some(Color::from_rgb8(channel(r)?, channel(g)?, channel(b)?)),
        ([r, g, b, a], true) => {
            let a = a.parse::<f32>().ok().filter(|a| (0.0..=1.0).contains(a))?;
            Some(Color::from_rgba8(channel(r)?, channel(g)?, channel(b)?, a))
        }
        _ => None,
    }
}

fn hex_color(hex: &str) -> Option<Color> {
    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let digit = |index: usize| u8::from_str_radix(&hex[index..=index], 16).ok();
    let pair = |index: usize| u8::from_str_radix(&hex[index..index + 2], 16).ok();
    let (r, g, b, a) = match hex.len() {
        3 | 4 => (
            digit(0)? * 17,
            digit(1)? * 17,
            digit(2)? * 17,
            if hex.len() == 4 { digit(3)? * 17 } else { 255 },
        ),
        6 | 8 => (
            pair(0)?,
            pair(2)?,
            pair(4)?,
            if hex.len() == 8 { pair(6)? } else { 255 },
        ),
        _ => return None,
    };
    Some(Color::from_rgba8(r, g, b, f32::from(a) / 255.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_are_recognized_in_common_notations() {
        assert_eq!(
            look(" #0a84ff\n"),
            Look::Color(Color::from_rgb8(10, 132, 255))
        );
        assert_eq!(look("#fff"), Look::Color(Color::WHITE));
        assert_eq!(
            look("rgba(255, 0, 0, 0.5)"),
            Look::Color(Color::from_rgba8(255, 0, 0, 0.5))
        );
        assert_eq!(look("RGB(0,0,0)"), Look::Color(Color::BLACK));
        for text in [
            "#12345",
            "#ggg",
            "rgb(256,0,0)",
            "rgba(1,2,3)",
            "rgba(1,2,3,2)",
            "# fff",
        ] {
            assert_eq!(look(text), Look::Plain, "{text}");
        }
    }

    #[test]
    fn links_need_an_http_scheme_and_no_spaces() {
        assert_eq!(
            look("https://user@example.test:8080/a?b#c"),
            Look::Link {
                host: "example.test:8080".into()
            }
        );
        assert_eq!(
            look("HTTP://example.test"),
            Look::Link {
                host: "example.test".into()
            }
        );
        for text in [
            "ftp://example.test",
            "https://",
            "see https://example.test",
            "https://a\u{202e}b",
        ] {
            assert_eq!(look(text), Look::Plain, "{text}");
        }
    }
}
