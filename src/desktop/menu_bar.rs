//! macOS menu bar adapter: keeps Repasta reachable while its panel is hidden.
//!
//! A primary click on the menu bar icon shows or hides the panel. A secondary
//! click (right-click or Control-click) opens a menu with Show/Hide and Quit.

use std::cell::OnceCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSEventMask, NSEventModifierFlags, NSEventType,
    NSImage, NSMenu, NSMenuItem, NSNormalWindowLevel, NSStatusBar, NSStatusItem,
    NSVariableStatusItemLength,
};
use objc2_foundation::NSString;

#[derive(Default)]
struct Ivars {
    item: OnceCell<Retained<NSStatusItem>>,
    menu: OnceCell<Retained<NSMenu>>,
    toggle: OnceCell<Retained<NSMenuItem>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this class has no Drop impl.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RepastaMenuBarTarget"]
    #[ivars = Ivars]
    struct Target;

    impl Target {
        #[unsafe(method(clicked:))]
        fn clicked(&self, _sender: Option<&AnyObject>) {
            let app = NSApplication::sharedApplication(self.mtm());
            let secondary = app.currentEvent().is_some_and(|event| {
                event.r#type() == NSEventType::RightMouseUp
                    || event.modifierFlags().contains(NSEventModifierFlags::Control)
            });
            if secondary {
                self.open_menu();
            } else {
                toggle_app(self.mtm());
            }
        }

        #[unsafe(method(toggle:))]
        fn toggle(&self, _sender: Option<&AnyObject>) {
            toggle_app(self.mtm());
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }
    }
);

impl Target {
    fn open_menu(&self) {
        let ivars = self.ivars();
        let (Some(item), Some(menu), Some(toggle)) =
            (ivars.item.get(), ivars.menu.get(), ivars.toggle.get())
        else {
            return;
        };
        let title = if panel_in_front(self.mtm()) {
            "Hide Repasta"
        } else {
            "Show Repasta"
        };
        toggle.setTitle(&NSString::from_str(title));

        // Attach the menu only while it is open so primary clicks reach `clicked:`.
        item.setMenu(Some(menu));
        if let Some(button) = item.button(self.mtm()) {
            // SAFETY: Clicking a status item button with a menu attached opens that menu.
            unsafe { button.performClick(None) };
        }
        item.setMenu(None);
    }
}

/// The menu bar item. Dropping it removes the item from the menu bar.
pub struct MenuBar {
    // The target owns the status item; buttons and menu items reference it weakly.
    target: Retained<Target>,
    mtm: MainThreadMarker,
}

impl MenuBar {
    /// Adds the menu bar item and turns Repasta into a menu bar app.
    /// Returns `None` off the main thread, where AppKit is unavailable.
    pub fn install() -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let target = Target::alloc(mtm).set_ivars(Ivars::default());
        // SAFETY: NSObject's designated initializer takes no arguments.
        let target: Retained<Target> = unsafe { msg_send![super(target), init] };

        let toggle = menu_item(mtm, "Show Repasta", sel!(toggle:), "", &target);
        let menu = NSMenu::new(mtm);
        menu.addItem(&toggle);
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        menu.addItem(&menu_item(mtm, "Quit Repasta", sel!(quit:), "q", &target));

        let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        if let Some(button) = item.button(mtm) {
            // SAFETY: `clicked:` is implemented by `target`, which outlives the button.
            unsafe {
                button.setTarget(Some(&target));
                button.setAction(Some(sel!(clicked:)));
            }
            button.sendActionOn(NSEventMask::LeftMouseUp | NSEventMask::RightMouseUp);
        }

        let ivars = target.ivars();
        let _ = ivars.item.set(item);
        let _ = ivars.menu.set(menu);
        let _ = ivars.toggle.set(toggle);

        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        // Changing the policy can deactivate the app; bring the panel back.
        show_app(mtm);

        let menu_bar = Self { target, mtm };
        menu_bar.set_recording(false);
        Some(menu_bar)
    }

    /// Shows whether background capture is on in the menu bar icon.
    pub fn set_recording(&self, recording: bool) {
        let Some(button) = self
            .target
            .ivars()
            .item
            .get()
            .and_then(|item| item.button(self.mtm))
        else {
            return;
        };
        let (symbol, label) = if recording {
            ("doc.on.clipboard.fill", "Repasta: recording")
        } else {
            ("doc.on.clipboard", "Repasta: paused")
        };
        let label = NSString::from_str(label);
        let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str(symbol),
            Some(&label),
        );
        match image {
            Some(image) => {
                image.setTemplate(true);
                button.setImage(Some(&image));
            }
            None => button.setTitle(&NSString::from_str("Repasta")),
        }
        button.setToolTip(Some(&label));
    }

    /// Hides the panel; the menu bar item keeps the app reachable.
    pub fn hide_app(&self) {
        NSApplication::sharedApplication(self.mtm).hide(None);
    }
}

fn menu_item(
    mtm: MainThreadMarker,
    title: &str,
    action: Sel,
    key: &str,
    target: &Target,
) -> Retained<NSMenuItem> {
    // SAFETY: `action` names a method implemented by `target`, which outlives the item.
    unsafe {
        let item = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            Some(action),
            &NSString::from_str(key),
        );
        item.setTarget(Some(target));
        item
    }
}

/// Whether the panel is on screen where the user can see it: the app is
/// active, or the panel is pinned above other windows.
fn panel_in_front(mtm: MainThreadMarker) -> bool {
    let app = NSApplication::sharedApplication(mtm);
    !app.isHidden()
        && app.windows().iter().any(|window| {
            window.canBecomeMainWindow()
                && window.isVisible()
                && !window.isMiniaturized()
                && (app.isActive() || window.level() > NSNormalWindowLevel)
        })
}

fn toggle_app(mtm: MainThreadMarker) {
    if panel_in_front(mtm) {
        NSApplication::sharedApplication(mtm).hide(None);
    } else {
        show_app(mtm);
    }
}

fn show_app(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    app.unhide(None);
    #[allow(deprecated)] // `activate` requires macOS 14; this keeps macOS 12 and 13 working.
    app.activateIgnoringOtherApps(true);
    // Skip the menu bar's own windows; only the panel can become main.
    for window in app
        .windows()
        .iter()
        .filter(|window| window.canBecomeMainWindow())
    {
        if window.isMiniaturized() {
            window.deminiaturize(None);
        }
        window.makeKeyAndOrderFront(None);
    }
}
