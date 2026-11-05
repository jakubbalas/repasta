#[cfg(target_os = "macos")]
mod appearance;
#[cfg(target_os = "macos")]
mod hotkey;
#[cfg(target_os = "macos")]
mod keychain;
#[cfg(target_os = "linux")]
mod linux;
mod looks;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod menu_bar;
mod settings;
mod style;
mod thumbnail;

use iced::keyboard::{Event as KeyEvent, Key, Modifiers, key::Named};
use iced::mouse::ScrollDelta;
use iced::widget::{
    Space, button, column, container, mouse_area, row, text, text::Wrapping, text_input, toggler,
};
use iced::{
    Alignment, Element, Event, Fill, Font, Point, Size, Subscription, Task, event, font, window,
};
use repasta::store::{FolderStore, Sealer, Store, StoreError};
use repasta::{Capture, Clip, History, MAX_CLIP_BYTES};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

const SEARCH_ID: &str = "clip-search";
const WINDOW_SIZE: Size = Size::new(400.0, 660.0);
/// Height of the header, search field, list padding, and footer.
const CHROME_HEIGHT: f32 = 232.0;
/// Height of a two-line clip card plus list spacing.
const CARD_HEIGHT: f32 = 82.0;
const PREVIEW_LINES: usize = 2;
const PREVIEW_CHARS: usize = 46;
/// Trackpad pixels per selection step when scrolling the list.
const SCROLL_STEP: f32 = 36.0;
/// Side of a color clip's swatch.
const SWATCH: f32 = 34.0;

const BOLD: Font = Font {
    weight: font::Weight::Semibold,
    ..Font::DEFAULT
};

#[derive(Clone)]
enum Message {
    Capture,
    #[cfg(target_os = "linux")]
    Captured(Result<Clip, &'static str>),
    #[cfg(target_os = "macos")]
    ToggleRecording,
    #[cfg(target_os = "macos")]
    Poll,
    TogglePin,
    #[cfg(target_os = "macos")]
    InstallMenuBar,
    /// Close button or ⌘W: hide to the menu bar where available.
    Hide,
    Quit,
    /// The system light/dark appearance changed or was first reported.
    /// macOS reads the appearance directly; see `AppearanceChanged`.
    #[cfg(target_os = "linux")]
    SystemTheme(iced::theme::Mode),
    /// The panel gained focus, or macOS reported an appearance or accent
    /// change; re-read both.
    AppearanceChanged,
    DragWindow,
    Query(String),
    Escape,
    Previous,
    Next,
    Scroll(f32),
    Resized(Size),
    RestoreSelected,
    Restore(usize),
    /// Deletes one clip, by its position in the current results.
    Delete(usize),
    /// Clears history, asking first when it is saved to a folder.
    Clear,
    ConfirmClear,
    CancelClear,
    OpenSettings,
    CloseSettings,
    /// Settings draft: keep history for this session only.
    DraftSession,
    /// Settings draft: save history to a folder.
    DraftFolder,
    DraftPath(String),
    DraftEncrypted(bool),
    /// Applies and saves the panel appearance at once.
    SetAppearance(settings::Appearance),
    /// Registers and saves the global shortcut at once.
    #[cfg(target_os = "macos")]
    SetShortcut(Option<hotkey::Shortcut>),
    /// Waits for the next key press to use as the global shortcut.
    #[cfg(target_os = "macos")]
    CaptureShortcut,
    /// A key press while capturing a shortcut.
    #[cfg(target_os = "macos")]
    CapturedKey(iced::keyboard::key::Physical, Modifiers),
    /// Stops capturing and keeps the current shortcut.
    #[cfg(target_os = "macos")]
    CancelCapture,
    /// The global shortcut was pressed.
    #[cfg(target_os = "macos")]
    ShortcutPressed,
    #[cfg(target_os = "macos")]
    ChooseFolder,
    ApplyStorage,
    /// A background save finished.
    Saved(Result<(), StoreError>),
    /// Shift selects plain-text restore.
    Modifiers(Modifiers),
    /// Makes thumbnails for loaded images, a few at a time.
    #[cfg(target_os = "macos")]
    MakeThumbnails,
}

/// Image thumbnails made per update, keeping the panel responsive.
#[cfg(target_os = "macos")]
const THUMBNAILS_PER_UPDATE: usize = 2;

/// Unapplied changes in the settings view.
#[derive(Default)]
struct Draft {
    folder: bool,
    path: String,
    encrypted: bool,
}

#[derive(Clone, Copy, Default)]
enum Tone {
    #[default]
    Neutral,
    Good,
    Warn,
}

#[derive(Default)]
struct Palette {
    history: History,
    query: String,
    selected: usize,
    /// Index of the first visible result.
    offset: usize,
    scroll: f32,
    /// Number of clip cards that fit in the window.
    visible: usize,
    status: String,
    tone: Tone,
    /// Where history lives, as chosen in settings.
    storage: settings::Storage,
    /// The open folder store; `None` for session-only history or when the
    /// chosen folder is unavailable. Saves run on a background thread.
    store: Option<Arc<Mutex<FolderStore>>>,
    store_encrypted: bool,
    /// A background save is running.
    saving: bool,
    /// History changed while saving; save again when it finishes.
    save_again: bool,
    modifiers: Modifiers,
    settings_open: bool,
    /// Clear is waiting for confirmation.
    confirm_clear: bool,
    draft: Draft,
    appearance: settings::Appearance,
    pinned: bool,
    mode: iced::theme::Mode,
    colors: style::Colors,
    theme: Option<iced::Theme>,
    #[cfg(target_os = "macos")]
    clipboard: macos::Clipboard,
    #[cfg(target_os = "macos")]
    menu_bar: Option<menu_bar::MenuBar>,
    /// The global shortcut; `None` when it's off.
    #[cfg(target_os = "macos")]
    shortcut: Option<hotkey::Shortcut>,
    /// Whether the next key press becomes the global shortcut.
    #[cfg(target_os = "macos")]
    capturing_shortcut: bool,
    /// The registered global shortcut; dropping it unregisters.
    #[cfg(target_os = "macos")]
    hotkey: Option<hotkey::HotKey>,
    #[cfg(target_os = "macos")]
    recording: bool,
    #[cfg(target_os = "linux")]
    reading: bool,
}

impl Palette {
    fn boot() -> (Self, Task<Message>) {
        let mut palette = Self {
            pinned: true,
            visible: visible_cards(WINDOW_SIZE.height),
            mode: iced::theme::Mode::Dark,
            storage: settings::load(),
            appearance: settings::load_appearance(),
            #[cfg(target_os = "macos")]
            shortcut: settings::load_shortcut(),
            ..Self::default()
        };
        palette.refresh_colors(true);
        palette.attach_storage(false);
        (
            palette,
            Task::batch([
                #[cfg(target_os = "macos")]
                Task::done(Message::MakeThumbnails),
                iced::widget::operation::focus(SEARCH_ID),
                // iced reads the mode from the window, whose appearance follows
                // our theme on macOS, so only Linux asks iced.
                #[cfg(target_os = "linux")]
                iced::system::theme().map(Message::SystemTheme),
                #[cfg(target_os = "macos")]
                Task::done(Message::InstallMenuBar),
            ]),
        )
    }

