//! Small Windows-only helpers: message boxes and attaching to the parent console.

use windows_sys::Win32::UI::WindowsAndMessaging::{
    MB_ICONERROR, MB_SETFOREGROUND, MB_TOPMOST, MessageBoxW,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Blocks until dismissed.
pub fn error_box(title: &str, text: &str) {
    let (title, text) = (wide(title), wide(text));
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_ICONERROR | MB_SETFOREGROUND | MB_TOPMOST,
        );
    }
}

/// The release build has no console of its own (so Steam launches show no window). When run
/// from a terminal, attach to it so command-line output is visible.
pub fn attach_parent_console() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}
