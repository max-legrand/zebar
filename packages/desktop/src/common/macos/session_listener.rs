use std::{ffi::CString, sync::Once};

use cocoa::base::{id, nil};
use objc::{
  class,
  declare::ClassDecl,
  msg_send,
  runtime::{Class, Object, Sel, NO},
  sel, sel_impl,
};
use tokio::sync::mpsc;
use tracing::{info, warn};

/// Events emitted by the session listener.
#[derive(Clone, Debug)]
pub enum SessionEvent {
  /// The session was unlocked or the system resumed from sleep.
  Unlocked,
}

/// Listens for macOS session unlock and system wake events.
///
/// Observes notifications from both the workspace notification center
/// (`NSWorkspace`) and the distributed notification center
/// (`NSDistributedNotificationCenter`) to cover all unlock scenarios:
///
/// - `com.apple.screenIsUnlocked` — screen lock dismissed (most
///   reliable for lock/unlock cycles)
/// - `NSWorkspaceDidWakeNotification` — system woke from full sleep
/// - `NSWorkspaceScreensDidWakeNotification` — displays woke from
///   sleep
/// - `NSWorkspaceSessionDidBecomeActiveNotification` — fast user
///   switch back to this session
///
/// The observer is registered on the main thread so that
/// notifications are delivered through the application's main run
/// loop.
pub struct SessionListener {
  event_rx: mpsc::UnboundedReceiver<SessionEvent>,
}

impl SessionListener {
  /// Creates a new `SessionListener`.
  pub fn new() -> anyhow::Result<Self> {
    let (event_tx, event_rx) = mpsc::unbounded_channel();

    // Store the sender globally so the ObjC callback can access it.
    // This is safe because we only create one SessionListener.
    SENDER.call_once(|| unsafe {
      SENDER_PTR =
        Box::into_raw(Box::new(event_tx)) as *mut std::ffi::c_void;
    });

    // Register the ObjC observer class.
    let _ = Self::register_observer_class();

    unsafe {
      // Create the observer instance.
      let observer: id =
        msg_send![class!(ZebarSessionObserver), alloc];
      let observer: id = msg_send![observer, init];

      // Register notifications on the main thread so they're
      // delivered via the main run loop.
      let _: () = msg_send![
        observer,
        performSelectorOnMainThread: sel!(registerNotifications)
        withObject: nil
        waitUntilDone: NO
      ];

      // Keep the observer alive for the lifetime of the process.
      std::mem::forget(observer);
    }

    info!("Session listener created.");
    Ok(Self { event_rx })
  }

  /// Returns the next session event.
  pub async fn next_event(&mut self) -> Option<SessionEvent> {
    self.event_rx.recv().await
  }

  /// Drains any buffered session events without waiting.
  pub fn drain(&mut self) {
    while self.event_rx.try_recv().is_ok() {}
  }

  /// Registers the custom ObjC class for receiving notifications.
  fn register_observer_class() -> &'static Class {
    static REGISTER: Once = Once::new();
    static mut CLASS: Option<&'static Class> = None;

    REGISTER.call_once(|| {
      let superclass = class!(NSObject);
      let mut decl =
        ClassDecl::new("ZebarSessionObserver", superclass).unwrap();

      unsafe {
        decl.add_method(
          sel!(registerNotifications),
          register_notifications as extern "C" fn(&Object, Sel),
        );
        decl.add_method(
          sel!(onSessionEvent:),
          on_session_event as extern "C" fn(&Object, Sel, id),
        );
      }

      unsafe { CLASS = Some(decl.register()) }
    });

    unsafe { CLASS.unwrap() }
  }
}

static SENDER: Once = Once::new();
static mut SENDER_PTR: *mut std::ffi::c_void = std::ptr::null_mut();

/// Helper to create an `NSString` from a Rust string literal.
unsafe fn nsstring(s: &str) -> id {
  let c = CString::new(s).expect("null byte in string");
  msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()]
}

/// Called on the main thread to register notification observers.
extern "C" fn register_notifications(this: &Object, _sel: Sel) {
  unsafe {
    let sel = sel!(onSessionEvent:);

    // --- Workspace notification center ---
    // These cover system sleep/wake and fast user switching.
    let workspace: id =
      msg_send![class!(NSWorkspace), sharedWorkspace];
    let workspace_nc: id =
      msg_send![workspace, notificationCenter];

    for name in [
      "NSWorkspaceDidWakeNotification",
      "NSWorkspaceScreensDidWakeNotification",
      "NSWorkspaceSessionDidBecomeActiveNotification",
    ] {
      let ns_name = nsstring(name);
      let _: () = msg_send![
        workspace_nc,
        addObserver: this
        selector: sel
        name: ns_name
        object: workspace
      ];
    }

    // --- Distributed notification center ---
    // `com.apple.screenIsUnlocked` is the most reliable notification
    // for detecting when the user dismisses the lock screen. It fires
    // for both password-protected and non-password display sleep
    // unlock, which the workspace notifications above may miss.
    let dist_nc: id = msg_send![
      class!(NSDistributedNotificationCenter),
      defaultCenter
    ];

    let unlock_name = nsstring("com.apple.screenIsUnlocked");
    let _: () = msg_send![
      dist_nc,
      addObserver: this
      selector: sel
      name: unlock_name
      object: nil
    ];

    info!("Session listener registered on main thread.");
  }
}

/// Called when a session notification fires (on the main thread).
extern "C" fn on_session_event(_this: &Object, _sel: Sel, notif: id) {
  unsafe {
    let name: id = msg_send![notif, name];
    let name_str: *const std::ffi::c_char = msg_send![name, UTF8String];
    let name_str = if !name_str.is_null() {
      std::ffi::CStr::from_ptr(name_str)
        .to_str()
        .unwrap_or("unknown")
    } else {
      "unknown"
    };

    info!("Session notification received: {}", name_str);

    if !SENDER_PTR.is_null() {
      let tx =
        &*(SENDER_PTR as *const mpsc::UnboundedSender<SessionEvent>);
      if let Err(err) = tx.send(SessionEvent::Unlocked) {
        warn!("Failed to send session event: {}", err);
      }
    }
  }
}
