use std::collections::HashMap;
use std::path::{Path, PathBuf};

use clove_fs::Entry;
use rune::text::Icon;
use rune::Pixels;

// Crisp 16x16 monochrome symbolic vector icons matched to GNOME Files (Nautilus 51)
// and Adwaita / Yaru symbolics. No hardcoded hex fills so `rune::icon().color()` tints cleanly.

pub const ICON_EDIT_FIND: &Icon = &Icon {
    name: "edit-find-symbolic",
    view: 16.0,
    body: r#"<path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" d="M 14 14 L 10.3891 10.3891 M 12 6.51379 C 11.9924 9.55136, 9.52377 12.0076, 6.48621 12 C 3.44864 11.9924, 0.992382 9.52377, 1 6.48621 C 1.00762 3.44864, 3.47623 0.992382, 6.51379 1 C 9.55136 1.00762, 12.0076 3.47623, 12 6.51379 Z"/>"#,
};

pub const ICON_OPEN_MENU: &Icon = &Icon {
    name: "open-menu-symbolic",
    view: 16.0,
    body: r#"<path d="m1 2h14v2h-14z"/><path d="m1 7h14v2h-14z"/><path d="m1 12h14v2h-14z"/>"#,
};

pub const ICON_USER_HOME: &Icon = &Icon {
    name: "user-home-symbolic",
    view: 16.0,
    body: r#"<path d="M7.972656.363281c-.5 0-1 .171875-1.40625.511719L2.101562 4.59375C1.386719 5.191406.972656 6.074219.972656 7.003906v6c0 1.65625 1.34375 3 3 3h8c1.65625 0 3-1.34375 3-3v-6c0-.929687-.414062-1.8125-1.128906-2.410156L9.378906.875c-.40625-.339844-.90625-.511719-1.40625-.511719m-.015625 2c.050781-.003906.101563.011719.144531.046875l4.464844 3.71875c.257813.21875.40625.539063.40625.875v6c0 .5625-.4375 1-1 1h-2v-5c0-.554687-.445312-1-1-1h-2c-.554687 0-1 .445313-1 1v5h-2c-.5625 0-1-.4375-1-1v-6c0-.335937.148438-.65625.40625-.875l4.464844-3.71875c.035156-.027344.074219-.042968.113281-.046875m0 0"/>"#,
};

pub const ICON_RECENT: &Icon = &Icon {
    name: "document-open-recent-symbolic",
    view: 16.0,
    body: r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M 8 2 C 11.3137 2 14 4.6863 14 8 C 14 11.3137 11.3137 14 8 14 C 4.6863 14 2 11.3137 2 8 C 2 4.6863 4.6863 2 8 2 Z"/><path fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" d="M 8 4.8 L 8 8.2 L 10.6 9.8"/>"#,
};

pub const ICON_STARRED: &Icon = &Icon {
    name: "starred-symbolic",
    view: 16.0,
    body: r#"<path d="M8 0c-.429688 0-.8125.273438-.949219.683594L5.601562 5.023438l-4.589843-.015626C.03125 5.003906-.371094 6.265625.429688 6.828125l3.625 2.554687-1.5 4.285157c-.316407.902343.6875 1.691406 1.492187 1.171875l4.003906-2.59375 3.894531 2.585937c.800782.53125 1.816407-.257812 1.5-1.160156l-1.503906-4.289063 3.644532-2.578124c.789062-.5625.394531-1.808594-.574219-1.8125l-4.660157-.015626L8.949219.691406C8.816406.28125 8.433594.003906 8 0m0 0"/>"#,
};

pub const ICON_NETWORK: &Icon = &Icon {
    name: "network-computer-symbolic",
    view: 16.0,
    body: r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M 3 2.5 L 13 2.5 C 14.1046 2.5, 15 3.39543, 15 4.5 L 15 9.5 C 15 10.6046, 14.1046 11.5, 13 11.5 L 3 11.5 C 1.89543 11.5, 1 10.6046, 1 9.5 L 1 4.5 C 1 3.39543, 1.89543 2.5, 3 2.5 Z"/><path fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" d="M 8 11.5 L 8 13.5 M 5 14 L 11 14"/>"#,
};

