//! Files the system asks EduPython to open ("Open with", double-click).
//!
//! Windows and Linux pass them on the command line. macOS sends an Apple event
//! instead, at launch and also while EduPython is already running.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use eframe::egui::Context;

static QUEUE: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static CONTEXT: OnceLock<Context> = OnceLock::new();

/// Repaint this window when a file arrives.
pub fn set_context(ctx: &Context) {
    let _ = CONTEXT.set(ctx.clone());
}

pub fn push(files: impl IntoIterator<Item = PathBuf>) {
    QUEUE.lock().unwrap_or_else(|e| e.into_inner()).extend(files);
    if let Some(ctx) = CONTEXT.get() {
        ctx.request_repaint();
    }
}

pub fn take() -> Vec<PathBuf> {
    std::mem::take(&mut *QUEUE.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Call once on the main thread, before the event loop starts.
#[cfg(not(target_os = "macos"))]
pub fn install() {}

#[cfg(target_os = "macos")]
pub use mac::install;

#[cfg(target_os = "macos")]
mod mac {
    use std::path::PathBuf;

    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject};
    use objc2::{AnyThread, class, define_class, msg_send, sel};
    use objc2_foundation::{NSString, ns_string};

    const fn code(c: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*c)
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements; no Drop, no ivars.
        #[unsafe(super(NSObject))]
        #[name = "EduPythonOpenFiles"]
        struct Handler;

        impl Handler {
            /// AppKit has installed its own handlers by now, and the event
            /// with the files comes right after: replace the "open" handler.
            #[unsafe(method(willFinishLaunching:))]
            fn will_finish_launching(&self, _notification: &AnyObject) {
                // SAFETY: documented NSAppleEventManager API; the selector
                // below has the required (event, reply) signature.
                unsafe {
                    let manager: Retained<AnyObject> = msg_send![class!(NSAppleEventManager), sharedAppleEventManager];
                    let _: () = msg_send![
                        &*manager,
                        setEventHandler: self,
                        andSelector: sel!(openDocuments:reply:),
                        forEventClass: code(b"aevt"),
                        andEventID: code(b"odoc"),
                    ];
                }
            }

            #[unsafe(method(openDocuments:reply:))]
            fn open_documents(&self, event: &AnyObject, _reply: &AnyObject) {
                // SAFETY: `event` is an NSAppleEventDescriptor; all results are checked for nil.
                unsafe {
                    let list: Option<Retained<AnyObject>> = msg_send![event, paramDescriptorForKeyword: code(b"----")];
                    let Some(list) = list else { return };
                    let count: isize = msg_send![&*list, numberOfItems];
                    let mut files = Vec::new();
                    // Items count from 1; a single file may come without a list.
                    for i in 1..=count.max(1) {
                        let item: Option<Retained<AnyObject>> =
                            if count == 0 { Some(list.clone()) } else { msg_send![&*list, descriptorAtIndex: i] };
                        let Some(item) = item else { continue };
                        let url: Option<Retained<AnyObject>> = msg_send![&*item, fileURLValue];
                        let Some(url) = url else { continue };
                        let path: Option<Retained<NSString>> = msg_send![&*url, path];
                        files.extend(path.map(|p| PathBuf::from(p.to_string())));
                    }
                    super::push(files);
                }
            }
        }
    );

    pub fn install() {
        // SAFETY: plain NSObject init; addObserver does not retain the
        // observer, so it is kept alive for the whole program.
        unsafe {
            let handler: Retained<Handler> = msg_send![Handler::alloc(), init];
            let center: Retained<AnyObject> = msg_send![class!(NSNotificationCenter), defaultCenter];
            let _: () = msg_send![
                &*center,
                addObserver: &*handler,
                selector: sel!(willFinishLaunching:),
                name: ns_string!("NSApplicationWillFinishLaunchingNotification"),
                object: Option::<&AnyObject>::None,
            ];
            std::mem::forget(handler);
        }
    }
}
