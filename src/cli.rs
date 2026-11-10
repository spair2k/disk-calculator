//! Консольный рендер отчёта и анимация анализа.
//!
//! Двухпанельная таблица: слева маркер типа (`D`/`F`/`L`) и имя, справа —
//! бар заполнения, размер и доля от общего объёма. В `legacy`-режиме
//! используется чистый ASCII. Разделитель панелей стоит в одном столбце
//! во всех строках, ширина считается по терминальным ячейкам (CJK/эмодзи
//! учитываются).

use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use crate::analyzer::{scan_directory, Node, ScanOptions, ScanResult, ScanStats};
use crate::sizing::{fmt_percent, fmt_size};
use crate::width::{display_width, pad_left, pad_right, sanitize, truncate};

/// Ширина бара заполнения.
pub const BAR_LEN: usize = 30;

const BAR_FULL: char = '█';
const BAR_EMPTY: char = '░';
const BAR_FULL_ASCII: char = '#';
const BAR_EMPTY_ASCII: char = '-';

const VERT_PSEUDO: char = '│';
const HLINE_PSEUDO: char = '─';
const JOINT_PSEUDO: char = '┼';
const VERT_ASCII: char = '|';
const HLINE_ASCII: char = '-';
const JOINT_ASCII: char = '+';

const LEGEND: &str = "Маркеры: D — папка, F — файл, L — ссылка";
const LEFT_TITLE: &str = "Содержимое (один уровень, по размеру)";
const RIGHT_TITLE: &str = "Заполнение относительно общего объёма";

/// Кадры «змейки» из символов Браиля: 3 точки ползут по периметру ячейки.
pub fn snake_frames() -> [char; 8] {
    const PERIMETER_DOTS: [u32; 8] = [1, 2, 3, 7, 8, 6, 5, 4];
    const SNAKE_LEN: usize = 3;
    let mut frames = [' '; 8];
    for (head, frame) in frames.iter_mut().enumerate() {
        let mut mask = 0u32;
        for offset in 0..SNAKE_LEN {
            let dot = PERIMETER_DOTS[(head + PERIMETER_DOTS.len() - offset) % PERIMETER_DOTS.len()];
            mask |= 1 << (dot - 1);
        }
        *frame = char::from_u32(0x2800 + mask).unwrap_or(' ');
    }
    frames
}

/// Один символ активности для кадра `step`.
pub fn activity_symbol(step: usize, legacy: bool) -> char {
    const SPINNER: [char; 4] = ['\\', '|', '/', '-'];
    if legacy {
        SPINNER[step % SPINNER.len()]
    } else {
        snake_frames()[step % 8]
    }
}

/// Полоса заполнения шириной `BAR_LEN`.
pub fn bar(ratio: f64, legacy: bool) -> String {
    let full_char = if legacy { BAR_FULL_ASCII } else { BAR_FULL };
    let empty_char = if legacy { BAR_EMPTY_ASCII } else { BAR_EMPTY };
    let full = (ratio.clamp(0.0, 1.0) * BAR_LEN as f64 + 0.5) as usize;
    let mut out = String::with_capacity(BAR_LEN);
    for _ in 0..full {
        out.push(full_char);
    }
    for _ in full..BAR_LEN {
        out.push(empty_char);
    }
    out
}

struct Row {
    label: String,
    bar: String,
    size: String,
    percent: String,
}

