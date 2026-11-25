use std::path::{Path, PathBuf};

use clove_fs::scan::KIND_COUNT;
use clove_fs::Entry;
use clove_render::tokens;
use rune::tok::TextStyle;
use rune::{
    col, el, row, spacer, text, text_input, Align, Color, Cx, DragItem, El, ScrollSpec, Side,
    SurfaceFit, Tx,
};

use crate::icons::{
    self, bookmark_icon, ICON_CLOSE, ICON_EDIT_FIND, ICON_FOLDER, ICON_FOLDER_LOUPE,
    ICON_FOLDER_NEW, ICON_GO_NEXT, ICON_GO_PREV, ICON_HARDDISK, ICON_NETWORK, ICON_OPEN_MENU,
    ICON_PAN_DOWN, ICON_RECENT, ICON_REFRESH, ICON_STARRED, ICON_TRASH, ICON_USER_HOME,
    ICON_VIEW_GRID, ICON_VIEW_LIST, ICON_VIEW_MORE,
};
use crate::{
    apply_batch_rename, archive_entry_count, compare_dirs, count_files, create_symlink, entry_path,
    find_duplicates, go_back, go_forward, handle_address_key, handle_drop_files, handle_rename_key,
    human_gb, is_archive, is_starred, keybinds_help, launch_desktop_app, load_thumbnails, navigate,
    open_properties, open_with_candidates, platform, props_owner, props_permissions, rebuild_bar,
    refilter, reload, run_paste, run_progress, save_settings, srgb_to_linear, storage_cache_path,
    sync_address, toggle_star, trigger_search, unique_link_path, FileOpProgress, Location, State,
    BAR_H, ROW_H,
};

// ---------------------------------------------------------------- Libadwaita dark glass palette

const fn ts(size: f32, weight: f32) -> TextStyle {
    TextStyle::new(size, weight)
}

fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    Color {
        r: srgb_to_linear(r as f32 / 255.0),
        g: srgb_to_linear(g as f32 / 255.0),
        b: srgb_to_linear(b as f32 / 255.0),
        a,
    }
}

/// Linear-light alpha for white overlays over dark surfaces (prevents washed-out sRGB glare).
fn white_a(a: f32) -> Color {
    Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a,
    }
}

fn view_bg() -> Color {
    if cfg!(target_os = "linux") {
        rgba(0x16, 0x16, 0x19, 0.84)
    } else {
        rgba(0x18, 0x18, 0x1B, 1.0)
    }
}

fn sidebar_bg() -> Color {
    if cfg!(target_os = "linux") {
        rgba(0x25, 0x25, 0x2A, 0.88)
    } else {
        rgba(0x26, 0x26, 0x2B, 1.0)
    }
}

fn popover_bg() -> Color {
    rgba(0x2B, 0x2B, 0x30, 0.96)
}

fn card_bg() -> Color {
    rgba(0x24, 0x24, 0x29, 0.97)
}

fn input_bg() -> Color {
    rgba(0x16, 0x16, 0x19, 0.90)
}

fn ink() -> Color {
    rgba(0xF4, 0xF4, 0xF6, 1.0)
}

fn ink_secondary() -> Color {
    rgba(0xC0, 0xC0, 0xC8, 1.0)
}

fn ink_muted() -> Color {
    rgba(0x88, 0x88, 0x92, 1.0)
}

fn hairline() -> Color {
    white_a(0.038)
}

fn accent() -> Color {
    rgba(0x35, 0x84, 0xE4, 1.0)
}

fn hover_fill() -> Color {
    white_a(0.036)
}

fn press_fill() -> Color {
    white_a(0.065)
}

fn sidebar_selected_fill() -> Color {
    white_a(0.062)
}

fn grid_selected_fill() -> Color {
    white_a(0.072)
}

fn pathbar_bg() -> Color {
    white_a(0.055)
}

fn danger() -> Color {
    rgba(0xF6, 0x61, 0x51, 1.0)
}

fn modal_scrim() -> Color {
    Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.55,
    }
}

fn color_rgb(rgb: [f32; 3], a: f32) -> Color {
    Color {
        r: srgb_to_linear(rgb[0]),
        g: srgb_to_linear(rgb[1]),
        b: srgb_to_linear(rgb[2]),
        a,
    }
}

// ---------------------------------------------------------------- formatting helpers

pub fn format_time(time: Option<std::time::SystemTime>) -> String {
    let Some(t) = time else { return "—".into() };
    let Ok(dur) = t.duration_since(std::time::UNIX_EPOCH) else { return "—".into() };
    let secs = dur.as_secs();
    let days_since = secs / 86400;
    let rem = secs % 86400;
    let hh = rem / 3600;
    let mm = (rem % 3600) / 60;
    let mut y = 1970i64;
    let mut d = days_since as i64;
    loop {
        let leap = (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0);
        let diy = if leap { 366 } else { 365 };
        if d < diy {
            break;
        }
        d -= diy;
        y += 1;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0);
    let mdays = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut m = 0usize;
    while m < 12 && d >= mdays[m] {
        d -= mdays[m];
        m += 1;
    }
    format!("{:04}-{:02}-{:02} {:02}:{:02}", y, m + 1, d + 1, hh, mm)
}

pub fn format_size(size: u64) -> String {
    if size >= 1_000_000_000 {
        format!("{:.1} GB", size as f64 / 1e9)
    } else if size >= 1_000_000 {
        format!("{:.1} MB", size as f64 / 1e6)
    } else if size >= 1_000 {
        format!("{:.1} KB", size as f64 / 1e3)
    } else {
        format!("{size} B")
    }
}

pub fn file_kind_label(entry: &Entry) -> &'static str {
    if entry.is_dir {
        return "Folder";
    }
    let ext = entry.name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico" => "Image",
        "mp4" | "mkv" | "webm" | "mov" | "avi" => "Video",
        "mp3" | "wav" | "flac" | "ogg" | "m4a" => "Audio",
        "pdf" => "PDF",
        "txt" | "md" | "doc" | "docx" | "rtf" | "csv" => "Document",
        "zip" | "tar" | "gz" | "xz" | "7z" | "rar" => "Archive",
        "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "c" | "cpp" | "h" | "json" | "toml"
        | "yaml" | "yml" | "sh" | "html" | "css" => "Code",
        "desktop" => "Desktop Entry",
        "exe" | "appimage" => "Application",
        _ => "File",
    }
}

fn entry_resolved_path(s: &State, idx: usize) -> Option<PathBuf> {
    if let Some(p) = s.virtual_paths.get(idx) {
        return Some(p.clone());
    }
    let entry = s.snapshot.entries.get(idx)?;
    entry_path(&s.location, entry)
}

fn close_popovers(s: &mut State) {
    s.main_menu_open = false;
    s.view_menu_open = false;
    s.path_menu_open = false;
    s.context_menu = None;
}

fn open_entry(s: &mut State, idx: usize) {
    let Some(entry) = s.snapshot.entries.get(idx).cloned() else { return };
    let Some(target) = entry_resolved_path(s, idx) else { return };
    if entry.is_dir {
        navigate(s, Location::Folder(target));
        sync_address(s);
    } else {
        platform::open_path(&target);
        s.status = format!("Opened {}", entry.name);
    }
}

// ---------------------------------------------------------------- sidebar

fn sidebar_row_el(
    key: String,
    icon_svg: &'static rune::text::Icon,
    label: String,
    active: bool,
    on: impl Fn(&mut State) + 'static,
) -> El {
    let text_color = if active { ink() } else { ink_secondary() };
    let icon_color = if active { ink() } else { rgba(0xE8, 0xE8, 0xEC, 0.90) };
    row()
        .key(key)
        .label(label.clone())
        .w_full()
        .h(36.0)
        .px(10.0)
        .radius(tokens::RADIUS_ROW)
        .gap(12.0)
        .center_y()
        .fill(if active { sidebar_selected_fill() } else { Color::TRANSPARENT })
        .hover(if active { white_a(0.085) } else { hover_fill() })
        .press(press_fill())
        .child(rune::icon(icon_svg, 16.0).color(icon_color))
        .child(
            text(label)
                .font(ts(13.5, if active { 500.0 } else { 400.0 }))
                .color(text_color)
                .single_line(),
        )
        .on_press(on)
}

