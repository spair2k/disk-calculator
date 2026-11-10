//! Сканирование каталога: узлы, рекурсивные размеры, статистика.
//!
//! Работа только с метаданными (`read_dir` + `symlink_metadata`), содержимое
//! файлов не открывается. Размер папки считается рекурсивно, но в отчёт
//! попадает только один уровень. Симлинки и Windows reparse point'ы
//! (junction'ы) по умолчанию не разворачиваются — это исключает двойной счёт
//! и зацикливание.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use crate::platform::{self, MetaInfo};
use crate::sizing::fmt_size;

/// Одна запись анализируемого каталога.
#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub mtime: f64,
    pub is_symlink: bool,
    pub readable: bool,
    pub metadata: Option<MetaInfo>,
}

impl Node {
    /// Человекочитаемый размер (пересчитывается, не хранится).
    pub fn size_human(&self) -> String {
        fmt_size(self.size_bytes)
    }

    /// Маркер типа: `L` — ссылка, `D` — папка, `F` — файл.
    pub fn marker(&self) -> char {
        if self.is_symlink {
            'L'
        } else if self.is_dir {
            'D'
        } else {
            'F'
        }
    }
}

/// Статистика сканирования для подвала отчёта (`--stats`).
#[derive(Debug, Clone, Default)]
pub struct ScanStats {
    pub files: u64,
    pub dirs: u64,
    pub errors: u64,
    pub elapsed: Duration,
}

/// Параметры сканирования.
#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    /// Собирать права/атрибуты (нужно только для экспорта сессии).
    pub collect_metadata: bool,
    /// Разворачивать ссылки (с защитой от циклов).
    pub follow_links: bool,
}

/// Результат сканирования.
#[derive(Debug)]
pub struct ScanResult {
    pub nodes: Vec<Node>,
    pub stats: ScanStats,
}

struct DirWalk {
    size: u64,
    files: u64,
    dirs: u64,
    errors: u64,
    readable: bool,
}

/// Анализирует содержимое каталога на один уровень и сортирует по размеру
/// (по убыванию, при равенстве — по имени).
pub fn scan_directory(root: &Path, options: &ScanOptions) -> ScanResult {
    let started = Instant::now();
    let mut nodes = Vec::new();
    let mut stats = ScanStats::default();

    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => {
            stats.elapsed = started.elapsed();
            return ScanResult { nodes, stats };
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                stats.errors += 1;
                continue;
            }
        };

        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        // `DirEntry::metadata` не разворачивает ссылки и на Windows берёт
        // данные из уже прочитанной записи каталога — без лишнего syscall'а.
        let meta = match entry.metadata() {
            Ok(meta) => meta,
            Err(_) => {
                stats.errors += 1;
                continue;
            }
        };

        let is_symlink = platform::is_link_like(&meta);
        let is_dir = meta.is_dir();
        let mut readable = true;

        let size_bytes = if is_dir && !is_symlink {
            let walk = dir_size(&path, options);
            stats.files += walk.files;
            stats.dirs += walk.dirs + 1;
            stats.errors += walk.errors;
            readable = walk.readable;
            walk.size
        } else {
            stats.files += 1;
            meta.len()
        };

        let mtime = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|elapsed| elapsed.as_secs_f64())
            .unwrap_or(0.0);

        let metadata = if options.collect_metadata {
            Some(platform::metadata_from(&meta))
        } else {
            None
        };

        nodes.push(Node {
            name,
            path,
            is_dir,
            size_bytes,
            mtime,
            is_symlink,
            readable,
            metadata,
        });
    }

    nodes.sort_by(|a, b| {
        b.size_bytes
            .cmp(&a.size_bytes)
            .then_with(|| a.name.cmp(&b.name))
    });

    stats.elapsed = started.elapsed();
    ScanResult { nodes, stats }
}