    /// Rebuilds the palette from the system appearance and accent color.
    fn refresh_colors(&mut self, force: bool) {
        #[cfg(target_os = "macos")]
        {
            self.mode = appearance::system_mode();
        }
        #[cfg(target_os = "macos")]
        let accent = appearance::accent_color();
        #[cfg(target_os = "linux")]
        let accent = None;
        // `self.mode` is the system mode; the preference may override it.
        let mode = match self.appearance {
            settings::Appearance::System => self.mode,
            settings::Appearance::Light => iced::theme::Mode::Light,
            settings::Appearance::Dark => iced::theme::Mode::Dark,
        };
        let colors = style::Colors::new(mode, accent);
        if force || colors != self.colors {
            self.colors = colors;
            self.theme = Some(colors.theme());
        }
    }

    fn say(&mut self, tone: Tone, status: impl Into<String>) {
        self.tone = tone;
        self.status = status.into();
    }

    /// Opens the store for `self.storage` and merges its saved history behind
    /// the current clips. Writes only when `save` is set or the folder needs
    /// converting. On failure, history stays session-only and the file is left
    /// untouched.
    fn attach_storage(&mut self, save: bool) {
        self.store = None;
        let Some(folder) = self.storage.folder.clone() else {
            self.say(
                Tone::Neutral,
                "Session-only history. Nothing is written to disk.",
            );
            return;
        };
        let result = open_store(&folder, self.storage.encrypted).and_then(|mut store| {
            let saved = store.load()?;
            let count = saved.len();
            self.history.extend_older(saved);
            if save || store.has_other_form() {
                store.save(&self.history)?;
            }
            Ok((store, count))
        });
        match result {
            Ok((store, count)) => {
                let name = folder_name(&folder);
                let form = if store.encrypted() {
                    "encrypted"
                } else {
                    "readable"
                };
                self.store_encrypted = store.encrypted();
                self.store = Some(Arc::new(Mutex::new(store)));
                self.reset_selection();
                self.say(
                    Tone::Good,
                    match count {
                        0 => format!("Saving to {name} ({form})."),
                        1 => format!("Loaded 1 clip from {name} ({form})."),
                        n => format!("Loaded {n} clips from {name} ({form})."),
                    },
                );
            }
            Err(error) => self.say(
                Tone::Warn,
                format!("{error} Keeping history for this session only."),
            ),
        }
    }

    /// Saves a snapshot of history on a background thread. Changes made
    /// while a save runs are saved once it finishes.
    fn persist(&mut self) -> Task<Message> {
        let Some(store) = self.store.clone() else {
            return Task::none();
        };
        if self.saving {
            self.save_again = true;
            return Task::none();
        }
        self.saving = true;
        // Cheap: image and formatted data are shared, not copied.
        let snapshot = self.history.clone();
        Task::perform(
            run_in_background(move || lock(&store).save(&snapshot)),
            Message::Saved,
        )
    }

    fn open_settings(&mut self) {
        self.draft = Draft {
            folder: self.storage.folder.is_some(),
            path: self
                .storage
                .folder
                .as_deref()
                .map(|folder| folder.display().to_string())
                .unwrap_or_default(),
            encrypted: self.storage.encrypted && settings::ENCRYPTION_AVAILABLE,
        };
        self.settings_open = true;
    }

    fn reset_selection(&mut self) {
        self.selected = 0;
        self.offset = 0;
        self.scroll = 0.0;
    }

    fn match_count(&self) -> usize {
        self.history.search(&self.query).count()
    }

    fn accept(&mut self, result: Result<Clip, &'static str>) -> Task<Message> {
        let clip = match result {
            Ok(clip) => clip,
            Err(status) => {
                self.say(Tone::Warn, status);
                return Task::none();
            }
        };
        let (captured, empty, too_large) = match &clip {
            Clip::Text(text) if text.rtf.is_some() || text.html.is_some() => (
                "Formatted text captured.",
                "Clipboard text is empty.",
                "Clipboard text exceeds 64 KiB.",
            ),
            Clip::Text(_) => (
                "Text captured.",
                "Clipboard text is empty.",
                "Clipboard text exceeds 64 KiB.",
            ),
            Clip::Image(_) => (
                "Image captured.",
                "Clipboard image is empty.",
                "Image exceeds 10 MiB.",
            ),
        };
        match self.history.add(clip) {
            Capture::Added | Capture::Refreshed => {
                self.reset_selection();
                self.say(Tone::Good, captured);
                return self.persist();
            }
            Capture::Empty => self.say(Tone::Warn, empty),
            Capture::TooLarge => self.say(Tone::Warn, too_large),
        }
        Task::none()
    }

    fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        self.offset = self.offset.min(self.selected);
    }

