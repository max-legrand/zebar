use cocoa::{
  appkit::{NSMainMenuWindowLevel, NSWindow},
  base::id,
};
use objc::{msg_send, sel, sel_impl};
use tauri::{Runtime, Window};

pub trait WindowExtMacOs {
  fn set_above_menu_bar(&self) -> anyhow::Result<()>;

  /// Forces the window onto the screen without activating the app.
  ///
  /// `WebviewWindowBuilder::visible(true)` ultimately calls
  /// `[NSWindow orderFront:]`, which is a no-op when the app is not
  /// active. Zebar runs as `ActivationPolicy::Accessory`, so windows
  /// created while another app has focus (e.g. when restoring widgets
  /// after a sleep/wake or screen unlock) never get placed on a
  /// visible space — the webview loads but nothing is drawn.
  ///
  /// `orderFrontRegardless` ignores the active-app requirement.
  fn show_regardless(&self) -> anyhow::Result<()>;
}

impl<R: Runtime> WindowExtMacOs for Window<R> {
  // AppKit window APIs must run on the main thread. These methods are
  // called from tokio tasks (widget startup, session-restore, etc.),
  // so we dispatch onto the main thread via Tauri.
  fn set_above_menu_bar(&self) -> anyhow::Result<()> {
    let window = self.clone();
    self.run_on_main_thread(move || {
      if let Ok(ns_win) = window.ns_window() {
        unsafe {
          (ns_win as id).setLevel_(NSMainMenuWindowLevel as i64 + 1);
        }
      }
    })?;
    Ok(())
  }

  fn show_regardless(&self) -> anyhow::Result<()> {
    let window = self.clone();
    self.run_on_main_thread(move || {
      if let Ok(ns_win) = window.ns_window() {
        unsafe {
          let _: () = msg_send![ns_win as id, orderFrontRegardless];
        }
      }
    })?;
    Ok(())
  }
}