fn sidebar_el(s: &State) -> El {
    let sidebar_w = (s.win_width * 0.20).clamp(210.0, 276.0);
    let home = platform::home_dir();
    let trash_dir = home.join(".local/share/Trash/files");

    let is_home = matches!(&s.location, Location::Folder(p) if p == &home);
    let is_recent = matches!(&s.location, Location::Recent);
    let is_starred = matches!(&s.location, Location::Starred);
    let is_network = matches!(&s.location, Location::Network);
    let is_trash = matches!(&s.location, Location::Folder(p) if p == &trash_dir);

    let bookmarks = platform::gtk_bookmarks();
    let mounts: Vec<(String, PathBuf)> = platform::mount_entries()
        .into_iter()
        .filter(|(_, p)| p != Path::new("/"))
        .collect();

    let mut list = col()
        .key("sidebar-list")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(6.0)
        .pt(2.0)
        .pb(8.0)
        .gap(2.0)
        .scroll(ScrollSpec { stick_bottom: false });

    // Core Nautilus places
    list = list
        .child(sidebar_row_el(
            "nav-home".into(),
            ICON_USER_HOME,
            "Home".into(),
            is_home,
            move |st| {
                close_popovers(st);
                navigate(st, Location::Folder(home.clone()));
                sync_address(st);
            },
        ))
        .child(sidebar_row_el(
            "nav-recent".into(),
            ICON_RECENT,
            "Recent".into(),
            is_recent,
            |st| {
                close_popovers(st);
                navigate(st, Location::Recent);
                sync_address(st);
            },
        ))
        .child(sidebar_row_el(
            "nav-starred".into(),
            ICON_STARRED,
            "Starred".into(),
            is_starred,
            |st| {
                close_popovers(st);
                navigate(st, Location::Starred);
                sync_address(st);
            },
        ))
        .child(sidebar_row_el(
            "nav-network".into(),
            ICON_NETWORK,
            "Network".into(),
            is_network,
            |st| {
                close_popovers(st);
                navigate(st, Location::Network);
                sync_address(st);
            },
        ))
        .child(sidebar_row_el(
            "nav-trash".into(),
            ICON_TRASH,
            "Trash".into(),
            is_trash,
            move |st| {
                close_popovers(st);
                let td = platform::home_dir().join(".local/share/Trash/files");
                if !td.exists() {
                    let _ = std::fs::create_dir_all(&td);
                }
                navigate(st, Location::Folder(td));
                sync_address(st);
            },
        ));

    // Separator before GTK bookmarks
    if !bookmarks.is_empty() {
        list = list.child(
            el().key("sb-sep-1")
                .w_full()
                .py(6.0)
                .px(4.0)
                .child(el().w_full().h(1.0).fill(hairline())),
        );
        for (i, (label, path)) in bookmarks.into_iter().enumerate() {
            let active = matches!(&s.location, Location::Folder(p) if p == &path);
            let icon = bookmark_icon(&label);
            let target = path.clone();
            list = list.child(sidebar_row_el(
                format!("bm-{i}"),
                icon,
                label,
                active,
                move |st| {
                    close_popovers(st);
                    navigate(st, Location::Folder(target.clone()));
                    sync_address(st);
                },
            ));
        }
    }

    // Removable / external drives (only if present)
    if !mounts.is_empty() {
        list = list.child(
            el().key("sb-sep-2")
                .w_full()
                .py(6.0)
                .px(4.0)
                .child(el().w_full().h(1.0).fill(hairline())),
        );
        for (i, (label, path)) in mounts.into_iter().enumerate() {
            let active = matches!(&s.location, Location::Folder(p) if p == &path);
            let target = path.clone();
            list = list.child(sidebar_row_el(
                format!("mnt-{i}"),
                ICON_HARDDISK,
                label,
                active,
                move |st| {
                    close_popovers(st);
                    navigate(st, Location::Folder(target.clone()));
                    sync_address(st);
                },
            ));
        }
    }

    col()
        .key("sidebar")
        .w(sidebar_w)
        .h_full()
        .fill(sidebar_bg())
        .child(
            // Sidebar headerbar: search | Files | hamburger menu
            row()
                .key("sidebar-header")
                .w_full()
                .h(47.0)
                .px(8.0)
                .center_y()
                .child(
                    el().key("btn-sb-search")
                        .label("Search")
                        .size(34.0, 34.0)
                        .radius(tokens::RADIUS_CONTROL)
                        .center()
                        .fill(if s.search_mode { hover_fill() } else { Color::TRANSPARENT })
                        .hover(hover_fill())
                        .press(press_fill())
                        .child(rune::icon(ICON_EDIT_FIND, 16.0).color(ink()))
                        .on_press(|st: &mut State| {
                            close_popovers(st);
                            st.search_mode = !st.search_mode;
                            if st.search_mode {
                                st.focus_target = Some("search-box".to_string());
                            }
                            save_settings(st);
                        }),
                )
                .child(
                    row()
                        .grow(1.0)
                        .h_full()
                        .center()
                        .window_drag()
                        .child(text("Files").font(ts(15.0, 700.0)).color(ink())),
                )
                .child(
                    el().key("btn-main-menu")
                        .label("Main Menu")
                        .size(34.0, 34.0)
                        .radius(tokens::RADIUS_CONTROL)
                        .center()
                        .fill(if s.main_menu_open { hover_fill() } else { Color::TRANSPARENT })
                        .hover(hover_fill())
                        .press(press_fill())
                        .child(rune::icon(ICON_OPEN_MENU, 16.0).color(ink()))
                        .on_press(|st: &mut State| {
                            let next = !st.main_menu_open;
                            close_popovers(st);
                            st.main_menu_open = next;
                        }),
                ),
        )
        .child(list)
}

// ---------------------------------------------------------------- pathbar & content headerbar

fn pathbar_el(s: &State) -> El {
    let home = platform::home_dir();
    let trash_dir = home.join(".local/share/Trash/files");

    let mut crumbs_row = row()
        .key("pathbar-crumbs")
        .grow(1.0)
        .h(28.0)
        .gap(2.0)
        .center_y()
        .on_background_press(|st: &mut State| {
            close_popovers(st);
            st.edit_address = true;
            st.focus_target = Some("addr-input".to_string());
        });

    match &s.location {
        Location::Recent => {
            crumbs_row = crumbs_row.child(
                row()
                    .px(10.0)
                    .h(28.0)
                    .radius(7.0)
                    .gap(6.0)
                    .center_y()
                    .child(rune::icon(ICON_RECENT, 16.0).color(ink()))
                    .child(text("Recent").font(ts(13.5, 600.0)).color(ink())),
            );
        }
        Location::Starred => {
            crumbs_row = crumbs_row.child(
                row()
                    .px(10.0)
                    .h(28.0)
                    .radius(7.0)
                    .gap(6.0)
                    .center_y()
                    .child(rune::icon(ICON_STARRED, 16.0).color(ink()))
                    .child(text("Starred").font(ts(13.5, 600.0)).color(ink())),
            );
        }
        Location::Network => {
            crumbs_row = crumbs_row.child(
                row()
                    .px(10.0)
                    .h(28.0)
                    .radius(7.0)
                    .gap(6.0)
                    .center_y()
                    .child(rune::icon(ICON_NETWORK, 16.0).color(ink()))
                    .child(text("Network").font(ts(13.5, 600.0)).color(ink())),
            );
        }
        Location::ThisPc => {
            crumbs_row = crumbs_row.child(
                row()
                    .px(10.0)
                    .h(28.0)
                    .radius(7.0)
                    .gap(6.0)
                    .center_y()
                    .child(rune::icon(ICON_HARDDISK, 16.0).color(ink()))
                    .child(text("This PC").font(ts(13.5, 600.0)).color(ink())),
            );
        }
        Location::Folder(dir) => {
            if dir == &trash_dir {
                crumbs_row = crumbs_row.child(
                    row()
                        .px(10.0)
                        .h(28.0)
                        .radius(7.0)
                        .gap(6.0)
                        .center_y()
                        .child(rune::icon(ICON_TRASH, 16.0).color(ink()))
                        .child(text("Trash").font(ts(13.5, 600.0)).color(ink())),
                );
            } else {
                let mut segments: Vec<(String, PathBuf, bool)> = Vec::new();
                if let Ok(rel) = dir.strip_prefix(&home) {
                    segments.push(("Home".to_string(), home.clone(), true));
                    let mut cur = home.clone();
                    for comp in rel.components() {
                        cur.push(comp.as_os_str());
                        segments.push((comp.as_os_str().to_string_lossy().to_string(), cur.clone(), false));
                    }
                } else {
                    let mut cur = PathBuf::new();
                    for (idx, comp) in dir.components().enumerate() {
                        cur.push(comp.as_os_str());
                        let label = if idx == 0 && comp.as_os_str() == "/" {
                            "Computer".to_string()
                        } else {
                            comp.as_os_str().to_string_lossy().to_string()
                        };
                        segments.push((label, cur.clone(), false));
                    }
                }
                let count = segments.len();
                let start = count.saturating_sub(4);
                if start > 0 {
                    crumbs_row = crumbs_row.child(
                        el().px(6.0).h(28.0).center()
                            .child(text("…").font(ts(13.0, 400.0)).color(ink_muted())),
                    );
                }
                for (i, (label, path, is_home)) in segments.into_iter().enumerate().skip(start) {
                    let is_last = i + 1 == count;
                    if i > start {
                        crumbs_row = crumbs_row.child(
                            text("/").font(ts(13.0, 400.0)).color(white_a(0.18)),
                        );
                    }
                    let mut btn = row()
                        .key(format!("crumb-{i}"))
                        .px(9.0)
                        .h(28.0)
                        .radius(7.0)
                        .gap(6.0)
                        .center_y()
                        .hover(hover_fill())
                        .press(press_fill());
                    if is_home {
                        btn = btn.child(
                            rune::icon(ICON_USER_HOME, 16.0)
                                .color(if is_last { ink() } else { ink_secondary() }),
                        );
                    }
                    btn = btn.child(
                        text(label)
                            .font(ts(13.5, if is_last { 600.0 } else { 400.0 }))
                            .color(if is_last { ink() } else { ink_secondary() })
                            .single_line(),
                    );
                    if !is_last {
                        let target = path.clone();
                        btn = btn.on_press(move |st: &mut State| {
                            close_popovers(st);
                            navigate(st, Location::Folder(target.clone()));
                            sync_address(st);
                        });
                    } else {
                        btn = btn.on_press(|st: &mut State| {
                            let next = !st.path_menu_open;
                            close_popovers(st);
                            st.path_menu_open = next;
                        });
                    }
                    crumbs_row = crumbs_row.child(btn);
                }
            }
        }
    }

    row()
        .key("pathbar")
        .grow(1.0)
        .h(34.0)
        .radius(tokens::RADIUS_PATHBAR)
        .fill(pathbar_bg())
        .px(4.0)
        .center_y()
        .child(crumbs_row)
        .child(
            el().key("btn-path-menu")
                .label("Current Folder Menu")
                .size(28.0, 28.0)
                .radius(7.0)
                .center()
                .fill(if s.path_menu_open { hover_fill() } else { Color::TRANSPARENT })
                .hover(hover_fill())
                .press(press_fill())
                .child(rune::icon(ICON_VIEW_MORE, 16.0).color(ink_secondary()))
                .on_press(|st: &mut State| {
                    let next = !st.path_menu_open;
                    close_popovers(st);
                    st.path_menu_open = next;
                }),
        )
}

