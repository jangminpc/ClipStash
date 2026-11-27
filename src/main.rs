#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod win;

#[cfg(windows)]
fn main() {
    win::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("ClipStash 는 Windows 전용 프로그램입니다. (Linux/macOS 에서는 `cargo test` 로 핵심 로직만 검사할 수 있습니다)");
    std::process::exit(1);
}
