//! Headless benchmark for the prototype's engine + Rune runtime.
//! Prints METRIC lines; run with `cargo run --release --bin bench`.
//! All numbers are medians; outliers are kept out by taking the median of
//! many runs, and the run count is printed with each metric.

use std::path::PathBuf;
use std::time::Instant;

use clove_rune_proto::{filter_entries, load_entries, render_storage_bar, Entry};
use rune::tok::type_;
use rune::{col, text, Color, Cx, El, Env, GrowthSpec, Runtime, ScrollSpec};

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn time<F: FnMut()>(runs: usize, mut f: F) -> f64 {
    let mut v = Vec::with_capacity(runs);
    for _ in 0..runs {
        let t = Instant::now();
        f();
        v.push(t.elapsed().as_secs_f64() * 1e6); // µs
    }
    median(v)
}

// ---------------------------------------------------------------------------
// Headless Rune runtime: the prototype's list over a synthetic 100k entries.
// ---------------------------------------------------------------------------

struct BState {
    filtered: Vec<usize>,
    selected: usize,
}

fn brow(s: &BState, i: usize) -> El {
    rune::row()
        .key(format!("row-{i}"))
        .h(28.0)
        .px(8.0)
        .center_y()
        .child(
            text(format!("entry-{:06}.dat", s.filtered[i]))
                .font(type_::BODY)
                .color(Color { r: 0.957, g: 0.961, b: 0.965, a: 1.0 })
                .grow(1.0),
        )
}

fn bview(s: &BState, _: &mut Cx<BState>) -> El {
    col()
        .w_full()
        .h_full()
        .fill(Color { r: 0.114, g: 0.118, b: 0.125, a: 1.0 })
        .padding(12.0)
        .child(
            col()
                .key("viewport")
                .scroll(ScrollSpec { stick_bottom: false })
                .grow(1.0)
                .min_h(0.0)
                .child(
                    col()
                        .key("rows")
                        .w_full()
                        .growth(GrowthSpec::default())
                        .virtual_rows(
                            s.filtered.len(),
                            28.0,
                            |s: &BState, i| format!("row-{}", s.filtered[i]).into(),
                            |s: &BState, i| brow(s, s.filtered[i]),
                        ),
                ),
        )
}

fn main() {
    println!("Clove prototype benchmark (release build, Windows, medians; µs = microseconds)\n");

    // --- 1. Dithered storage bar (CPU render, the future clove-render cost) ---
    for (w, runs) in [(512.0f32, 30), (936.0, 30), (1920.0, 30)] {
        let us = time(runs, || {
            let (_, _, buf) = render_storage_bar(w);
            std::hint::black_box(buf);
        });
        println!(
            "METRIC bar_render_width_{w}us={us:.1} runs={runs} (CPU dither, 2× scale, incl. glow blur)"
        );
    }

    // --- 2. Directory listing (the future clove-fs read path) ---
    let user = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\".into());
    for dir in [PathBuf::from(&user), PathBuf::from("C:\\Windows\\System32")] {
        let label = dir.to_string_lossy().replace('\\', "_");
        let mut n = 0usize;
        let us = time(15, || {
            let e = load_entries(&dir);
            n = e.len();
            std::hint::black_box(e);
        });
        println!("METRIC list_dir_{label}us={us:.1} items={n} runs=15 (read_dir + metadata + sort)");
    }

    // --- 3. Per-keystroke filter over 100k entries (no index, as the Swift app) ---
    let mut fake: Vec<Entry> = Vec::with_capacity(100_000);
    for i in 0..100_000 {
        fake.push(Entry {
            name: format!("entry-{i:06}-{}{}", ["data", "photo", "song", "clip", "doc"][i % 5], if i % 3 == 0 { ".dat" } else { ".bin" }),
            is_dir: i % 20 == 0,
            size: i as u64 * 13,
        });
    }
    for q in ["", "a", "app", "readme", "entry-099"] {
        let us = time(200, || {
            let f = filter_entries(&fake, q);
            std::hint::black_box(f);
        });
        println!("METRIC filter_100k_query_{q}us={us:.1} runs=200 (substring, lowercase each name)");
    }
    // Pre-lowered variant: the cost the engine pays if it caches folded names.
    let lowered: Vec<String> = fake.iter().map(|e| e.name.to_lowercase()).collect();
    for q in ["app", "readme"] {
        let us = time(200, || {
            let f: Vec<usize> = lowered
                .iter()
                .enumerate()
                .filter(|(_, n)| n.contains(q))
                .map(|(i, _)| i)
                .collect();
            std::hint::black_box(f);
        });
        println!("METRIC filter_100k_folded_{q}us={us:.1} runs=200 (names pre-folded once)");
    }

    // --- 4. Headless Rune runtime with the real view: 100,000 virtual rows ---
    let filtered: Vec<usize> = (0..100_000).collect();
    let mut rt = Runtime::new(BState { filtered, selected: 0 }, bview, (960.0, 680.0), Env::new(true, 2.0));
    let t = Instant::now();
    rt.frame(0.0);
    println!("METRIC rune_first_frame_100k_rowsus={:.1} (initial build; CPU scene, headless, no GPU upload)",
        t.elapsed().as_secs_f64() * 1e6);

    // A state change that shrinks the virtual list, then the re-build frame.
    // (The filter cost itself is measured in §3; this measures what the view
    // and layout pay when 100k rows become fewer.)
    let mut us = Vec::with_capacity(50);
    for k in 0..50u32 {
        let t = Instant::now();
        rt.update(|s: &mut BState, _| {
            let keep = 100_000 - (k as usize) * 1_800;
            s.filtered.truncate(keep.max(1_000));
        });
        rt.frame(1.0 / 60.0);
        us.push(t.elapsed().as_secs_f64() * 1e6);
    }
    println!("METRIC rune_state_change_plus_frame_100kus={:.1} runs=50 (update + view + layout, shrinking list)",
        median(us));

    // --- 5. Selected-row change (keyboard navigation) with 100k rows mounted-window only ---
    let mut us = Vec::with_capacity(50);
    for _ in 0..50 {
        let t = Instant::now();
        rt.update(|s: &mut BState, _| s.selected = (s.selected + 1) % 1_000);
        rt.frame(1.0 / 60.0);
        us.push(t.elapsed().as_secs_f64() * 1e6);
    }
    println!("METRIC rune_keynav_frame_100kus={:.1} runs=50 (selection change; idle elsewhere)",
        median(us));
}
