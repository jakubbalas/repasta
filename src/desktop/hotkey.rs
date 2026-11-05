//! macOS global shortcut adapter. Registers a system hot key with the Carbon
//! Event Manager and reports each press. A registered hot key needs no
//! Accessibility or Input Monitoring permission, and Repasta sees only that
//! its own shortcut was pressed, not other keystrokes.

use std::ffi::c_void;
use std::sync::{Mutex, OnceLock, PoisonError};

use iced::futures::channel::mpsc;
use iced::futures::{Stream, StreamExt, stream};
use iced::keyboard::Modifiers;
use iced::keyboard::key::{Code, Physical};

type OSStatus = i32;
type EventTargetRef = *mut c_void;
type EventHandlerCallRef = *mut c_void;
type EventRef = *mut c_void;
type EventHandlerRef = *mut c_void;
type EventHotKeyRef = *mut c_void;
type EventHandler = extern "C" fn(EventHandlerCallRef, EventRef, *mut c_void) -> OSStatus;

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

#[repr(C)]
struct EventHotKeyId {
    signature: u32,
    id: u32,
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> EventTargetRef;
    fn InstallEventHandler(
        target: EventTargetRef,
        handler: EventHandler,
        type_count: u32,
        types: *const EventTypeSpec,
        user_data: *mut c_void,
        handler_ref: *mut EventHandlerRef,
    ) -> OSStatus;
    fn RegisterEventHotKey(
        key_code: u32,
        modifiers: u32,
        id: EventHotKeyId,
        target: EventTargetRef,
        options: u32,
        hot_key: *mut EventHotKeyRef,
    ) -> OSStatus;
    fn UnregisterEventHotKey(hot_key: EventHotKeyRef) -> OSStatus;
}

const NO_ERR: OSStatus = 0;
const HOT_KEY_EXISTS: OSStatus = -9878;
const EVENT_CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
const EVENT_HOT_KEY_PRESSED: u32 = 5;
const SIGNATURE: u32 = u32::from_be_bytes(*b"Rpst");

const COMMAND: u32 = 1 << 8;
const SHIFT: u32 = 1 << 9;
const OPTION: u32 = 1 << 11;
const CONTROL: u32 = 1 << 12;

/// Modifiers in the order macOS shows them, with their symbol and saved name.
const MODIFIERS: [(u32, &str, &str); 4] = [
    (CONTROL, "⌃", "control"),
    (OPTION, "⌥", "option"),
    (SHIFT, "⇧", "shift"),
    (COMMAND, "⌘", "command"),
];