fn headerbar_el(s: &State) -> El {
    let sidebar_w = (s.win_width * 0.20).clamp(210.0, 276.0);
    let main_w = (s.win_width - sidebar_w).max(400.0);
    let toolbar_w = (main_w * 0.56).clamp(320.0, 820.0);

    let can_back = !s.history_back.is_empty();
    let can_fwd = !s.history_forward.is_empty();
    let is_grid = s.view_mode == 2;

    row()
        .key("headerbar")
        .w_full()
        .h(47.0)
        .px(8.0)
        .gap(6.0)
        .center_y()
        // History controls (< >)
        .child(
            row()
                .key("history-controls")
                .gap(2.0)
                .center_y()
                .child(
                    el().key("btn-back")
                        .label("Back")
                        .size(34.0, 34.0)
                        .radius(tokens::RADIUS_CONTROL)
                        .center()
                        .hover(if can_back { hover_fill() } else { Color::TRANSPARENT })
                        .press(if can_back { press_fill() } else { Color::TRANSPARENT })
                        .child(
                            rune::icon(ICON_GO_PREV, 16.0)
                                .color(if can_back { ink() } else { white_a(0.14) }),
                        )
                        .on_press(|st: &mut State| {
                            close_popovers(st);
                            go_back(st);
                        }),
                )
                .child(
                    el().key("btn-fwd")
                        .label("Forward")
                        .size(34.0, 34.0)
                        .radius(tokens::RADIUS_CONTROL)
                        .center()
                        .hover(if can_fwd { hover_fill() } else { Color::TRANSPARENT })
                        .press(if can_fwd { press_fill() } else { Color::TRANSPARENT })
                        .child(
                            rune::icon(ICON_GO_NEXT, 16.0)
                                .color(if can_fwd { ink() } else { white_a(0.14) }),
                        )
                        .on_press(|st: &mut State| {
                            close_popovers(st);
                            go_forward(st);
                        }),
                ),
        )
        .child(spacer().h_full().window_drag())
        // Centered NautilusPathBar + search button (or active Location/Search bar)
        .child(
            row()
                .key("header-center")
                .w(toolbar_w)
                .gap(6.0)
                .center_y()
                .child(if s.edit_address {
                    row()
                        .key("addr-box")
                        .grow(1.0)
                        .h(34.0)
                        .px(12.0)
                        .gap(8.0)
                        .radius(tokens::RADIUS_PATHBAR)
                        .fill(white_a(0.08))
                        .border(1.0, white_a(0.15))
                        .center_y()
                        .child(
                            text_input(&s.address, ts(13.5, 400.0))
                                .key("addr-input")
                                .label("Location")
                                .single_line()
                                .grow(1.0)
                                .color(ink())
                                .placeholder("Enter folder path...", ink_muted())
                                .on_edit(|st: &mut State| &mut st.address)
                                .on_input_key(handle_address_key),
                        )
                        .child(
                            el().key("addr-cancel")
                                .size(22.0, 22.0)
                                .radius(999.0)
                                .center()
                                .hover(hover_fill())
                                .child(rune::icon(ICON_CLOSE, 12.0).color(ink_secondary()))
                                .on_press(|st: &mut State| {
                                    st.edit_address = false;
                                    sync_address(st);
                                }),
                        )
                } else if s.search_mode {
                    row()
                        .key("search-bar-inline")
                        .grow(1.0)
                        .h(34.0)
                        .px(12.0)
                        .gap(8.0)
                        .radius(tokens::RADIUS_PATHBAR)
                        .fill(white_a(0.08))
                        .border(1.0, white_a(0.15))
                        .center_y()
                        .child(rune::icon(ICON_EDIT_FIND, 15.0).color(ink_secondary()))
                        .child(
                            text_input(&s.search_query, ts(13.5, 400.0))
                                .key("search-box")
                                .label("Search files")
                                .single_line()
                                .grow(1.0)
                                .color(ink())
                                .placeholder("Search current folder...", ink_muted())
                                .on_edit(|st: &mut State| &mut st.search_query)
                                .on_input_change(|st: &mut State, _| trigger_search(st)),
                        )
                        .child(
                            el().key("search-close")
                                .size(22.0, 22.0)
                                .radius(999.0)
                                .center()
                                .hover(hover_fill())
                                .child(rune::icon(ICON_CLOSE, 12.0).color(ink_secondary()))
                                .on_press(|st: &mut State| {
                                    st.search_mode = false;
                                    st.search_query.text.clear();
                                    st.search_results.clear();
                                }),
                        )
                } else {
                    pathbar_el(s)
                })
                .child(
                    el().key("btn-folder-loupe")
                        .label("Search Current Folder")
                        .size(34.0, 34.0)
                        .radius(tokens::RADIUS_CONTROL)
                        .center()
                        .fill(if s.search_mode { hover_fill() } else { Color::TRANSPARENT })
                        .hover(hover_fill())
                        .press(press_fill())
                        .child(rune::icon(ICON_FOLDER_LOUPE, 16.0).color(ink()))
                        .on_press(|st: &mut State| {
                            close_popovers(st);
                            st.search_mode = !st.search_mode;
                            if st.search_mode {
                                st.edit_address = false;
                                st.focus_target = Some("search-box".to_string());
                            }
                            save_settings(st);
                        }),
                ),
        )
        .child(spacer().h_full().window_drag())
        // View SplitButton + Close button
        .child(
            row()
                .key("header-right")
                .gap(8.0)
                .center_y()
                .child(
                    row()
                        .key("view-split-button")
                        .h(34.0)
                        .radius(tokens::RADIUS_CONTROL)
                        .center_y()
                        .child(
                            el().key("btn-view-toggle")
                                .label(if is_grid { "List View" } else { "Grid View" })
                                .size(32.0, 34.0)
                                .radius(tokens::RADIUS_CONTROL)
                                .center()
                                .hover(hover_fill())
                                .press(press_fill())
                                .child(
                                    rune::icon(
                                        if is_grid { ICON_VIEW_LIST } else { ICON_VIEW_GRID },
                                        16.0,
                                    )
                                    .color(ink()),
                                )
                                .on_press(|st: &mut State| {
                                    close_popovers(st);
                                    st.view_mode = if st.view_mode == 2 { 0 } else { 2 };
                                    save_settings(st);
                                    if st.view_mode == 2 {
                                        load_thumbnails(st);
                                    }
                                }),
                        )
                        .child(el().w(1.0).h(16.0).fill(white_a(0.06)))
                        .child(
                            el().key("btn-view-menu")
                                .label("View Options")
                                .size(24.0, 34.0)
                                .radius(tokens::RADIUS_CONTROL)
                                .center()
                                .fill(if s.view_menu_open { hover_fill() } else { Color::TRANSPARENT })
                                .hover(hover_fill())
                                .press(press_fill())
                                .child(rune::icon(ICON_PAN_DOWN, 14.0).color(ink_secondary()))
                                .on_press(|st: &mut State| {
                                    let next = !st.view_menu_open;
                                    close_popovers(st);
                                    st.view_menu_open = next;
                                }),
                        ),
                )
                .child(
                    el().key("btn-win-close")
                        .label("Close")
                        .size(24.0, 24.0)
                        .radius(999.0)
                        .fill(white_a(0.055))
                        .hover(white_a(0.11))
                        .press(white_a(0.18))
                        .center()
                        .child(rune::icon(ICON_CLOSE, 12.0).color(ink()))
                        .on_press_tx(|_st: &mut State, tx: &mut Tx| {
                            tx.close_window();
                        }),
                ),
        )
}

// ---------------------------------------------------------------- grid & list views