pub const ICON_TRASH: &Icon = &Icon {
    name: "user-trash-symbolic",
    view: 16.0,
    body: r#"<path d="M 3 4 L 3 13 C 3 14.1045694, 3.89543056 15, 5 15 L 11 15 C 12.1045694 15, 13 14.1045694, 13 13 L 13 4" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" stroke="currentColor" fill="none"/><path d="M 14.5 4 L 1.5 4" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" stroke="currentColor" fill="none"/><path d="M 5 4 L 5 3 C 5 1.89543045, 5.89543056 1, 7 1 L 9 1 C 10.1045694 1, 11 1.89543045, 11 3 L 11 4" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" stroke="currentColor" fill="none"/><path d="M 6.5 7 L 6.5 12 M 9.5 7 L 9.5 12" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" stroke="currentColor" fill="none"/>"#,
};

pub const ICON_FOLDER_DOWNLOAD: &Icon = &Icon {
    name: "folder-download-symbolic",
    view: 16.0,
    body: r#"<path d="M7.984375 0c-.550781 0-1 .449219-1 1v8.585938L4.691406 7.292969C4.503906 7.105469 4.25 7 3.984375 7s-.519531.105469-.707031.292969c-.390625.390625-.390625 1.023437 0 1.414062l4 4c.390625.390625 1.023437.390625 1.414062 0l4-4c.390625-.390625.390625-1.023437 0-1.414062s-1.023437-.390625-1.414062 0L8.984375 9.585938V1c0-.550781-.449219-1-1-1m-6 14v2h12v-2zm0 0"/>"#,
};

pub const ICON_FOLDER_PROJECTS: &Icon = &Icon {
    name: "folder-projects-symbolic",
    view: 16.0,
    body: r#"<path fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="1.8" d="M4 15 15 4v11ZM.998929 10H4l5.34828-5.34828c.83602-.83604.83602-2.19154 0-3.02758-.83605-.820812-2.17919-.808461-3 .02758L1 7Z"/><path fill="none" stroke="currentColor" stroke-width="1.2" d="m12.5 9.5-3 3h3Z"/>"#,
};

pub const ICON_FOLDER_PICTURES: &Icon = &Icon {
    name: "folder-pictures-symbolic",
    view: 16.0,
    body: r#"<path d="M6.015625 5.03125c-.550781 0-1 .449219-1 1s.449219 1 1 1 1-.449219 1-1-.449219-1-1-1m3.5 3-2 2-1.5-1-2 2v.5c0 .5.5.5.5.5h7s.472656-.035156.5-.5v-1zm0 0"/><path d="m4.01562 1.03125c-1.64453 0-3 1.35547-3 3v8c0 1.64453 1.35547 3 3 3h8c1.64453 0 3-1.35547 3-3v-8c0-1.64453-1.35547-3-3-3zm0 2h8c0.570313 0 1 0.429688 1 1v8c0 0.570312-0.429687 1-1 1h-8c-0.570313 0-1-0.429688-1-1v-8c0-0.570312 0.429687-1 1-1"/>"#,
};

pub const ICON_FOLDER_VIDEOS: &Icon = &Icon {
    name: "folder-videos-symbolic",
    view: 16.0,
    body: r#"<path d="M7 3.003906c-1.644531 0-3 1.355469-3 3v1.425782L.679688 3.996094H0v9h.644531L4 9.503906v1.5c0 1.644532 1.355469 3 3 3h6c1.644531 0 3-1.355468 3-3v-5c0-1.644531-1.355469-3-3-3zm0 2h6c.570312 0 1 .429688 1 1v5c0 .570313-.429688 1-1 1H7c-.570312 0-1-.429687-1-1v-5c0-.570312.429688-1 1-1m0 0"/>"#,
};

pub const ICON_FOLDER_DOCUMENTS: &Icon = &Icon {
    name: "folder-documents-symbolic",
    view: 16.0,
    body: r#"<path d="M4.96875 1.003906c-1.644531 0-3 1.355469-3 3v9c0 1.644532 1.355469 3 3 3h6c1.644531 0 3-1.355468 3-3v-7.5c0-.265625-.105469-.519531-.292969-.707031l-3.5-3.5c-.1875-.1875-.441406-.292969-.707031-.292969zm0 2h4v1.5q0 1.5 1.5 1.5h1.5v7c0 .570313-.429688 1-1 1h-6c-.570312 0-1-.429687-1-1v-9c0-.570312.429688-1 1-1m0 0"/>"#,
};

