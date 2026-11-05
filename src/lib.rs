use std::collections::VecDeque;

pub const MAX_CLIP_BYTES: usize = 64 * 1024;
const MAX_HISTORY_BYTES: usize = 8 * 1024 * 1024;
const MAX_ITEMS: usize = 512;

/// In-memory UTF-8 history ordered from most to least recently captured.
#[derive(Default)]
pub struct History {
    clips: VecDeque<String>,
    bytes: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Capture {
    Added,
    Refreshed,
    Empty,
    TooLarge,
}

impl History {
    pub fn capture(&mut self, clip: &str) -> Capture {
        if clip.is_empty() {
            return Capture::Empty;
        }
        if clip.len() > MAX_CLIP_BYTES {
            return Capture::TooLarge;
        }

        if let Some(index) = self.clips.iter().position(|existing| existing == clip) {
            let existing = self
                .clips
                .remove(index)
                .expect("index came from this history");
            self.clips.push_front(existing);
            return Capture::Refreshed;
        }

        while self.clips.len() >= MAX_ITEMS || self.bytes + clip.len() > MAX_HISTORY_BYTES {
            let oldest = self.clips.pop_back().expect("a full history has an entry");
            self.bytes -= oldest.len();
        }
        self.clips.push_front(clip.into());
        self.bytes += clip.len();
        Capture::Added
    }

    /// Searches by case-sensitive substring. An empty query lists all entries.
    pub fn search<'a>(&'a self, query: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.clips
            .iter()
            .map(String::as_str)
            .filter(move |clip| clip.contains(query))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_refresh_preserves_other_entries_and_byte_accounting() {
        let mut history = History::default();
        for clip in ["alpha", "beta", "gamma", "delta"] {
            assert_eq!(history.capture(clip), Capture::Added);
        }
        assert_eq!(history.capture("beta"), Capture::Refreshed);
        assert_eq!(
            history.search("").collect::<Vec<_>>(),
            ["beta", "delta", "gamma", "alpha"]
        );
        assert_eq!(history.bytes, 19);
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
        assert_eq!(history.bytes, MAX_HISTORY_BYTES);
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
}