fn dir_size(root: &Path, options: &ScanOptions) -> DirWalk {
    let mut walk = DirWalk {
        size: 0,
        files: 0,
        dirs: 0,
        errors: 0,
        readable: false,
    };

    let mut stack = vec![root.to_path_buf()];
    let mut visited: HashSet<PathBuf> = HashSet::new();
    if options.follow_links {
        if let Ok(canonical) = fs::canonicalize(root) {
            visited.insert(canonical);
        }
    }

    let mut first = true;
    while let Some(directory) = stack.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => {
                if first {
                    walk.readable = false;
                }
                walk.errors += 1;
                first = false;
                continue;
            }
        };
        if first {
            walk.readable = true;
        }
        first = false;

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    walk.errors += 1;
                    continue;
                }
            };
            let path = entry.path();
            let meta = match entry.metadata() {
                Ok(meta) => meta,
                Err(_) => {
                    walk.errors += 1;
                    continue;
                }
            };

            if platform::is_link_like(&meta) {
                if !options.follow_links {
                    continue;
                }
                let canonical = match fs::canonicalize(&path) {
                    Ok(path) => path,
                    Err(_) => {
                        walk.errors += 1;
                        continue;
                    }
                };
                if !visited.insert(canonical.clone()) {
                    continue;
                }
                let target = match fs::metadata(&path) {
                    Ok(meta) => meta,
                    Err(_) => {
                        walk.errors += 1;
                        continue;
                    }
                };
                if target.is_dir() {
                    walk.dirs += 1;
                    stack.push(canonical);
                } else {
                    walk.files += 1;
                    walk.size += target.len();
                }
                continue;
            }

            if meta.is_dir() {
                walk.dirs += 1;
                if options.follow_links {
                    match fs::canonicalize(&path) {
                        Ok(canonical) => {
                            if !visited.insert(canonical.clone()) {
                                continue;
                            }
                            stack.push(canonical);
                        }
                        Err(_) => walk.errors += 1,
                    }
                } else {
                    stack.push(path);
                }
            } else {
                walk.files += 1;
                walk.size += meta.len();
            }
        }
    }

    walk
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
            "dc-rust-test-{}-{}-{nanos}",
            std::process::id(),
            tag
        ));
        fs::create_dir_all(&path).expect("временный каталог");
        path
    }

    fn write_file(path: &Path, size: usize) {
        fs::write(path, vec![b'x'; size]).expect("файл");
    }

    #[test]
    fn recursive_sizes_and_sorting() {
        let root = temp_dir("sizes");

        fs::create_dir_all(root.join("docs/sub")).unwrap();
        write_file(&root.join("docs/sub/deep.bin"), 150);
        write_file(&root.join("docs/top.bin"), 50);
        write_file(&root.join("single.bin"), 10);
        fs::create_dir_all(root.join("empty")).unwrap();

        let result = scan_directory(&root, &ScanOptions::default());
        let names: Vec<&str> = result.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["docs", "single.bin", "empty"]);

        let docs = &result.nodes[0];
        assert!(docs.is_dir);
        assert_eq!(docs.size_bytes, 200);
        assert_eq!(docs.marker(), 'D');

        assert_eq!(result.nodes[1].size_bytes, 10);
        assert_eq!(result.nodes[1].marker(), 'F');
        assert_eq!(result.nodes[2].size_bytes, 0);
        assert_eq!(result.stats.files, 3);
        assert_eq!(result.stats.dirs, 3);
        assert_eq!(result.stats.errors, 0);

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn metadata_is_collected_only_when_requested() {
        let root = temp_dir("meta");
        write_file(&root.join("a.txt"), 1);

        let plain = scan_directory(&root, &ScanOptions::default());
        assert!(plain.nodes[0].metadata.is_none());

        let with_meta = scan_directory(
            &root,
            &ScanOptions {
                collect_metadata: true,
                follow_links: false,
            },
        );
        assert!(with_meta.nodes[0].metadata.is_some());

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_directory_is_empty() {
        let root = temp_dir("missing").join("nope");
        let result = scan_directory(&root, &ScanOptions::default());
        assert!(result.nodes.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_not_followed_by_default() {
        let root = temp_dir("symlink");
        fs::create_dir_all(root.join("real")).unwrap();
        write_file(&root.join("real/data.bin"), 100);
        std::os::unix::fs::symlink(root.join("real"), root.join("alias")).unwrap();
        std::os::unix::fs::symlink(root.join("real/data.bin"), root.join("file-link")).unwrap();

        let result = scan_directory(&root, &ScanOptions::default());
        let alias = result.nodes.iter().find(|n| n.name == "alias").unwrap();
        assert!(alias.is_symlink);
        assert!(alias.is_dir);
        assert_eq!(alias.marker(), 'L');
        // Содержимое цели не посчитано: только длина самой ссылки.
        assert!(alias.size_bytes < 100);

        let link = result.nodes.iter().find(|n| n.name == "file-link").unwrap();
        assert!(link.is_symlink);
        assert!(!link.is_dir);
        assert_eq!(link.marker(), 'L');

        fs::remove_dir_all(&root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn follow_links_counts_target_and_survives_cycles() {
        let root = temp_dir("follow");
        fs::create_dir_all(root.join("real")).unwrap();
        write_file(&root.join("real/data.bin"), 64);
        std::os::unix::fs::symlink(root.join("real"), root.join("alias")).unwrap();
        // Цикл: ссылка внутри real обратно на root.
        std::os::unix::fs::symlink(&root, root.join("real/loop")).unwrap();

        let result = scan_directory(
            &root,
            &ScanOptions {
                collect_metadata: false,
                follow_links: true,
            },
        );

        // alias и real указывают на один каталог: visited не даёт посчитать
        // его дважды, цикл loop -> root тоже обрывается.
        let total: u64 = result.nodes.iter().map(|n| n.size_bytes).sum();
        assert_eq!(total, 64);

        fs::remove_dir_all(&root).ok();
    }
}