fn grid_cell_el(s: &State, i: usize, idx: usize) -> El {
    let entry = &s.snapshot.entries[idx];
    let is_selected = s.selected.contains(&idx);
    let renaming = s.renaming == Some(idx);
    let path = entry_resolved_path(s, idx);
    let is_dir = entry.is_dir;
    let icon_box = s.icon_size;
    let cell_w = icon_box + 36.0;

    let thumb_px = path.as_ref().and_then(|p| s.thumbnails.get(p));
    let sys_px = icons::entry_system_icon_key(entry, &s.system_icons)
        .and_then(|k| s.system_icons.get(k));

    let icon_el = if let Some((w, h, rgba_buf)) = thumb_px {
        let px = rune::Pixels::new();
        px.set(*w, *h, rgba_buf.clone());
        el().size(icon_box, icon_box)
            .radius(8.0)
            .clip_rounded(8.0)
            .center()
            .pixels_fit(&px, SurfaceFit::Fit)
    } else if let Some(px) = sys_px {
        el().size(icon_box, icon_box)
            .center()
            .pixels_fit(px, SurfaceFit::Fit)
    } else {
        el().size(icon_box, icon_box).center().child(
            rune::icon(
                if is_dir { ICON_FOLDER } else { icons::ICON_FOLDER_DOCUMENTS },
                (icon_box * 0.68).round(),
            )
            .color(if is_dir { rgba(0xE9, 0x54, 0x20, 1.0) } else { ink_secondary() }),
        )
    };

    let name_color = if entry.hidden {
        white_a(0.35)
    } else {
        ink()
    };

    let mut cell = col()
        .key(format!("cell-{idx}"))
        .w(cell_w)
        .padding(6.0)
        .gap(6.0)
        .radius(tokens::RADIUS_CARD)
        .items_center()
        .fill(if is_selected {
            grid_selected_fill()
        } else {
            Color::TRANSPARENT
        })
        .hover(if is_selected {
            white_a(0.095)
        } else {
            hover_fill()
        })
        .press(press_fill())
        .child(icon_el);

    if renaming {
        cell = cell.child(
            el().w_full()
                .px(6.0)
                .py(2.0)
                .radius(6.0)
                .fill(input_bg())
                .border(1.0, accent())
                .child(
                    text_input(&s.rename_text, ts(13.0, 400.0))
                        .key(format!("rename-{idx}"))
                        .label("Rename")
                        .single_line()
                        .w_full()
                        .color(ink())
                        .on_edit(|st: &mut State| &mut st.rename_text)
                        .on_input_key(handle_rename_key),
                ),
        );
    } else {
        cell = cell.child(
            el().w_full().px(2.0).justify_center().child(
                text(entry.name.clone())
                    .font(ts(13.5, 400.0))
                    .color(name_color)
                    .w_full()
                    .wrap()
                    .max_lines(3)
                    .align(Align::Center),
            ),
        );
    }

    cell = cell
        .on_press(move |st: &mut State| {
            close_popovers(st);
            st.selected.clear();
            st.selected.insert(idx);
            st.cursor = i;
        })
        .on_double_press(move |st: &mut State| {
            close_popovers(st);
            open_entry(st, idx);
        })
        .on_secondary_press(move |st: &mut State, _, _| {
            close_popovers(st);
            if !st.selected.contains(&idx) {
                st.selected.clear();
                st.selected.insert(idx);
                st.cursor = i;
            }
            st.context_menu = Some(idx);
        });

    if let Some(p) = path {
        cell = cell.drag_out(move |_: &State| {
            let mut item = DragItem::new();
            item = item.file(p.clone());
            Some(item)
        });
    }

    cell
}

fn list_row_el(s: &State, i: usize, idx: usize) -> El {
    let entry = &s.snapshot.entries[idx];
    let is_selected = s.selected.contains(&idx);
    let path = entry_resolved_path(s, idx);
    let sys_px = icons::entry_system_icon_key(entry, &s.system_icons)
        .and_then(|k| s.system_icons.get(k));

    let small_icon = if let Some(px) = sys_px {
        el().size(24.0, 24.0).center().pixels_fit(px, SurfaceFit::Fit)
    } else {
        el().size(24.0, 24.0).center().child(
            rune::icon(
                if entry.is_dir { ICON_FOLDER } else { icons::ICON_FOLDER_DOCUMENTS },
                18.0,
            )
            .color(ink_secondary()),
        )
    };

    let size_text = if entry.is_dir {
        "—".to_string()
    } else {
        format_size(entry.size)
    };
    let name_color = if entry.hidden { white_a(0.35) } else { ink() };

    let mut r = row()
        .key(format!("row-{idx}"))
        .w_full()
        .h(ROW_H + 8.0)
        .px(12.0)
        .gap(12.0)
        .radius(8.0)
        .center_y()
        .fill(if is_selected {
            grid_selected_fill()
        } else {
            Color::TRANSPARENT
        })
        .hover(if is_selected { white_a(0.095) } else { hover_fill() })
        .press(press_fill())
        .child(small_icon)
        .child(
            text(entry.name.clone())
                .font(ts(13.5, 400.0))
                .color(name_color)
                .single_line()
                .grow(1.0),
        )
        .child(
            el().w(88.0).child(
                text(file_kind_label(entry))
                    .font(ts(12.0, 400.0))
                    .color(ink_muted())
                    .align(Align::End),
            ),
        )
        .child(
            el().w(84.0).child(
                text(size_text)
                    .font(ts(12.0, 400.0))
                    .color(ink_muted())
                    .align(Align::End),
            ),
        )
        .child(
            el().w(130.0).child(
                text(format_time(entry.modified))
                    .font(ts(12.0, 400.0))
                    .color(ink_muted())
                    .align(Align::End),
            ),
        )
        .on_press(move |st: &mut State| {
            close_popovers(st);
            st.selected.clear();
            st.selected.insert(idx);
            st.cursor = i;
        })
        .on_double_press(move |st: &mut State| {
            close_popovers(st);
            open_entry(st, idx);
        })
        .on_secondary_press(move |st: &mut State, _, _| {
            close_popovers(st);
            if !st.selected.contains(&idx) {
                st.selected.clear();
                st.selected.insert(idx);
                st.cursor = i;
            }
            st.context_menu = Some(idx);
        });

    if let Some(p) = path {
        r = r.drag_out(move |_: &State| {
            let mut item = DragItem::new();
            item = item.file(p.clone());
            Some(item)
        });
    }
    r
}

fn empty_state_el(s: &State) -> El {
    let (icon, title, subtitle) = match &s.location {
        Location::Network => (
            ICON_NETWORK,
            "No Network Locations",
            "Mounted remote locations will appear here",
        ),
        Location::Starred => (
            ICON_STARRED,
            "Starred Files Will Appear Here",
            "Right-click any item to star it for quick access",
        ),
        Location::Recent => (
            ICON_RECENT,
            "No Recent Files",
            "Recently modified files across your folders appear here",
        ),
        _ => (
            ICON_FOLDER,
            "Folder is Empty",
            "Drop files here or use the menu to create one",
        ),
    };
    col()
        .key("empty-state")
        .w_full()
        .grow(1.0)
        .center()
        .gap(10.0)
        .child(rune::icon(icon, 56.0).color(white_a(0.14)))
        .child(text(title).font(ts(18.0, 700.0)).color(ink_secondary()))
        .child(text(subtitle).font(ts(13.0, 400.0)).color(ink_muted()))
}

// ---------------------------------------------------------------- minimal animated Bayer storage dock

