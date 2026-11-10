//! Импорт и экспорт сессии в JSON.
//!
//! Формат совместим с Python-версией (`format: "dc-session"`), но парсер
//! собственный, без serde. Импорт валидирует структуру, версию и типы;
//! пути из файла никогда не используются для доступа к диску. Экспорт
//! атомарный: сначала временный файл, затем `rename`.

use std::collections::hash_map::RandomState;
use std::fmt;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::analyzer::Node;
use crate::json::{self, obj, Json};
use crate::platform::MetaInfo;

pub const SESSION_VERSION: u64 = 1;
pub const DEFAULT_STORAGE_DIR: &str = "storage";

/// Предохранитель от чтения гигантских/битых файлов.
const MAX_SESSION_BYTES: u64 = 64 * 1024 * 1024;

/// Снимок результатов анализа.
#[derive(Debug, Clone)]
pub struct Session {
    pub name: String,
    pub root_dir: String,
    pub os_name: String,
    pub created_at: String,
    pub total_bytes: u64,
    pub nodes: Vec<Node>,
    pub meta: Json,
}

impl Session {
    /// Строит сессию из результатов сканирования.
    pub fn from_nodes(root: &Path, nodes: &[Node]) -> Session {
        let total_bytes = nodes.iter().map(|node| node.size_bytes).sum();
        let name = root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| root.to_string_lossy().into_owned());

        Session {
            name,
            root_dir: root.to_string_lossy().into_owned(),
            os_name: crate::platform::current_os().to_string(),
            created_at: utc_now_iso(),
            total_bytes,
            nodes: nodes.to_vec(),
            meta: Json::Object(Vec::new()),
        }
    }

    /// Преобразует сессию в JSON-значение (схема Python-версии).
    pub fn to_json(&self) -> Json {
        let nodes = self.nodes.iter().map(node_to_json).collect();
        obj(vec![
            ("format", Json::String("dc-session".to_string())),
            ("version", Json::Number(SESSION_VERSION as f64)),
            ("name", Json::String(self.name.clone())),
            ("os", Json::String(self.os_name.clone())),
            ("root", Json::String(self.root_dir.clone())),
            ("created_at", Json::String(self.created_at.clone())),
            ("total_bytes", Json::Number(self.total_bytes as f64)),
            ("count", Json::Number(self.nodes.len() as f64)),
            ("meta", self.meta.clone()),
            ("nodes", Json::Array(nodes)),
        ])
    }

    /// Разбирает и валидирует сессию из JSON-значения.
    pub fn from_json(value: &Json) -> Result<Session, SessionError> {
        let format = value
            .get("format")
            .and_then(Json::as_str)
            .unwrap_or_default();
        if format != "dc-session" {
            return Err(SessionError::Invalid(
                "Файл не является сессией Disk Calculator (format != dc-session)".to_string(),
            ));
        }

        let version = value
            .get("version")
            .and_then(Json::as_u64)
            .unwrap_or(SESSION_VERSION);
        if version > SESSION_VERSION {
            return Err(SessionError::Invalid(format!(
                "Сессия версии {version} новее поддерживаемой ({SESSION_VERSION})"
            )));
        }

        let nodes_value = value.get("nodes").ok_or_else(|| {
            SessionError::Invalid("В сессии отсутствуют данные (nodes)".to_string())
        })?;
        let nodes_array = nodes_value
            .as_array()
            .ok_or_else(|| SessionError::Invalid("Поле nodes должно быть списком".to_string()))?;

        let mut nodes = Vec::with_capacity(nodes_array.len());
        for (index, item) in nodes_array.iter().enumerate() {
            nodes.push(
                node_from_json(item).map_err(|message| {
                    SessionError::Invalid(format!("nodes[{index}]: {message}"))
                })?,
            );
        }

        let computed_total: u64 = nodes.iter().map(|node| node.size_bytes).sum();
        let total_bytes = value
            .get("total_bytes")
            .and_then(Json::as_u64)
            .unwrap_or(computed_total);

        Ok(Session {
            name: value
                .get("name")
                .and_then(Json::as_str)
                .unwrap_or("session")
                .to_string(),
            root_dir: value
                .get("root")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_string(),
            os_name: value
                .get("os")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_string(),
            created_at: value
                .get("created_at")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_string(),
            total_bytes,
            nodes,
            meta: value
                .get("meta")
                .cloned()
                .unwrap_or_else(|| Json::Object(Vec::new())),
        })
    }
}