    fn select_next(&mut self) {
        if self
            .history
            .search(&self.query)
            .nth(self.selected + 1)
            .is_some()
        {
            self.selected += 1;
            if self.selected >= self.offset + self.visible {
                self.offset = self.selected + 1 - self.visible;
            }
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            #[cfg(target_os = "macos")]
            Message::Capture => {
                let result = self.clipboard.read();
                return self.accept(result);
            }
            #[cfg(target_os = "linux")]
            Message::Capture => {
                if !self.reading {
                    self.reading = true;
                    self.say(Tone::Neutral, "Reading clipboard…");
                    return Task::perform(linux::capture(), Message::Captured);
                }
            }
            #[cfg(target_os = "linux")]
            Message::Captured(result) => {
                self.reading = false;
                return self.accept(result);
            }
            #[cfg(target_os = "macos")]
            Message::ToggleRecording => {
                if self.recording {
                    self.recording = false;
                    self.say(Tone::Neutral, "Background capture paused.");
                } else if self.clipboard.reset_observation() {
                    self.recording = true;
                    self.say(Tone::Good, "Recording future copies.");
                } else {
                    self.say(Tone::Warn, "System clipboard is unavailable.");
                }
                if let Some(menu_bar) = &self.menu_bar {
                    menu_bar.set_recording(self.recording);
                }
            }
            #[cfg(target_os = "macos")]
            Message::InstallMenuBar => {
                self.menu_bar = menu_bar::MenuBar::install();
                if self.menu_bar.is_none() {
                    self.say(
                        Tone::Warn,
                        "Menu bar unavailable. Closing the window quits.",
                    );
                }
                self.register_shortcut();
            }
            #[cfg(target_os = "macos")]
            Message::SetShortcut(shortcut) => {
                self.capturing_shortcut = false;
                self.shortcut = shortcut;
                settings::save_shortcut(shortcut);
                if self.register_shortcut() {
                    self.say(
                        Tone::Neutral,
                        match shortcut {
                            None => "Global shortcut off.".to_owned(),
                            Some(shortcut) => {
                                format!("Press {} anywhere to show Repasta.", shortcut.label())
                            }
                        },
                    );
                }
            }
            #[cfg(target_os = "macos")]
            Message::CaptureShortcut => {
                self.capturing_shortcut = true;
                // Let the current shortcut reach the panel so it can be chosen again.
                self.hotkey = None;
                self.say(Tone::Neutral, "Press the new shortcut. Esc cancels.");
            }
            #[cfg(target_os = "macos")]
            Message::CapturedKey(key, modifiers) => {
                use iced::keyboard::key::{Code, Physical};
                match key {
                    Physical::Code(Code::Escape) => return self.update(Message::CancelCapture),
                    // Wait for the key that goes with the modifiers.
                    Physical::Code(
                        Code::ShiftLeft
                        | Code::ShiftRight
                        | Code::ControlLeft
                        | Code::ControlRight
                        | Code::AltLeft
                        | Code::AltRight
                        | Code::SuperLeft
                        | Code::SuperRight
                        | Code::Meta
                        | Code::Fn
                        | Code::CapsLock,
                    ) => {}
                    key => match hotkey::Shortcut::from_press(&key, modifiers) {
                        Ok(shortcut) => return self.update(Message::SetShortcut(Some(shortcut))),
                        Err(reason) => self.say(Tone::Warn, reason),
                    },
                }
            }
            #[cfg(target_os = "macos")]
            Message::CancelCapture => {
                if self.cancel_capture() {
                    self.say(Tone::Neutral, "Shortcut unchanged.");
                }
            }
            #[cfg(target_os = "macos")]
            Message::ShortcutPressed => {
                let Some(menu_bar) = &self.menu_bar else {
                    return Task::none();
                };
                if menu_bar.toggle_app() {
                    // Open fresh, ready to type, like a launcher.
                    self.settings_open = false;
                    self.confirm_clear = false;
                    self.query.clear();
                    self.reset_selection();
                    return iced::widget::operation::focus(SEARCH_ID);
                }
            }
            Message::Hide => {
                #[cfg(target_os = "macos")]
                if let Some(menu_bar) = &self.menu_bar {
                    menu_bar.hide_app();
                    self.say(Tone::Neutral, "Still running in the menu bar.");
                    return Task::none();
                }
                return iced::exit();
            }
            Message::Quit => return iced::exit(),
            #[cfg(target_os = "linux")]
            Message::SystemTheme(mode) => {
                self.mode = mode;
                self.refresh_colors(false);
            }
            Message::AppearanceChanged => self.refresh_colors(false),
            #[cfg(target_os = "macos")]
            Message::Poll => {
                if self.recording
                    && let Some(result) = self.clipboard.poll()
                {
                    return self.accept(result);
                }
            }
            Message::TogglePin => {
                self.pinned = !self.pinned;
                let level = if self.pinned {
                    window::Level::AlwaysOnTop
                } else {
                    window::Level::Normal
                };
                return window::latest().and_then(move |id| window::set_level(id, level));
            }
            Message::DragWindow => return window::latest().and_then(window::drag),
            Message::Query(mut query) => {
                query.truncate(query.floor_char_boundary(MAX_CLIP_BYTES));
                self.query = query;
                self.reset_selection();
            }
            Message::Escape if self.confirm_clear => self.confirm_clear = false,
            Message::Escape if self.settings_open => self.settings_open = false,
            Message::Escape => {
                self.query.clear();
                self.reset_selection();
            }
            Message::Previous => self.select_previous(),
            Message::Next => self.select_next(),
            Message::Scroll(pixels) => {
                self.scroll += pixels;
                while self.scroll >= SCROLL_STEP {
                    self.scroll -= SCROLL_STEP;
                    self.select_previous();
                }
                while self.scroll <= -SCROLL_STEP {
                    self.scroll += SCROLL_STEP;
                    self.select_next();
                }
            }
            Message::Resized(size) => {
                self.visible = visible_cards(size.height);
                self.offset = self
                    .offset
                    .max((self.selected + 1).saturating_sub(self.visible));
            }
            Message::RestoreSelected => return self.restore(self.selected),
            Message::Restore(index) => {
                if index < self.match_count() {
                    self.selected = index;
                    self.offset = self
                        .offset
                        .min(index)
                        .max((index + 1).saturating_sub(self.visible));
                }
                return self.restore(index);
            }
            Message::Delete(index) => return self.delete(index),
            Message::Clear if self.store.is_some() => self.confirm_clear = true,
            Message::CancelClear => self.confirm_clear = false,
            Message::Clear | Message::ConfirmClear => {
                self.confirm_clear = false;
                self.history = History::default();
                self.query.clear();
                self.reset_selection();
                self.save_again = false;
                #[cfg(target_os = "macos")]
                self.clipboard.reset_observation();
                // Waits for a running save, so the erase comes last.
                match self.store.as_ref().map(|store| lock(store).erase()) {
                    None => self.say(Tone::Neutral, "History cleared."),
                    Some(Ok(())) => {
                        self.say(Tone::Neutral, "History cleared here and in the folder.")
                    }
                    Some(Err(error)) => self.say(Tone::Warn, format!("Cleared here. {error}")),
                }
            }
            Message::OpenSettings => self.open_settings(),
            Message::SetAppearance(appearance) => {
                self.appearance = appearance;
                settings::save_appearance(appearance);
                self.refresh_colors(false);
            }
            Message::CloseSettings => {
                #[cfg(target_os = "macos")]
                self.cancel_capture();
                self.settings_open = false;
            }
            Message::DraftSession => self.draft.folder = false,
            Message::DraftFolder => self.draft.folder = true,
            Message::DraftPath(path) => self.draft.path = path,
            Message::DraftEncrypted(encrypted) => self.draft.encrypted = encrypted,
            #[cfg(target_os = "macos")]
            Message::ChooseFolder => {
                let current = Some(PathBuf::from(&self.draft.path)).filter(|path| path.is_dir());
                if let Some(folder) = settings::choose_folder(current.as_deref()) {
                    self.draft.folder = true;
                    self.draft.path = folder.display().to_string();
                }
            }
            Message::ApplyStorage => {
                let folder = PathBuf::from(self.draft.path.trim());
                if self.draft.folder && !folder.is_absolute() {
                    self.say(Tone::Warn, "Choose a folder, or enter its full path.");
                    return Task::none();
                }
                let storage = settings::Storage {
                    folder: self.draft.folder.then_some(folder),
                    encrypted: self.draft.encrypted,
                };
                let leaving = self
                    .storage
                    .folder
                    .as_deref()
                    .filter(|_| self.store.is_some() && storage.folder.is_none())
                    .map(folder_name);
                self.storage = storage;
                settings::save(&self.storage);
                if let Some(store) = &self.store {
                    // Let a running save finish before switching.
                    drop(lock(store));
                }
                self.attach_storage(true);
                if let Some(name) = leaving {
                    self.say(
                        Tone::Neutral,
                        format!("Session-only history. The file in {name} was kept."),
                    );
                }
                #[cfg(target_os = "macos")]
                self.cancel_capture();
                self.settings_open = false;
                #[cfg(target_os = "macos")]
                return Task::done(Message::MakeThumbnails);
            }
            Message::Saved(result) => {
                self.saving = false;
                if let Err(error) = result {
                    self.store = None;
                    self.save_again = false;
                    self.say(
                        Tone::Warn,
                        format!("{error} Saving is off until you apply storage again."),
                    );
                } else if std::mem::take(&mut self.save_again) {
                    return self.persist();
                }
            }
            Message::Modifiers(modifiers) => self.modifiers = modifiers,
            #[cfg(target_os = "macos")]
            Message::MakeThumbnails => {
                let mut made = 0;
                for image in self
                    .history
                    .images_without_thumbnail()
                    .take(THUMBNAILS_PER_UPDATE)
                {
                    // An empty thumbnail marks an image that cannot be previewed.
                    image.thumbnail = Some(Arc::new(macos::thumbnail(&image.png).unwrap_or(
                        repasta::Thumbnail {
                            width: 0,
                            height: 0,
                            rgba: Vec::new(),
                        },
                    )));
                    made += 1;
                }
                if made == THUMBNAILS_PER_UPDATE {
                    return Task::done(Message::MakeThumbnails);
                }
            }
        }
        Task::none()
    }