pub const ICON_FOLDER_MUSIC: &Icon = &Icon {
    name: "folder-music-symbolic",
    view: 16.0,
    body: r#"<path d="M8 1C6.355469 1 5 2.355469 5 4v4.339844C4.542969 8.125 4.035156 8 3.5 8 1.578125 8 0 9.578125 0 11.5S1.578125 15 3.5 15c1.914062 0 3.492188-1.570312 3.5-3.484375V4c0-.5625.4375-1 1-1h4c.5625 0 1 .4375 1 1v4.339844C12.542969 8.125 12.035156 8 11.5 8 9.578125 8 8 9.578125 8 11.5S9.578125 15 11.5 15s3.5-1.578125 3.5-3.5V4c0-1.644531-1.355469-3-3-3zm-4.5 9c.839844 0 1.5.660156 1.5 1.5v.015625C4.992188 12.347656 4.335938 13 3.5 13c-.839844 0-1.5-.660156-1.5-1.5S2.660156 10 3.5 10m8 0c.839844 0 1.5.660156 1.5 1.5s-.660156 1.5-1.5 1.5-1.5-.660156-1.5-1.5.660156-1.5 1.5-1.5m0 0"/>"#,
};

pub const ICON_FOLDER: &Icon = &Icon {
    name: "folder-symbolic",
    view: 16.0,
    body: r#"<path d="M2.96875 1.003906c-1.644531 0-3 1.355469-3 3v8c0 1.644532 1.355469 3 3 3h10c1.644531 0 3-1.355468 3-3v-6c0-1.644531-1.355469-3-3-3H9.382812L7.675781 1.296875c-.1875-.1875-.441406-.292969-.707031-.292969zm0 2h3.585938l1 1H1.96875c0-.5625.4375-1 1-1m-1 2h11c.5625 0 1 .4375 1 1v6c0 .5625-.4375 1-1 1h-10c-.5625 0-1-.4375-1-1zm0 0"/>"#,
};

pub const ICON_GO_PREV: &Icon = &Icon {
    name: "go-previous-symbolic",
    view: 16.0,
    body: r#"<path d="M12 2c0-.265625-.105469-.519531-.292969-.707031-.390625-.390625-1.023437-.390625-1.414062 0l-6 6C4.105469 7.480469 4 7.734375 4 8s.105469.519531.292969.707031l6 6c.390625.390625 1.023437.390625 1.414062 0C11.894531 14.519531 12 14.265625 12 14s-.105469-.519531-.292969-.707031L6.414062 8l5.292969-5.292969C11.894531 2.519531 12 2.265625 12 2m0 0"/>"#,
};

pub const ICON_GO_NEXT: &Icon = &Icon {
    name: "go-next-symbolic",
    view: 16.0,
    body: r#"<path d="M4 2c0-.265625.105469-.519531.292969-.707031.390625-.390625 1.023437-.390625 1.414062 0l6 6C11.894531 7.480469 12 7.734375 12 8s-.105469.519531-.292969.707031l-6 6c-.390625.390625-1.023437.390625-1.414062 0C4.105469 14.519531 4 14.265625 4 14s.105469-.519531.292969-.707031L9.585938 8 4.292969 2.707031C4.105469 2.519531 4 2.265625 4 2m0 0"/>"#,
};

pub const ICON_VIEW_MORE: &Icon = &Icon {
    name: "view-more-symbolic",
    view: 16.0,
    body: r#"<path d="M7.996094 0c-1.105469 0-2 .894531-2 2s.894531 2 2 2c1.101562 0 2-.894531 2-2s-.898438-2-2-2m0 6c-1.105469 0-2 .894531-2 2s.894531 2 2 2c1.101562 0 2-.894531 2-2s-.898438-2-2-2m0 6c-1.105469 0-2 .894531-2 2s.894531 2 2 2c1.101562 0 2-.894531 2-2s-.898438-2-2-2m0 0"/>"#,
};