/// Ошибка импорта/экспорта сессии.
#[derive(Debug)]
pub enum SessionError {
    Io(io::Error),
    Json(json::JsonError),
    Invalid(String),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::Io(error) => write!(f, "{error}"),
            SessionError::Json(error) => write!(f, "Некорректный JSON сессии: {error}"),
            SessionError::Invalid(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<io::Error> for SessionError {
    fn from(error: io::Error) -> Self {
        SessionError::Io(error)
    }
}

impl From<json::JsonError> for SessionError {
    fn from(error: json::JsonError) -> Self {
        SessionError::Json(error)
    }
}

/// Загружает сессию из файла.
pub fn import_session(path: &Path) -> Result<Session, SessionError> {
    let meta = fs::metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            SessionError::Invalid(format!("Файл сессии не найден: {}", path.display()))
        } else {
            SessionError::Io(error)
        }
    })?;

    if meta.len() > MAX_SESSION_BYTES {
        return Err(SessionError::Invalid(format!(
            "Файл сессии слишком большой: {} байт (лимит {MAX_SESSION_BYTES})",
            meta.len()
        )));
    }

    let text = fs::read_to_string(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            SessionError::Invalid(format!("Файл сессии не найден: {}", path.display()))
        } else {
            SessionError::Io(error)
        }
    })?;

    let value = json::parse(&text)?;
    Session::from_json(&value)
}

/// Записывает сессию в JSON и возвращает путь к файлу.
///
/// `target_dir` — каталог назначения; по умолчанию `storage/` текущего
/// каталога. Запись атомарная (временный файл + `rename`).
pub fn export_session(session: &Session, target_dir: Option<&Path>) -> io::Result<PathBuf> {
    let directory = target_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_STORAGE_DIR));
    fs::create_dir_all(&directory)?;

    let path = directory.join(format!(
        "dc-session-{}-{}.json",
        utc_stamp_compact(),
        random_hex6()
    ));
    let temp = directory.join(format!(".dc-session-{}.tmp", random_hex6()));
    let text = json::to_string_pretty(&session.to_json());

    fs::write(&temp, text.as_bytes())?;
    if let Err(error) = fs::rename(&temp, &path) {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    Ok(path)
}

fn node_to_json(node: &Node) -> Json {
    let metadata = match &node.metadata {
        Some(MetaInfo::Posix { mode, owner, group }) => obj(vec![
            ("os", Json::String("posix".to_string())),
            ("mode", Json::String(mode.clone())),
            ("owner", Json::String(owner.clone())),
            ("group", Json::String(group.clone())),
        ]),
        Some(MetaInfo::Windows { attributes }) => obj(vec![
            ("os", Json::String("nt".to_string())),
            (
                "attributes",
                Json::Array(
                    attributes
                        .iter()
                        .map(|attribute| Json::String(attribute.clone()))
                        .collect(),
                ),
            ),
        ]),
        None => Json::Object(Vec::new()),
    };

    obj(vec![
        ("name", Json::String(node.name.clone())),
        (
            "path",
            Json::String(node.path.to_string_lossy().into_owned()),
        ),
        ("is_dir", Json::Bool(node.is_dir)),
        ("size_bytes", Json::Number(node.size_bytes as f64)),
        ("size_human", Json::String(node.size_human())),
        ("mtime", Json::Number(node.mtime)),
        ("is_symlink", Json::Bool(node.is_symlink)),
        ("readable", Json::Bool(node.readable)),
        ("metadata", metadata),
    ])
}

fn node_from_json(value: &Json) -> Result<Node, String> {
    let name = value
        .get("name")
        .and_then(Json::as_str)
        .ok_or("отсутствует поле name")?
        .to_string();

    let size_bytes = value
        .get("size_bytes")
        .and_then(Json::as_u64)
        .ok_or("поле size_bytes должно быть неотрицательным числом")?;

    let path = value
        .get("path")
        .and_then(Json::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&name));

    Ok(Node {
        name,
        path,
        is_dir: value.get("is_dir").and_then(Json::as_bool).unwrap_or(false),
        size_bytes,
        mtime: value.get("mtime").and_then(Json::as_f64).unwrap_or(0.0),
        is_symlink: value
            .get("is_symlink")
            .and_then(Json::as_bool)
            .unwrap_or(false),
        readable: value
            .get("readable")
            .and_then(Json::as_bool)
            .unwrap_or(true),
        metadata: value.get("metadata").and_then(metadata_from_json),
    })
}

