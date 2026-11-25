//! Routes quits that macOS starts (Dock, App Switcher, AppleScript, logout) through the same
//! confirmations as ⌘Q: gpui's app delegate would let them terminate the app at once.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::*;

/// Set right before an approved quit, so the delegate lets that `terminate:` through.
static APPROVED: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// Where an OS quit request is handed to the app (set once the app runs).
    static APP: RefCell<Option<(ForegroundExecutor, AsyncApp)>> = const { RefCell::new(None) };
}

/// Quits without asking again: the user confirmed, or there was nothing to confirm.
pub fn quit(cx: &mut App) {
    APPROVED.store(true, Ordering::SeqCst);
    cx.quit();
}

/// Lets OS quit requests reach `workspace::request_quit` from now on.
pub fn connect(cx: &App) {
    let target = (cx.foreground_executor().clone(), cx.to_async());
    APP.with(|app| *app.borrow_mut() = Some(target));
}

/// Answers the OS: true quits now; false cancels and asks the user first (the confirmed
/// path ends in [`quit`], which then lets the repeated request through).
fn allow_os_quit() -> bool {
    if APPROVED.load(Ordering::SeqCst) {
        return true;
    }
    APP.with(|app| {
        let Some((executor, app)) = app.borrow().clone() else {
            // Still launching: nothing could be unsaved yet.
            return true;
        };
        // Run from the event loop: the app state may be borrowed while AppKit asks.
        executor
            .spawn(async move { app.update(crate::workspace::request_quit) })
            .detach();
        false
    })
}

/// Adds `applicationShouldTerminate:` to gpui's app delegate. Call before the app runs:
/// AppKit looks up the delegate's optional methods when the delegate is set.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)] // Adding a method to an Objective-C class; see the SAFETY note.
pub fn install() {
    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::{ffi, sel};

    /// `NSApplicationTerminateReply`: `NSTerminateCancel` = 0, `NSTerminateNow` = 1.
    extern "C-unwind" fn should_terminate(_: &AnyObject, _: Sel, _: *mut AnyObject) -> usize {
        usize::from(allow_os_quit())
    }

    let Some(class) = AnyClass::get(c"GPUIApplicationDelegate") else {
        eprintln!("quit guard: gpui's app delegate class not found; Dock Quit will not ask");
        return;
    };
    let method: extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject) -> usize = should_terminate;
    // SAFETY: the function matches the selector's signature `- (NSUInteger)…:(id)sender`
    // (type encoding `Q@:@` on 64-bit macOS); `Imp` is the erased form `class_addMethod` expects,
    // and the runtime calls it back with exactly those arguments. The class is registered
    // (looked up by name) and lives for the whole process.
    let added = unsafe {
        let imp = std::mem::transmute::<extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject) -> usize, Imp>(
            method,
        );
        ffi::class_addMethod(
            std::ptr::from_ref(class).cast_mut(),
            sel!(applicationShouldTerminate:),
            imp,
            c"Q@:@".as_ptr(),
        )
    };
    if !added.as_bool() {
        eprintln!("quit guard: the app delegate already answers applicationShouldTerminate:");
    }
}

#[cfg(not(target_os = "macos"))]
pub fn install() {}
