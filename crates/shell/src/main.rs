#![cfg_attr(windows, windows_subsystem = "windows")]
#[cfg(any(windows, test))]
mod health;
#[cfg(windows)]
mod native;
fn main() {
    #[cfg(windows)]
    if let Err(e) = native::run() {
        native::error(&e.to_string());
    }
    #[cfg(not(windows))]
    eprintln!("The first KERN shell currently supports Windows only");
}
