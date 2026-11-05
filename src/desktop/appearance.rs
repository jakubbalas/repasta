//! macOS appearance adapter: reads the system light/dark setting and accent
//! color, and reports when either changes.
//!
//! iced derives the window appearance from the app theme on macOS, so the
//! window cannot report system changes once Repasta picks a theme. This adapter
//! reads the user's setting directly instead.

use iced::Color;
use iced::futures::channel::mpsc;
use iced::futures::{Stream, StreamExt, stream};
use iced::theme::Mode;
use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{AnyThread, DefinedClass, define_class, msg_send, sel};
use objc2_app_kit::{NSColor, NSColorSpace, NSSystemColorsDidChangeNotification};
use objc2_foundation::{
    NSDistributedNotificationCenter, NSNotification, NSNotificationCenter,
    NSNotificationSuspensionBehavior, NSString, NSUserDefaults,
};

/// Posted by macOS when the user switches between Light and Dark.
const THEME_CHANGED: &str = "AppleInterfaceThemeChangedNotification";
/// Posted by macOS when the user changes the accent or highlight color.
const COLORS_CHANGED: &str = "AppleColorPreferencesChangedNotification";

/// The system appearance from System Settings → Appearance. With Auto, this is
/// the appearance currently in effect.
pub fn system_mode() -> Mode {
    let style = NSUserDefaults::standardUserDefaults()
        .stringForKey(&NSString::from_str("AppleInterfaceStyle"));
    // The key is set to "Dark" in Dark mode and absent in Light mode.
    if style.is_some_and(|style| style.to_string().eq_ignore_ascii_case("dark")) {
        Mode::Dark
    } else {
        Mode::Light
    }
}

/// The user's accent color from System Settings → Appearance, or `None` when
/// it is set to Multicolor, which lets each app use its own accent.
pub fn accent_color() -> Option<Color> {
    // Multicolor leaves this global default unset.
    NSUserDefaults::standardUserDefaults().objectForKey(&NSString::from_str("AppleAccentColor"))?;
    let accent =
        NSColor::controlAccentColor().colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    Some(Color::from_rgb(
        accent.redComponent() as f32,
        accent.greenComponent() as f32,
        accent.blueComponent() as f32,
    ))
}

/// Yields once each time macOS reports an appearance or color change. It does
/// no work between notifications.
pub fn changes() -> impl Stream<Item = ()> {
    let (sender, receiver) = mpsc::unbounded();
    let watch = Watch::register(sender);
    stream::unfold((watch, receiver), |(watch, mut receiver)| async move {
        receiver.next().await.map(|()| ((), (watch, receiver)))
    })
}

struct Ivars {
    changes: mpsc::UnboundedSender<()>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this class has no Drop impl.
    #[unsafe(super(NSObject))]
    #[name = "RepastaAppearanceObserver"]
    #[ivars = Ivars]
    struct Observer;

    impl Observer {
        #[unsafe(method(appearanceChanged:))]
        fn appearance_changed(&self, _notification: &NSNotification) {
            // A closed receiver means the subscription ended; nothing to do.
            let _ = self.ivars().changes.unbounded_send(());
        }
    }
);

// SAFETY: The only ivar is a thread-safe channel sender, and NSObject reference
// counting is thread-safe.
unsafe impl Send for Observer {}
// SAFETY: See above; `appearance_changed` only uses the thread-safe sender.
unsafe impl Sync for Observer {}

/// Keeps the observer registered; dropping it unregisters.
struct Watch(Retained<Observer>);

impl Watch {
    fn register(changes: mpsc::UnboundedSender<()>) -> Self {
        let observer = Observer::alloc().set_ivars(Ivars { changes });
        // SAFETY: NSObject's designated initializer takes no arguments.
        let observer: Retained<Observer> = unsafe { msg_send![super(observer), init] };
        let distributed = NSDistributedNotificationCenter::defaultCenter();
        // SAFETY: `observer` implements the selector, and `Drop` unregisters it
        // from both centers before it is deallocated. Both centers are thread-safe.
        unsafe {
            for name in [THEME_CHANGED, COLORS_CHANGED] {
                // Deliver while inactive too, so a pinned panel updates in place.
                distributed.addObserver_selector_name_object_suspensionBehavior(
                    &observer,
                    sel!(appearanceChanged:),
                    Some(&NSString::from_str(name)),
                    None,
                    NSNotificationSuspensionBehavior::DeliverImmediately,
                );
            }
            NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                &observer,
                sel!(appearanceChanged:),
                Some(NSSystemColorsDidChangeNotification),
                None,
            );
        }
        Self(observer)
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        // SAFETY: Removes the registrations made in `register`.
        unsafe {
            NSDistributedNotificationCenter::defaultCenter().removeObserver(&self.0);
            NSNotificationCenter::defaultCenter().removeObserver(&self.0);
        }
    }
}
