//! Remembers where history lives. macOS keeps the setting in the app's user
//! defaults; Linux keeps it in `$XDG_CONFIG_HOME/repasta/settings`.

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

/// Whether this platform has a key storage adapter.
pub const ENCRYPTION_AVAILABLE: bool = cfg!(target_os = "macos");

#[cfg(target_os = "macos")]
mod platform {
    use super::Storage;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSModalPanelWindowLevel, NSModalResponseOK, NSOpenPanel};
    use objc2_foundation::{NSString, NSURL, NSUserDefaults};
    use std::path::PathBuf;

    const FOLDER: &str = "StorageFolder";
    const ENCRYPTED: &str = "StorageEncrypted";

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
    use super::Storage;
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

    pub fn load() -> Storage {
        let mut storage = Storage::default();
        let mut text = String::new();
        let read = file()
            .and_then(|path| std::fs::File::open(path).ok())
            .and_then(|file| file.take(MAX_SETTINGS_BYTES).read_to_string(&mut text).ok());
        if read.is_none() {
            return storage;
        }
        for line in text.lines() {
            if let Some(folder) = line.strip_prefix("folder=") {
                storage.folder = Some(PathBuf::from(folder)).filter(|path| path.is_absolute());
            }
        }
        storage
    }

    pub fn save(storage: &Storage) {
        let Some(path) = file() else { return };
        let mut text = String::new();
        if let Some(folder) = storage.folder.as_deref().and_then(|folder| folder.to_str())
            && !folder.contains('\n')
        {
            text = format!("folder={folder}\n");
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, text);
    }
}

pub use platform::*;