fn storage_dock_el(s: &State) -> El {
    if !s.storage_bar_open {
        return el().key("storage-off").w_full().h(0.0);
    }

    let capacity = s.storage_capacity;
    let used = s.storage_used;
    let pct = if capacity > 0 {
        (used as f64 / capacity as f64) * 100.0
    } else {
        0.0
    };

    let summary = if capacity > 0 {
        format!("{} / {} ({:.0}%)", human_gb(used), human_gb(capacity), pct)
    } else if s.storage_ready {
        format!("{} used", human_gb(used))
    } else {
        "Measuring storage…".to_string()
    };

    let mut dock = col()
        .key("storage-dock")
        .w_full()
        .px(18.0)
        .pt(4.0)
        .pb(12.0)
        .gap(6.0);

    // Optional expanded 9-kind breakdown card right above the thin bar
    if s.storage_expanded {
        let mut legend = row().w_full().flex_wrap().gaps(8.0, 14.0);
        for k in 0..KIND_COUNT {
            let bytes = s.storage_bytes[k];
            let has_kind = bytes > 0 && s.storage_has_data;
            let text_str = if has_kind {
                format!("{} · {}", tokens::STORAGE_KIND_NAMES[k], human_gb(bytes))
            } else {
                tokens::STORAGE_KIND_NAMES[k].to_string()
            };
            legend = legend.child(
                row()
                    .key(format!("leg-{k}"))
                    .gap(6.0)
                    .center_y()
                    .child(
                        el().size(8.0, 8.0)
                            .radius(999.0)
                            .fill(color_rgb(
                                tokens::STORAGE_KIND_RGB[k],
                                if has_kind { 1.0 } else { 0.35 },
                            )),
                    )
                    .child(
                        text(text_str)
                            .font(ts(12.0, 400.0))
                            .color(if has_kind { ink_secondary() } else { ink_muted() }),
                    ),
            );
        }

        dock = dock.child(
            col()
                .key("storage-expanded-card")
                .w_full()
                .padding(12.0)
                .gap(10.0)
                .radius(tokens::RADIUS_CARD)
                .fill(popover_bg())
                .border(1.0, hairline())
                .child(
                    row()
                        .w_full()
                        .center_y()
                        .gap(8.0)
                        .child(text("Disk Breakdown").font(ts(13.0, 600.0)).color(ink()))
                        .child(text(summary.clone()).font(ts(12.0, 400.0)).color(ink_muted()))
                        .child(spacer())
                        .child(
                            row()
                                .key("btn-bar-rescan")
                                .px(8.0)
                                .h(24.0)
                                .gap(5.0)
                                .radius(6.0)
                                .fill(hover_fill())
                                .hover(white_a(0.08))
                                .center()
                                .child(rune::icon(ICON_REFRESH, 12.0).color(ink_secondary()))
                                .child(text("Rescan").font(ts(11.5, 500.0)).color(ink_secondary()))
                                .on_press(|st: &mut State| {
                                    if let Some(p) = storage_cache_path() {
                                        let _ = std::fs::remove_file(p);
                                    }
                                    st.storage_bytes = [0; KIND_COUNT];
                                    st.storage_capacity = 0;
                                    st.storage_used = 0;
                                    st.storage_ready = false;
                                    st.storage_has_data = false;
                                    st.scan = None;
                                    st.scan_started = false;
                                    st.wake_bar();
                                    rebuild_bar(st);
                                }),
                        )
                        .child(
                            el().key("btn-bar-hide")
                                .px(8.0)
                                .h(24.0)
                                .radius(6.0)
                                .fill(hover_fill())
                                .hover(white_a(0.08))
                                .center()
                                .child(text("Hide").font(ts(11.5, 500.0)).color(ink_secondary()))
                                .on_press(|st: &mut State| {
                                    st.storage_bar_open = false;
                                    st.storage_expanded = false;
                                }),
                        ),
                )
                .child(legend),
        );
    }

    // Minimal always-neat dithered footer strip
    dock = dock
        .child(
            row()
                .key("storage-strip-meta")
                .w_full()
                .center_y()
                .gap(8.0)
                .child(text("Storage").font(ts(11.5, 600.0)).color(ink_secondary()))
                .child(text(summary).font(ts(11.5, 400.0)).color(ink_muted()))
                .child(spacer())
                .child(
                    text(if s.storage_expanded { "Hide details ▾" } else { "Details ▴" })
                        .font(ts(11.5, 500.0))
                        .color(ink_muted()),
                )
                .on_press(|st: &mut State| {
                    close_popovers(st);
                    st.storage_expanded = !st.storage_expanded;
                }),
        )
        .child(
            el().key("storage-bar-canvas")
                .w_full()
                .h(BAR_H)
                .radius(tokens::RADIUS_BAR)
                .clip_rounded(tokens::RADIUS_BAR)
                .fill(Color { r: 0.0, g: 0.0, b: 0.0, a: 0.30 })
                .on_layout(|st: &mut State, [_, _, w, _]| {
                    if (st.bar_width - w).abs() >= 0.5 && w > 10.0 {
                        st.bar_width = w;
                        rebuild_bar(st);
                    }
                })
                .pixels_fit(&s.bar, SurfaceFit::Fill)
                .on_press(|st: &mut State| {
                    close_popovers(st);
                    st.storage_expanded = !st.storage_expanded;
                }),
        );

    dock
}

// ---------------------------------------------------------------- popovers & context menus

fn menu_row(
    key: &'static str,
    label: &str,
    hint: &str,
    checked: Option<bool>,
    on: impl Fn(&mut State) + 'static,
) -> El {
    let mut r = row()
        .key(key)
        .w_full()
        .h(32.0)
        .px(10.0)
        .radius(6.0)
        .gap(8.0)
        .center_y()
        .hover(hover_fill())
        .press(press_fill())
        .child(text(label.to_string()).font(ts(13.0, 400.0)).color(ink()).grow(1.0));
    if !hint.is_empty() {
        r = r.child(text(hint.to_string()).font(ts(11.5, 400.0)).color(ink_muted()));
    }
    if let Some(on_flag) = checked {
        r = r.child(
            text(if on_flag { "✓" } else { "" })
                .font(ts(13.0, 700.0))
                .color(ink()),
        );
    }
    r.on_press(on)
}

fn menu_row_icon(
    key: &'static str,
    icon_svg: &'static rune::text::Icon,
    label: &str,
    hint: &str,
    on: impl Fn(&mut State) + 'static,
) -> El {
    let mut r = row()
        .key(key)
        .w_full()
        .h(32.0)
        .px(10.0)
        .radius(6.0)
        .gap(8.0)
        .center_y()
        .hover(hover_fill())
        .press(press_fill())
        .child(rune::icon(icon_svg, 14.0).color(ink_secondary()))
        .child(text(label.to_string()).font(ts(13.0, 400.0)).color(ink()).grow(1.0));
    if !hint.is_empty() {
        r = r.child(text(hint.to_string()).font(ts(11.5, 400.0)).color(ink_muted()));
    }
    r.on_press(on)
}

fn menu_separator(key: &'static str) -> El {
    el().key(key)
        .w_full()
        .py(4.0)
        .child(el().w_full().h(1.0).fill(hairline()))
}

fn main_menu_popover(s: &State) -> El {
    col().key("main-menu-pop")
        .anchored("btn-main-menu", Side::Bottom, Align::Start, 6.0)
        .w(230.0)
        .padding(6.0)
        .gap(2.0)
        .radius(tokens::RADIUS_CARD)
        .fill(popover_bg())
        .border(1.0, hairline())
        .shadow(8.0, 24.0, rgba(0, 0, 0, 0.55))
        .child(menu_row_icon("mm-new-folder", ICON_FOLDER_NEW, "New Folder", "Ctrl+Shift+N", |st| {
            close_popovers(st);
            crate::do_new_folder(st);
        }))
        .child(menu_row("mm-new-file", "New File", "Ctrl+Alt+N", None, |st| {
            close_popovers(st);
            crate::do_new_file(st);
        }))
        .child(menu_separator("mm-sep1"))
        .child(menu_row(
            "mm-hidden",
            "Show Hidden Files",
            "Ctrl+H",
            Some(s.show_hidden),
            |st| {
                st.show_hidden = !st.show_hidden;
                refilter(st);
                save_settings(st);
                close_popovers(st);
            },
        ))
        .child(menu_row(
            "mm-storage",
            "Show Storage Bar",
            "",
            Some(s.storage_bar_open),
            |st| {
                st.storage_bar_open = !st.storage_bar_open;
                close_popovers(st);
            },
        ))
        .child(menu_separator("mm-sep2"))
        .child(menu_row("mm-shortcuts", "Keyboard Shortcuts", "Ctrl+/", None, |st| {
            close_popovers(st);
            st.show_shortcuts = true;
        }))
}

fn view_menu_popover(s: &State) -> El {
    col().key("view-menu-pop")
        .anchored("btn-view-menu", Side::Bottom, Align::End, 6.0)
        .w(230.0)
        .padding(6.0)
        .gap(2.0)
        .radius(tokens::RADIUS_CARD)
        .fill(popover_bg())
        .border(1.0, hairline())
        .shadow(8.0, 24.0, rgba(0, 0, 0, 0.55))
        .child(
            row()
                .key("vm-zoom-row")
                .w_full()
                .h(32.0)
                .px(6.0)
                .gap(4.0)
                .center_y()
                .child(text("Icon Size").font(ts(12.5, 500.0)).color(ink_secondary()).grow(1.0))
                .child(
                    el().key("zm-s").px(8.0).h(24.0).radius(6.0).center()
                        .fill(if (s.icon_size - 64.0).abs() < 1.0 { white_a(0.08) } else { Color::TRANSPARENT })
                        .hover(hover_fill())
                        .child(text("S").font(ts(12.0, 600.0)).color(ink()))
                        .on_press(|st: &mut State| { st.icon_size = 64.0; close_popovers(st); }),
                )
                .child(
                    el().key("zm-m").px(8.0).h(24.0).radius(6.0).center()
                        .fill(if (s.icon_size - 96.0).abs() < 1.0 { white_a(0.08) } else { Color::TRANSPARENT })
                        .hover(hover_fill())
                        .child(text("M").font(ts(12.0, 600.0)).color(ink()))
                        .on_press(|st: &mut State| { st.icon_size = 96.0; close_popovers(st); }),
                )
                .child(
                    el().key("zm-l").px(8.0).h(24.0).radius(6.0).center()
                        .fill(if (s.icon_size - 128.0).abs() < 1.0 { white_a(0.08) } else { Color::TRANSPARENT })
                        .hover(hover_fill())
                        .child(text("L").font(ts(12.0, 600.0)).color(ink()))
                        .on_press(|st: &mut State| { st.icon_size = 128.0; close_popovers(st); }),
                ),
        )
        .child(menu_separator("vm-sep1"))
        .child(menu_row("vm-sort-name", "Sort by A–Z Name", "", Some(s.sort_by == 0), |st| {
            st.sort_by = 0; refilter(st); save_settings(st); close_popovers(st);
        }))
        .child(menu_row("vm-sort-date", "Last Modified", "", Some(s.sort_by == 3), |st| {
            st.sort_by = 3; refilter(st); save_settings(st); close_popovers(st);
        }))
        .child(menu_row("vm-sort-size", "Size", "", Some(s.sort_by == 1), |st| {
            st.sort_by = 1; refilter(st); save_settings(st); close_popovers(st);
        }))
        .child(menu_row("vm-sort-type", "Type", "", Some(s.sort_by == 2), |st| {
            st.sort_by = 2; refilter(st); save_settings(st); close_popovers(st);
        }))
        .child(menu_row("vm-sort-desc", "Reversed Order", "", Some(s.sort_desc), |st| {
            st.sort_desc = !st.sort_desc; refilter(st); save_settings(st); close_popovers(st);
        }))
        .child(menu_separator("vm-sep2"))
        .child(menu_row("vm-hidden", "Show Hidden Files", "Ctrl+H", Some(s.show_hidden), |st| {
            st.show_hidden = !st.show_hidden; refilter(st); save_settings(st); close_popovers(st);
        }))
        .child(menu_row("vm-storage", "Show Storage Bar", "", Some(s.storage_bar_open), |st| {
            st.storage_bar_open = !st.storage_bar_open; close_popovers(st);
        }))
}

