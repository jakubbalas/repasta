//! Where history lives between sessions.
//!
//! A [`Store`] loads, saves, and erases a whole history. [`FolderStore`] keeps
//! one file in a user-chosen folder, either readable or sealed by a
//! [`Sealer`] that a platform adapter provides. Saved files are treated as
//! hostile input and bounded by the same limits as live history.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::{History, MAX_CLIP_BYTES, MAX_HISTORY_BYTES, MAX_ITEMS};

/// File name of a readable history.
pub const READABLE_FILE: &str = "repasta-history.txt";
/// File name of an encrypted history.
pub const SEALED_FILE: &str = "repasta-history.sealed";
const TEMPORARY_FILE: &str = ".repasta-history.tmp";

const READABLE_HEADER: &[u8] = b"Repasta history v1\n";
const SEALED_HEADER: &[u8] = b"Repasta sealed history v1\n";
/// Room for per-clip length lines and a sealer's nonce and tag.
const MAX_FILE_BYTES: u64 = (MAX_HISTORY_BYTES + MAX_ITEMS * 8 + 4096) as u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreError {
    /// The folder or file could not be read or written.
    Io(io::ErrorKind),
    /// The saved file is malformed, exceeds the limits, or failed to decrypt.
    Unreadable,
    /// The folder holds encrypted history but no key is available.
    Locked,
    /// Encryption was requested but this platform has no key storage adapter.
    EncryptionUnavailable,
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Io(io::ErrorKind::NotFound) => "The folder does not exist.",
            Self::Io(io::ErrorKind::PermissionDenied) => "Repasta may not access the folder.",
            Self::Io(_) => "The folder could not be read or written.",
            Self::Unreadable => "The saved history is damaged or was sealed with another key.",
            Self::Locked => "The folder holds encrypted history and its key is unavailable.",
            Self::EncryptionUnavailable => "Encryption is unavailable on this platform.",
        })
    }
}

impl std::error::Error for StoreError {}

impl From<io::Error> for StoreError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}

/// Authenticated encryption for saved history.
pub trait Sealer {
    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, StoreError>;
    /// Fails with [`StoreError::Unreadable`] when authentication fails.
    fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, StoreError>;
}

/// Persistent storage for a history. Implementations keep clip contents out
/// of errors and logs.
pub trait Store {
    /// Saved clips, newest first.
    fn load(&mut self) -> Result<Vec<String>, StoreError>;
    /// Replaces the saved history with `history`.
    fn save(&mut self, history: &History) -> Result<(), StoreError>;
    /// Deletes the saved history.
    fn erase(&mut self) -> Result<(), StoreError>;
}

/// History kept in one file inside a folder.
pub struct FolderStore {
    folder: PathBuf,
    encrypted: bool,
    /// Seals saves when `encrypted`, and opens sealed history either way so
    /// a folder can switch to readable.
    sealer: Option<Box<dyn Sealer>>,
}

impl FolderStore {
    /// Saves sealed history when `encrypted`, which requires a `sealer`.
    pub fn new(
        folder: impl Into<PathBuf>,
        encrypted: bool,
        sealer: Option<Box<dyn Sealer>>,
    ) -> Self {
        Self {
            folder: folder.into(),
            encrypted,
            sealer,
        }
    }

    /// Whether `folder` holds encrypted history.
    pub fn has_sealed_history(folder: &Path) -> bool {
        folder.join(SEALED_FILE).is_file()
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    pub fn encrypted(&self) -> bool {
        self.encrypted
    }

    fn file(&self) -> PathBuf {
        self.folder.join(if self.encrypted() {
            SEALED_FILE
        } else {
            READABLE_FILE
        })
    }

    fn other_file(&self) -> PathBuf {
        self.folder.join(if self.encrypted() {
            READABLE_FILE
        } else {
            SEALED_FILE
        })
    }

    /// Whether the folder holds history in the other form, which the next
    /// save converts and removes.
    pub fn has_other_form(&self) -> bool {
        self.other_file().is_file()
    }

    fn read_file(path: &Path) -> Result<Option<Vec<u8>>, StoreError> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(StoreError::Unreadable);
        }
        Ok(Some(bytes))
    }

    fn decode_file(&self, bytes: &[u8], sealed: bool) -> Result<Vec<String>, StoreError> {
        if !sealed {
            return decode(bytes);
        }
        let body = bytes
            .strip_prefix(SEALED_HEADER)
            .ok_or(StoreError::Unreadable)?;
        let sealer = self.sealer.as_ref().ok_or(StoreError::Locked)?;
        decode(&sealer.open(body)?)
    }
}