/// Собирает многострочный двухпанельный отчёт.
///
/// * `max_width` — предел ширины в ячейках (0 — без ограничения);
/// * `top` — вывести только первые N элементов.
pub fn render_table(
    nodes: &[Node],
    total_bytes: u64,
    legacy: bool,
    max_width: usize,
    top: Option<usize>,
) -> String {
    if nodes.is_empty() {
        return "  (папка пуста либо недоступна)".to_string();
    }

    let mut rows: Vec<Row> = nodes
        .iter()
        .map(|node| {
            let ratio = if total_bytes > 0 {
                node.size_bytes as f64 / total_bytes as f64
            } else {
                0.0
            };
            Row {
                label: format!("{}  {}", node.marker(), sanitize(&node.name)),
                bar: bar(ratio, legacy),
                size: fmt_size(node.size_bytes),
                percent: fmt_percent(ratio),
            }
        })
        .collect();

    let hidden = match top {
        Some(limit) if limit > 0 && rows.len() > limit => {
            let hidden = rows.len() - limit;
            rows.truncate(limit);
            hidden
        }
        _ => 0,
    };

    let size_width = rows
        .iter()
        .map(|row| display_width(&row.size))
        .max()
        .unwrap_or(0)
        .max(1);
    let percent_width = rows
        .iter()
        .map(|row| display_width(&row.percent))
        .max()
        .unwrap_or(0)
        .max(1);

    let right_cells: Vec<String> = rows
        .iter()
        .map(|row| {
            format!(
                "{} {} ({})",
                row.bar,
                pad_left(&row.size, size_width),
                pad_left(&row.percent, percent_width)
            )
        })
        .collect();

    let right_width = right_cells
        .iter()
        .map(|cell| display_width(cell))
        .chain(std::iter::once(display_width(RIGHT_TITLE)))
        .max()
        .unwrap_or(0);

    let natural_left = rows
        .iter()
        .map(|row| display_width(&row.label))
        .chain(std::iter::once(display_width(LEFT_TITLE)))
        .max()
        .unwrap_or(0);

    // Оставляем место под правую панель и разделитель «  │  ».
    let left_width = if max_width == 0 {
        natural_left
    } else {
        let budget = max_width.saturating_sub(right_width + 5);
        natural_left.min(budget)
    }
    .max(8.min(natural_left));

    let vert = if legacy { VERT_ASCII } else { VERT_PSEUDO };
    let hline = if legacy { HLINE_ASCII } else { HLINE_PSEUDO };
    let joint = if legacy { JOINT_ASCII } else { JOINT_PSEUDO };

    let mut lines = Vec::with_capacity(rows.len() + 4 + usize::from(hidden > 0));
    lines.push(LEGEND.to_string());

    let left_title = pad_right(&truncate(LEFT_TITLE, left_width), left_width);
    lines.push(format!("{left_title}  {vert}  {RIGHT_TITLE}"));
    lines.push(format!(
        "{}{joint}{}",
        hline.to_string().repeat(left_width + 2),
        hline.to_string().repeat(2 + right_width)
    ));

    for (row, right) in rows.iter().zip(right_cells.iter()) {
        let left = pad_right(&truncate(&row.label, left_width), left_width);
        lines.push(format!("{left}  {vert}  {right}"));
    }

    if hidden > 0 {
        lines.push(format!("… ещё {hidden} элем."));
    }

    lines.push(format!("Всего: {}", fmt_size(total_bytes)));
    lines.join("\n")
}

/// Строка статистики для `--stats`.
pub fn stats_line(stats: &ScanStats) -> String {
    format!(
        "файлов: {}, папок: {}, ошибок: {}, время: {} с",
        stats.files,
        stats.dirs,
        stats.errors,
        format!("{:.2}", stats.elapsed.as_secs_f64()).replace('.', ",")
    )
}

