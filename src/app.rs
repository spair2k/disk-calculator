//! Разбор аргументов командной строки и сценарий запуска.
//!
//! Ручной парсер вместо clap: проект должен собираться без внешних крейтов.

use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::analyzer::{scan_directory, ScanOptions};
use crate::cli::{render_table, scan_with_activity, stats_line};
use crate::session::{export_session, import_session, Session};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
DC — Disk Calculator: анализ заполнения папки одним консольным отчётом.

Использование:
  dc [параметры] [путь]

  Путь — папка для анализа или .json-файл сохранённой сессии.
  Без пути анализируется текущий каталог.

Параметры:
  -e, --export           сохранить результат в JSON-сессию (storage/)
  -l, --legacy           вывод без псевдографики (ASCII, бары #/-)
  -n, --top N            показать только первые N элементов
  -w, --width N          ограничить ширину отчёта (0 — без ограничения)
      --storage-dir DIR  каталог для экспортированных сессий
      --stats            добавить статистику сканирования
      --follow-links     разворачивать ссылки (с защитой от циклов)
      --no-animation     отключить анимацию анализа
  -h, --help             показать эту справку
  -V, --version          показать версию

Примеры:
  dc                     отчёт по текущему каталогу
  dc C:\\Users            отчёт по папке
  dc -e D:\\Data          отчёт + экспорт сессии
  dc -l --top 10 .       ASCII-отчёт, только 10 крупнейших
  dc report.json         открыть сохранённую сессию

Маркеры: D — папка, F — файл, L — ссылка.
Анализируются только метаданные: содержимое файлов не открывается.
";

#[derive(Debug, Default)]
struct Options {
    target: Option<PathBuf>,
    export: bool,
    legacy: bool,
    no_animation: bool,
    follow_links: bool,
    stats: bool,
    width: Option<usize>,
    top: Option<usize>,
    storage_dir: Option<PathBuf>,
}

enum Command {
    Help,
    Version,
    Run(Options),
}

/// Выполняет команду и возвращает код выхода.
pub fn run(args: Vec<OsString>) -> Result<i32, String> {
    match parse_args(args)? {
        Command::Help => {
            print_stdout(HELP)?;
            Ok(0)
        }
        Command::Version => {
            print_stdout(&format!("dc {VERSION}\n"))?;
            Ok(0)
        }
        Command::Run(options) => run_report(options),
    }
}

fn parse_args(args: Vec<OsString>) -> Result<Command, String> {
    let mut options = Options::default();
    let mut index = 0usize;
    let mut positional_only = false;

    while index < args.len() {
        let arg = &args[index];
        let text = arg.to_string_lossy();

        if !positional_only {
            match text.as_ref() {
                "-h" | "--help" => return Ok(Command::Help),
                "-V" | "--version" => return Ok(Command::Version),
                "-e" | "--export" => {
                    options.export = true;
                    index += 1;
                    continue;
                }
                "-l" | "--legacy" => {
                    options.legacy = true;
                    index += 1;
                    continue;
                }
                "--no-animation" => {
                    options.no_animation = true;
                    index += 1;
                    continue;
                }
                "--follow-links" => {
                    options.follow_links = true;
                    index += 1;
                    continue;
                }
                "--stats" => {
                    options.stats = true;
                    index += 1;
                    continue;
                }
                "--" => {
                    positional_only = true;
                    index += 1;
                    continue;
                }
                flag @ ("-w" | "--width" | "-n" | "--top" | "--storage-dir") => {
                    let value = args
                        .get(index + 1)
                        .ok_or_else(|| format!("Для {flag} нужно значение"))?;
                    match flag {
                        "-w" | "--width" => {
                            options.width = Some(parse_usize(value, flag)?);
                        }
                        "-n" | "--top" => {
                            let top = parse_usize(value, flag)?;
                            if top == 0 {
                                return Err("--top должен быть больше нуля".to_string());
                            }
                            options.top = Some(top);
                        }
                        _ => options.storage_dir = Some(PathBuf::from(value)),
                    }
                    index += 2;
                    continue;
                }
                _ => {}
            }

            if text.starts_with('-') && text.len() > 1 {
                return Err(format!("Неизвестный параметр: {text}. См. --help."));
            }
        }

        if options.target.is_some() {
            return Err("Указано больше одного пути".to_string());
        }
        options.target = Some(PathBuf::from(arg));
        index += 1;
    }

    Ok(Command::Run(options))
}

fn parse_usize(value: &OsString, flag: &str) -> Result<usize, String> {
    value
        .to_string_lossy()
        .trim()
        .parse::<usize>()
        .map_err(|_| format!("{flag}: ожидалось целое неотрицательное число"))
}

fn run_report(options: Options) -> Result<i32, String> {
    let target = match &options.target {
        Some(path) => absolutize(path)?,
        None => std::env::current_dir()
            .map_err(|error| format!("Не удалось определить текущий каталог: {error}"))?,
    };

    if is_session_path(&target) {
        let session = import_session(&target).map_err(|error| error.to_string())?;
        let width = report_width(options.width);
        let text = render_table(
            &session.nodes,
            session.total_bytes,
            options.legacy,
            width,
            options.top,
        );
        print_stdout(&format!("{text}\n"))?;
        print_stdout(&format!(
            "Сессия: {} ({}) — {}\n",
            session.name, session.os_name, session.created_at
        ))?;
        return Ok(0);
    }

    if !target.is_dir() {
        return Err(format!(
            "Путь не является папкой или .json сессией: {}",
            target.display()
        ));
    }

    let scan_options = ScanOptions {
        // Метаданные нужны только для экспорта — не тратим syscall'ы зря.
        collect_metadata: options.export,
        follow_links: options.follow_links,
    };

    let result = if options.no_animation {
        scan_directory(&target, &scan_options)
    } else {
        scan_with_activity(&target, &scan_options, options.legacy)
    };

    let total_bytes: u64 = result.nodes.iter().map(|node| node.size_bytes).sum();
    let width = report_width(options.width);
    let text = render_table(
        &result.nodes,
        total_bytes,
        options.legacy,
        width,
        options.top,
    );
    print_stdout(&format!("{text}\n"))?;

    if options.stats {
        print_stdout(&format!("{}\n", stats_line(&result.stats)))?;
    }

    if options.export {
        let session = Session::from_nodes(&target, &result.nodes);
        let path = export_session(&session, options.storage_dir.as_deref())
            .map_err(|error| format!("Не удалось сохранить сессию: {error}"))?;
        print_stdout(&format!("Сессия сохранена: {}\n", path.display()))?;
    }

    Ok(0)
}

fn is_session_path(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
}

fn absolutize(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let current = std::env::current_dir()
        .map_err(|error| format!("Не удалось определить текущий каталог: {error}"))?;
    Ok(current.join(path))
}

/// Ширина отчёта: явный флаг, иначе COLUMNS для TTY, иначе без ограничения.
fn report_width(flag: Option<usize>) -> usize {
    if let Some(width) = flag {
        return width;
    }
    if !io::stdout().is_terminal() {
        return 0;
    }
    std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(100)
        .max(40)
}

fn print_stdout(text: &str) -> Result<(), String> {
    let mut out = io::stdout().lock();
    match out.write_all(text.as_bytes()).and_then(|_| out.flush()) {
        Ok(()) => Ok(()),
        // Пользователь закрыл пайп (например, `dc | head`) — это не ошибка.
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(format!("Ошибка вывода: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_flags_and_target() {
        let Command::Run(options) = parse_args(args(&[
            "-e", "-l", "--top", "5", "--width", "80", "C:\\Data",
        ]))
        .unwrap() else {
            panic!("ожидалась команда Run");
        };
        assert!(options.export);
        assert!(options.legacy);
        assert_eq!(options.top, Some(5));
        assert_eq!(options.width, Some(80));
        assert_eq!(options.target, Some(PathBuf::from("C:\\Data")));
    }

    #[test]
    fn help_and_version_are_commands() {
        assert!(matches!(
            parse_args(args(&["--help"])).unwrap(),
            Command::Help
        ));
        assert!(matches!(
            parse_args(args(&["-V"])).unwrap(),
            Command::Version
        ));
    }

    #[test]
    fn rejects_unknown_flags_and_extra_paths() {
        assert!(parse_args(args(&["--nope"])).is_err());
        assert!(parse_args(args(&["a", "b"])).is_err());
        assert!(parse_args(args(&["--width"])).is_err());
        assert!(parse_args(args(&["--top", "0"])).is_err());
    }

    #[test]
    fn double_dash_allows_dash_prefixed_path() {
        let Command::Run(options) = parse_args(args(&["--", "-weird"])).unwrap() else {
            panic!("ожидалась команда Run");
        };
        assert_eq!(options.target, Some(PathBuf::from("-weird")));
    }

    #[test]
    fn session_extension_is_case_insensitive() {
        let path = std::env::temp_dir().join("dc-session-case-test.JSON");
        std::fs::write(&path, "{}").unwrap();
        assert!(is_session_path(&path));
        let _ = std::fs::remove_file(&path);
    }
}
