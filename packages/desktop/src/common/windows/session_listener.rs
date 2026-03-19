use std::thread;

use tokio::sync::mpsc;
use tracing::{error, info, warn};
use windows::{
  core::w,
  Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::RemoteDesktop::{
      WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
      NOTIFY_FOR_THIS_SESSION,
    },
    UI::WindowsAndMessaging::{
      CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
      GetMessageW, PostMessageW, RegisterClassW, TranslateMessage,
      CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, MSG, PBT_APMRESUMEAUTOMATIC,
      PBT_APMRESUMESUSPEND, PBT_APMSUSPEND, WINDOW_EX_STYLE,
      WM_POWERBROADCAST, WM_QUIT, WM_WTSSESSION_CHANGE, WNDCLASSW,
      WS_OVERLAPPEDWINDOW, WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
    },
  },
};

/// Events emitted by the session listener.
#[derive(Clone, Debug)]
pub enum SessionEvent {
  /// The session was unlocked or the system resumed from sleep.
  Unlocked,
}

/// Listens for Windows session lock/unlock and sleep/resume events.
///
/// Creates a hidden message window on a background thread to receive
/// `WM_WTSSESSION_CHANGE` and `WM_POWERBROADCAST` messages.
pub struct SessionListener {
  event_rx: mpsc::UnboundedReceiver<SessionEvent>,
  window_handle: Option<isize>,
}

impl SessionListener {
  /// Creates a new `SessionListener`.
  ///
  /// Spawns a background thread with a hidden message window that
  /// listens for session and power state changes.
  pub fn new() -> anyhow::Result<Self> {
    let (event_tx, event_rx) = mpsc::unbounded_channel();

    // Channel for the background thread to send the HWND back.
    let (hwnd_tx, hwnd_rx) = std::sync::mpsc::channel();

    thread::Builder::new()
      .name("session-listener".into())
      .spawn(move || {
        if let Err(err) =
          Self::run_message_loop(event_tx, hwnd_tx)
        {
          error!("Session listener thread error: {:?}", err);
        }
      })?;

    let window_handle = hwnd_rx
      .recv_timeout(std::time::Duration::from_secs(5))
      .ok();

    Ok(Self {
      event_rx,
      window_handle,
    })
  }

  /// Returns the next session event.
  ///
  /// Returns `None` if the channel has been closed.
  pub async fn next_event(&mut self) -> Option<SessionEvent> {
    self.event_rx.recv().await
  }

  /// Drains any buffered session events without waiting.
  pub fn drain(&mut self) {
    while self.event_rx.try_recv().is_ok() {}
  }

  /// Runs the Win32 message loop on the current thread.
  fn run_message_loop(
    event_tx: mpsc::UnboundedSender<SessionEvent>,
    hwnd_tx: std::sync::mpsc::Sender<isize>,
  ) -> anyhow::Result<()> {
    let wnd_class = WNDCLASSW {
      lpszClassName: w!("ZebarSessionListener"),
      style: CS_HREDRAW | CS_VREDRAW,
      lpfnWndProc: Some(Self::window_proc),
      ..Default::default()
    };

    unsafe { RegisterClassW(&raw const wnd_class) };

    let hwnd = unsafe {
      CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("ZebarSessionListener"),
        w!("ZebarSessionListener"),
        WS_OVERLAPPEDWINDOW,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        None,
        None,
        wnd_class.hInstance,
        None,
      )
    };

    if hwnd.0 == 0 {
      anyhow::bail!("Failed to create session listener window.");
    }

    // Register for session change notifications.
    let reg_result = unsafe {
      WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION)
    };

    if let Err(err) = reg_result {
      warn!(
        "Failed to register for session notifications: {}",
        err
      );
    }

    // Send HWND back so the main thread can track it.
    let _ = hwnd_tx.send(hwnd.0);

    // Store the event sender in a thread-local so the window proc
    // can access it.
    EVENT_TX.set(event_tx);

    info!("Session listener started.");

    // Run the message loop.
    let mut msg = MSG::default();

    loop {
      if unsafe { GetMessageW(&raw mut msg, None, 0, 0) }.as_bool() {
        unsafe {
          TranslateMessage(&raw const msg);
          DispatchMessageW(&raw const msg);
        }
      } else {
        break;
      }
    }

    // Cleanup.
    let _ = unsafe { WTSUnRegisterSessionNotification(hwnd) };
    let _ = unsafe { DestroyWindow(hwnd) };

    info!("Session listener stopped.");
    Ok(())
  }

  /// Window procedure for the session listener.
  unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
  ) -> LRESULT {
    let event = match msg {
      WM_WTSSESSION_CHANGE => {
        #[allow(clippy::cast_possible_truncation)]
        match wparam.0 as u32 {
          WTS_SESSION_UNLOCK => Some(SessionEvent::Unlocked),
          // We only care about unlock events - on lock, Zebar
          // doesn't need to do anything since the widgets will
          // just be invisible behind the lock screen.
          WTS_SESSION_LOCK => {
            info!("Session locked.");
            None
          }
          _ => None,
        }
      }
      WM_POWERBROADCAST => {
        #[allow(clippy::cast_possible_truncation)]
        match wparam.0 as u32 {
          PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => {
            Some(SessionEvent::Unlocked)
          }
          PBT_APMSUSPEND => {
            info!("System suspending.");
            None
          }
          _ => None,
        }
      }
      _ => None,
    };

    if let Some(event) = event {
      EVENT_TX.with(|tx| {
        if let Some(tx) = tx.get() {
          let _ = tx.send(event);
        }
      });
    }

    DefWindowProcW(hwnd, msg, wparam, lparam)
  }
}

impl Drop for SessionListener {
  fn drop(&mut self) {
    // Post WM_QUIT to the message loop thread to shut it down.
    if let Some(hwnd) = self.window_handle {
      let _ = unsafe {
        PostMessageW(HWND(hwnd), WM_QUIT, WPARAM(0), LPARAM(0))
      };
    }
  }
}

thread_local! {
  static EVENT_TX: std::cell::OnceCell<mpsc::UnboundedSender<SessionEvent>> =
    std::cell::OnceCell::new();
}
