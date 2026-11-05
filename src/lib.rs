use std::collections::VecDeque;
use std::sync::Arc;

pub mod store;

/// Largest plain text clip.
pub const MAX_CLIP_BYTES: usize = 64 * 1024;
/// Largest formatted (RTF or HTML) representation kept with a text clip.
/// Larger ones are dropped and the clip keeps its plain text.
pub const MAX_RICH_BYTES: usize = 256 * 1024;
/// Largest image clip, as PNG.
pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
/// Largest image, in pixels, that adapters decode for previews or conversion.
pub const MAX_IMAGE_PIXELS: u64 = 64 * 1024 * 1024;
/// Budget for text clips, including their formatted representations.
pub(crate) const MAX_HISTORY_BYTES: usize = 8 * 1024 * 1024;
/// Budget for image clips.
pub(crate) const MAX_IMAGE_HISTORY_BYTES: usize = 128 * 1024 * 1024;
pub(crate) const MAX_ITEMS: usize = 512;

/// One history entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Clip {
    Text(Text),
    Image(Image),
}

/// Text with optional formatted representations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Text {
    pub plain: String,
    pub rtf: Option<Arc<[u8]>>,
    pub html: Option<Arc<[u8]>>,
}

/// A PNG image. `thumbnail` is a display cache that adapters fill in; it is
/// never saved and does not affect equality.
#[derive(Clone, Debug)]
pub struct Image {
    pub png: Arc<[u8]>,
    pub width: u32,
    pub height: u32,
    pub thumbnail: Option<Arc<Thumbnail>>,
}

impl PartialEq for Image {
    fn eq(&self, other: &Self) -> bool {
        self.png == other.png
    }
}

impl Eq for Image {}

/// A small RGBA preview of an image.
#[derive(Debug, PartialEq, Eq)]
pub struct Thumbnail {
    pub width: u16,
    pub height: u16,
    /// Unpremultiplied RGBA rows, top to bottom.
    pub rgba: Vec<u8>,
}

impl Clip {
    /// A plain text clip.
    pub fn text(plain: impl Into<String>) -> Self {
        Self::Text(Text {
            plain: plain.into(),
            ..Text::default()
        })
    }

    /// The plain text, for text clips.
    pub fn plain(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(&text.plain),
            Self::Image(_) => None,
        }
    }

    /// Bytes counted against the text and image budgets.
    fn cost(&self) -> (usize, usize) {
        match self {
            Self::Text(text) => (
                text.plain.len()
                    + text.rtf.as_ref().map_or(0, |rtf| rtf.len())
                    + text.html.as_ref().map_or(0, |html| html.len()),
                0,
            ),
            Self::Image(image) => (0, image.png.len()),
        }
    }

    /// Text clips match by case-sensitive substring. Images match an empty
    /// query or any part of the word "image", ignoring case.
    pub fn matches(&self, query: &str) -> bool {
        match self {
            Self::Text(text) => text.plain.contains(query),
            Self::Image(_) => query.is_empty() || "image".contains(&query.to_lowercase()),
        }
    }

    /// Same content: text by plain text, images by bytes.
    fn same_content(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Text(a), Self::Text(b)) => a.plain == b.plain,
            (Self::Image(a), Self::Image(b)) => a == b,
            _ => false,
        }
    }
}

/// In-memory history ordered from most to least recently captured.
#[derive(Clone, Default)]
pub struct History {
    clips: VecDeque<Clip>,
    text_bytes: usize,
    image_bytes: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Capture {
    Added,
    Refreshed,
    Empty,
    TooLarge,
}

impl History {
    /// Captures plain text.
    pub fn capture(&mut self, clip: &str) -> Capture {
        self.add(Clip::text(clip))
    }

