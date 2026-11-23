//! Native OS folder picker and message boxes backed by SDL3.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sdl3::dialog::{DialogError, show_open_folder_dialog};
use sdl3::messagebox::{MessageBoxFlag, show_simple_message_box};

/// Opens a native OS folder picker dialog and waits for the user to choose a
/// folder or cancel.
///
/// Returns `Ok(Some(path))` if a folder was selected, `Ok(None)` if the user
/// canceled, or `Err(message)` if SDL failed to open the dialog.
pub fn pick_folder(default_location: Option<&Path>) -> Result<Option<PathBuf>, String> {
    let sdl = sdl3::init().map_err(|error| format!("SDL3 init failed: {error}"))?;
    let _video = sdl
        .video()
        .map_err(|error| format!("SDL3 video init failed: {error}"))?;
    let mut events = sdl
        .event_pump()
        .map_err(|error| format!("SDL3 event pump failed: {error}"))?;

    let result: Arc<Mutex<Option<Result<Option<PathBuf>, String>>>> = Arc::new(Mutex::new(None));
    let slot = Arc::clone(&result);

    show_open_folder_dialog(
        default_location,
        false,
        None,
        Box::new(move |outcome, _filter| {
            let mapped = match outcome {
                Ok(paths) => Ok(paths.into_iter().next()),
                Err(DialogError::Canceled) => Ok(None),
                Err(error) => Err(error.to_string()),
            };
            if let Ok(mut guard) = slot.lock() {
                *guard = Some(mapped);
            }
        }),
    );

    loop {
        events.pump_events();
        if let Ok(mut guard) = result.lock()
            && let Some(outcome) = guard.take()
        {
            return outcome;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
}

/// Shows a native informational message box (best-effort; ignores errors in
/// headless environments).
pub fn show_info(title: &str, message: &str) {
    let _ = show_simple_message_box(MessageBoxFlag::INFORMATION, title, message, None);
}

/// Shows a native error message box (best-effort; ignores errors in headless
/// environments).
pub fn show_error(title: &str, message: &str) {
    let _ = show_simple_message_box(MessageBoxFlag::ERROR, title, message, None);
}