    /// Puts a clip on the system clipboard. Holding Shift drops formatting.
    /// Registers the chosen global shortcut in place of the current one.
    /// Returns whether it worked; failures are reported in the status line.
    #[cfg(target_os = "macos")]
    fn register_shortcut(&mut self) -> bool {
        // Unregister first so re-registering the same shortcut succeeds.
        self.hotkey = None;
        let Some(shortcut) = self.shortcut else {
            return true;
        };
        match hotkey::HotKey::register(shortcut) {
            Ok(hotkey) => {
                self.hotkey = Some(hotkey);
                true
            }
            Err(error) => {
                self.say(Tone::Warn, format!("{} {error}", shortcut.label()));
                false
            }
        }
    }

    /// Stops capturing a shortcut and restores the current one. Returns
    /// whether a capture was running.
    #[cfg(target_os = "macos")]
    fn cancel_capture(&mut self) -> bool {
        if !self.capturing_shortcut {
            return false;
        }
        self.capturing_shortcut = false;
        self.register_shortcut();
        true
    }

    fn delete(&mut self, index: usize) -> Task<Message> {
        if self.history.remove_match(&self.query, index).is_none() {
            return Task::none();
        }
        let count = self.match_count();
        self.selected = self.selected.min(count.saturating_sub(1));
        // Pull later cards up instead of leaving a gap at the end.
        self.offset = self
            .offset
            .min(self.selected)
            .min(count.saturating_sub(self.visible));
        self.say(
            Tone::Neutral,
            if self.store.is_some() {
                "Clip deleted here and in the folder."
            } else {
                "Clip deleted."
            },
        );
        self.persist()
    }