    /// Captures a clip. Formatted representations over [`MAX_RICH_BYTES`]
    /// are dropped; the plain text is kept.
    pub fn add(&mut self, mut clip: Clip) -> Capture {
        match &mut clip {
            Clip::Text(text) => {
                if text.plain.is_empty() {
                    return Capture::Empty;
                }
                if text.plain.len() > MAX_CLIP_BYTES {
                    return Capture::TooLarge;
                }
                for rich in [&mut text.rtf, &mut text.html] {
                    if rich
                        .as_ref()
                        .is_some_and(|rich| rich.is_empty() || rich.len() > MAX_RICH_BYTES)
                    {
                        *rich = None;
                    }
                }
            }
            Clip::Image(image) => {
                if image.png.is_empty() || image.width == 0 || image.height == 0 {
                    return Capture::Empty;
                }
                if image.png.len() > MAX_IMAGE_BYTES {
                    return Capture::TooLarge;
                }
            }
        }

        let refreshed = match self.clips.iter().position(|old| old.same_content(&clip)) {
            Some(index) => {
                let old = self
                    .clips
                    .remove(index)
                    .expect("index came from this history");
                self.uncount(&old);
                // Keep a thumbnail already made for the same image.
                if let (Clip::Image(old), Clip::Image(new)) = (&old, &mut clip)
                    && new.thumbnail.is_none()
                {
                    new.thumbnail = old.thumbnail.clone();
                }
                true
            }
            None => false,
        };

        let (text, image) = clip.cost();
        self.text_bytes += text;
        self.image_bytes += image;
        self.clips.push_front(clip);
        self.evict();
        if refreshed {
            Capture::Refreshed
        } else {
            Capture::Added
        }
    }

    fn uncount(&mut self, clip: &Clip) {
        let (text, image) = clip.cost();
        self.text_bytes -= text;
        self.image_bytes -= image;
    }

    /// Drops the oldest clips until every budget holds. Only the newest clip
    /// can push a budget over, and it always fits on its own.
    fn evict(&mut self) {
        loop {
            let index = if self.clips.len() > MAX_ITEMS {
                Some(self.clips.len() - 1)
            } else if self.text_bytes > MAX_HISTORY_BYTES {
                self.clips
                    .iter()
                    .rposition(|clip| matches!(clip, Clip::Text(_)))
            } else if self.image_bytes > MAX_IMAGE_HISTORY_BYTES {
                self.clips
                    .iter()
                    .rposition(|clip| matches!(clip, Clip::Image(_)))
            } else {
                None
            };
            let Some(index) = index else { return };
            let old = self
                .clips
                .remove(index)
                .expect("index came from this history");
            self.uncount(&old);
        }
    }

    /// Every clip, newest first.
    pub fn iter(&self) -> impl Iterator<Item = &Clip> {
        self.clips.iter()
    }

    /// Images that have no thumbnail yet.
    pub fn images_without_thumbnail(&mut self) -> impl Iterator<Item = &mut Image> {
        self.clips.iter_mut().filter_map(|clip| match clip {
            Clip::Image(image) if image.thumbnail.is_none() => Some(image),
            _ => None,
        })
    }

    /// Adds saved clips, given newest first, behind the current ones.
    /// Duplicates keep their current position; limits still apply.
    pub fn extend_older(&mut self, clips: Vec<Clip>) {
        let current = std::mem::take(self);
        for clip in clips.into_iter().rev() {
            self.add(clip);
        }
        for clip in current.clips.into_iter().rev() {
            self.add(clip);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }

    /// Clips matching `query`, newest first. An empty query lists all clips.
    pub fn search<'a>(&'a self, query: &'a str) -> impl Iterator<Item = &'a Clip> + 'a {
        self.clips.iter().filter(move |clip| clip.matches(query))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts<'a>(clips: impl Iterator<Item = &'a Clip>) -> Vec<&'a str> {
        clips.filter_map(Clip::plain).collect()
    }

    fn image(seed: u8, len: usize) -> Clip {
        Clip::Image(Image {
            png: vec![seed; len].into(),
            width: 2,
            height: 1,
            thumbnail: None,
        })
    }

