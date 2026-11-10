//! Кроссплатформенные метаданные: права POSIX и атрибуты Windows.
//!
//! Метаданные не читают содержимое файлов — только `stat`-информацию.
//! Используются при экспорте сессии (`dc -e`).

use std::fs::Metadata;

/// Флаг Windows «это reparse point» (symlink, junction, mount point...).
pub const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

/// Метаданные записи, зависящие от ОС.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetaInfo {
    Posix {
        mode: String,
        owner: String,
        group: String,
    },
    Windows {
        attributes: Vec<String>,
    },
}

/// Идентификатор ОС в формате исходного проекта: `nt` или `posix`.
pub fn current_os() -> &'static str {
    if cfg!(windows) {
        "nt"
    } else {
        "posix"
    }
}

/// Является ли запись ссылкой (symlink, а на Windows — любой reparse point,
/// включая junction). Такие записи не разворачиваются при подсчёте размеров.
pub fn is_link_like(meta: &Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(unix)]
pub fn metadata_from(meta: &Metadata) -> MetaInfo {
    posix_meta(meta)
}

#[cfg(windows)]
pub fn metadata_from(meta: &Metadata) -> MetaInfo {
    windows_meta(meta)
}

#[cfg(not(any(unix, windows)))]
pub fn metadata_from(_meta: &Metadata) -> MetaInfo {
    MetaInfo::Windows {
        attributes: Vec::new(),
    }
}

/// Строка прав вида `rwxr-xr-x` (с учётом setuid/setgid/sticky).
#[cfg(unix)]
pub fn mode_string(mode: u32) -> String {
    let mut out = String::with_capacity(9);
    for (class, shift) in [6u32, 3, 0].into_iter().enumerate() {
        let bits = (mode >> shift) & 0o7;
        out.push(if bits & 0o4 != 0 { 'r' } else { '-' });
        out.push(if bits & 0o2 != 0 { 'w' } else { '-' });

        let executable = bits & 0o1 != 0;
        let special = match class {
            0 => mode & 0o4000 != 0, // setuid
            1 => mode & 0o2000 != 0, // setgid
            _ => mode & 0o1000 != 0, // sticky
        };
        out.push(match (executable, special) {
            (true, false) => 'x',
            (false, false) => '-',
            (true, true) if class == 2 => 't',
            (false, true) if class == 2 => 'T',
            (true, true) => 's',
            (false, true) => 'S',
        });
    }
    out
}

#[cfg(unix)]
fn posix_meta(meta: &Metadata) -> MetaInfo {
    use std::os::unix::fs::MetadataExt;

    MetaInfo::Posix {
        mode: mode_string(meta.mode()),
        owner: uid_name(meta.uid()),
        group: gid_name(meta.gid()),
    }
}

#[cfg(unix)]
fn uid_name(uid: u32) -> String {
    use std::collections::HashMap;
    use std::sync::OnceLock;

    static CACHE: OnceLock<HashMap<u32, String>> = OnceLock::new();
    CACHE
        .get_or_init(|| parse_id_file("/etc/passwd"))
        .get(&uid)
        .cloned()
        .unwrap_or_else(|| uid.to_string())
}

#[cfg(unix)]
fn gid_name(gid: u32) -> String {
    use std::collections::HashMap;
    use std::sync::OnceLock;

    static CACHE: OnceLock<HashMap<u32, String>> = OnceLock::new();
    CACHE
        .get_or_init(|| parse_id_file("/etc/group"))
        .get(&gid)
        .cloned()
        .unwrap_or_else(|| gid.to_string())
}

/// Разбирает `/etc/passwd` или `/etc/group`: имя в поле 0, id — в поле 2.
#[cfg(unix)]
fn parse_id_file(path: &str) -> std::collections::HashMap<u32, String> {
    let mut map = std::collections::HashMap::new();
    if let Ok(text) = std::fs::read_to_string(path) {
        for line in text.lines() {
            let mut fields = line.split(':');
            let Some(name) = fields.next().filter(|name| !name.is_empty()) else {
                continue;
            };
            if let Some(id) = fields.nth(1).and_then(|raw| raw.trim().parse::<u32>().ok()) {
                map.insert(id, name.to_string());
            }
        }
    }
    map
}

#[cfg(windows)]
fn windows_meta(meta: &Metadata) -> MetaInfo {
    use std::os::windows::fs::MetadataExt;

    const ATTRIBUTES: [(u32, &str); 8] = [
        (0x1, "ReadOnly"),
        (0x2, "Hidden"),
        (0x4, "System"),
        (0x10, "Directory"),
        (0x20, "Archive"),
        (0x40, "Device"),
        (0x400, "ReparsePoint"),
        (0x800, "Compressed"),
    ];

    let flags = meta.file_attributes();
    let mut attributes: Vec<String> = ATTRIBUTES
        .iter()
        .filter(|(bit, _)| flags & bit != 0)
        .map(|(_, name)| (*name).to_string())
        .collect();
    attributes.sort();
    MetaInfo::Windows { attributes }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn mode_string_plain() {
        assert_eq!(mode_string(0o755), "rwxr-xr-x");
        assert_eq!(mode_string(0o644), "rw-r--r--");
        assert_eq!(mode_string(0o000), "---------");
    }

    #[cfg(unix)]
    #[test]
    fn mode_string_special_bits() {
        assert_eq!(mode_string(0o4755), "rwsr-xr-x");
        assert_eq!(mode_string(0o2644), "rw-r-Sr--");
        assert_eq!(mode_string(0o1777), "rwxrwxrwt");
    }

    #[test]
    fn current_os_matches_platform() {
        if cfg!(windows) {
            assert_eq!(current_os(), "nt");
        } else {
            assert_eq!(current_os(), "posix");
        }
    }

    #[test]
    fn symlink_is_link_like() {
        let meta = std::fs::symlink_metadata(file!()).expect("метаданные исходника");
        assert!(!is_link_like(&meta));
    }
}
