//! Минимальный JSON-парсер и сериализатор без внешних зависимостей.
//!
//! Нужен только для формата сессий: объекты, массивы, строки, числа, bool,
//! null. Парсер строгий к структуре, но не к порядку ключей; глубина
//! ограничена, чтобы битый файл не уронил стек.

use std::fmt;

const MAX_DEPTH: usize = 128;

/// Значение JSON.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    /// Ищет значение по ключу объекта (последнее вхождение выигрывает).
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(pairs) => pairs
                .iter()
                .rev()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Number(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Json::Number(value) if value.is_finite() && *value >= 0.0 => Some(*value as u64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Object(pairs) => Some(pairs),
            _ => None,
        }
    }
}

/// Удобный конструктор объекта: `obj(vec![("name", Json::String(...))])`.
pub fn obj(pairs: Vec<(&str, Json)>) -> Json {
    Json::Object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

/// Ошибка разбора JSON с позицией в байтах.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonError {
    pub pos: usize,
    pub message: String,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (позиция {})", self.message, self.pos)
    }
}

impl std::error::Error for JsonError {}

/// Разбирает JSON из строки.
pub fn parse(text: &str) -> Result<Json, JsonError> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        pos: 0,
    };
    parser.skip_whitespace();
    let value = parser.parse_value(0)?;
    parser.skip_whitespace();
    if parser.pos != parser.bytes.len() {
        return Err(parser.error("лишние данные после JSON"));
    }
    Ok(value)
}

/// Сериализует JSON с отступами в 2 пробела (аналог `json.dump(indent=2)`).
pub fn to_string_pretty(value: &Json) -> String {
    let mut out = String::new();
    write_value(&mut out, value, 0);
    out.push('\n');
    out
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn error(&self, message: impl Into<String>) -> JsonError {
        JsonError {
            pos: self.pos,
            message: message.into(),
        }
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.bytes.get(self.pos), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.pos += 1;
        }
    }

    fn parse_value(&mut self, depth: usize) -> Result<Json, JsonError> {
        if depth > MAX_DEPTH {
            return Err(self.error("слишком глубокая вложенность JSON"));
        }
        match self.bytes.get(self.pos).copied() {
            Some(b'{') => self.parse_object(depth),
            Some(b'[') => self.parse_array(depth),
            Some(b'"') => Ok(Json::String(self.parse_string()?)),
            Some(b't') => {
                self.expect_literal(b"true")?;
                Ok(Json::Bool(true))
            }
            Some(b'f') => {
                self.expect_literal(b"false")?;
                Ok(Json::Bool(false))
            }
            Some(b'n') => {
                self.expect_literal(b"null")?;
                Ok(Json::Null)
            }
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            Some(_) => Err(self.error("неожиданный символ")),
            None => Err(self.error("неожиданный конец JSON")),
        }
    }

    fn expect_literal(&mut self, literal: &[u8]) -> Result<(), JsonError> {
        if self.bytes[self.pos..].starts_with(literal) {
            self.pos += literal.len();
            Ok(())
        } else {
            Err(self.error("некорректный литерал"))
        }
    }

    fn parse_object(&mut self, depth: usize) -> Result<Json, JsonError> {
        self.pos += 1; // {
        self.skip_whitespace();
        let mut pairs = Vec::new();

        if self.bytes.get(self.pos) == Some(&b'}') {
            self.pos += 1;
            return Ok(Json::Object(pairs));
        }

        loop {
            self.skip_whitespace();
            let key = self.parse_string()?;
            self.skip_whitespace();
            if self.bytes.get(self.pos) != Some(&b':') {
                return Err(self.error("ожидался ':'"));
            }
            self.pos += 1;
            self.skip_whitespace();
            let value = self.parse_value(depth + 1)?;
            pairs.push((key, value));
            self.skip_whitespace();
            match self.bytes.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Object(pairs));
                }
                _ => return Err(self.error("ожидалась ',' или '}'")),
            }
        }
    }

    fn parse_array(&mut self, depth: usize) -> Result<Json, JsonError> {
        self.pos += 1; // [
        self.skip_whitespace();
        let mut items = Vec::new();

        if self.bytes.get(self.pos) == Some(&b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }

        loop {
            self.skip_whitespace();
            items.push(self.parse_value(depth + 1)?);
            self.skip_whitespace();
            match self.bytes.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(self.error("ожидалась ',' или ']'")),
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, JsonError> {
        if self.bytes.get(self.pos) != Some(&b'"') {
            return Err(self.error("ожидалась строка"));
        }
        self.pos += 1;
        let mut out = String::new();

        loop {
            let start = self.pos;
            while let Some(&byte) = self.bytes.get(self.pos) {
                if byte == b'"' || byte == b'\\' {
                    break;
                }
                if byte < 0x20 {
                    return Err(self.error("управляющий символ в строке"));
                }
                self.pos += 1;
            }

            if self.pos > start {
                let chunk = std::str::from_utf8(&self.bytes[start..self.pos])
                    .map_err(|_| self.error("некорректный UTF-8 в строке"))?;
                out.push_str(chunk);
            }

            match self.bytes.get(self.pos).copied() {
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    self.parse_escape(&mut out)?;
                }
                _ => return Err(self.error("незавершённая строка")),
            }
        }
    }

    fn parse_escape(&mut self, out: &mut String) -> Result<(), JsonError> {
        let byte = self
            .bytes
            .get(self.pos)
            .copied()
            .ok_or_else(|| self.error("незавершённая escape-последовательность"))?;
        self.pos += 1;

        match byte {
            b'"' => out.push('"'),
            b'\\' => out.push('\\'),
            b'/' => out.push('/'),
            b'b' => out.push('\u{0008}'),
            b'f' => out.push('\u{000C}'),
            b'n' => out.push('\n'),
            b'r' => out.push('\r'),
            b't' => out.push('\t'),
            b'u' => {
                let mut code = self.parse_hex4()?;
                if (0xD800..=0xDBFF).contains(&code) {
                    if self.bytes.get(self.pos) == Some(&b'\\')
                        && self.bytes.get(self.pos + 1) == Some(&b'u')
                    {
                        self.pos += 2;
                        let low = self.parse_hex4()?;
                        if !(0xDC00..=0xDFFF).contains(&low) {
                            return Err(self.error("некорректная суррогатная пара"));
                        }
                        code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                    } else {
                        return Err(self.error("незавершённая суррогатная пара"));
                    }
                } else if (0xDC00..=0xDFFF).contains(&code) {
                    return Err(self.error("некорректный суррогат"));
                }

                let ch =
                    char::from_u32(code).ok_or_else(|| self.error("некорректный код символа"))?;
                out.push(ch);
            }
            _ => return Err(self.error("неизвестная escape-последовательность")),
        }
        Ok(())
    }

    fn parse_hex4(&mut self) -> Result<u32, JsonError> {
        let end = self.pos + 4;
        if end > self.bytes.len() {
            return Err(self.error("незавершённый \\u"));
        }
        let mut value = 0u32;
        for &byte in &self.bytes[self.pos..end] {
            let digit = match byte {
                b'0'..=b'9' => (byte - b'0') as u32,
                b'a'..=b'f' => (byte - b'a' + 10) as u32,
                b'A'..=b'F' => (byte - b'A' + 10) as u32,
                _ => return Err(self.error("некорректная hex-цифра")),
            };
            value = value * 16 + digit;
        }
        self.pos = end;
        Ok(value)
    }

    fn parse_number(&mut self) -> Result<Json, JsonError> {
        let start = self.pos;
        while let Some(&byte) = self.bytes.get(self.pos) {
            if byte.is_ascii_digit() || matches!(byte, b'-' | b'+' | b'.' | b'e' | b'E') {
                self.pos += 1;
            } else {
                break;
            }
        }
        let raw = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| self.error("некорректное число"))?;
        match raw.parse::<f64>() {
            Ok(value) if value.is_finite() => Ok(Json::Number(value)),
            _ => Err(self.error("некорректное число")),
        }
    }
}