pub const ICON_FOLDER_LOUPE: &Icon = &Icon {
    name: "folder-loupe-symbolic",
    view: 16.0,
    body: r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M 13 13 L 15 15 M 13.779 11 C 13.779 12.6569, 12.4359 14, 10.779 14 C 9.12217 14, 7.77903 12.6569, 7.77903 11 C 7.77903 9.34315, 9.12217 8, 10.779 8 C 12.4359 8, 13.779 9.34315, 13.779 11 Z"/><path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M 15 7 L 15 5 C 15 3.89543, 14.1046 3, 13 3 L 9.55862 3 C 9.20094 3, 8.85791 2.85791, 8.605 2.605 L 7.395 1.395 C 7.14209 1.14209, 6.79906 1, 6.44138 1 L 3 1 C 1.89543 1, 1 1.89543, 1 3 L 1 11 C 1 12.1046, 1.89543 13, 3 13 L 5 13"/><path fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" d="M 1 5.49564 L 8.33474 5.49564 C 8.98625 5.49564, 9.51375 4.96626, 9.51145 4.31476 L 9.51034 3"/>"#,
};

pub const ICON_VIEW_LIST: &Icon = &Icon {
    name: "view-list-symbolic",
    view: 16.0,
    body: r#"<path d="M2 1c-.554688 0-1 .445312-1 1v2c0 .554688.445312 1 1 1h2c.554688 0 1-.445312 1-1V2c0-.554688-.445312-1-1-1zm5 1v2h8V2zM2 6.015625c-.554688 0-1 .445313-1 1v1.980469c0 .550781.445312 1 1 1h2c.554688 0 1-.449219 1-1V7.015625c0-.554687-.445312-1-1-1zM7 7v2h8V7zm-5 4c-.554688 0-1 .445312-1 1v1.980469c0 .550781.445312 1 1 1h2c.554688 0 1-.449219 1-1V12c0-.554688-.445312-1-1-1zm5 .984375v2h8v-2zm0 0"/>"#,
};

pub const ICON_VIEW_GRID: &Icon = &Icon {
    name: "view-grid-symbolic",
    view: 16.0,
    body: r#"<path d="M2 1c-.554688 0-1 .445312-1 1v4c0 .554688.445312 1 1 1h4c.554688 0 1-.445312 1-1V2c0-.554688-.445312-1-1-1zm8 0c-.554688 0-1 .445312-1 1v4c0 .554688.445312 1 1 1h4c.554688 0 1-.445312 1-1V2c0-.554688-.445312-1-1-1zM2 9c-.554688 0-1 .445312-1 1v4c0 .554688.445312 1 1 1h4c.554688 0 1-.445312 1-1v-4c0-.554688-.445312-1-1-1zm8 0c-.554688 0-1 .445312-1 1v4c0 .554688.445312 1 1 1h4c.554688 0 1-.445312 1-1v-4c0-.554688-.445312-1-1-1zm0 0"/>"#,
};

pub const ICON_PAN_DOWN: &Icon = &Icon {
    name: "pan-down-symbolic",
    view: 16.0,
    body: r#"<path d="m3.292969 7.707031 4 4c.390625.390625 1.023437.390625 1.414062 0l4-4c.390625-.390625.390625-1.023437 0-1.414062s-1.023437-.390625-1.414062 0L8 9.585938 4.707031 6.292969c-.390625-.390625-1.023437-.390625-1.414062 0s-.390625 1.023437 0 1.414062m0 0"/>"#,
};

pub const ICON_CLOSE: &Icon = &Icon {
    name: "window-close-symbolic",
    view: 16.0,
    body: r#"<path d="M4 4h1.03125c.253906.011719.511719.128906.6875.3125L8 6.59375l2.3125-2.28125c.265625-.230469.445312-.304688.6875-.3125h1v1c0 .285156-.035156.550781-.25.75L9.46875 8.03125l2.25 2.25c.1875.1875.28125.453125.28125.71875v1h-1c-.265625 0-.53125-.09375-.71875-.28125L8 9.4375l-2.28125 2.28125C5.53125 11.90625 5.265625 12 5 12H4v-1c0-.265625.09375-.53125.28125-.71875l2.28125-2.25L4.28125 5.75C4.070312 5.554688 3.976562 5.28125 4 5zm0 0"/>"#,
};

