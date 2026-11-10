//! Сквозные тесты собранного бинарника `dc`.
//!
//! Проверяют реальный запуск процесса: справку, отчёт по каталогу,
//! экспорт сессии и её повторное открытие.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_dc")
}

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "dc-cli-test-{}-{}-{nanos}",
        std::process::id(),
        tag
    ));
    fs::create_dir_all(&path).expect("временный каталог");
    path
}

fn run(arguments: &[&str]) -> Output {
    Command::new(binary())
        .args(arguments)
        .output()
        .expect("запуск dc")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("UTF-8 в stdout")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("UTF-8 в stderr")
}

fn demo_tree() -> PathBuf {
    let root = temp_dir("tree");
    fs::create_dir_all(root.join("docs/sub")).unwrap();
    fs::write(root.join("docs/sub/deep.bin"), vec![b'x'; 200]).unwrap();
    fs::write(root.join("docs/top.bin"), vec![b'x'; 100]).unwrap();
    fs::write(root.join("a.txt"), vec![b'x'; 100]).unwrap();
    root
}

#[test]
fn help_and_version() {
    let help = run(&["--help"]);
    assert!(help.status.success());
    assert!(stdout(&help).contains("DC — Disk Calculator"));
    assert!(stdout(&help).contains("--legacy"));

    let version = run(&["--version"]);
    assert!(version.status.success());
    assert!(stdout(&version).starts_with("dc "));
}

#[test]
fn legacy_report_contains_markers_and_totals() {
    let root = demo_tree();
    let output = run(&["-l", "--no-animation", root.to_str().expect("путь в UTF-8")]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let text = stdout(&output);
    assert!(text.contains("Маркеры: D — папка, F — файл, L — ссылка"));
    assert!(text.contains("D  docs"));
    assert!(text.contains("F  a.txt"));
    assert!(text.contains("Всего: 400 Б"));
    assert!(text.contains("75,0%"));
    // legacy-режим не должен печатать псевдографику.
    for symbol in ['─', '│', '┼', '█', '░'] {
        assert!(!text.contains(symbol), "найден {symbol}");
    }

    fs::remove_dir_all(&root).ok();
}

#[test]
fn export_then_open_session() {
    let root = demo_tree();
    let storage = temp_dir("storage");

    let exported = run(&[
        "--no-animation",
        "--storage-dir",
        storage.to_str().expect("путь в UTF-8"),
        "-e",
        root.to_str().expect("путь в UTF-8"),
    ]);
    assert!(exported.status.success(), "stderr: {}", stderr(&exported));
    assert!(stdout(&exported).contains("Сессия сохранена:"));

    let session_file = fs::read_dir(&storage)
        .expect("storage")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .expect("JSON-сессия");

    let json = fs::read_to_string(&session_file).unwrap();
    assert!(json.contains("\"format\": \"dc-session\""));
    assert!(json.contains("\"version\": 1"));
    assert!(json.contains("\"total_bytes\": 400"));

    let reopened = run(&[session_file.to_str().expect("путь в UTF-8")]);
    assert!(reopened.status.success(), "stderr: {}", stderr(&reopened));
    let text = stdout(&reopened);
    assert!(text.contains("D  docs"));
    assert!(text.contains("F  a.txt"));
    assert!(text.contains("Сессия:"));

    fs::remove_dir_all(&root).ok();
    fs::remove_dir_all(&storage).ok();
}

#[test]
fn invalid_path_fails_with_message() {
    let missing = temp_dir("missing").join("nope");
    let output = run(&[missing.to_str().expect("путь в UTF-8")]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("Не удалось выполнить команду"));
}
