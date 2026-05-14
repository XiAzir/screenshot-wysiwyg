#![windows_subsystem = "windows"]

mod app;
mod capture;
mod hotkey;
mod output;
mod overlay;
mod settings;

fn main() {
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }

    if let Err(err) = app::run() {
        app::show_error_message(&format!("{err:#}"));
    }
}
