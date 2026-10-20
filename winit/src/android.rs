//! Hand the Android activity to the event loop.
use std::sync::OnceLock;

pub use winit::platform::android::activity::AndroidApp;

static APP: OnceLock<AndroidApp> = OnceLock::new();

/// Stores the [`AndroidApp`] received by `android_main`.
///
/// Call this before running an application; later calls are ignored.
pub fn set_app(app: AndroidApp) {
    let _ = APP.set(app);
}

pub(crate) fn app() -> AndroidApp {
    APP.get()
        .cloned()
        .expect("call `iced::android::set_app` from `android_main` first")
}
