//! Форматирование размеров (Б, КБ, МБ, ГБ, ТБ).
//!
//! Используются двоичные единицы (1 КБ = 1024 Б) и запятая как десятичный
//! разделитель — как в исходном Python-проекте.

/// Порог перехода к следующей единице.
const KIB: f64 = 1024.0;

/// Префиксы единиц: байты и далее степени 1024.
const UNITS: [&str; 5] = ["", "К", "М", "Г", "Т"];

/// Размер в человекочитаемом виде.
///
/// Примеры: `123 -> "123 Б"`, `1536 -> "1,5 КБ"`, `3623878656 -> "3,4 ГБ"`.
pub fn fmt_size(size_bytes: u64) -> String {
    let mut value = size_bytes as f64;
    let mut unit = 0usize;

    while value >= KIB && unit + 1 < UNITS.len() {
        value /= KIB;
        unit += 1;
    }

    // Округление может перевести значение в следующую единицу:
    // 1048575 Б — это 1023,999 КБ, то есть «1,0 МБ», а не «1024 КБ».
    if unit + 1 < UNITS.len() && value.round() >= KIB {
        value /= KIB;
        unit += 1;
    }

    let number = if unit == 0 {
        format!("{}", value.round() as u64)
    } else if value < 9.95 {
        format!("{value:.1}").replace('.', ",")
    } else {
        format!("{}", value.round() as u64)
    };

    if unit == 0 {
        format!("{number} Б")
    } else {
        format!("{number} {}Б", UNITS[unit])
    }
}

/// Доля в процентах с запятой вместо точки: `0.75 -> "75,0%"`.
pub fn fmt_percent(ratio: f64) -> String {
    let value = format!("{:.1}", ratio.clamp(0.0, 1.0) * 100.0);
    format!("{}%", value.replace('.', ","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_and_units() {
        assert_eq!(fmt_size(0), "0 Б");
        assert_eq!(fmt_size(1), "1 Б");
        assert_eq!(fmt_size(512), "512 Б");
        assert_eq!(fmt_size(1023), "1023 Б");
        assert_eq!(fmt_size(1024), "1,0 КБ");
        assert_eq!(fmt_size(1536), "1,5 КБ");
        assert_eq!(fmt_size(10 * 1024), "10 КБ");
        assert_eq!(fmt_size(1024 * 1024), "1,0 МБ");
        assert_eq!(fmt_size(3 * 1024 * 1024 * 1024), "3,0 ГБ");
        assert_eq!(fmt_size(4 * 1024u64 * 1024 * 1024 * 1024), "4,0 ТБ");
    }

    #[test]
    fn boundary_rounding_does_not_print_1024() {
        // Раньше здесь получалось «1024 КБ» / «1024 ГБ».
        assert_eq!(fmt_size(1024 * 1024 - 1), "1,0 МБ");
        assert_eq!(fmt_size(1024u64 * 1024 * 1024 - 1), "1,0 ГБ");
        assert_eq!(fmt_size(1024u64 * 1024 * 1024 * 1024 - 1), "1,0 ТБ");
    }

    #[test]
    fn ten_and_more_have_no_decimals() {
        assert_eq!(fmt_size(10 * 1024 * 1024 - 1), "10 МБ");
        assert_eq!(fmt_size(200 * 1024 * 1024), "200 МБ");
    }

    #[test]
    fn percent_uses_comma() {
        assert_eq!(fmt_percent(0.75), "75,0%");
        assert_eq!(fmt_percent(1.0), "100,0%");
        assert_eq!(fmt_percent(0.0), "0,0%");
    }
}
