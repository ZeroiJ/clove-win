//! `clove` — headless test surface for the engine and the identity renderer.
//!
//! Every subcommand runs without a window, which is what makes the engine testable in CI
//! and lets the renderer be diffed frame-by-frame in `bench`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use clove_fs::{list_dir, Entry, Snapshot};
use clove_render::{BarRenderer, BarSpec, Rgb, Segment};
use clove_render::tokens;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        usage();
        return ExitCode::from(2);
    };
    let rest = &args[1..];

    let result = match cmd.as_str() {
        "list" => cmd_list(rest),
        "find" => cmd_find(rest),
        "bar" => cmd_bar(rest),
        "bench" => cmd_bench(rest),
        "-h" | "--help" | "help" => {
            usage();
            Ok(())
        }
        other => Err(format!("unknown command `{other}`\n")),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprint!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn usage() {
    println!(
        "clove — headless Clove engine\n\n\
         USAGE:\n  \
         clove list <dir> [--hidden]        list a directory (dirs first, case-insensitive)\n  \
         clove find <dir> <query>           scored fuzzy filter over the listing\n  \
         clove bar [opts]                   render the dithered storage bar to PNG\n  \
         clove bench [dir]                  measure listing, filtering, and bar frames\n\n\
         BAR OPTIONS:\n  \
         --width <pt>    bar width in points (default 936)\n  \
         --height <pt>   bar height in points (default 88)\n  \
         --scale <n>     pixels per point (default 2)\n  \
         --time <s>      render at this time in seconds (default 0)\n  \
         --frames <n>    render n frames at 1/60 s, report the mean (default 1)\n  \
         --out <file>    PNG path (default bar.png)\n  \
         --kind <n>      use one kind only, filling the bar (0..8)"
    );
}

// ---------------------------------------------------------------- list / find

fn cmd_list(args: &[String]) -> Result<(), String> {
    let (path, flags) = parse_path_and_flags(args)?;
    let show_hidden = flags.contains(&"--hidden".to_string());
    let started = Instant::now();
    let snapshot = list_dir(&path).map_err(|e| format!("{}: {e}\n", path.display()))?;
    let took = started.elapsed();

    let mut shown = 0usize;
    for entry in &snapshot.entries {
        if entry.hidden && !show_hidden {
            continue;
        }
        shown += 1;
        println!("{}", describe(entry));
    }
    eprintln!(
        "\n{} of {} entries in {:?} ({})",
        shown,
        snapshot.len(),
        took,
        path.display()
    );
    Ok(())
}

fn cmd_find(args: &[String]) -> Result<(), String> {
    let (path, flags) = parse_path_and_flags(args)?;
    let query = flags
        .iter()
        .find(|a| !a.starts_with("--"))
        .ok_or_else(|| "find needs a query\n".to_string())?
        .clone();

    let snapshot = list_dir(&path).map_err(|e| format!("{}: {e}\n", path.display()))?;
    let started = Instant::now();
    let indices = clove_search::filter(&snapshot.entries, &query);
    let took = started.elapsed();

    for &i in indices.iter().take(200) {
        println!("{}", describe(&snapshot.entries[i]));
    }
    if indices.len() > 200 {
        println!("… {} more", indices.len() - 200);
    }
    eprintln!(
        "\n{} of {} entries match {:?} in {:?}",
        indices.len(),
        snapshot.len(),
        query,
        took
    );
    Ok(())
}

fn describe(entry: &Entry) -> String {
    let kind = if entry.is_dir { "dir " } else { "file" };
    let size = if entry.is_dir {
        String::new()
    } else {
        human_size(entry.size)
    };
    let hidden = if entry.hidden { " hidden" } else { "" };
    format!("{kind}  {:>9}  {}{hidden}", size, entry.name)
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// First non-flag argument is the path; everything else is returned as flags.
fn parse_path_and_flags(args: &[String]) -> Result<(PathBuf, Vec<String>), String> {
    let mut path = None;
    let mut flags = Vec::new();
    for arg in args {
        if arg.starts_with("--") {
            flags.push(arg.clone());
        } else if path.is_none() {
            path = Some(PathBuf::from(arg));
        } else {
            flags.push(arg.clone());
        }
    }
    Ok((path.unwrap_or_else(|| PathBuf::from(".")), flags))
}

// ---------------------------------------------------------------- bar

struct BarArgs {
    width: f32,
    height: f32,
    scale: f32,
    time: f32,
    frames: usize,
    out: PathBuf,
    kind: Option<usize>,
}

fn parse_bar_args(args: &[String]) -> Result<BarArgs, String> {
    let mut parsed = BarArgs {
        width: 936.0,
        height: 88.0,
        scale: 2.0,
        time: 0.0,
        frames: 1,
        out: PathBuf::from("bar.png"),
        kind: None,
    };
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let mut value = || {
            iter.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value\n"))
        };
        match arg.as_str() {
            "--width" => parsed.width = parse_num(&value()?)?,
            "--height" => parsed.height = parse_num(&value()?)?,
            "--scale" => parsed.scale = parse_num(&value()?)?,
            "--time" => parsed.time = parse_num(&value()?)?,
            "--frames" => parsed.frames = parse_num::<u32>(&value()?)? as usize,
            "--out" => parsed.out = PathBuf::from(value()?),
            "--kind" => parsed.kind = Some(parse_num::<u32>(&value()?)? as usize),
            other if !other.starts_with("--") => parsed.width = parse_num(other)?,
            other => return Err(format!("unknown bar option `{other}`\n")),
        }
    }
    if parsed.frames == 0 {
        return Err("--frames must be at least 1\n".into());
    }
    Ok(parsed)
}

fn parse_num<T: std::str::FromStr>(text: &str) -> Result<T, String> {
    text.parse::<T>()
        .map_err(|_| format!("not a number: `{text}`\n"))
}

fn cmd_bar(args: &[String]) -> Result<(), String> {
    let args = parse_bar_args(args)?;
    let spec = bar_spec(&args);
    let mut renderer = BarRenderer::new(spec);
    let (w, h) = renderer.size();

    let started = Instant::now();
    let mut last = Vec::new();
    for frame in 0..args.frames {
        let t = args.time + frame as f32 / 60.0;
        last = renderer.render(t).to_vec();
    }
    let total = started.elapsed();
    let per_frame = total / args.frames as u32;

    write_png(&args.out, &last, w, h, tokens::SURFACE)?;
    println!(
        "{}×{} px ({}×{} pt @ {:.0}×), {} cell{} → {}",
        w,
        h,
        args.width,
        args.height,
        args.scale,
        renderer.cell_count(),
        if renderer.cell_count() == 1 { "" } else { "s" },
        args.out.display()
    );
    if args.frames > 1 {
        println!(
            "{} frames in {:?} — mean {:?} per frame",
            args.frames, total, per_frame
        );
    } else {
        println!("rendered in {:?}", total);
    }
    Ok(())
}

fn bar_spec(args: &BarArgs) -> BarSpec {
    let segments = match args.kind {
        Some(kind) => vec![Segment::kind(kind, 1.0)],
        None => {
            // Placeholder fractions until the storage scan lands; nine kinds, uneven like a
            // real disk so the bar reads as a stack rather than a rainbow.
            const PLACEHOLDER: [f32; 9] = [0.18, 0.16, 0.11, 0.09, 0.12, 0.07, 0.13, 0.09, 0.05];
            PLACEHOLDER
                .iter()
                .enumerate()
                .map(|(k, &f)| Segment::kind(k, f))
                .collect()
        }
    };
    BarSpec::new(args.width, segments)
        .with_height(args.height)
        .with_scale(args.scale)
}

/// Writes the straight-alpha bar buffer over `background`, as the app composites it.
fn write_png(path: &Path, rgba: &[u8], w: usize, h: usize, background: Rgb) -> Result<(), String> {
    let mut out = Vec::with_capacity(w * h * 3);
    for px in rgba.chunks_exact(4) {
        let a = px[3] as f32 / 255.0;
        for ch in 0..3 {
            let base = background.to_array()[ch] as f32;
            out.push((px[ch] as f32 * a + base * (1.0 - a)).round() as u8);
        }
    }

    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}\n", path.display()))?;
    let writer = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, w as u32, h as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut png = encoder
        .write_header()
        .map_err(|e| format!("png header: {e}\n"))?;
    png.write_image_data(&out)
        .map_err(|e| format!("png data: {e}\n"))?;
    Ok(())
}

