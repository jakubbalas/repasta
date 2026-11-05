//! Remembers the panel's preferences: where history lives, its appearance,
//! and its global shortcut. macOS keeps them in the app's user defaults; Linux keeps them
//! in `$XDG_CONFIG_HOME/repasta/settings`.

use std::path::PathBuf;

/// Where history lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Storage {
    /// `None` keeps history for this session only.
    pub folder: Option<PathBuf>,
    /// Whether a folder's history is encrypted.
    pub encrypted: bool,
}

impl Default for Storage {
    fn default() -> Self {
        Self {
            folder: None,
            encrypted: ENCRYPTION_AVAILABLE,
        }
    }
}

/// The panel's light or dark appearance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
    /// Follow the system setting.
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    fn from_key(key: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|appearance| appearance.key() == key)
            .unwrap_or_default()
    }
}

/// Whether this platform has a key storage adapter.
pub const ENCRYPTION_AVAILABLE: bool = cfg!(target_os = "macos");

#[cfg(target_os = "macos")]
mod platform {
    use super::{Appearance, Storage};
    use crate::hotkey::Shortcut;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSModalPanelWindowLevel, NSModalResponseOK, NSOpenPanel};
    use objc2_foundation::{NSString, NSURL, NSUserDefaults};
    use std::path::PathBuf;

    const FOLDER: &str = "StorageFolder";
    const ENCRYPTED: &str = "StorageEncrypted";
    const APPEARANCE: &str = "Appearance";
    const SHORTCUT: &str = "Shortcut";

    const SHORTCUT_OFF: &str = "off";

    /// The global shortcut, or `None` when it's off. Defaults to ⌃⌘V, which
    /// other apps rarely use, and also when the saved value is unreadable.
    pub fn load_shortcut() -> Option<Shortcut> {
        let Some(name) =
            NSUserDefaults::standardUserDefaults().stringForKey(&NSString::from_str(SHORTCUT))
        else {
            return Some(Shortcut::CONTROL_COMMAND_V);
        };
        let name = name.to_string();
        if name == SHORTCUT_OFF {
            return None;
        }
        Some(Shortcut::parse(&name).unwrap_or(Shortcut::CONTROL_COMMAND_V))
    }

    pub fn save_shortcut(shortcut: Option<Shortcut>) {
        let name = shortcut.map_or_else(|| SHORTCUT_OFF.to_owned(), Shortcut::name);
        let defaults = NSUserDefaults::standardUserDefaults();
        // SAFETY: An NSString is a valid property list value.
        unsafe {
            defaults.setObject_forKey(
                Some(&NSString::from_str(&name)),
                &NSString::from_str(SHORTCUT),
            );
        }
    }

    pub fn load_appearance() -> Appearance {
        NSUserDefaults::standardUserDefaults()
            .stringForKey(&NSString::from_str(APPEARANCE))
            .map(|key| Appearance::from_key(&key.to_string()))
            .unwrap_or_default()
    }

    pub fn save_appearance(appearance: Appearance) {
        let defaults = NSUserDefaults::standardUserDefaults();
        // SAFETY: An NSString is a valid property list value.
        unsafe {
            defaults.setObject_forKey(
                Some(&NSString::from_str(appearance.key())),
                &NSString::from_str(APPEARANCE),
            );
        }
    }

    pub fn load() -> Storage {
        let defaults = NSUserDefaults::standardUserDefaults();
        let key = |name| NSString::from_str(name);
        let folder = defaults
            .stringForKey(&key(FOLDER))
            .map(|folder| PathBuf::from(folder.to_string()));
        // Encryption defaults to on when never set.
        let encrypted = defaults.objectForKey(&key(ENCRYPTED)).is_none()
            || defaults.boolForKey(&key(ENCRYPTED));
        Storage { folder, encrypted }
    }

    pub fn save(storage: &Storage) {
        let defaults = NSUserDefaults::standardUserDefaults();
        let key = NSString::from_str(FOLDER);
        match storage.folder.as_deref().and_then(|folder| folder.to_str()) {
            // SAFETY: An NSString is a valid property list value.
            Some(folder) => unsafe {
                defaults.setObject_forKey(Some(&NSString::from_str(folder)), &key)
            },
            None => defaults.removeObjectForKey(&key),
        }
        defaults.setBool_forKey(storage.encrypted, &NSString::from_str(ENCRYPTED));
    }

    /// Asks for a folder with the system folder picker. Blocks until the
    /// user chooses or cancels.
    pub fn choose_folder(current: Option<&std::path::Path>) -> Option<PathBuf> {
        let mtm = MainThreadMarker::new()?;
        let panel = NSOpenPanel::openPanel(mtm);
        panel.setCanChooseFiles(false);
        panel.setCanChooseDirectories(true);
        panel.setCanCreateDirectories(true);
        panel.setAllowsMultipleSelection(false);
        panel.setPrompt(Some(&NSString::from_str("Use Folder")));
        panel.setMessage(Some(&NSString::from_str(
            "Choose where Repasta saves clip history.",
        )));
        if let Some(current) = current.and_then(|path| path.to_str()) {
            panel.setDirectoryURL(Some(&NSURL::fileURLWithPath(&NSString::from_str(current))));
        }
        // Appear above the pinned panel.
        panel.setLevel(NSModalPanelWindowLevel);
        if panel.runModal() != NSModalResponseOK {
            return None;
        }
        let path = panel.URL()?.path()?;
        Some(PathBuf::from(path.to_string()))
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::{Appearance, Storage};
    use std::io::Read;
    use std::path::PathBuf;

    const MAX_SETTINGS_BYTES: u64 = 16 * 1024;

    fn file() -> Option<PathBuf> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
        Some(config.join("repasta").join("settings"))
    }

    /// `key=value` lines from the settings file.
    fn entries() -> Vec<(String, String)> {
        let mut text = String::new();
        let read = file()
            .and_then(|path| std::fs::File::open(path).ok())
            .and_then(|file| file.take(MAX_SETTINGS_BYTES).read_to_string(&mut text).ok());
        if read.is_none() {
            return Vec::new();
        }
        text.lines()
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect()
    }

    fn value(key: &str) -> Option<String> {
        entries()
            .into_iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    /// Sets or removes one entry and keeps the others.
    fn store(key: &str, value: Option<&str>) {
        let Some(path) = file() else { return };
        let mut text = String::new();
        for (name, existing) in entries() {
            if name != key {
                text.push_str(&format!("{name}={existing}\n"));
            }
        }
        if let Some(value) = value.filter(|value| !value.contains('\n')) {
            text.push_str(&format!("{key}={value}\n"));
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, text);
    }

    pub fn load() -> Storage {
        Storage {
            folder: value("folder")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute()),
            ..Storage::default()
        }
    }

    pub fn save(storage: &Storage) {
        store(
            "folder",
            storage.folder.as_deref().and_then(|folder| folder.to_str()),
        );
    }

    pub fn load_appearance() -> Appearance {
        value("appearance")
            .map(|key| Appearance::from_key(&key))
            .unwrap_or_default()
    }

    pub fn save_appearance(appearance: Appearance) {
        store("appearance", Some(appearance.key()));
    }
}

pub use platform::*;