    #[test]
    fn duplicate_refresh_preserves_other_entries_and_byte_accounting() {
        let mut history = History::default();
        for clip in ["alpha", "beta", "gamma", "delta"] {
            assert_eq!(history.capture(clip), Capture::Added);
        }
        assert_eq!(history.capture("beta"), Capture::Refreshed);
        assert_eq!(
            texts(history.search("")),
            ["beta", "delta", "gamma", "alpha"]
        );
        assert_eq!(history.text_bytes, 19);
    }

    #[test]
    fn byte_budget_evicts_oldest_and_rejects_oversized_unicode() {
        let mut history = History::default();
        let clip = "é".repeat(MAX_CLIP_BYTES / 2);
        assert_eq!(history.capture(&(clip.clone() + "é")), Capture::TooLarge);
        let slots = MAX_HISTORY_BYTES / MAX_CLIP_BYTES;
        for index in 0..=slots {
            let prefix = format!("{index:04}");
            assert_eq!(history.capture(&(prefix + &clip[4..])), Capture::Added);
        }
        assert_eq!(history.search("").count(), slots);
        assert!(history.search("0000").next().is_none());
        assert_eq!(history.text_bytes, MAX_HISTORY_BYTES);
    }

    #[test]
    fn saved_clips_go_behind_the_session() {
        let mut history = History::default();
        history.capture("old shared");
        history.capture("session");
        history.extend_older(vec![
            Clip::text("saved new"),
            Clip::text("old shared"),
            Clip::text("saved old"),
        ]);
        assert_eq!(
            texts(history.iter()),
            ["session", "old shared", "saved new", "saved old"]
        );
        assert_eq!(history.text_bytes, 7 + 10 + 9 + 9);
    }

    #[test]
    fn item_budget_evicts_oldest() {
        let mut history = History::default();
        for index in 0..=MAX_ITEMS {
            history.capture(&format!("clip {index}"));
        }
        assert_eq!(history.search("").count(), MAX_ITEMS);
        assert_eq!(history.search("clip 0").count(), 0);
    }

    #[test]
    fn formatted_text_refreshes_by_plain_text_and_drops_oversized_formats() {
        let mut history = History::default();
        let rich = |rtf: &[u8], html: usize| {
            Clip::Text(Text {
                plain: "bold".into(),
                rtf: Some(rtf.into()),
                html: Some(vec![b'h'; html].into()),
            })
        };
        assert_eq!(history.add(rich(b"{\\rtf1 one}", 4)), Capture::Added);
        assert_eq!(
            history.add(rich(b"{\\rtf1 two}", MAX_RICH_BYTES + 1)),
            Capture::Refreshed
        );
        let Some(Clip::Text(text)) = history.iter().next() else {
            panic!("expected text");
        };
        assert_eq!(text.rtf.as_deref(), Some(b"{\\rtf1 two}".as_slice()));
        assert_eq!(text.html, None);
        assert_eq!(history.text_bytes, 4 + 11);
    }

    #[test]
    fn images_have_their_own_budget_and_match_the_word_image() {
        let mut history = History::default();
        history.capture("note");
        assert_eq!(
            history.add(image(1, MAX_IMAGE_BYTES + 1)),
            Capture::TooLarge
        );
        let slots = MAX_IMAGE_HISTORY_BYTES / MAX_IMAGE_BYTES;
        for seed in 0..=slots as u8 {
            assert_eq!(history.add(image(seed, MAX_IMAGE_BYTES)), Capture::Added);
        }
        // The oldest image went; the older text stayed.
        assert_eq!(history.iter().count(), slots + 1);
        assert_eq!(texts(history.iter()), ["note"]);
        assert_eq!(history.image_bytes, slots * MAX_IMAGE_BYTES);
        assert_eq!(history.add(image(3, MAX_IMAGE_BYTES)), Capture::Refreshed);
        assert_eq!(history.search("IMA").count(), slots);
        assert_eq!(history.search("no").count(), 1);
    }
}
