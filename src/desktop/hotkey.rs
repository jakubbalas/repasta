//! macOS global shortcut adapter. Registers a system hot key with the Carbon
//! Event Manager and reports each press. A registered hot key needs no
//! Accessibility or Input Monitoring permission, and Repasta sees only that
//! its own shortcut was pressed, not other keystrokes.

use std::ffi::c_void;
use std::sync::{Mutex, OnceLock, PoisonError};

use iced::futures::channel::mpsc;
use iced::futures::{Stream, StreamExt, stream};

use crate::settings::Shortcut;

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

const KEY_V: u32 = 0x09;
const COMMAND: u32 = 1 << 8;
const SHIFT: u32 = 1 << 9;
const CONTROL: u32 = 1 << 12;

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
    /// Registers `shortcut`. Returns `Ok(None)` for [`Shortcut::Off`].
    /// Call on the main thread.
    pub fn register(shortcut: Shortcut) -> Result<Option<Self>, Error> {
        let modifiers = match shortcut {
            Shortcut::Off => return Ok(None),
            Shortcut::ShiftCommandV => COMMAND | SHIFT,
            Shortcut::ControlCommandV => COMMAND | CONTROL,
        };
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
                KEY_V,
                modifiers,
                id,
                GetApplicationEventTarget(),
                0,
                &mut hot_key,
            )
        };
        match status {
            NO_ERR if !hot_key.is_null() => Ok(Some(Self(hot_key))),
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