/// Keys a shortcut can use: the physical key, its saved name, its label, and
/// its macOS virtual key code. Codes are positions on an ANSI keyboard, so
/// labels follow the US layout.
#[rustfmt::skip]
const KEYS: [(Code, &str, &str, u32); 69] = [
    (Code::KeyA, "a", "A", 0x00), (Code::KeyB, "b", "B", 0x0B),
    (Code::KeyC, "c", "C", 0x08), (Code::KeyD, "d", "D", 0x02),
    (Code::KeyE, "e", "E", 0x0E), (Code::KeyF, "f", "F", 0x03),
    (Code::KeyG, "g", "G", 0x05), (Code::KeyH, "h", "H", 0x04),
    (Code::KeyI, "i", "I", 0x22), (Code::KeyJ, "j", "J", 0x26),
    (Code::KeyK, "k", "K", 0x28), (Code::KeyL, "l", "L", 0x25),
    (Code::KeyM, "m", "M", 0x2E), (Code::KeyN, "n", "N", 0x2D),
    (Code::KeyO, "o", "O", 0x1F), (Code::KeyP, "p", "P", 0x23),
    (Code::KeyQ, "q", "Q", 0x0C), (Code::KeyR, "r", "R", 0x0F),
    (Code::KeyS, "s", "S", 0x01), (Code::KeyT, "t", "T", 0x11),
    (Code::KeyU, "u", "U", 0x20), (Code::KeyV, "v", "V", 0x09),
    (Code::KeyW, "w", "W", 0x0D), (Code::KeyX, "x", "X", 0x07),
    (Code::KeyY, "y", "Y", 0x10), (Code::KeyZ, "z", "Z", 0x06),
    (Code::Digit0, "0", "0", 0x1D), (Code::Digit1, "1", "1", 0x12),
    (Code::Digit2, "2", "2", 0x13), (Code::Digit3, "3", "3", 0x14),
    (Code::Digit4, "4", "4", 0x15), (Code::Digit5, "5", "5", 0x17),
    (Code::Digit6, "6", "6", 0x16), (Code::Digit7, "7", "7", 0x1A),
    (Code::Digit8, "8", "8", 0x1C), (Code::Digit9, "9", "9", 0x19),
    (Code::Minus, "minus", "-", 0x1B), (Code::Equal, "equal", "=", 0x18),
    (Code::BracketLeft, "left-bracket", "[", 0x21),
    (Code::BracketRight, "right-bracket", "]", 0x1E),
    (Code::Semicolon, "semicolon", ";", 0x29), (Code::Quote, "quote", "'", 0x27),
    (Code::Backslash, "backslash", "\\", 0x2A), (Code::Comma, "comma", ",", 0x2B),
    (Code::Period, "period", ".", 0x2F), (Code::Slash, "slash", "/", 0x2C),
    (Code::Backquote, "grave", "`", 0x32), (Code::Space, "space", "Space", 0x31),
    (Code::Enter, "return", "↩", 0x24), (Code::Tab, "tab", "⇥", 0x30),
    (Code::ArrowLeft, "left", "←", 0x7B), (Code::ArrowRight, "right", "→", 0x7C),
    (Code::ArrowDown, "down", "↓", 0x7D), (Code::ArrowUp, "up", "↑", 0x7E),
    (Code::F1, "f1", "F1", 0x7A), (Code::F2, "f2", "F2", 0x78),
    (Code::F3, "f3", "F3", 0x63), (Code::F4, "f4", "F4", 0x76),
    (Code::F5, "f5", "F5", 0x60), (Code::F6, "f6", "F6", 0x61),
    (Code::F7, "f7", "F7", 0x62), (Code::F8, "f8", "F8", 0x64),
    (Code::F9, "f9", "F9", 0x65), (Code::F10, "f10", "F10", 0x6D),
    (Code::F11, "f11", "F11", 0x67), (Code::F12, "f12", "F12", 0x6F),
    (Code::F13, "f13", "F13", 0x69), (Code::F14, "f14", "F14", 0x6B),
    (Code::F15, "f15", "F15", 0x71),
];

/// Function keys work without modifiers; other keys would type text.
fn is_function_key(code: Code) -> bool {
    matches!(
        code,
        Code::F1
            | Code::F2
            | Code::F3
            | Code::F4
            | Code::F5
            | Code::F6
            | Code::F7
            | Code::F8
            | Code::F9
            | Code::F10
            | Code::F11
            | Code::F12
            | Code::F13
            | Code::F14
            | Code::F15
    )
}

/// A key combination for the global shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    /// Index into [`KEYS`].
    key: usize,
    /// Carbon modifier bits.
    modifiers: u32,
}

impl Shortcut {
    /// ⌃⌘V: free in nearly every app.
    pub const CONTROL_COMMAND_V: Self = Self {
        key: 21,
        modifiers: CONTROL | COMMAND,
    };
    /// ⇧⌘V: many apps use it to paste and match style.
    pub const SHIFT_COMMAND_V: Self = Self {
        key: 21,
        modifiers: SHIFT | COMMAND,
    };
    /// The choices offered before recording one.
    pub const PRESETS: [Self; 2] = [Self::CONTROL_COMMAND_V, Self::SHIFT_COMMAND_V];

