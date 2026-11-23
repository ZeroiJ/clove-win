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
        ("Desktop", home.join("Desktop")),
        ("Documents", home.join("Documents")),
        ("Downloads", home.join("Downloads")),
        ("Pictures", home.join("Pictures")),
        ("Music", home.join("Music")),
        ("Videos", home.join("Videos")),
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
            let mut parts = line.split_whitespace();
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

pub fn open_path(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("cmd").args(["/C", "start", "", &path.to_string_lossy()]).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("xdg-open").arg(path).spawn();
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

pub fn trash_name() -> &'static str {
    "Trash"
}
