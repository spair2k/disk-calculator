//! Ширина строк в терминале и безопасный вывод имён.
//!
//! Внешних крейтов нет, поэтому ширина считается приближённо: комбинирующие
//! и нулевые символы дают 0, широкие восточноазиатские и эмодзи — 2,
//! все остальные — 1. Этого достаточно для выравнивания таблицы, где имена
//! файлов могут быть любыми.

/// Ширина одного символа в терминальных ячейках.
pub fn char_width(c: char) -> usize {
    let cp = c as u32;
    if is_zero_width(cp) {
        return 0;
    }
    if is_wide(cp) {
        return 2;
    }
    1
}

/// Ширина строки в терминальных ячейках.
pub fn display_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

/// Обрезает строку до `width` ячеек, добавляя многоточие.
pub fn truncate(text: &str, width: usize) -> String {
    if display_width(text) <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    if width == 1 {
        return "…".to_string();
    }

    let mut out = String::new();
    let mut used = 0usize;
    for c in text.chars() {
        let w = char_width(c);
        if used + w > width - 1 {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// Дополняет строку пробелами справа до нужной ширины.
pub fn pad_right(text: &str, width: usize) -> String {
    let mut out = String::with_capacity(text.len() + width);
    out.push_str(text);
    for _ in display_width(text)..width {
        out.push(' ');
    }
    out
}

/// Дополняет строку пробелами слева до нужной ширины.
pub fn pad_left(text: &str, width: usize) -> String {
    let mut out = String::with_capacity(text.len() + width);
    for _ in display_width(text)..width {
        out.push(' ');
    }
    out.push_str(text);
    out
}

/// Убирает управляющие и bidi-символы, чтобы имя файла не могло
/// переписать/подменить содержимое терминала.
pub fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| {
            let cp = c as u32;
            let dangerous =
                c.is_control() || matches!(cp, 0x200E | 0x200F | 0x202A..=0x202E | 0x2066..=0x2069);
            if dangerous {
                '?'
            } else {
                c
            }
        })
        .collect()
}

/// Нулевая ширина: комбинирующие, variation selectors, форматные символы.
fn is_zero_width(cp: u32) -> bool {
    matches!(
        cp,
        0x0300..=0x036F
            | 0x0483..=0x0489
            | 0x0591..=0x05BD
            | 0x05BF
            | 0x05C1..=0x05C2
            | 0x05C4..=0x05C5
            | 0x05C7
            | 0x0610..=0x061A
            | 0x064B..=0x065F
            | 0x0670
            | 0x06D6..=0x06DC
            | 0x06DF..=0x06E4
            | 0x06E7..=0x06E8
            | 0x06EA..=0x06ED
            | 0x0711
            | 0x0730..=0x074A
            | 0x0E31
            | 0x0E34..=0x0E3A
            | 0x0E47..=0x0E4E
            | 0x1AB0..=0x1AFF
            | 0x1DC0..=0x1DFF
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x2064
            | 0x2066..=0x2069
            | 0x20D0..=0x20FF
            | 0xFE00..=0xFE0F
            | 0xFE20..=0xFE2F
            | 0xE0100..=0xE01EF
    )
}

/// Широкие символы (CJK, эмодзи), занимающие две ячейки.
fn is_wide(cp: u32) -> bool {
    matches!(
        cp,
        0x1100..=0x115F
            | 0x2E80..=0x303E
            | 0x3041..=0x33FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA000..=0xA4CF
            | 0xA960..=0xA97F
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE10..=0xFE19
            | 0xFE30..=0xFE6F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1F64F
            | 0x1F680..=0x1F6FF
            | 0x1F900..=0x1F9FF
            | 0x1FA70..=0x1FAFF
            | 0x20000..=0x2FFFD
            | 0x30000..=0x3FFFD
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_width() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width(""), 0);
    }

    #[test]
    fn cyrillic_width_is_one_per_char() {
        assert_eq!(display_width("папка"), 5);
    }

    #[test]
    fn wide_and_combining() {
        assert_eq!(display_width("漢字"), 4);
        assert_eq!(display_width("e\u{0301}"), 1);
    }

    #[test]
    fn truncation_respects_width() {
        let text = "abcdef";
        assert_eq!(truncate(text, 10), text);
        assert_eq!(truncate(text, 6), text);
        assert_eq!(truncate(text, 4), "abc…");
        assert_eq!(truncate("漢字漢字", 5), "漢字…");
        assert_eq!(truncate(text, 0), "");
        assert_eq!(truncate(text, 1), "…");
    }

    #[test]
    fn padding_uses_display_width() {
        assert_eq!(display_width(&pad_right("漢字", 6)), 6);
        assert_eq!(display_width(&pad_left("漢字", 6)), 6);
        assert_eq!(pad_right("ab", 4), "ab  ");
        assert_eq!(pad_left("ab", 4), "  ab");
    }

    #[test]
    fn sanitize_removes_control_and_bidi() {
        assert_eq!(sanitize("a\nb"), "a?b");
        assert_eq!(sanitize("a\u{202E}b"), "a?b");
        assert_eq!(sanitize("обычное имя"), "обычное имя");
    }
}