    fn restore(&mut self, index: usize) -> Task<Message> {
        let Some(clip) = self.history.search(&self.query).nth(index).cloned() else {
            return Task::none();
        };
        let plain = self.modifiers.shift();
        #[cfg(target_os = "macos")]
        {
            let formatted =
                matches!(&clip, Clip::Text(text) if text.rtf.is_some() || text.html.is_some());
            if !self.clipboard.write(&clip, plain) {
                self.say(Tone::Warn, "Clipboard write failed.");
            } else if formatted && plain {
                self.say(Tone::Good, "Copied as plain text. Paste it with ⌘V.");
            } else {
                self.say(Tone::Good, "Copied. Paste it anywhere with ⌘V.");
            }
            Task::none()
        }
        #[cfg(target_os = "linux")]
        {
            let _ = plain;
            match clip {
                Clip::Text(text) => {
                    self.say(Tone::Good, "Clipboard write requested.");
                    iced::clipboard::write(text.plain)
                }
                Clip::Image(_) => {
                    self.say(
                        Tone::Warn,
                        "Restoring images is not available on Linux yet.",
                    );
                    Task::none()
                }
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        // While capturing a shortcut, every key press goes to the capture.
        #[cfg(target_os = "macos")]
        let keys = if self.capturing_shortcut {
            Self::capture_keys()
        } else {
            Self::panel_keys()
        };
        #[cfg(not(target_os = "macos"))]
        let keys = Self::panel_keys();
        let keyboard = Subscription::batch([
            keys,
            window::resize_events().map(|(_, size)| Message::Resized(size)),
            window::close_requests().map(|_| Message::Hide),
            #[cfg(target_os = "linux")]
            iced::system::theme_changes().map(Message::SystemTheme),
            #[cfg(target_os = "macos")]
            Subscription::run(appearance::changes).map(|()| Message::AppearanceChanged),
            #[cfg(target_os = "macos")]
            Subscription::run(hotkey::presses).map(|()| Message::ShortcutPressed),
        ]);
        #[cfg(target_os = "macos")]
        if self.recording {
            return Subscription::batch([
                keyboard,
                iced::time::every(std::time::Duration::from_millis(200)).map(|_| Message::Poll),
            ]);
        }
        keyboard
    }

    /// The panel's own keys: selection, Escape, ⌘1–⌘9, ⌘Q, and ⌘W.
    fn panel_keys() -> Subscription<Message> {
        event::listen_with(|event, _, _| match event {
            Event::Keyboard(KeyEvent::KeyPressed {
                key: Key::Named(key),
                modifiers,
                ..
            }) if modifiers.is_empty() => match key {
                Named::ArrowUp => Some(Message::Previous),
                Named::ArrowDown => Some(Message::Next),
                Named::Escape => Some(Message::Escape),
                _ => None,
            },
            // ⌘1–⌘9 (Ctrl on Linux) copy the numbered clip.
            Event::Keyboard(KeyEvent::KeyPressed {
                key: Key::Character(key),
                modifiers,
                ..
            }) if modifiers == Modifiers::COMMAND
                || modifiers == Modifiers::COMMAND | Modifiers::SHIFT =>
            {
                match key.as_str() {
                    digit @ ("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9") => {
                        digit.parse::<usize>().ok().map(|n| Message::Restore(n - 1))
                    }
                    "q" if !modifiers.shift() => Some(Message::Quit),
                    "w" if !modifiers.shift() => Some(Message::Hide),
                    _ => None,
                }
            }
            Event::Keyboard(KeyEvent::ModifiersChanged(modifiers)) => {
                Some(Message::Modifiers(modifiers))
            }
            Event::Window(window::Event::Focused) => Some(Message::AppearanceChanged),
            _ => None,
        })
    }

    /// Every key press, for choosing a new global shortcut. Leaving the
    /// panel cancels, so the old shortcut is never left unregistered.
    #[cfg(target_os = "macos")]
    fn capture_keys() -> Subscription<Message> {
        event::listen_with(|event, _, _| match event {
            Event::Keyboard(KeyEvent::KeyPressed {
                physical_key,
                modifiers,
                repeat: false,
                ..
            }) => Some(Message::CapturedKey(physical_key, modifiers)),
            Event::Window(window::Event::Unfocused) => Some(Message::CancelCapture),
            _ => None,
        })
    }

    fn view(&self) -> Element<'_, Message> {
        let count = self.match_count();
        if self.settings_open {
            return column![self.header(count), self.settings_view(), self.footer()].into();
        }
        column![
            self.header(count),
            container(
                text_input("Search clips…", &self.query)
                    .id(SEARCH_ID)
                    .on_input(Message::Query)
                    .on_submit(Message::RestoreSelected)
                    .padding([11, 14])
                    .size(15)
                    .style(style::search(self.colors))
            )
            .padding([0, 16]),
            container(self.results(count))
                .padding([12, 16])
                .height(Fill),
            self.footer(),
        ]
        .into()
    }

    fn header(&self, count: usize) -> Element<'_, Message> {
        #[cfg(target_os = "macos")]
        let capture_mode: Element<'_, Message> = button(
            row![
                text("●").size(9),
                text(if self.recording {
                    "Recording"
                } else {
                    "Paused"
                })
                .size(12)
                .font(BOLD),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .padding([4, 10])
        .style(style::pill(self.colors, self.recording))
        .on_press(Message::ToggleRecording)
        .into();
        #[cfg(target_os = "linux")]
        let capture_mode: Element<'_, Message> = button(text("Manual capture").size(12).font(BOLD))
            .padding([4, 10])
            .style(style::pill(self.colors, false))
            .into();

        let place = match &self.store {
            // Read a cached flag so drawing never waits on a running save.
            Some(_) if self.store_encrypted => "encrypted folder",
            Some(_) => "readable folder",
            None if self.storage.folder.is_some() => "not saved",
            None => "this session",
        };
        let summary = match (count, self.query.is_empty()) {
            (1, true) => format!("1 clip · {place}"),
            (n, true) => format!("{n} clips · {place}"),
            (1, false) => "1 match".to_owned(),
            (n, false) => format!("{n} matches"),
        };

        mouse_area(
            container(
                row![
                    column![
                        text("Repasta").size(20).font(BOLD),
                        text(summary).size(12).color(self.colors.muted),
                    ]
                    .spacing(2),
                    Space::new().width(Fill),
                    capture_mode,
                    button(text("⚙").size(15))
                        .padding([2, 7])
                        .style(style::ghost(self.colors))
                        .on_press(if self.settings_open {
                            Message::CloseSettings
                        } else {
                            Message::OpenSettings
                        }),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            // Leave room for the macOS traffic lights above the title.
            .padding(iced::Padding {
                top: 38.0,
                right: 16.0,
                bottom: 14.0,
                left: 18.0,
            })
            .width(Fill),
        )
        .on_press(Message::DragWindow)
        .into()
    }

    fn settings_view(&self) -> Element<'_, Message> {
        let choice = |selected: bool, title: &'static str, detail: &'static str, message| {
            button(
                column![
                    text(title).size(14).font(BOLD).color(self.colors.text),
                    text(detail).size(12).color(self.colors.muted),
                ]
                .spacing(3),
            )
            .width(Fill)
            .padding([10, 12])
            .style(style::clip_card(self.colors, selected))
            .on_press(message)
        };

        let appearance = Space::new().width(Fill);
        let appearance = settings::Appearance::ALL.into_iter().fold(
            row![text("Appearance").size(15).font(BOLD), appearance]
                .spacing(6)
                .align_y(Alignment::Center),
            |row, choice| {
                row.push(
                    button(text(choice.label()).size(12).font(BOLD))
                        .padding([5, 12])
                        .style(style::pill(self.colors, choice == self.appearance))
                        .on_press(Message::SetAppearance(choice)),
                )
            },
        );

        #[cfg(target_os = "macos")]
        let shortcut: Element<'_, Message> = {
            let pill = |label: String, active: bool, message: Message| {
                button(text(label).size(12).font(BOLD))
                    .padding([5, 12])
                    .style(style::pill(self.colors, active))
                    .on_press(message)
            };
            let capturing = self.capturing_shortcut;
            let mut pills = row![
                text("Shortcut").size(15).font(BOLD),
                Space::new().width(Fill)
            ]
            .spacing(6)
            .align_y(Alignment::Center);
            for preset in hotkey::Shortcut::PRESETS {
                pills = pills.push(pill(
                    preset.label(),
                    !capturing && self.shortcut == Some(preset),
                    Message::SetShortcut(Some(preset)),
                ));
            }
            let custom = self
                .shortcut
                .filter(|shortcut| !hotkey::Shortcut::PRESETS.contains(shortcut));
            pills = pills.push(if capturing {
                pill("Press keys…".to_owned(), true, Message::CancelCapture)
            } else if let Some(custom) = custom {
                pill(custom.label(), true, Message::CaptureShortcut)
            } else {
                pill("Record…".to_owned(), false, Message::CaptureShortcut)
            });
            pills = pills.push(pill(
                "Off".to_owned(),
                !capturing && self.shortcut.is_none(),
                Message::SetShortcut(None),
            ));

            let note = match self.shortcut {
                _ if capturing => "Press the keys for the new shortcut, holding ⌘, ⌃, or ⌥. \
                     Esc cancels."
                    .to_owned(),
                None => "Use the menu bar icon to show Repasta.".to_owned(),
                Some(hotkey::Shortcut::SHIFT_COMMAND_V) => {
                    "Shows or hides Repasta from any app. Many apps use ⇧⌘V to paste and \
                     match style; Repasta takes it from them while it runs."
                        .to_owned()
                }
                Some(hotkey::Shortcut::CONTROL_COMMAND_V) => {
                    "Shows or hides Repasta from any app. Other apps rarely use ⌃⌘V. \
                     Click Record… to choose your own."
                        .to_owned()
                }
                Some(custom) => format!(
                    "Shows or hides Repasta from any app. If another app uses {}, Repasta \
                     takes it while it runs. Click it to record another.",
                    custom.label()
                ),
            };
            column![pills, text(note).size(12).color(self.colors.muted)]
                .spacing(6)
                .into()
        };
        #[cfg(target_os = "linux")]
        let shortcut: Element<'_, Message> = column![
            text("Shortcut").size(15).font(BOLD),
            text("Global shortcuts are not available on Linux yet.")
                .size(12)
                .color(self.colors.muted),
        ]
        .spacing(6)
        .into();

        let mut content = column![
            appearance,
            shortcut,
            Space::new().height(6),
            text("History storage").size(15).font(BOLD),
            choice(
                !self.draft.folder,
                "This session only",
                "Kept in memory and forgotten when Repasta quits.",
                Message::DraftSession,
            ),
            choice(
                self.draft.folder,
                "Save to a folder",
                "Kept in a file in a folder you choose.",
                Message::DraftFolder,
            ),
        ]
        .spacing(10);

        if self.draft.folder {
            let path = text_input("/full/path/to/folder", &self.draft.path)
                .on_input(Message::DraftPath)
                .on_submit(Message::ApplyStorage)
                .padding([8, 10])
                .size(13)
                .style(style::search(self.colors));
            #[cfg(target_os = "macos")]
            let path = row![
                path,
                button(text("Choose…").size(12).font(BOLD))
                    .padding([7, 10])
                    .style(style::pill(self.colors, false))
                    .on_press(Message::ChooseFolder),
            ]
            .spacing(8)
            .align_y(Alignment::Center);

            let encrypt = toggler(self.draft.encrypted)
                .label("Encrypt with a key in the Keychain")
                .text_size(13)
                .size(18)
                .on_toggle_maybe(settings::ENCRYPTION_AVAILABLE.then_some(Message::DraftEncrypted));
            let note = if !settings::ENCRYPTION_AVAILABLE {
                text("Encryption is not available on this platform yet. Anything you copy, including passwords, is saved as readable text.")
                    .color(self.colors.warning)
            } else if self.draft.encrypted {
                text("Only this Mac account can read the file. Files sealed here cannot be opened on another Mac.")
                    .color(self.colors.muted)
            } else {
                text("Anything you copy, including passwords, is saved as readable text.")
                    .color(self.colors.warning)
            };
            content = content.push(path).push(encrypt).push(note.size(12));
        }

        let buttons = row![
            text("Existing files are kept when you switch.")
                .size(11)
                .color(self.colors.faint),
            Space::new().width(Fill),
            button(text("Cancel").size(12).font(BOLD))
                .padding([6, 12])
                .style(style::ghost(self.colors))
                .on_press(Message::CloseSettings),
            button(text("Apply").size(12).font(BOLD))
                .padding([6, 14])
                .style(style::pill(self.colors, true))
                .on_press(Message::ApplyStorage),
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        container(column![content, Space::new().height(Fill), buttons])
            .padding(iced::Padding {
                top: 4.0,
                right: 16.0,
                bottom: 14.0,
                left: 16.0,
            })
            .height(Fill)
            .into()
    }

    fn results(&self, count: usize) -> Element<'_, Message> {
        if count == 0 {
            let (title, hint) = if self.query.is_empty() {
                (
                    "Nothing here yet",
                    if cfg!(target_os = "macos") {
                        "Click Paused to start recording copies,\nor press Capture to save the current clip."
                    } else {
                        "Press Capture to save the current clip."
                    },
                )
            } else {
                (
                    "No matching clips",
                    "Search is case-sensitive. Esc clears it.",
                )
            };
            return container(
                column![
                    text(title).size(15).font(BOLD).color(self.colors.text),
                    text(hint)
                        .size(13)
                        .align_x(Alignment::Center)
                        .color(self.colors.muted),
                ]
                .spacing(6)
                .align_x(Alignment::Center),
            )
            .center(Fill)
            .style(style::empty_state(self.colors))
            .into();
        }

        let mut list = column![].spacing(8).width(Fill);
        let visible = self
            .history
            .search(&self.query)
            .enumerate()
            .skip(self.offset)
            .take(self.visible);
        for (index, clip) in visible {
            list = list.push(self.clip_card(index, clip));
        }
        mouse_area(container(list).height(Fill).clip(true))
            .on_scroll(|delta| {
                Message::Scroll(match delta {
                    ScrollDelta::Lines { y, .. } => y * SCROLL_STEP,
                    ScrollDelta::Pixels { y, .. } => y,
                })
            })
            .into()
    }