impl Store for FolderStore {
    /// Reads this store's file, and also the other form's file so a folder
    /// can switch between encrypted and readable without losing history.
    fn load(&mut self) -> Result<Vec<String>, StoreError> {
        if !self.folder.is_dir() {
            return Err(StoreError::Io(io::ErrorKind::NotFound));
        }
        let mut clips = Vec::new();
        for (path, sealed) in [
            (self.file(), self.encrypted()),
            (self.other_file(), !self.encrypted()),
        ] {
            if let Some(bytes) = Self::read_file(&path)? {
                clips.extend(self.decode_file(&bytes, sealed)?);
            }
        }
        Ok(clips)
    }

    fn save(&mut self, history: &History) -> Result<(), StoreError> {
        let plain = encode(history);
        let bytes = match (self.encrypted, &self.sealer) {
            (true, Some(sealer)) => [SEALED_HEADER, &sealer.seal(&plain)?].concat(),
            (true, None) => return Err(StoreError::EncryptionUnavailable),
            (false, _) => plain,
        };
        write_atomically(&self.folder, &self.file(), &bytes)?;
        remove_if_present(&self.other_file())
    }

    fn erase(&mut self) -> Result<(), StoreError> {
        remove_if_present(&self.file())?;
        remove_if_present(&self.other_file())
    }
}

fn remove_if_present(path: &Path) -> Result<(), StoreError> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}