fn write_value(out: &mut String, value: &Json, level: usize) {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(true) => out.push_str("true"),
        Json::Bool(false) => out.push_str("false"),
        Json::Number(number) => out.push_str(&format_number(*number)),
        Json::String(text) => write_string(out, text),
        Json::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push_str("[\n");
            for (index, item) in items.iter().enumerate() {
                push_indent(out, level + 1);
                write_value(out, item, level + 1);
                if index + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            push_indent(out, level);
            out.push(']');
        }
        Json::Object(pairs) => {
            if pairs.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push_str("{\n");
            for (index, (key, item)) in pairs.iter().enumerate() {
                push_indent(out, level + 1);
                write_string(out, key);
                out.push_str(": ");
                write_value(out, item, level + 1);
                if index + 1 < pairs.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            push_indent(out, level);
            out.push('}');
        }
    }
}

fn push_indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str("  ");
    }
}

fn format_number(number: f64) -> String {
    if number == 0.0 {
        return "0".to_string();
    }
    if number.fract() == 0.0 && number.abs() < 9_007_199_254_740_992.0 {
        return format!("{}", number as i64);
    }
    let text = format!("{number}");
    if text.contains('.') || text.contains('e') || text.contains('E') {
        text
    } else {
        format!("{text}.0")
    }
}

fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_document() {
        let value = parse(r#"{"a":[1,2.5,true,false,null],"b":{"c":"текст"}}"#).unwrap();
        assert_eq!(value.get("a").unwrap().as_array().unwrap().len(), 5);
        assert_eq!(
            value.get("b").unwrap().get("c").unwrap().as_str(),
            Some("текст")
        );
        assert_eq!(
            value.get("a").unwrap().as_array().unwrap()[1].as_f64(),
            Some(2.5)
        );
    }

    #[test]
    fn parses_escapes_and_surrogates() {
        let value = parse(r#""a\nb\t\"c\\d\u0416\uD83D\uDE00""#).unwrap();
        assert_eq!(value.as_str(), Some("a\nb\t\"c\\dЖ😀"));
    }

    #[test]
    fn rejects_broken_json() {
        assert!(parse("{").is_err());
        assert!(parse(r#"{"a":}"#).is_err());
        assert!(parse("[1,]").is_err());
        assert!(parse("{} trailing").is_err());
        assert!(parse(r#""\uD800""#).is_err());
    }

    #[test]
    fn guards_against_deep_nesting() {
        let deep = format!("{}{}", "[".repeat(200), "]".repeat(200));
        assert!(parse(&deep).is_err());
    }

    #[test]
    fn pretty_print_roundtrip() {
        let text = r#"{"name":"тест","size":1536,"ratio":1.5,"ok":true,"none":null,"list":[1,2]}"#;
        let value = parse(text).unwrap();
        let printed = to_string_pretty(&value);
        assert_eq!(parse(&printed).unwrap(), value);
        assert!(printed.contains("\"size\": 1536"));
        assert!(printed.contains("\"ratio\": 1.5"));
        assert!(printed.contains("\"none\": null"));
    }

    #[test]
    fn numbers_without_fraction_print_as_integers() {
        assert_eq!(to_string_pretty(&Json::Number(3145728.0)), "3145728\n");
        assert_eq!(to_string_pretty(&Json::Number(0.25)), "0.25\n");
    }
}