    fn clip_card<'a>(&self, index: usize, clip: &'a Clip) -> Element<'a, Message> {
        let selected = index == self.selected;
        let formatted =
            matches!(clip, Clip::Text(text) if text.rtf.is_some() || text.html.is_some());
        let hint: Element<'a, Message> = if selected {
            text(if formatted {
                "⏎ copy · ⇧⏎ plain"
            } else {
                "⏎ copy"
            })
            .size(11)
            .color(self.colors.accent)
            .into()
        } else if index < 9 {
            text(format!(
                "{}{}",
                if cfg!(target_os = "macos") {
                    "⌘"
                } else {
                    "Ctrl "
                },
                index + 1
            ))
            .size(11)
            .color(self.colors.faint)
            .into()
        } else {
            Space::new().into()
        };

        let delete = button(text("×").size(14).line_height(1.0))
            .padding([1, 6])
            .style(style::ghost(self.colors))
            .on_press(Message::Delete(index));
        let hint = row![hint, delete].spacing(6).align_y(Alignment::Center);

        let content: Element<'a, Message> = match clip {
            Clip::Text(clip_text) => {
                let card = Preview::of(&clip_text.plain);
                let look = looks::look(&clip_text.plain);
                let line_color = match look {
                    looks::Look::Link { .. } => self.colors.accent,
                    _ => self.colors.text,
                };
                let mut body = column![].spacing(2);
                for line in card.lines {
                    body = body.push(
                        text(line)
                            .font(Font::MONOSPACE)
                            .size(12)
                            .color(line_color)
                            .wrapping(Wrapping::None),
                    );
                }

                let mut meta = match &look {
                    looks::Look::Link { host } => format!("Link · {}", preview_line(host)),
                    looks::Look::Color(_) => "Color".to_owned(),
                    looks::Look::Plain => {
                        let mut meta = format!("{} chars", card.chars);
                        if card.total_lines > 1 {
                            meta.push_str(&format!(" · {} lines", card.total_lines));
                        }
                        meta
                    }
                };
                if formatted {
                    meta.insert_str(0, "Formatted · ");
                }

                let body: Element<'a, Message> = match look {
                    looks::Look::Color(color) => row![
                        container(Space::new())
                            .width(SWATCH)
                            .height(SWATCH)
                            .style(style::swatch(self.colors, color)),
                        container(body).width(Fill).clip(true),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center)
                    .into(),
                    _ => container(body).width(Fill).clip(true).into(),
                };
                column![
                    body,
                    row![
                        text(meta).size(11).color(self.colors.faint),
                        Space::new().width(Fill),
                        hint
                    ]
                    .align_y(Alignment::Center),
                ]
                .spacing(6)
                .into()
            }
            Clip::Image(image) => row![
                thumbnail::Preview::new(
                    image.thumbnail.clone(),
                    Size::new(84.0, 52.0),
                    self.colors.surface_raised,
                    self.colors.outline,
                ),
                column![
                    text("Image").size(13).font(BOLD).color(self.colors.text),
                    text(format!(
                        "{} × {} · {}",
                        image.width,
                        image.height,
                        human_size(image.png.len())
                    ))
                    .size(11)
                    .color(self.colors.faint),
                    Space::new().height(Fill),
                    row![Space::new().width(Fill), hint],
                ]
                .spacing(3)
                .height(52),
            ]
            .spacing(12)
            .into(),
        };

        button(content)
            .width(Fill)
            .padding([10, 12])
            .style(style::clip_card(self.colors, selected))
            .on_press(Message::Restore(index))
            .into()
    }

    fn footer(&self) -> Element<'_, Message> {
        #[cfg(target_os = "macos")]
        let busy = false;
        #[cfg(target_os = "linux")]
        let busy = self.reading;

        let action = |label, message: Message| {
            button(text(label).size(12).font(BOLD))
                .padding([5, 9])
                .style(style::ghost(self.colors))
                .on_press_maybe((!busy).then_some(message))
        };

        let keys = row![
            keycap(self.colors, "↑↓"),
            text("select").size(11).color(self.colors.faint),
            keycap(self.colors, "⏎"),
            text("copy").size(11).color(self.colors.faint),
            keycap(self.colors, "esc"),
            text("reset").size(11).color(self.colors.faint),
        ]
        .spacing(5)
        .align_y(Alignment::Center);

        let tone = match self.tone {
            Tone::Neutral => self.colors.muted,
            Tone::Good => self.colors.success,
            Tone::Warn => self.colors.warning,
        };

        let actions: Element<'_, Message> = if self.confirm_clear {
            let folder = self
                .storage
                .folder
                .as_deref()
                .map(folder_name)
                .unwrap_or_default();
            row![
                text(format!(
                    "Delete every clip, including the file in {folder}?"
                ))
                .size(12)
                .color(self.colors.warning)
                .width(Fill),
                action("Cancel", Message::CancelClear),
                button(text("Clear all").size(12).font(BOLD))
                    .padding([5, 9])
                    .style(button::danger)
                    .on_press(Message::ConfirmClear),
            ]
            .spacing(4)
            .align_y(Alignment::Center)
            .into()
        } else {
            row![
                action("Capture", Message::Capture),
                action("Clear", Message::Clear),
                button(
                    text(if self.pinned { "Pinned" } else { "Pin" })
                        .size(12)
                        .font(BOLD)
                )
                .padding([5, 9])
                .style(style::ghost(self.colors))
                .on_press(Message::TogglePin),
                Space::new().width(Fill),
                keys,
            ]
            .spacing(2)
            .align_y(Alignment::Center)
            .into()
        };

        container(
            column![
                actions,
                row![
                    text(&self.status).size(12).color(tone),
                    Space::new().width(Fill),
                    button(text("Quit").size(11))
                        .padding([2, 8])
                        .style(style::ghost(self.colors))
                        .on_press(Message::Quit),
                ]
                .align_y(Alignment::Center),
            ]
            .spacing(6),
        )
        .padding(iced::Padding {
            top: 10.0,
            right: 14.0,
            bottom: 12.0,
            left: 10.0,
        })
        .width(Fill)
        .style(style::footer(self.colors))
        .into()
    }
}

