# disk-calculator (Rust)

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Release](https://img.shields.io/github/v/release/spair2k/disk-calculator)](https://github.com/spair2k/disk-calculator/releases)
![Rust](https://img.shields.io/badge/rust-1.74%2B-orange)
![Platform](https://img.shields.io/badge/platform-windows%20%7C%20linux%20%7C%20macos-lightgrey)

**English** | [Русский](README.ru.md)

DC — Disk Calculator: analyze folder disk usage with a single console report.
A Rust port of the Python project `DiskCalculator`.

**Zero dependencies**: standard library only. The JSON parser, serializer,
animation, Windows attribute handling and POSIX permissions are implemented
by hand.

Works with **metadata only** (`read_dir` + `symlink_metadata`). File contents
are never opened or read.

## Features

- **single-level** directory analysis: folders and files sorted by size;
  folder size is recursive (all nested files);
- type marker: `D` — folder, `F` — file, `L` — link;
- two-pane report: name on the left, usage bar, size and share on the right;
- pseudo-graphics (`│─┼`, `█░`) or `-l/--legacy` for pure ASCII (`| - +`, `#/-`);
- analysis animation on the first line (Braille snake / spinner) — TTY only;
- sessions: JSON export (`-e`) and opening a saved report; the format is
  compatible with the Python version;
- correct alignment with Cyrillic, CJK and emoji; long names are truncated
  to the terminal width;
- Windows junctions and reparse points are not followed — no double counting
  or infinite loops.

## Building

Only Rust (1.74+) is required, no network, no `cargo add`:

```
cargo build --release
```

The binary is `target\release\dc.exe` (Windows) or `target/release/dc`.
For a regular build — `cargo build`.

## Usage

```
dc                        # report for the current directory
dc C:\Users               # report for a folder
dc -e D:\Data             # report + export session to storage/
dc -l --top 10 .          # ASCII report, top 10 largest only
dc report.json            # open a saved session
dc --help                 # all options
```

### Options

| Flag | Purpose |
|---|---|
| `-e`, `--export` | export the result to a JSON session |
| `-l`, `--legacy` | ASCII instead of pseudo-graphics |
| `-n`, `--top N` | only the first N items |
| `-w`, `--width N` | limit the report width (0 — unlimited) |
| `--storage-dir DIR` | directory for sessions (default `storage/`) |
| `--stats` | statistics: files, folders, errors, time |
| `--follow-links` | follow links (with cycle protection) |
| `--no-animation` | disable animation |
| `-h`, `--help` | help |
| `-V`, `--version` | version |

### Example output

```
Markers: D — folder, F — file, L — link
Contents (single level, by size)  │  Usage relative to total size
──────────────────────────────────┼───────────────────────────────────────
D  docs                           │  ███████████████░░░░░░░░░░░░░░░ 1.5 KB (50.0%)
F  report.txt                     │  ██████████░░░░░░░░░░░░░░░░░░░░ 1.0 KB (33.3%)
Total: 3.0 KB
```

## Differences from the Python version

Improvements that do not change the session format:

- fixed rounding at unit boundaries: `1048575` B is `1.0 MB`, not `1024 KB`;
- Windows junction/reparse points are marked as `L` and not traversed;
- symlink cycles with `--follow-links` are cut off by visited directories;
- no recursion in tree traversal — deep hierarchies do not overflow the stack;
- fewer `stat` calls: metadata is taken from the already obtained
  `symlink_metadata`;
- file names are cleaned of control/bidi characters (terminal protection);
- real Unicode width is taken into account, long names are truncated;
- sessions are validated (version, field types, size limit), writes are atomic;
- `--stats`, `--top`, `--width`, `--follow-links`, `--no-animation`.

Differences in details:

- time in the session and file name is UTC (`...Z`), not local;
- `metadata` for POSIX contains owner/group names from `/etc/passwd`
  and `/etc/group` when available;
- during export only metadata is collected, during a regular report — not
  (no unnecessary system calls).

## Tests

```
cargo test
```

Covered: size formatting and unit boundaries, Unicode width/truncation,
JSON (escaping, surrogates, deep nesting protection), recursive sizes and
symlinks, session import/export, rendering and alignment, and running the
binary end-to-end.
