//! Small OS seam for the parts of Clove that touch the host.
//!
//! Keep every `cfg(target_os = ...)` decision here so the rest of the UI stays
//! platform-neutral. On Linux this follows the freedesktop/XDG conventions.

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("/"))
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| home_dir().join(".config"))
}

pub fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| home_dir().join(".cache"))
}

#[allow(dead_code)]
pub fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| home_dir().join(".local").join("share"))
}

pub fn quick_access() -> Vec<(&'static str, PathBuf)> {
    let home = home_dir();
    let mut items = Vec::new();
    let candidates = [
        ("Downloads", home.join("Downloads")),
        ("Projects", home.join("Projects")),
        ("Pictures", home.join("Pictures")),
        ("Videos", home.join("Videos")),
        ("Music", home.join("Music")),
        ("Documents", home.join("Documents")),
        ("Work", home.join("Work")),
        ("Desktop", home.join("Desktop")),
    ];
    for (name, path) in candidates {
        if path.exists() {
            items.push((name, path));
        }
    }
    items
}

/// Mount points worth showing in "This PC", plus the root filesystem.
pub fn mount_entries() -> Vec<(String, PathBuf)> {
    let mut entries = Vec::new();
    entries.push(("/".to_string(), PathBuf::from("/")));

    // Removable/media mounts and any extra real filesystems.
    if let Ok(text) = std::fs::read_to_string("/proc/self/mountinfo") {
        for line in text.lines() {
            // mountinfo: ... mountpoint ... - fstype source super
            // Fields are variable; the mount point is field 5 (1-based) in the common case.
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 5 {
                continue;
            }
            let mount_point = fields[4];
            if mount_point == "/" {
                continue;
            }
            // Skip pseudo/virtual filesystems.
            let fs_type = fields.iter().rev().take(3).nth(1).copied().unwrap_or("");
            if matches!(
                fs_type,
                "proc" | "sysfs" | "tmpfs" | "devtmpfs" | "devpts" | "cgroup" | "cgroup2"
                    | "pstore" | "bpf" | "tracefs" | "debugfs" | "securityfs" | "configfs"
                    | "fusectl" | "mqueue" | "hugetlbfs" | "ramfs" | "overlay" | "squashfs"
            ) {
                continue;
            }
            let path = PathBuf::from(mount_point);
            if path.exists() {
                entries.push((mount_point.to_string(), path));
            }
        }
    }

    // De-duplicate by path.
    let mut seen = std::collections::HashSet::new();
    entries.retain(|(_, p)| seen.insert(p.clone()));
    entries
}

#[cfg(target_os = "linux")]
fn open_command(path: &Path) -> Command {
    let mut c = Command::new("xdg-open");
    c.arg(path);
    c
}

pub fn open_path(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("cmd").args(["/C", "start", "", &path.to_string_lossy()]).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let mut c = open_command(path);
        let _ = c.spawn();
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = path;
    }
}

pub fn open_terminal_here(dir: &Path) {
    #[cfg(target_os = "linux")]
    {
        // Try common terminal emulators; Omarchy/Arch users often have one of these.
        let candidates: &[(&str, &[&str])] = &[
            ("xdg-terminal-exec", &[]),
            ("ghostty", &["--working-directory"]),
            ("foot", &["--working-directory"]),
            ("kitty", &["--directory"]),
            ("alacritty", &["--working-directory"]),
            ("gnome-terminal", &["--working-directory"]),
            ("konsole", &["--workdir"]),
            ("xterm", &["-e"]),
        ];
        for (cmd, args) in candidates {
            let mut c = Command::new(cmd);
            if *cmd == "xdg-terminal-exec" {
                c.current_dir(dir);
            } else if *cmd == "xterm" {
                c.current_dir(dir).arg("-e").arg("sh");
            } else {
                c.args(*args).arg(dir);
            }
            if c.spawn().is_ok() {
                return;
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("cmd").args(["/C", "start", "cmd"]).current_dir(dir).spawn();
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = dir;
    }
}

#[allow(dead_code)]
pub fn trash_name() -> &'static str {
    "Trash"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_access_returns_vec() {
        let _ = quick_access();
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn mounts_parse() {
        let mounts = mount_entries();
        assert!(!mounts.is_empty(), "/proc/mounts should list at least the root");
    }

    #[test]
    fn dirs_nonempty() {
        assert!(!config_dir().as_os_str().is_empty());
        assert!(!cache_dir().as_os_str().is_empty());
        assert!(!data_dir().as_os_str().is_empty());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn open_command_uses_xdg_open() {
        let cmd = open_command(Path::new("/tmp/clove-open-test"));
        assert_eq!(cmd.get_program(), "xdg-open");
    }

    #[test]
    fn trash_refuses_outside_root() {
        let base = std::env::temp_dir().join("clove_test_trash");
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("root");
        let outside = base.join("outside.txt");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&outside, b"x").unwrap();
        let outcome = clove_shell::trash(&outside, &root, false);
        assert_eq!(outcome.failures, 1);
        assert!(outside.exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