/// Opens the store for a folder. A sealer is needed to encrypt, or to read
/// encrypted history when switching the folder to readable.
fn open_store(folder: &Path, encrypted: bool) -> Result<FolderStore, StoreError> {
    let sealed = FolderStore::has_sealed_history(folder);
    let sealer: Option<Box<dyn Sealer>> = if encrypted || sealed {
        #[cfg(target_os = "macos")]
        {
            // Create a key only when no sealed history could need the old one.
            Some(Box::new(keychain::KeychainSealer::open(!sealed)?))
        }
        #[cfg(target_os = "linux")]
        {
            return Err(if encrypted {
                StoreError::EncryptionUnavailable
            } else {
                StoreError::Locked
            });
        }
    } else {
        None
    };
    Ok(FolderStore::new(folder, encrypted, sealer))
}

/// Runs blocking work on its own thread and resolves with its result.
async fn run_in_background<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, StoreError> + Send + 'static,
) -> Result<T, StoreError> {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    std::thread::Builder::new()
        .name("repasta-save".into())
        .spawn(move || {
            let _ = sender.send(work());
        })
        .map_err(|error| StoreError::Io(error.kind()))?;
    receiver
        .await
        .unwrap_or(Err(StoreError::Io(std::io::ErrorKind::Interrupted)))
}