fn path_menu_popover() -> El {
    col().key("path-menu-pop")
        .anchored("btn-path-menu", Side::Bottom, Align::End, 6.0)
        .w(215.0)
        .padding(6.0)
        .gap(2.0)
        .radius(tokens::RADIUS_CARD)
        .fill(popover_bg())
        .border(1.0, hairline())
        .shadow(8.0, 24.0, rgba(0, 0, 0, 0.55))
        .child(menu_row_icon("pm-new-folder", ICON_FOLDER_NEW, "New Folder", "", |st| {
            close_popovers(st);
            crate::do_new_folder(st);
        }))
        .child(menu_row("pm-term", "Open in Terminal", "", None, |st| {
            close_popovers(st);
            let dir = match &st.location {
                Location::Folder(d) => d.clone(),
                _ => platform::home_dir(),
            };
            platform::open_terminal_here(&dir);
        }))
        .child(menu_row("pm-dups", "Find Duplicates", "", None, |st| {
            close_popovers(st);
            if let Location::Folder(root) = st.location.clone() {
                let handle = st.handle.clone();
                st.status = "Scanning duplicates…".into();
                std::thread::spawn(move || {
                    let dups = find_duplicates(&root);
                    let files: usize = dups.iter().map(|(_, g)| g.len()).sum();
                    handle.update(move |s, _| {
                        s.status = format!("{} duplicate groups ({} files)", dups.len(), files);
                    });
                });
            }
        }))
        .child(menu_row("pm-cmp", "Compare 2 Selected", "", None, |st| {
            close_popovers(st);
            let dirs: Vec<PathBuf> = st
                .selected
                .iter()
                .filter_map(|&idx| entry_resolved_path(st, idx))
                .filter(|p| p.is_dir())
                .collect();
            if dirs.len() == 2 {
                let (a, b) = (dirs[0].clone(), dirs[1].clone());
                let handle = st.handle.clone();
                st.status = "Comparing folders…".into();
                std::thread::spawn(move || {
                    let (oa, ob, diff) = compare_dirs(&a, &b);
                    handle.update(move |s, _| {
                        s.status = format!("Only A: {oa} · Only B: {ob} · Diff: {diff}");
                    });
                });
            } else {
                st.status = "Select 2 folders to compare".into();
            }
        }))
        .child(menu_row("pm-edit-loc", "Edit Location", "Ctrl+L", None, |st| {
            close_popovers(st);
            st.edit_address = true;
            st.focus_target = Some("addr-input".to_string());
        }))
        .child(menu_row_icon("pm-reload", ICON_REFRESH, "Reload", "", |st| {
            close_popovers(st);
            reload(st);
        }))
}

fn context_menu_popover(s: &State, idx: usize) -> El {
    let path_opt = entry_resolved_path(s, idx);
    let starred_now = path_opt.as_ref().is_some_and(|p| is_starred(s, p));

    let mut card = if s.view_mode == 2 {
        col().key("ctx-menu")
            .anchored_fit(format!("cell-{idx}"), Side::Bottom, Align::Start, 4.0)
    } else {
        col().key("ctx-menu")
            .anchored_fit(format!("row-{idx}"), Side::Bottom, Align::Start, 4.0)
    };

    card = card
        .w(230.0)
        .padding(6.0)
        .gap(2.0)
        .radius(tokens::RADIUS_CARD)
        .fill(popover_bg())
        .border(1.0, hairline())
        .shadow(10.0, 28.0, rgba(0, 0, 0, 0.60))
        .child(menu_row("ctx-open", "Open", "Enter", None, move |st| {
            close_popovers(st);
            open_entry(st, idx);
        }))
        .child(menu_row("ctx-open-with", "Open With...", "", None, move |st| {
            if let Some(p) = entry_resolved_path(st, idx) {
                st.open_with_apps = open_with_candidates(&p);
                st.open_with_path = Some(p);
                st.open_with_open = true;
            }
            close_popovers(st);
        }))
        .child(menu_row(
            "ctx-star",
            if starred_now { "Remove from Starred" } else { "Add to Starred" },
            "",
            None,
            move |st| {
                if let Some(p) = entry_resolved_path(st, idx) {
                    toggle_star(st, p);
                }
                close_popovers(st);
            },
        ))
        .child(menu_separator("ctx-sep1"))
        .child(menu_row("ctx-copy", "Copy", "Ctrl+C", None, move |st| {
            close_popovers(st);
            crate::do_copy(st);
        }))
        .child(menu_row("ctx-cut", "Cut", "Ctrl+X", None, move |st| {
            close_popovers(st);
            crate::do_copy(st);
            st.clipboard_cut = true;
        }))
        .child(menu_row("ctx-paste", "Paste", "Ctrl+V", None, move |st| {
            close_popovers(st);
            crate::do_paste(st);
        }))
        .child(menu_row("ctx-copy-path", "Copy Path", "", None, move |st| {
            if let Some(p) = entry_resolved_path(st, idx) {
                let text = p.to_string_lossy().to_string();
                std::thread::spawn(move || {
                    if let Ok(mut cb) = arboard::Clipboard::new() {
                        let _ = cb.set_text(text);
                    }
                });
                st.status = "Path copied".into();
            }
            close_popovers(st);
        }))
        .child(menu_separator("ctx-sep2"))
        .child(menu_row("ctx-rename", "Rename", "F2", None, move |st| {
            close_popovers(st);
            st.renaming = Some(idx);
            if let Some(e) = st.snapshot.entries.get(idx) {
                st.rename_text = rune::TextEdit::new(&e.name);
            }
            st.focus_target = Some(format!("rename-{idx}"));
        }))
        .child(menu_row("ctx-symlink", "Create Symlink", "", None, move |st| {
            if let Some(p) = entry_resolved_path(st, idx) {
                if let Some(parent) = p.parent() {
                    let dest = unique_link_path(parent, &p, "symlink");
                    st.status = if create_symlink(&p, &dest) {
                        "Symlink created".into()
                    } else {
                        "Symlink failed".into()
                    };
                    reload(st);
                }
            }
            close_popovers(st);
        }))
        .child(menu_row("ctx-compress", "Compress (.tar.gz)", "", None, move |st| {
            if let Some(p) = entry_resolved_path(st, idx) {
                let target = p.parent().map(Path::to_path_buf).unwrap_or_else(platform::home_dir);
                let name = p
                    .file_name()
                    .map(|n| format!("{}.tar.gz", n.to_string_lossy()))
                    .unwrap_or_else(|| "archive.tar.gz".into());
                let dest = target.join(name);
                let handle = st.handle.clone();
                st.op_cancel.store(false, std::sync::atomic::Ordering::Relaxed);
                let cancel = st.op_cancel.clone();
                st.op_progress = Some(FileOpProgress {
                    label: "Compressing".into(),
                    done: 0,
                    total: 1,
                    bytes_done: 0,
                    bytes_total: 0,
                    failures: 0,
                    cancelled: false,
                });
                std::thread::spawn(move || {
                    let total = count_files(&p).max(1);
                    handle.update(move |s, _| {
                        if let Some(op) = &mut s.op_progress {
                            op.total = total;
                        }
                    });
                    let mut cmd = std::process::Command::new("tar");
                    cmd.args(["czvf", &dest.to_string_lossy(), &p.to_string_lossy()]);
                    let ok = run_progress(&mut cmd, &cancel, &handle);
                    handle.update(move |s, _| {
                        s.op_progress = None;
                        s.status = if ok { "Compressed archive".into() } else { "Compress failed".into() };
                        reload(s);
                    });
                });
            }
            close_popovers(st);
        }));

    if path_opt.as_ref().is_some_and(|p| is_archive(p)) {
        card = card.child(menu_row("ctx-extract", "Extract Here", "", None, move |st| {
            if let Some(p) = entry_resolved_path(st, idx) {
                let target = p.parent().map(Path::to_path_buf).unwrap_or_else(platform::home_dir);
                let handle = st.handle.clone();
                st.op_cancel.store(false, std::sync::atomic::Ordering::Relaxed);
                let cancel = st.op_cancel.clone();
                st.op_progress = Some(FileOpProgress {
                    label: "Extracting".into(),
                    done: 0,
                    total: 1,
                    bytes_done: 0,
                    bytes_total: 0,
                    failures: 0,
                    cancelled: false,
                });
                std::thread::spawn(move || {
                    let total = archive_entry_count(&p).max(1);
                    handle.update(move |s, _| {
                        if let Some(op) = &mut s.op_progress {
                            op.total = total;
                        }
                    });
                    let mut cmd = if p.to_string_lossy().to_lowercase().ends_with(".zip") {
                        let mut c = std::process::Command::new("unzip");
                        c.args(["-o", &p.to_string_lossy(), "-d", &target.to_string_lossy()]);
                        c
                    } else {
                        let mut c = std::process::Command::new("tar");
                        c.args(["xvf", &p.to_string_lossy(), "-C", &target.to_string_lossy()]);
                        c
                    };
                    let ok = run_progress(&mut cmd, &cancel, &handle);
                    handle.update(move |s, _| {
                        s.op_progress = None;
                        s.status = if ok { "Extracted archive".into() } else { "Extract failed".into() };
                        reload(s);
                    });
                });
            }
            close_popovers(st);
        }));
    }

    card = card
        .child(menu_row("ctx-term", "Open in Terminal", "", None, move |st| {
            let dir = entry_resolved_path(st, idx)
                .map(|p| {
                    if p.is_dir() {
                        p
                    } else {
                        p.parent()
                            .map(Path::to_path_buf)
                            .unwrap_or_else(platform::home_dir)
                    }
                })
                .unwrap_or_else(platform::home_dir);
            platform::open_terminal_here(&dir);
            close_popovers(st);
        }))
        .child(menu_row("ctx-props", "Properties", "Ctrl+I", None, move |st| {
            close_popovers(st);
            open_properties(st);
        }))
        .child(menu_separator("ctx-sep3"))
        .child(menu_row("ctx-trash", "Move to Trash", "Del", None, move |st| {
            close_popovers(st);
            if !st.selected.contains(&idx) {
                st.selected.clear();
                st.selected.insert(idx);
            }
            crate::delete_selected(st);
        }));

    card
}