/// Сканирует каталог, показывая анимацию в первой строке (только для TTY).
pub fn scan_with_activity(root: &Path, options: &ScanOptions, legacy: bool) -> ScanResult {
    if !io::stdout().is_terminal() {
        return scan_directory(root, options);
    }

    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let animation = std::thread::spawn(move || {
        let mut out = io::stdout();
        let mut step = 0usize;
        loop {
            let symbol = activity_symbol(step, legacy);
            if out
                .write_all(format!("\r{symbol}").as_bytes())
                .and_then(|_| out.flush())
                .is_err()
            {
                break;
            }
            step += 1;
            match stop_rx.recv_timeout(Duration::from_millis(80)) {
                Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    });

    let result = scan_directory(root, options);

    // Останавливаем и дожидаемся потока, затем стираем кадр.
    let _ = stop_tx.send(());
    let _ = animation.join();
    let mut out = io::stdout();
    let _ = out.write_all(b"\r        \r");
    let _ = out.flush();

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn node(name: &str, is_dir: bool, size_bytes: u64) -> Node {
        Node {
            name: name.to_string(),
            path: PathBuf::from(format!("/tmp/{name}")),
            is_dir,
            size_bytes,
            mtime: 0.0,
            is_symlink: false,
            readable: true,
            metadata: None,
        }
    }

    fn divider_positions(table: &str, vert: char, joint: char) -> (usize, usize, Vec<usize>) {
        let lines: Vec<&str> = table.lines().collect();
        let position = |line: &str, ch: char| -> usize {
            let index = line.find(ch).expect("разделитель");
            display_width(&line[..index])
        };
        let header = position(lines[1], vert);
        let joint_position = position(lines[2], joint);
        let rows = lines[3..lines.len() - 1]
            .iter()
            .map(|line| position(line, vert))
            .collect();
        (header, joint_position, rows)
    }

    #[test]
    fn markers_and_legend() {
        let nodes = vec![node("docs", true, 300), node("a.txt", false, 100)];
        let table = render_table(&nodes, 400, false, 0, None);

        assert!(table.contains("Маркеры: D — папка, F — файл, L — ссылка"));
        assert!(table.contains("D  docs"));
        assert!(table.contains("F  a.txt"));
        assert!(table.contains("75,0%"));
        assert!(table.contains("25,0%"));
        assert!(table.contains("Всего: 400 Б"));
    }

    #[test]
    fn divider_is_aligned_pseudo_and_legacy() {
        let nodes = vec![node("docs", true, 300), node("a.txt", false, 100)];
        for (legacy, vert, joint) in [(false, '│', '┼'), (true, '|', '+')] {
            let table = render_table(&nodes, 400, legacy, 0, None);
            let (header, joint_position, rows) = divider_positions(&table, vert, joint);
            assert_eq!(header, joint_position);
            assert!(rows.iter().all(|position| *position == header));
        }
    }

    #[test]
    fn divider_stays_aligned_with_wide_and_long_names() {
        let nodes = vec![
            node("漢字漢字漢字漢字", false, 10),
            node(&"x".repeat(70), false, 5),
        ];
        let table = render_table(&nodes, 15, false, 0, None);
        let (header, joint_position, rows) = divider_positions(&table, '│', '┼');
        assert_eq!(header, joint_position);
        assert!(rows.iter().all(|position| *position == header));
    }

    #[test]
    fn legacy_has_no_pseudographics() {
        let nodes = vec![node("docs", true, 300)];
        let table = render_table(&nodes, 300, true, 0, None);
        for symbol in ['─', '│', '┼', '█', '░'] {
            assert!(!table.contains(symbol), "найден {symbol}");
        }
        assert!(table.contains('|'));
        assert!(table.contains('+'));
        assert!(table.contains('#'));
    }

    #[test]
    fn empty_directory_note() {
        let table = render_table(&[], 0, false, 0, None);
        assert!(table.contains("папка пуста"));
    }

    #[test]
    fn width_limit_truncates_names() {
        let nodes = vec![node(&"y".repeat(200), false, 1)];
        let table = render_table(&nodes, 1, false, 60, None);
        assert!(table.contains('…'));
        for line in table.lines() {
            assert!(
                display_width(line) <= 60,
                "строка шире лимита: {}",
                display_width(line)
            );
        }
    }

    #[test]
    fn top_limits_rows_and_reports_rest() {
        let nodes = vec![
            node("a", false, 3),
            node("b", false, 2),
            node("c", false, 1),
        ];
        let table = render_table(&nodes, 6, false, 0, Some(2));
        assert!(table.contains("… ещё 1 элем."));
        assert!(!table.contains("F  c"));
    }

    #[test]
    fn bar_width_and_rounding() {
        assert_eq!(display_width(&bar(0.0, false)), BAR_LEN);
        assert_eq!(display_width(&bar(1.0, false)), BAR_LEN);
        assert_eq!(bar(0.5, true), "#".repeat(15) + &"-".repeat(15));
        assert_eq!(bar(1.0, false), "█".repeat(BAR_LEN));
    }

    #[test]
    fn spinner_cycles() {
        let frames: Vec<char> = (0..8).map(|step| activity_symbol(step, true)).collect();
        assert_eq!(frames, vec!['\\', '|', '/', '-', '\\', '|', '/', '-']);
    }

    #[test]
    fn snake_frames_are_braille_with_three_dots() {
        let frames = snake_frames();
        assert_eq!(
            frames
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            8
        );
        for frame in frames {
            let code = frame as u32;
            assert!((0x2800..=0x28FF).contains(&code));
            assert_eq!((code - 0x2800).count_ones(), 3);
        }
    }

    #[test]
    fn stats_line_uses_comma() {
        let stats = ScanStats {
            files: 10,
            dirs: 2,
            errors: 0,
            elapsed: Duration::from_millis(1234),
        };
        let line = stats_line(&stats);
        assert!(line.contains("файлов: 10"));
        assert!(line.contains("время: 1,23 с"));
    }
}
