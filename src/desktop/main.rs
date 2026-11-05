#[cfg(target_os = "macos")]
mod appearance;
#[cfg(target_os = "macos")]
mod keychain;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod menu_bar;
mod settings;
mod style;

use iced::keyboard::{Event as KeyEvent, Key, Modifiers, key::Named};
use iced::mouse::ScrollDelta;
use iced::widget::{
    Space, button, column, container, mouse_area, row, text, text::Wrapping, text_input, toggler,
};
use iced::{
    Alignment, Element, Event, Fill, Font, Point, Size, Subscription, Task, event, font, window,
};
use repasta::store::{FolderStore, Sealer, Store, StoreError};
use repasta::{Capture, History, MAX_CLIP_BYTES};
use std::path::{Path, PathBuf};

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

const BOLD: Font = Font {
    weight: font::Weight::Semibold,
    ..Font::DEFAULT
};

#[derive(Clone)]
enum Message {
    Capture,
    #[cfg(target_os = "linux")]
    Captured(Result<String, &'static str>),
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
    Clear,
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
    #[cfg(target_os = "macos")]
    ChooseFolder,
    ApplyStorage,
}

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
    /// chosen folder is unavailable.
    store: Option<FolderStore>,
    settings_open: bool,
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
            ..Self::default()
        };
        palette.refresh_colors(true);
        palette.attach_storage(false);
        (
            palette,
            Task::batch([
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
            self.history.extend_older(saved.iter().map(String::as_str));
            if save || store.has_other_form() {
                store.save(&self.history)?;
            }
            Ok((store, saved.len()))
        });
        match result {
            Ok((store, count)) => {
                let name = folder_name(&folder);
                let form = if store.encrypted() {
                    "encrypted"
                } else {
                    "readable"
                };
                self.store = Some(store);
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

    /// Saves history after a change. On failure, keeps the session going and
    /// stops writing until storage is applied again.
    fn persist(&mut self) {
        if let Some(store) = &mut self.store
            && let Err(error) = store.save(&self.history)
        {
            self.store = None;
            self.say(
                Tone::Warn,
                format!("{error} Saving is off until you apply storage again."),
            );
        }
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

    fn accept(&mut self, result: Result<String, &'static str>) {
        match result.map(|clip| self.history.capture(&clip)) {
            Ok(Capture::Added | Capture::Refreshed) => {
                self.reset_selection();
                self.say(Tone::Good, "Text captured.");
                self.persist();
            }
            Ok(Capture::Empty) => self.say(Tone::Warn, "Clipboard text is empty."),
            Ok(Capture::TooLarge) => self.say(Tone::Warn, "Clipboard text exceeds 64 KiB."),
            Err(status) => self.say(Tone::Warn, status),
        }
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
                self.accept(result);
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
                self.accept(result);
            }
            #[cfg(target_os = "macos")]
            Message::ToggleRecording => {
                if self.recording {
                    self.recording = false;
                    self.say(Tone::Neutral, "Background capture paused.");
                } else if self.clipboard.reset_observation() {
                    self.recording = true;
                    self.say(Tone::Good, "Recording future text copies.");
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
                    self.accept(result);
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
            Message::Clear => {
                self.history = History::default();
                self.query.clear();
                self.reset_selection();
                #[cfg(target_os = "macos")]
                self.clipboard.reset_observation();
                match self.store.as_mut().map(|store| store.erase()) {
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
            Message::CloseSettings => self.settings_open = false,
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
                    .store
                    .as_ref()
                    .filter(|_| storage.folder.is_none())
                    .map(|store| folder_name(store.folder()));
                self.storage = storage;
                settings::save(&self.storage);
                self.attach_storage(true);
                if let Some(name) = leaving {
                    self.say(
                        Tone::Neutral,
                        format!("Session-only history. The file in {name} was kept."),
                    );
                }
                self.settings_open = false;
            }
        }
        Task::none()
    }

    fn restore(&mut self, index: usize) -> Task<Message> {
        let Some(clip) = self
            .history
            .search(&self.query)
            .nth(index)
            .map(str::to_owned)
        else {
            return Task::none();
        };
        #[cfg(target_os = "macos")]
        {
            if self.clipboard.write(&clip) {
                self.say(Tone::Good, "Copied. Paste it anywhere with ⌘V.");
            } else {
                self.say(Tone::Warn, "Clipboard write failed.");
            }
            Task::none()
        }
        #[cfg(target_os = "linux")]
        {
            self.say(Tone::Good, "Clipboard write requested.");
            iced::clipboard::write(clip)
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let keyboard = event::listen_with(|event, _, _| match event {
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
            }) if modifiers == Modifiers::COMMAND => match key.as_str() {
                digit @ ("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9") => {
                    digit.parse::<usize>().ok().map(|n| Message::Restore(n - 1))
                }
                "q" => Some(Message::Quit),
                "w" => Some(Message::Hide),
                _ => None,
            },
            Event::Window(window::Event::Focused) => Some(Message::AppearanceChanged),
            _ => None,
        });
        let keyboard = Subscription::batch([
            keyboard,
            window::resize_events().map(|(_, size)| Message::Resized(size)),
            window::close_requests().map(|_| Message::Hide),
            #[cfg(target_os = "linux")]
            iced::system::theme_changes().map(Message::SystemTheme),
            #[cfg(target_os = "macos")]
            Subscription::run(appearance::changes).map(|()| Message::AppearanceChanged),
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
            Some(store) if store.encrypted() => "encrypted folder",
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

        let mut content = column![
            appearance,
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

    fn clip_card<'a>(&self, index: usize, clip: &'a str) -> Element<'a, Message> {
        let selected = index == self.selected;
        let card = Preview::of(clip);

        let mut body = column![].spacing(2);
        for line in card.lines {
            body = body.push(
                text(line)
                    .font(Font::MONOSPACE)
                    .size(12)
                    .wrapping(Wrapping::None),
            );
        }

        let mut meta = format!("{} chars", card.chars);
        if card.total_lines > 1 {
            meta.push_str(&format!(" · {} lines", card.total_lines));
        }

        let hint: Element<'a, Message> = if selected {
            text("⏎ copy").size(11).color(self.colors.accent).into()
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

        button(
            column![
                container(body).width(Fill).clip(true),
                row![
                    text(meta).size(11).color(self.colors.faint),
                    Space::new().width(Fill),
                    hint
                ]
                .align_y(Alignment::Center),
            ]
            .spacing(6),
        )
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

        container(
            column![
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
                .align_y(Alignment::Center),
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