fn metadata_from_json(value: &Json) -> Option<MetaInfo> {
    if value.as_object()?.is_empty() {
        return None;
    }
    match value.get("os").and_then(Json::as_str)? {
        "posix" => Some(MetaInfo::Posix {
            mode: value
                .get("mode")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_string(),
            owner: value
                .get("owner")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_string(),
            group: value
                .get("group")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        "nt" => Some(MetaInfo::Windows {
            attributes: value
                .get("attributes")
                .and_then(Json::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        }),
        _ => None,
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn utc_parts() -> (i64, u32, u32, u32, u32, u32) {
    let seconds = unix_now() as i64;
    let days = seconds.div_euclid(86_400);
    let remainder = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    (
        year,
        month,
        day,
        (remainder / 3_600) as u32,
        ((remainder % 3_600) / 60) as u32,
        (remainder % 60) as u32,
    )
}

fn utc_now_iso() -> String {
    let (year, month, day, hour, minute, second) = utc_parts();
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn utc_stamp_compact() -> String {
    let (year, month, day, hour, minute, second) = utc_parts();
    format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}")
}

/// Гражданская дата из числа дней с 1970-01-01 (алгоритм Говарда Хиннанта).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }
    (year, month as u32, day as u32)
}

fn random_hex6() -> String {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0),
    );
    hasher.write_u32(std::process::id());
    format!("{:06x}", hasher.finish() & 0x00FF_FFFF)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "dc-session-test-{}-{}-{nanos}",
            std::process::id(),
            tag
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn sample_nodes() -> Vec<Node> {
        vec![
            Node {
                name: "docs".to_string(),
                path: PathBuf::from(r"C:\tmp\docs"),
                is_dir: true,
                size_bytes: 300,
                mtime: 1_700_000_000.5,
                is_symlink: false,
                readable: true,
                metadata: None,
            },
            Node {
                name: "a.txt".to_string(),
                path: PathBuf::from(r"C:\tmp\a.txt"),
                is_dir: false,
                size_bytes: 100,
                mtime: 0.0,
                is_symlink: false,
                readable: true,
                metadata: None,
            },
        ]
    }

    #[test]
    fn export_import_roundtrip() {
        let directory = temp_dir("roundtrip");
        let session = Session::from_nodes(Path::new(r"C:\tmp"), &sample_nodes());

        let path = export_session(&session, Some(&directory)).unwrap();
        assert!(path.exists());

        let loaded = import_session(&path).unwrap();
        assert_eq!(loaded.name, session.name);
        assert_eq!(loaded.total_bytes, 400);
        assert_eq!(loaded.nodes.len(), 2);
        assert_eq!(loaded.nodes[0].name, "docs");
        assert_eq!(loaded.nodes[0].size_bytes, 300);
        assert_eq!(loaded.nodes[0].mtime, 1_700_000_000.5);
        assert_eq!(loaded.nodes[0].marker(), 'D');

        fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn imports_python_session_format() {
        let text = r#"{
          "format": "dc-session",
          "version": 1,
          "name": "demo",
          "os": "nt",
          "root": "C:\\Temp\\demo",
          "created_at": "2026-09-23T23:27:23",
          "total_bytes": 3147264,
          "count": 2,
          "meta": {},
          "nodes": [
            {
              "name": "big.bin",
              "path": "C:\\Temp\\demo\\big.bin",
              "is_dir": false,
              "size_bytes": 3145728,
              "size_human": "3,0 МБ",
              "mtime": 1790195243.294152,
              "is_symlink": false,
              "readable": true,
              "metadata": {"os": "nt", "attributes": ["Archive"]}
            },
            {
              "name": "docs",
              "path": "C:\\Temp\\demo\\docs",
              "is_dir": true,
              "size_bytes": 1536,
              "size_human": "1,5 КБ",
              "mtime": 1790195243.2951534,
              "is_symlink": false,
              "readable": true,
              "metadata": {"os": "nt", "attributes": ["Directory"]}
            }
          ]
        }"#;

        let session = Session::from_json(&json::parse(text).unwrap()).unwrap();
        assert_eq!(session.name, "demo");
        assert_eq!(session.total_bytes, 3147264);
        assert_eq!(session.nodes.len(), 2);
        assert_eq!(session.nodes[1].name, "docs");
        assert!(session.nodes[1].is_dir);
        assert_eq!(session.nodes[0].size_human(), "3,0 МБ");
        assert_eq!(
            session.nodes[0].metadata,
            Some(MetaInfo::Windows {
                attributes: vec!["Archive".to_string()]
            })
        );
    }

    #[test]
    fn rejects_foreign_and_future_sessions() {
        let foreign = json::parse(r#"{"format":"other","nodes":[]}"#).unwrap();
        assert!(Session::from_json(&foreign).is_err());

        let future = json::parse(r#"{"format":"dc-session","version":99,"nodes":[]}"#).unwrap();
        assert!(Session::from_json(&future).is_err());

        let bad_nodes = json::parse(r#"{"format":"dc-session","nodes":{}}"#).unwrap();
        assert!(Session::from_json(&bad_nodes).is_err());
    }

    #[test]
    fn rejects_negative_or_non_numeric_sizes() {
        for text in [
            r#"{"format":"dc-session","nodes":[{"name":"a","size_bytes":-1}]}"#,
            r#"{"format":"dc-session","nodes":[{"name":"a","size_bytes":"12"}]}"#,
            r#"{"format":"dc-session","nodes":[{"name":"a"}]}"#,
        ] {
            let value = json::parse(text).unwrap();
            assert!(Session::from_json(&value).is_err(), "должно падать: {text}");
        }
    }

    #[test]
    fn utc_date_conversion_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_000), (2024, 10, 4));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }
}