pub const ICON_FOLDER_NEW: &Icon = &Icon {
    name: "folder-new-symbolic",
    view: 16.0,
    body: r#"<path d="M3 1C1.355469 1 0 2.355469 0 4v8c0 1.644531 1.355469 3 3 3h3c.550781 0 1-.449219 1-1s-.449219-1-1-1H3c-.5625 0-1-.4375-1-1V5h11c.5625 0 1 .4375 1 1v1c0 .550781.449219 1 1 1s1-.449219 1-1V6c0-1.644531-1.355469-3-3-3H9.414062L7.707031 1.292969C7.519531 1.105469 7.265625 1 7 1zm0 2h3.585938l1 1H2c0-.5625.4375-1 1-1m8 5v3H8v2h3v3h2v-3h3v-2h-3V8zm0 0"/>"#,
};

pub const ICON_REFRESH: &Icon = &Icon {
    name: "view-refresh-symbolic",
    view: 16.0,
    body: r#"<path d="M7.40625 1c-.613281.007812-1.234375.089844-1.847656.253906C2.285156 2.132812 0 5.109375 0 8.5s2.285156 6.367188 5.558594 7.242188c3.273437.878906 6.742187-.558594 8.4375-3.492188.277344-.480469.109375-1.089844-.367188-1.367188-.476562-.273437-1.089844-.109374-1.367187.367188-1.246094 2.160156-3.777344 3.207031-6.1875 2.5625C3.667969 13.167969 2 10.992188 2 8.5c0-2.496094 1.667969-4.667969 4.074219-5.3125 2.410156-.644531 4.941406.402344 6.1875 2.5625.058593.085938.125.164062.203125.226562l-.019532.015626L12.4375 6H11c-.550781 0-1 .449219-1 1 0 0 0 1 1 1h5V3s.003906-1-1-1c-.550781 0-1 .449219-1 1v1.6875l-.015625.011719-.011719.011719c-1.277344-2.179688-3.53125-3.519532-5.953125-3.691407C7.816406 1.003906 7.613281 1 7.40625 1m0 0"/>"#,
};

pub const ICON_HARDDISK: &Icon = &Icon {
    name: "drive-harddisk-symbolic",
    view: 16.0,
    body: r#"<path d="M4 0C2.355469 0 1 1.355469 1 3v10c0 1.644531 1.355469 3 3 3h8c1.644531 0 3-1.355469 3-3V3c0-1.644531-1.355469-3-3-3zm0 2h8c.570312 0 1 .429688 1 1v9c0 .570312-.429688 1-1 1H4c-.554688 0-1-.445312-1-1V3c0-.554688.445312-1 1-1m4 1C5.789062 3 4 4.789062 4 7v4h4c2.5 0 4-1.789062 4-4s-1.789062-4-4-4m0 2c1.105469 0 2 .894531 2 2s-.894531 2-2 2-2-.894531-2-2 .894531-2 2-2m0 0"/>"#,
};

pub fn bookmark_icon(name: &str) -> &'static Icon {
    match name.to_lowercase().as_str() {
        "home" => ICON_USER_HOME,
        "recent" => ICON_RECENT,
        "starred" => ICON_STARRED,
        "network" => ICON_NETWORK,
        "trash" => ICON_TRASH,
        "downloads" => ICON_FOLDER_DOWNLOAD,
        "projects" | "code" | "work" | "repos" | "workspace" => ICON_FOLDER_PROJECTS,
        "pictures" | "photos" | "screenshots" => ICON_FOLDER_PICTURES,
        "videos" | "movies" => ICON_FOLDER_VIDEOS,
        "documents" | "docs" => ICON_FOLDER_DOCUMENTS,
        "music" | "audio" => ICON_FOLDER_MUSIC,
        _ => ICON_FOLDER,
    }
}