    /// The shortcut as macOS shows it, such as ⌃⌘V.
    pub fn label(self) -> String {
        let mut label: String = MODIFIERS
            .iter()
            .filter(|(bit, _, _)| self.modifiers & bit != 0)
            .map(|(_, symbol, _)| *symbol)
            .collect();
        label.push_str(KEYS[self.key].2);
        label
    }

    /// The saved form, such as `control-command-v`.
    pub fn name(self) -> String {
        let mut parts: Vec<&str> = MODIFIERS
            .iter()
            .filter(|(bit, _, _)| self.modifiers & bit != 0)
            .map(|(_, _, name)| *name)
            .collect();
        parts.push(KEYS[self.key].1);
        parts.join("-")
    }

    /// Reads [`Self::name`]'s form. Returns `None` for anything else.
    pub fn parse(name: &str) -> Option<Self> {
        let (modifier_names, key_name) = match name.rsplit_once('-') {
            Some((modifiers, key)) => (Some(modifiers), key),
            None => (None, name),
        };
        let key = KEYS.iter().position(|(_, name, _, _)| *name == key_name)?;
        let mut modifiers = 0;
        for part in modifier_names
            .into_iter()
            .flat_map(|names| names.split('-'))
        {
            let (bit, _, _) = MODIFIERS.iter().find(|(_, _, name)| *name == part)?;
            modifiers |= bit;
        }
        Self::new(key, modifiers)
    }

    /// The shortcut for a key press while recording one. Fails with a reason
    /// the panel can show.
    pub fn from_press(key: &Physical, modifiers: Modifiers) -> Result<Self, &'static str> {
        let Physical::Code(code) = key else {
            return Err("Repasta can't use that key. Try a letter, number, or function key.");
        };
        let Some(key) = KEYS.iter().position(|(known, _, _, _)| known == code) else {
            return Err("Repasta can't use that key. Try a letter, number, or function key.");
        };
        let modifiers = [
            (modifiers.control(), CONTROL),
            (modifiers.alt(), OPTION),
            (modifiers.shift(), SHIFT),
            (modifiers.logo(), COMMAND),
        ]
        .into_iter()
        .filter(|(held, _)| *held)
        .fold(0, |bits, (_, bit)| bits | bit);
        Self::new(key, modifiers).ok_or("Hold ⌘, ⌃, or ⌥ with the key.")
    }

    /// Keys other than function keys need ⌘, ⌃, or ⌥, or they would take
    /// over plain typing.
    fn new(key: usize, modifiers: u32) -> Option<Self> {
        (is_function_key(KEYS[key].0) || modifiers & (COMMAND | CONTROL | OPTION) != 0)
            .then_some(Self { key, modifiers })
    }
}

/// Where presses go; set by the subscription in [`presses`].
static PRESSES: Mutex<Option<mpsc::UnboundedSender<()>>> = Mutex::new(None);
/// The result of installing the press handler, which happens once.
static HANDLER: OnceLock<OSStatus> = OnceLock::new();

extern "C" fn pressed(
    _call: EventHandlerCallRef,
    _event: EventRef,
    _data: *mut c_void,
) -> OSStatus {
    // Repasta registers one hot key at a time, so any press is the shortcut.
    if let Some(sender) = PRESSES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
    {
        let _ = sender.unbounded_send(());
    }
    NO_ERR
}

/// Yields once per press of the registered shortcut.
pub fn presses() -> impl Stream<Item = ()> {
    let (sender, receiver) = mpsc::unbounded();
    *PRESSES.lock().unwrap_or_else(PoisonError::into_inner) = Some(sender);
    stream::unfold(receiver, |mut receiver| async move {
        receiver.next().await.map(|()| ((), receiver))
    })
}

#[derive(Debug)]
pub enum Error {
    /// Another app holds the shortcut.
    Taken,
    System(OSStatus),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Taken => f.write_str("Another app already uses this shortcut."),
            Self::System(status) => {
                write!(f, "macOS refused the shortcut (error {status}).")
            }
        }
    }
}