/// Writes `bytes` to a temporary file, syncs it, and renames it over `path`,
/// so a crash leaves either the old or the new history.
fn write_atomically(folder: &Path, path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let temporary = folder.join(TEMPORARY_FILE);
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let result = (|| {
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        // Persist the rename itself.
        File::open(folder)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    Ok(result?)
}

/// Serializes `history`, newest first: a header line, then for each clip its
/// byte length on one line followed by the clip and a newline.
pub fn encode(history: &History) -> Vec<u8> {
    let mut bytes = READABLE_HEADER.to_vec();
    for clip in history.iter() {
        bytes.extend_from_slice(clip.len().to_string().as_bytes());
        bytes.push(b'\n');
        bytes.extend_from_slice(clip.as_bytes());
        bytes.push(b'\n');
    }
    bytes
}

/// Parses [`encode`] output within the history limits.
pub fn decode(bytes: &[u8]) -> Result<Vec<String>, StoreError> {
    let mut rest = bytes
        .strip_prefix(READABLE_HEADER)
        .ok_or(StoreError::Unreadable)?;
    let mut clips = Vec::new();
    let mut total = 0;
    while !rest.is_empty() {
        let newline = rest
            .iter()
            .take(8)
            .position(|&byte| byte == b'\n')
            .ok_or(StoreError::Unreadable)?;
        let digits = &rest[..newline];
        if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
            return Err(StoreError::Unreadable);
        }
        let length: usize = std::str::from_utf8(digits)
            .ok()
            .and_then(|digits| digits.parse().ok())
            .ok_or(StoreError::Unreadable)?;
        total += length;
        if length == 0
            || length > MAX_CLIP_BYTES
            || total > MAX_HISTORY_BYTES
            || clips.len() == MAX_ITEMS
        {
            return Err(StoreError::Unreadable);
        }
        rest = &rest[newline + 1..];
        if rest.len() < length + 1 || rest[length] != b'\n' {
            return Err(StoreError::Unreadable);
        }
        let clip = std::str::from_utf8(&rest[..length]).map_err(|_| StoreError::Unreadable)?;
        clips.push(clip.to_owned());
        rest = &rest[length + 1..];
    }
    Ok(clips)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reverses bytes behind a fixed prefix; enough to test the sealed path.
    struct Mirror;

    impl Sealer for Mirror {
        fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, StoreError> {
            Ok([
                b"mirror:".as_slice(),
                &plain.iter().rev().copied().collect::<Vec<_>>(),
            ]
            .concat())
        }

        fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, StoreError> {
            let body = sealed
                .strip_prefix(b"mirror:")
                .ok_or(StoreError::Unreadable)?;
            Ok(body.iter().rev().copied().collect())
        }
    }

    fn history(clips: &[&str]) -> History {
        let mut history = History::default();
        for clip in clips.iter().rev() {
            history.capture(clip);
        }
        history
    }

    fn folder(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("repasta-store-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn readable_format_round_trips_newlines_and_unicode() {
        let clips = ["first\nline\n", "é ✂︎", "12\n"];
        let bytes = encode(&history(&clips));
        assert!(bytes.starts_with(READABLE_HEADER));
        assert_eq!(decode(&bytes).unwrap(), clips);
    }

    #[test]
    fn decoding_rejects_malformed_and_oversized_files() {
        let bad: [&[u8]; 7] = [
            b"",
            b"Not repasta\n",
            b"Repasta history v1\n3\nab\n",
            b"Repasta history v1\n2\nabc\n",
            b"Repasta history v1\n0\n\n",
            b"Repasta history v1\n+2\nab\n",
            b"Repasta history v1\n2\n\xff\xfe\n",
        ];
        for bytes in bad {
            assert_eq!(decode(bytes), Err(StoreError::Unreadable), "{bytes:?}");
        }
        let oversized = format!("Repasta history v1\n{}\n", MAX_CLIP_BYTES + 1);
        assert_eq!(decode(oversized.as_bytes()), Err(StoreError::Unreadable));
        let too_many = format!("Repasta history v1\n{}", "1\nx\n".repeat(MAX_ITEMS + 1));
        assert_eq!(decode(too_many.as_bytes()), Err(StoreError::Unreadable));
    }

    #[test]
    fn folder_store_saves_converts_and_erases() {
        let dir = folder("convert");
        let mut readable = FolderStore::new(&dir, false, None);
        assert!(readable.load().unwrap().is_empty());
        readable.save(&history(&["beta", "alpha"])).unwrap();
        assert_eq!(readable.load().unwrap(), ["beta", "alpha"]);

        // Switching the folder to encrypted reads the readable file, then
        // replaces it with a sealed one.
        let mut sealed = FolderStore::new(&dir, true, Some(Box::new(Mirror)));
        assert!(sealed.has_other_form());
        let clips = sealed.load().unwrap();
        assert_eq!(clips, ["beta", "alpha"]);
        sealed.save(&history(&["beta", "alpha"])).unwrap();
        assert!(!dir.join(READABLE_FILE).exists());
        let raw = fs::read(dir.join(SEALED_FILE)).unwrap();
        assert!(raw.starts_with(SEALED_HEADER));
        assert!(!raw.windows(5).any(|window| window == b"alpha"));
        assert_eq!(sealed.load().unwrap(), ["beta", "alpha"]);

        // Without the key the sealed file is left alone.
        let mut keyless = FolderStore::new(&dir, false, None);
        assert_eq!(keyless.load(), Err(StoreError::Locked));
        assert!(FolderStore::has_sealed_history(&dir));
        assert_eq!(
            FolderStore::new(&dir, true, None).save(&History::default()),
            Err(StoreError::EncryptionUnavailable)
        );

        // With the key, the folder can switch back to readable.
        let mut readable = FolderStore::new(&dir, false, Some(Box::new(Mirror)));
        assert_eq!(readable.load().unwrap(), ["beta", "alpha"]);
        readable.save(&history(&["beta", "alpha"])).unwrap();
        assert!(!FolderStore::has_sealed_history(&dir));
        let mut sealed = FolderStore::new(&dir, true, Some(Box::new(Mirror)));
        sealed.load().unwrap();
        sealed.save(&history(&["beta", "alpha"])).unwrap();

        sealed.erase().unwrap();
        assert!(fs::read_dir(&dir).unwrap().next().is_none());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_folder_is_reported() {
        let dir = folder("missing");
        let mut store = FolderStore::new(dir.join("absent"), false, None);
        assert_eq!(store.load(), Err(StoreError::Io(io::ErrorKind::NotFound)));
        fs::remove_dir_all(&dir).unwrap();
    }
}