fn find_icon_png(themes: &[String], cat: &str, name: &str) -> Option<PathBuf> {
    let home_icons = crate::platform::data_dir().join("icons");
    let sizes = ["256x256", "256x256@2x", "128x128", "96x96", "64x64", "48x48"];
    for theme in themes {
        for base in [home_icons.as_path(), Path::new("/usr/share/icons")] {
            let root = base.join(theme);
            if !root.is_dir() {
                continue;
            }
            for size in sizes {
                let p = root.join(size).join(cat).join(format!("{name}.png"));
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Loads high-resolution PNG icons from the active GTK/Omarchy icon theme chain.
pub fn load_system_icons() -> HashMap<String, Pixels> {
    let mut map = HashMap::new();
    #[cfg(target_os = "linux")]
    {
        let themes = crate::platform::active_icon_theme();
        let places = [
            "folder",
            "folder-documents",
            "folder-download",
            "folder-music",
            "folder-pictures",
            "folder-videos",
            "folder-publicshare",
            "folder-templates",
            "user-desktop",
            "user-home",
            "user-trash",
        ];
        for key in places {
            if let Some(path) = find_icon_png(&themes, "places", key) {
                if let Ok(img) = image::open(&path) {
                    let rgba = img.into_rgba8();
                    let px = Pixels::new();
                    px.set(rgba.width(), rgba.height(), rgba.into_raw());
                    map.insert(key.to_string(), px);
                }
            }
        }
        let mimes = [
            "application-x-desktop",
            "application-x-executable",
            "application-octet-stream",
            "application-pdf",
            "application-archive",
            "application-x-archive",
            "application-zip",
            "application-x-shellscript",
            "application-javascript",
            "application-json",
            "application-typescript",
            "package-x-generic",
            "text-x-generic",
            "text-x-python",
            "text-x-csrc",
            "text-x-script",
            "image-x-generic",
            "audio-x-generic",
            "video-x-generic",
        ];
        for key in mimes {
            if let Some(path) = find_icon_png(&themes, "mimetypes", key) {
                if let Ok(img) = image::open(&path) {
                    let rgba = img.into_rgba8();
                    let px = Pixels::new();
                    px.set(rgba.width(), rgba.height(), rgba.into_raw());
                    map.insert(key.to_string(), px);
                }
            }
        }
    }
    map
}

pub fn entry_system_icon_key<'a>(
    entry: &Entry,
    icons: &HashMap<String, Pixels>,
) -> Option<&'static str> {
    if entry.is_dir {
        let candidate = match entry.name.as_str() {
            "Documents" => Some("folder-documents"),
            "Downloads" => Some("folder-download"),
            "Music" => Some("folder-music"),
            "Pictures" => Some("folder-pictures"),
            "Videos" => Some("folder-videos"),
            "Desktop" => Some("user-desktop"),
            "Public" => Some("folder-publicshare"),
            "Templates" => Some("folder-templates"),
            _ => None,
        };
        if let Some(k) = candidate {
            if icons.contains_key(k) {
                return Some(k);
            }
        }
        if icons.contains_key("folder") {
            return Some("folder");
        }
        return None;
    }

    let ext = entry
        .name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_lowercase();
    let candidates: &[&str] = match ext.as_str() {
        "desktop" => &["application-x-desktop", "application-x-executable", "text-x-generic"],
        "pdf" => &["application-pdf", "text-x-generic"],
        "zip" => &["application-zip", "application-archive", "package-x-generic"],
        "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" | "zst" => {
            &["application-x-archive", "application-archive", "package-x-generic"]
        }
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico" => &["image-x-generic"],
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "opus" => &["audio-x-generic"],
        "mp4" | "mkv" | "webm" | "mov" | "avi" => &["video-x-generic"],
        "sh" | "bash" | "zsh" | "fish" => &["application-x-shellscript", "text-x-script", "text-x-generic"],
        "py" => &["text-x-python", "text-x-script", "text-x-generic"],
        "js" | "mjs" | "cjs" => &["application-javascript", "text-x-generic"],
        "ts" | "tsx" => &["application-typescript", "text-x-generic"],
        "json" | "toml" | "yaml" | "yml" => &["application-json", "text-x-generic"],
        "c" | "h" | "cpp" | "cc" | "hpp" | "rs" | "go" | "java" => &["text-x-csrc", "text-x-generic"],
        "appimage" | "exe" | "bin" | "out" => &["application-x-executable", "application-octet-stream"],
        _ => &["text-x-generic", "application-octet-stream"],
    };
    for &k in candidates {
        if icons.contains_key(k) {
            return Some(k);
        }
    }
    None
}
