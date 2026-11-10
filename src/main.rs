//! Точка входа Disk Calculator (Rust).
//!
//! Только стандартная библиотека: ни одного внешнего крейта. Консоль —
//! единственный интерфейс. Анализируются метаданные (`read_dir` +
//! `symlink_metadata`), содержимое файлов не открывается.

mod analyzer;
mod app;
mod cli;
mod json;
mod platform;
mod session;
mod sizing;
mod width;

fn main() {
    configure_console();

    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let code = match app::run(args) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("Не удалось выполнить команду: {message}");
            1
        }
    };
    std::process::exit(code);
}

/// Переключает консоль Windows в UTF-8, чтобы кириллица и псевдографика
/// отображались независимо от активной кодовой страницы.
fn configure_console() {
    #[cfg(windows)]
    unsafe {
        let _ = SetConsoleOutputCP(65001);
        let _ = SetConsoleCP(65001);
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn SetConsoleOutputCP(code_page: u32) -> i32;
    fn SetConsoleCP(code_page: u32) -> i32;
}