/// Locks the store, recovering it if a save thread panicked.
fn lock(store: &Mutex<FolderStore>) -> std::sync::MutexGuard<'_, FolderStore> {
    store.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A byte count for display, such as "2.3 MB".
fn human_size(bytes: usize) -> String {
    match bytes {
        0..1_000 => format!("{bytes} B"),
        1_000..1_000_000 => format!("{:.0} KB", bytes as f64 / 1e3),
        _ => format!("{:.1} MB", bytes as f64 / 1e6),
    }
}

/// The folder's own name for status messages.
fn folder_name(folder: &Path) -> String {
    folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| folder.display().to_string())
}

fn visible_cards(height: f32) -> usize {
    ((height - CHROME_HEIGHT) / CARD_HEIGHT).max(1.0) as usize
}

fn keycap(colors: style::Colors, label: &str) -> Element<'_, Message> {
    container(text(label).size(10).font(Font::MONOSPACE))
        .padding([1, 5])
        .style(style::keycap(colors))
        .into()
}

/// A bounded, display-safe excerpt of a clip.
struct Preview {
    lines: Vec<String>,
    total_lines: usize,
    chars: usize,
}

impl Preview {
    fn of(clip: &str) -> Self {
        let lines = clip
            .lines()
            .filter(|line| !line.trim().is_empty())
            .take(PREVIEW_LINES)
            .map(preview_line)
            .collect();
        Self {
            lines,
            total_lines: clip.lines().count(),
            chars: clip.chars().count(),
        }
    }
}

fn preview_line(line: &str) -> String {
    let mut chars = line.trim_end().chars();
    let mut preview = String::new();
    for character in chars.by_ref().take(PREVIEW_CHARS) {
        match character {
            '\t' => preview.push_str("  "),
            // Quotes and backslashes are printable; keep them readable.
            '"' | '\'' | '\\' => preview.push(character),
            // escape_debug escapes controls and Unicode format characters, including bidi overrides.
            _ => preview.extend(character.escape_debug()),
        }
    }
    if chars.next().is_some() {
        preview.push('…');
    }
    preview
}

fn main() -> iced::Result {
    iced::application(Palette::boot, Palette::update, Palette::view)
        .title("Repasta")
        .theme(|palette: &Palette| palette.theme.clone())
        .style(|palette, _| iced::theme::Style {
            background_color: palette.colors.background,
            text_color: palette.colors.text,
        })
        .subscription(Palette::subscription)
        .window(window::Settings {
            size: WINDOW_SIZE,
            min_size: Some(Size::new(340.0, 460.0)),
            // Float near the top-right corner like a desktop widget.
            position: window::Position::SpecificWith(|window, monitor| {
                Point::new((monitor.width - window.width - 24.0).max(0.0), 56.0)
            }),
            level: window::Level::AlwaysOnTop,
            // Close requests arrive as `Message::Hide`.
            exit_on_close_request: false,
            #[cfg(target_os = "macos")]
            platform_specific: window::settings::PlatformSpecific {
                title_hidden: true,
                titlebar_transparent: true,
                fullsize_content_view: true,
            },
            ..window::Settings::default()
        })
        .run()
}

#[cfg(test)]
mod tests {
    use super::{PREVIEW_CHARS, Preview, preview_line};

    #[test]
    fn clip_previews_escape_controls_and_bound_visible_text() {
        assert_eq!(preview_line("row\u{202e}\u{1b}x"), r"row\u{202e}\u{1b}x");
        assert_eq!(preview_line("say \"hi\"\tnow"), "say \"hi\"  now");
        assert_eq!(
            preview_line(&"é".repeat(PREVIEW_CHARS + 2)),
            format!("{}…", "é".repeat(PREVIEW_CHARS))
        );
    }

    #[test]
    fn clip_previews_skip_blank_lines_and_count_the_rest() {
        let card = Preview::of("\n  \nfirst\n\nsecond\nthird\n");
        assert_eq!(card.lines, ["first", "second"]);
        assert_eq!(card.total_lines, 6);
        assert_eq!(card.chars, 24);
    }
}