// ---------------------------------------------------------------- bench

fn cmd_bench(args: &[String]) -> Result<(), String> {
    let dir = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    println!("clove bench — {}", dir.display());

    // Listing, cold and warm.
    let mut best = f64::MAX;
    let mut snapshot: Option<Snapshot> = None;
    for _ in 0..5 {
        let started = Instant::now();
        let snap = list_dir(&dir).map_err(|e| format!("{}: {e}\n", dir.display()))?;
        best = best.min(started.elapsed().as_secs_f64() * 1000.0);
        snapshot = Some(snap);
    }
    let snapshot = snapshot.expect("at least one listing");
    println!(
        "listing          {:>8.3} ms  ({} entries, best of 5)",
        best,
        snapshot.len()
    );

    // Filtering at a few sizes, with the pre-folded names the engine stores.
    let cases: [(&str, usize); 3] = [("a", 5), ("st", 5), ("storage", 20)];
    for (query, reps) in cases {
        let started = Instant::now();
        let mut matched = 0;
        for _ in 0..reps {
            matched = clove_search::filter(&snapshot.entries, query).len();
        }
        let each = started.elapsed().as_secs_f64() * 1000.0 / reps as f64;
        println!("filter {query:<9} {:>8.3} ms  ({matched} matches)", each);
    }

    // Synthetic 100k listing: the per-keystroke budget that matters.
    let big: Vec<Entry> = (0..100_000)
        .map(|i| {
            Entry::new(
                format!("synthetic-file-{i:06}-{}.dat", if i % 3 == 0 { "alpha" } else { "beta" }),
                i % 50 == 0,
                1024,
            )
        })
        .collect();
    for query in ["a", "file-000", "alpha"] {
        let started = Instant::now();
        let n = clove_search::filter(&big, query).len();
        println!(
            "filter 100k {query:<9} {:>8.3} ms  ({n} matches)",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }

    // Bar: first frame, then the incremental frames that a settled bar costs.
    for width in [512.0f32, 936.0, 1920.0] {
        let mut renderer = BarRenderer::new(BarSpec::even(width));
        let started = Instant::now();
        renderer.render(0.0);
        let first = started.elapsed().as_secs_f64() * 1000.0;

        let started = Instant::now();
        for frame in 1..=60 {
            renderer.render(frame as f32 / 60.0);
        }
        let animated = started.elapsed().as_secs_f64() * 1000.0 / 60.0;

        // The same frame twice: fully settled, nothing to recomposite.
        renderer.render(3.0);
        let started = Instant::now();
        for _ in 0..60 {
            renderer.render(3.0);
        }
        let settled = started.elapsed().as_secs_f64() * 1000.0 / 60.0;

        println!(
            "bar {width:>4.0} pt     first {:>8.3} ms · animated {:>8.3} ms/frame · settled {:>8.3} ms/frame",
            first, animated, settled
        );
    }

    Ok(())
}