/// A registered shortcut. Dropping it unregisters the shortcut.
pub struct HotKey(EventHotKeyRef);

impl HotKey {
    /// Registers `shortcut`. Call on the main thread.
    pub fn register(shortcut: Shortcut) -> Result<Self, Error> {
        let status = *HANDLER.get_or_init(|| {
            let spec = EventTypeSpec {
                event_class: EVENT_CLASS_KEYBOARD,
                event_kind: EVENT_HOT_KEY_PRESSED,
            };
            // SAFETY: `pressed` matches the handler signature and lives for the
            // whole program; the spec is read during the call. The handler
            // stays installed for the life of the app.
            unsafe {
                InstallEventHandler(
                    GetApplicationEventTarget(),
                    pressed,
                    1,
                    &spec,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            }
        });
        if status != NO_ERR {
            return Err(Error::System(status));
        }

        let mut hot_key: EventHotKeyRef = std::ptr::null_mut();
        let id = EventHotKeyId {
            signature: SIGNATURE,
            id: 1,
        };
        // SAFETY: Plain values and a valid out pointer; the application event
        // target exists for the life of the app.
        let status = unsafe {
            RegisterEventHotKey(
                KEYS[shortcut.key].3,
                shortcut.modifiers,
                id,
                GetApplicationEventTarget(),
                0,
                &mut hot_key,
            )
        };
        match status {
            NO_ERR if !hot_key.is_null() => Ok(Self(hot_key)),
            HOT_KEY_EXISTS => Err(Error::Taken),
            status => Err(Error::System(status)),
        }
    }
}

impl Drop for HotKey {
    fn drop(&mut self) {
        // SAFETY: Registered in `register` and unregistered only here.
        unsafe { UnregisterEventHotKey(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::{KEYS, Shortcut};
    use iced::keyboard::Modifiers;
    use iced::keyboard::key::{Code, NativeCode, Physical};

    #[test]
    fn names_round_trip_and_read_older_settings() {
        for shortcut in Shortcut::PRESETS {
            assert_eq!(Shortcut::parse(&shortcut.name()), Some(shortcut));
        }
        assert_eq!(Shortcut::CONTROL_COMMAND_V.label(), "⌃⌘V");
        assert_eq!(
            Shortcut::parse("shift-command-v"),
            Some(Shortcut::SHIFT_COMMAND_V)
        );
        assert_eq!(
            Shortcut::parse("f5").map(Shortcut::label).as_deref(),
            Some("F5")
        );
        assert_eq!(Shortcut::parse("v"), None);
        assert_eq!(Shortcut::parse("hyper-v"), None);
        assert_eq!(Shortcut::parse("off"), None);
    }

    #[test]
    fn recording_needs_a_known_key_and_a_modifier() {
        let space = Physical::Code(Code::Space);
        let recorded =
            Shortcut::from_press(&space, Modifiers::ALT | Modifiers::LOGO).expect("valid");
        assert_eq!(recorded.label(), "⌥⌘Space");
        assert_eq!(recorded.name(), "option-command-space");
        assert!(Shortcut::from_press(&space, Modifiers::SHIFT).is_err());
        assert!(Shortcut::from_press(&Physical::Code(Code::F9), Modifiers::empty()).is_ok());
        let unknown = Physical::Unidentified(NativeCode::Unidentified);
        assert!(Shortcut::from_press(&unknown, Modifiers::LOGO).is_err());
    }

    #[test]
    fn keys_are_unique() {
        for (index, (code, name, _, key_code)) in KEYS.iter().enumerate() {
            assert!(
                KEYS[index + 1..]
                    .iter()
                    .all(|(other, other_name, _, other_key)| {
                        other != code && other_name != name && other_key != key_code
                    })
            );
        }
        assert_eq!(KEYS[Shortcut::CONTROL_COMMAND_V.key].0, Code::KeyV);
    }
}