// ---------------------------------------------------------------- root view

pub fn view(s: &State, _cx: &mut Cx<State>) -> El {
    let main_pane = col()
        .key("main-pane")
        .grow(1.0)
        .h_full()
        .fill(view_bg())
        .on_drop_hover(|st: &mut State, over, _| {
            st.drop_hover = over;
        })
        .on_drop_files(handle_drop_files)
        .child(headerbar_el(s))
        // Optional subtle filter pill when user starts typing to filter
        .child(if !s.query.text.is_empty() {
            row().key("filter-bar")
                .w_full()
                .px(18.0)
                .pb(6.0)
                .justify_center()
                .child(
                    row()
                        .w(360.0)
                        .h(32.0)
                        .px(12.0)
                        .gap(8.0)
                        .radius(999.0)
                        .fill(popover_bg())
                        .border(1.0, white_a(0.08))
                        .center_y()
                        .child(rune::icon(ICON_EDIT_FIND, 14.0).color(ink_secondary()))
                        .child(
                            text_input(&s.query, ts(13.0, 400.0))
                                .key("filter")
                                .label("Filter folder")
                                .single_line()
                                .grow(1.0)
                                .color(ink())
                                .placeholder("Filter...", ink_muted())
                                .on_edit(|st: &mut State| &mut st.query)
                                .on_input_change(|st: &mut State, _| refilter(st)),
                        )
                        .child(
                            el().key("filter-clear")
                                .size(20.0, 20.0)
                                .radius(999.0)
                                .center()
                                .hover(hover_fill())
                                .child(rune::icon(ICON_CLOSE, 11.0).color(ink_secondary()))
                                .on_press(|st: &mut State| {
                                    st.query.text.clear();
                                    refilter(st);
                                }),
                        ),
                )
        } else {
            el().key("filter-off").w_full().h(0.0)
        })
        // File area (grid / list / search results / empty state)
        .child(if s.search_mode && !s.search_query.text.trim().is_empty() {
            let mut list = col()
                .key("search-results")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .px(18.0)
                .pt(8.0)
                .pb(12.0)
                .gap(2.0)
                .scroll(ScrollSpec { stick_bottom: false });
            if s.search_results.is_empty() {
                list = list.child(empty_state_el(s));
            } else {
                for (i, p) in s.search_results.iter().enumerate() {
                    let target = p.clone();
                    let is_dir = target.is_dir();
                    let label = p.display().to_string();
                    list = list.child(
                        row()
                            .key(format!("sr-{i}"))
                            .w_full()
                            .h(34.0)
                            .px(10.0)
                            .gap(10.0)
                            .radius(8.0)
                            .center_y()
                            .hover(hover_fill())
                            .child(
                                rune::icon(
                                    if is_dir { ICON_FOLDER } else { icons::ICON_FOLDER_DOCUMENTS },
                                    16.0,
                                )
                                .color(ink_secondary()),
                            )
                            .child(text(label).font(ts(13.0, 400.0)).color(ink()).single_line())
                            .on_press(move |st: &mut State| {
                                if is_dir {
                                    navigate(st, Location::Folder(target.clone()));
                                    sync_address(st);
                                } else {
                                    platform::open_path(&target);
                                }
                            }),
                    );
                }
            }
            list
        } else if s.filtered.is_empty() && !s.loading {
            empty_state_el(s)
        } else if s.view_mode == 2 {
            let mut grid = row()
                .key("grid")
                .w_full()
                .flex_wrap()
                .gaps(12.0, 12.0)
                .px(18.0)
                .pt(10.0)
                .pb(16.0)
                .on_background_press(|st: &mut State| {
                    close_popovers(st);
                    st.selected.clear();
                });
            for (i, &idx) in s.filtered.iter().enumerate() {
                grid = grid.child(grid_cell_el(s, i, idx));
            }
            col()
                .key("grid-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec { stick_bottom: false })
                .on_background_press(|st: &mut State| {
                    close_popovers(st);
                    st.selected.clear();
                })
                .child(grid)
        } else {
            let mut list = col()
                .key("list-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .px(16.0)
                .pt(6.0)
                .pb(12.0)
                .gap(2.0)
                .scroll(ScrollSpec { stick_bottom: false })
                .on_background_press(|st: &mut State| {
                    close_popovers(st);
                    st.selected.clear();
                });
            for (i, &idx) in s.filtered.iter().enumerate() {
                list = list.child(list_row_el(s, i, idx));
            }
            list
        })
        // Clove's animated Bayer 4x4 storage bar, minimal dock at bottom
        .child(storage_dock_el(s));

    let mut root = el()
        .key("window-root")
        .w_full()
        .h_full()
        .radius(12.0)
        .clip_rounded(12.0)
        .border(1.0, white_a(0.045))
        .on_layout(|st: &mut State, [_, _, w, _]| {
            if (st.win_width - w).abs() >= 1.0 && w > 200.0 {
                st.win_width = w;
            }
        })
        .child(
            row()
                .key("split-view")
                .w_full()
                .h_full()
                .on_background_press(|st: &mut State| close_popovers(st))
                .child(sidebar_el(s))
                .child(el().key("split-sep").w(1.0).h_full().fill(rgba(0x00, 0x00, 0x00, 0.42)))
                .child(main_pane),
        );

    if !s.status.is_empty() && s.status != "loading..." {
        root = root.child(
            row().key("status-toast")
                .pin(None, None, None, Some(60.0))
                .w_full()
                .justify_center()
                .child(
                    row()
                        .px(14.0)
                        .py(7.0)
                        .gap(10.0)
                        .radius(999.0)
                        .fill(popover_bg())
                        .border(1.0, hairline())
                        .shadow(6.0, 16.0, rgba(0, 0, 0, 0.45))
                        .center_y()
                        .child(text(s.status.clone()).font(ts(12.5, 500.0)).color(ink()))
                        .child(
                            el().key("toast-dismiss")
                                .size(18.0, 18.0)
                                .radius(999.0)
                                .hover(hover_fill())
                                .center()
                                .child(rune::icon(ICON_CLOSE, 10.0).color(ink_secondary()))
                                .on_press(|st: &mut State| st.status.clear()),
                        ),
                ),
        );
    }

    let any_popover =
        s.main_menu_open || s.view_menu_open || s.path_menu_open || s.context_menu.is_some();
    if any_popover {
        root = root.child(
            el().key("popover-dismiss")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .on_background_press(|st: &mut State| close_popovers(st)),
        );
    }
    if s.main_menu_open {
        root = root.child(main_menu_popover(s));
    }
    if s.view_menu_open {
        root = root.child(view_menu_popover(s));
    }
    if s.path_menu_open {
        root = root.child(path_menu_popover());
    }
    if let Some(idx) = s.context_menu {
        root = root.child(context_menu_popover(s, idx));
    }

    // Properties modal
    if let Some(prop_path) = &s.properties_path {
        let name = prop_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| prop_path.display().to_string());
        let size_str = match s.properties_size {
            Some(sz) => format_size(sz),
            None if s.properties_loading => "Calculating…".to_string(),
            None => "—".to_string(),
        };
        let mime_str = s.properties_mime.clone().unwrap_or_else(|| "—".to_string());
        let hash_str = s.properties_checksum.clone().unwrap_or_else(|| "—".to_string());
        let perms_str = props_permissions(prop_path);
        let owner_str = props_owner(prop_path);
        root = root.child(
            el().key("props-modal")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .fill(modal_scrim())
                .center()
                .on_background_press(|st: &mut State| {
                    st.properties_path = None;
                })
                .child(
                    col().key("props-card")
                        .w(420.0)
                        .padding(20.0)
                        .gap(10.0)
                        .radius(tokens::RADIUS_CARD)
                        .fill(card_bg())
                        .border(1.0, hairline())
                        .shadow(12.0, 32.0, rgba(0, 0, 0, 0.65))
                        .child(text(name).font(ts(16.0, 700.0)).color(ink()))
                        .child(
                            text(prop_path.display().to_string())
                                .font(ts(12.0, 400.0))
                                .color(ink_muted())
                                .wrap(),
                        )
                        .child(menu_separator("p-sep"))
                        .child(text(format!("Size: {size_str}")).font(ts(13.0, 400.0)).color(ink_secondary()))
                        .child(text(format!("Type: {mime_str}")).font(ts(13.0, 400.0)).color(ink_secondary()))
                        .child(text(format!("Permissions: {perms_str}  ·  Owner: {owner_str}")).font(ts(12.5, 400.0)).color(ink_secondary()))
                        .child(text(format!("SHA-256: {hash_str}")).font(ts(11.5, 400.0)).color(ink_muted()).wrap())
                        .child(
                            row().w_full().pt(6.0).child(spacer()).child(
                                el().key("props-close")
                                    .px(14.0)
                                    .h(30.0)
                                    .radius(tokens::RADIUS_CONTROL)
                                    .fill(hover_fill())
                                    .hover(white_a(0.08))
                                    .center()
                                    .child(text("Close").font(ts(13.0, 500.0)).color(ink()))
                                    .on_press(|st: &mut State| {
                                        st.properties_path = None;
                                    }),
                            ),
                        ),
                ),
        );
    }

    // Keyboard shortcuts modal
    if s.show_shortcuts {
        let items = keybinds_help();
        let mut card = col()
            .key("shortcuts-card")
            .w(460.0)
            .padding(20.0)
            .gap(8.0)
            .radius(tokens::RADIUS_CARD)
            .fill(card_bg())
            .border(1.0, hairline())
            .shadow(12.0, 32.0, rgba(0, 0, 0, 0.65))
            .child(text("Keyboard Shortcuts").font(ts(16.0, 700.0)).color(ink()))
            .child(menu_separator("sc-sep"));
        for (idx, (_act, k, desc)) in items.iter().enumerate() {
            card = card.child(
                row()
                    .key(format!("sc-{idx}"))
                    .w_full()
                    .center_y()
                    .child(text(*desc).font(ts(13.0, 400.0)).color(ink_secondary()).grow(1.0))
                    .child(
                        el().px(8.0)
                            .py(3.0)
                            .radius(6.0)
                            .fill(white_a(0.05))
                            .child(text(*k).font(ts(12.0, 500.0)).color(ink())),
                    ),
            );
        }
        card = card.child(
            row().w_full().pt(8.0).child(spacer()).child(
                el().key("sc-close")
                    .px(14.0)
                    .h(30.0)
                    .radius(tokens::RADIUS_CONTROL)
                    .fill(hover_fill())
                    .center()
                    .child(text("Close").font(ts(13.0, 500.0)).color(ink()))
                    .on_press(|st: &mut State| st.show_shortcuts = false),
            ),
        );
        root = root.child(
            el().key("shortcuts-modal")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .fill(modal_scrim())
                .center()
                .on_background_press(|st: &mut State| st.show_shortcuts = false)
                .child(card),
        );
    }

    // Batch rename modal
    if s.batch_rename_open {
        root = root.child(
            el().key("br-modal")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .fill(modal_scrim())
                .center()
                .on_background_press(|st: &mut State| st.batch_rename_open = false)
                .child(
                    col().key("br-card")
                        .w(440.0)
                        .padding(20.0)
                        .gap(10.0)
                        .radius(tokens::RADIUS_CARD)
                        .fill(card_bg())
                        .border(1.0, hairline())
                        .child(text("Batch Rename").font(ts(16.0, 700.0)).color(ink()))
                        .child(text_input(&s.batch_rename_find, ts(13.0, 400.0)).key("br-find").placeholder("Find text...", ink_muted()).on_edit(|st: &mut State| &mut st.batch_rename_find))
                        .child(text_input(&s.batch_rename_replace, ts(13.0, 400.0)).key("br-repl").placeholder("Replace with...", ink_muted()).on_edit(|st: &mut State| &mut st.batch_rename_replace))
                        .child(
                            row().gap(8.0).child(spacer())
                                .child(el().key("br-cancel").px(12.0).h(28.0).radius(6.0).fill(hover_fill()).center().child(text("Cancel").font(ts(13.0, 500.0)).color(ink())).on_press(|st: &mut State| st.batch_rename_open = false))
                                .child(el().key("br-apply").px(12.0).h(28.0).radius(6.0).fill(accent()).center().child(text("Rename").font(ts(13.0, 600.0)).color(ink())).on_press(|st: &mut State| { apply_batch_rename(st); st.batch_rename_open = false; })),
                        ),
                ),
        );
    }

    // Conflict prompt
    if let Some((target, paths, _cut)) = &s.pending_paste {
        let collisions: Vec<String> = paths
            .iter()
            .filter_map(|p| {
                let n = p.file_name()?;
                if target.join(n).exists() {
                    Some(n.to_string_lossy().to_string())
                } else {
                    None
                }
            })
            .collect();
        let summary = if collisions.len() == 1 {
            format!("\"{}\" already exists", collisions[0])
        } else {
            format!("{} conflicting items", collisions.len())
        };
        root = root.child(
            el().key("conflict-modal")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .fill(modal_scrim())
                .center()
                .child(
                    col().key("conflict-card")
                        .w(420.0)
                        .padding(20.0)
                        .gap(12.0)
                        .radius(tokens::RADIUS_CARD)
                        .fill(card_bg())
                        .border(1.0, hairline())
                        .child(text(summary).font(ts(15.0, 700.0)).color(ink()))
                        .child(
                            row().gap(8.0)
                                .child(el().key("cf-replace").px(12.0).h(28.0).radius(6.0).fill(accent()).center().child(text("Replace").font(ts(13.0, 600.0)).color(ink())).on_press(|st: &mut State| {
                                    if let Some((t, p, c)) = st.pending_paste.take() { run_paste(st, t, p, c, 1); }
                                }))
                                .child(el().key("cf-rename").px(12.0).h(28.0).radius(6.0).fill(hover_fill()).center().child(text("Keep Both").font(ts(13.0, 500.0)).color(ink())).on_press(|st: &mut State| {
                                    if let Some((t, p, c)) = st.pending_paste.take() { run_paste(st, t, p, c, 3); }
                                }))
                                .child(el().key("cf-skip").px(12.0).h(28.0).radius(6.0).fill(hover_fill()).center().child(text("Skip").font(ts(13.0, 500.0)).color(ink())).on_press(|st: &mut State| {
                                    if let Some((t, p, c)) = st.pending_paste.take() { run_paste(st, t, p, c, 2); }
                                }))
                                .child(spacer())
                                .child(el().key("cf-cancel").px(12.0).h(28.0).radius(6.0).fill(hover_fill()).center().child(text("Cancel").font(ts(13.0, 500.0)).color(danger())).on_press(|st: &mut State| { st.pending_paste = None; })),
                        ),
                ),
        );
    }

    // Open With modal
    if s.open_with_open {
        if let Some(target) = &s.open_with_path {
            let file = target.clone();
            let apps = if s.open_with_apps.is_empty() {
                open_with_candidates(target)
            } else {
                s.open_with_apps.clone()
            };
            let mut card = col()
                .key("ow-card")
                .w(360.0)
                .padding(18.0)
                .gap(8.0)
                .radius(tokens::RADIUS_CARD)
                .fill(card_bg())
                .border(1.0, hairline())
                .child(text("Open With").font(ts(15.0, 700.0)).color(ink()));
            for (i, (app_name, desktop)) in apps.into_iter().take(8).enumerate() {
                let f2 = file.clone();
                let d2 = desktop.clone();
                card = card.child(
                    el().key(format!("ow-{i}"))
                        .w_full()
                        .h(32.0)
                        .px(10.0)
                        .radius(6.0)
                        .hover(hover_fill())
                        .center_y()
                        .child(text(app_name).font(ts(13.0, 400.0)).color(ink()))
                        .on_press(move |st: &mut State| {
                            launch_desktop_app(&d2, &f2);
                            st.open_with_open = false;
                        }),
                );
            }
            card = card.child(
                row().child(spacer()).child(
                    el().key("ow-close").px(12.0).h(28.0).radius(6.0).fill(hover_fill()).center()
                        .child(text("Cancel").font(ts(13.0, 500.0)).color(ink()))
                        .on_press(|st: &mut State| st.open_with_open = false),
                ),
            );
            root = root.child(
                el().key("ow-modal")
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .fill(modal_scrim())
                    .center()
                    .on_background_press(|st: &mut State| st.open_with_open = false)
                    .child(card),
            );
        }
    }

    // Active file operation progress toast
    if let Some(op) = &s.op_progress {
        root = root.child(
            row().key("op-toast")
                .pin(None, None, Some(20.0), Some(72.0))
                .px(14.0)
                .py(8.0)
                .gap(10.0)
                .radius(tokens::RADIUS_PILL)
                .fill(popover_bg())
                .border(1.0, hairline())
                .center_y()
                .child(text(format!("{} ({}/{})", op.label, op.done, op.total)).font(ts(12.5, 500.0)).color(ink())),
        );
    }

    root
}
