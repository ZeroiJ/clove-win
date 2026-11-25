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

/// GNOME/GTK bookmarks (`~/.config/gtk-3.0/bookmarks`, falling back to `gtk-4.0`).
pub fn gtk_bookmarks() -> Vec<(String, PathBuf)> {
    let mut items = Vec::new();
    let cfg = config_dir();
    let home = home_dir();
    let candidates = [
        cfg.join("gtk-3.0").join("bookmarks"),
        cfg.join("gtk-4.0").join("bookmarks"),
    ];
    for file in &candidates {
        let Ok(raw) = std::fs::read_to_string(file) else { continue };
        for line in raw.lines() {
            let line = line.trim();
            if line.is_empty() { continue; }
            let (uri, custom) = match line.split_once(' ') {
                Some((u, label)) => (u, Some(label.trim().to_string())),
                None => (line, None),
            };
            if let Some(rest) = uri.strip_prefix("file://") {
                let decoded = percent_decode(rest);
                let path = PathBuf::from(&decoded);
                if path.is_dir() && path != home {
                    let label = custom
                        .filter(|s| !s.is_empty())
                        .or_else(|| path.file_name().map(|n| n.to_string_lossy().to_string()))
                        .unwrap_or_else(|| decoded.clone());
                    if !items.iter().any(|(_, p): &(String, PathBuf)| p == &path) {
                        items.push((label, path));
                    }
                }
            }
        }
        if !items.is_empty() {
            return items;
        }
    }
    // Fallback to standard user folders that exist if bookmarks file is missing/empty.
    for name in ["Documents", "Music", "Pictures", "Videos", "Downloads"] {
        let p = home.join(name);
        if p.is_dir() {
            items.push((name.to_string(), p));
        }
    }
    items
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()
                .and_then(|s| u8::from_str_radix(s, 16).ok());
            if let Some(b) = hex {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Ordered icon theme search list from the active desktop / Omarchy / GTK config.
pub fn active_icon_theme() -> Vec<String> {
    fn push_theme(themes: &mut Vec<String>, name: &str) {
        let trimmed = name.trim().trim_matches(|c| c == '\'' || c == '"');
        if !trimmed.is_empty() && !themes.iter().any(|t| t == trimmed) {
            themes.push(trimmed.to_string());
        }
    }
    let mut themes = Vec::new();
    if let Ok(raw) = std::fs::read_to_string(config_dir().join("omarchy/current/theme/icons.theme")) {
        push_theme(&mut themes, &raw);
    }
    if let Ok(out) = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "icon-theme"])
        .output()
    {
        if out.status.success() {
            push_theme(&mut themes, &String::from_utf8_lossy(&out.stdout));
        }
    }
    for sub in ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini"] {
        if let Ok(raw) = std::fs::read_to_string(config_dir().join(sub)) {
            for line in raw.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    if k.trim() == "gtk-icon-theme-name" {
                        push_theme(&mut themes, v);
                    }
                }
            }
        }
    }
    // Resolve inheritance hops via index.theme (e.g. Yaru-red -> Yaru -> hicolor).
    let mut idx = 0;
    while idx < themes.len() && themes.len() < 8 {
        let name = themes[idx].clone();
        let index_path = PathBuf::from("/usr/share/icons").join(&name).join("index.theme");
        if let Ok(raw) = std::fs::read_to_string(&index_path) {
            for line in raw.lines() {
                if let Some(rest) = line.strip_prefix("Inherits=") {
                    for parent in rest.split(',') {
                        push_theme(&mut themes, parent);
                    }
                    break;
                }
            }
        }
        idx += 1;
    }
    for fallback in ["Yaru", "Adwaita", "hicolor"] {
        push_theme(&mut themes, fallback);
    }
    themes
}

/// Mount points worth showing as removable/external drives (excludes pseudo/virtual filesystems).
pub fn mount_entries() -> Vec<(String, PathBuf)> {
    let mut entries = Vec::new();
    entries.push(("/".to_string(), PathBuf::from("/")));

    // Removable/media mounts and any extra real filesystems.
    if let Ok(text) = std::fs::read_to_string("/proc/self/mountinfo") {
        for line in text.lines() {
            // mountinfo: <id> <parent> <maj:min> <root> <mount-point> <opts> [optional...] - <fstype> <src> <super>
            let Some((before, after)) = line.split_once(" - ") else {
                continue;
            };
            let before_fields: Vec<&str> = before.split_whitespace().collect();
            if before_fields.len() < 5 {
                continue;
            }
            let mount_point = before_fields[4]
                .replace("\\040", " ")
                .replace("\\011", "\t");
            if mount_point == "/" {
                continue;
            }
            if mount_point == "/sys"
                || mount_point.starts_with("/sys/")
                || mount_point == "/proc"
                || mount_point.starts_with("/proc/")
                || mount_point == "/dev"
                || mount_point.starts_with("/dev/")
                || mount_point == "/run"
                || (mount_point.starts_with("/run/") && !mount_point.starts_with("/run/media/"))
                || mount_point == "/boot"
                || mount_point.starts_with("/boot/")
                || mount_point == "/efi"
                || mount_point.starts_with("/efi/")
                || mount_point == "/tmp"
                || mount_point.starts_with("/tmp/")
                || mount_point == "/home"
                || mount_point.starts_with("/home/")
                || mount_point == "/var"
                || mount_point.starts_with("/var/")
                || mount_point.starts_with("/.")
            {
                continue;
            }
            // Skip pseudo/virtual filesystems.
            let fs_type = after.split_whitespace().next().unwrap_or("");
            if matches!(
                fs_type,
                "proc" | "sysfs" | "tmpfs" | "devtmpfs" | "devpts" | "cgroup" | "cgroup2"
                    | "pstore" | "bpf" | "tracefs" | "debugfs" | "securityfs" | "configfs"
                    | "fusectl" | "mqueue" | "hugetlbfs" | "ramfs" | "overlay" | "squashfs"
                    | "efivarfs" | "binfmt_misc" | "autofs" | "rpc_pipefs" | "fuse.portal"
            ) {
                continue;
            }
            let path = PathBuf::from(&mount_point);
            if path.exists() {
                let label = if mount_point.starts_with("/run/media/")
                    || mount_point.starts_with("/media/")
                    || mount_point.starts_with("/mnt/")
                {
                    path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| mount_point.clone())
                } else {
                    mount_point.clone()
                };
                entries.push((label, path));
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
