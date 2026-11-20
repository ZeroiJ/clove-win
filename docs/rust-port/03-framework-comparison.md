# 3 · Framework comparison — GPUI (Zed) vs Rune (Anomaly)

Evaluation date: 2026-11-15. Evidence: official READMEs/docs (gpui 0.2.2 on crates.io, Rune `dev`),
hands-on prototyping on this Windows 11 machine (Rust 1.99.1, MSVC), and public issue trackers.
Popularity was explicitly **not** used as a criterion; the recommendation is grounded in
Windows behaviour observed in the prototypes and feature fit for a file manager.

| | **GPUI 0.2.2** (zed-industries/zed) | **Rune `dev`** (anomalyco/rune) |
|---|---|---|
| Licence / maturity | Apache-2.0, pre-1.0, "breaking changes between versions" | MIT, pre-release ("pin a dev rev"), 2,449 commits, dev branch fast-forward-only |
| Model | Hybrid immediate/retained: `Entity` views + tailwind-style `div`s + custom elements | Retained keyed element tree built by plain view functions; diff retargets animation tracks |
| Renderer | GPU (blade/wgpu under the hood; DX12 on Windows), Taffy layout, DirectWrite text on Windows | Own wgpu renderer: **one instanced uber-quad pipeline, one draw call per frame**; Taffy 0.15; own text stack (Parley + own rasteriser "Windfoil", DirectWrite/ClearType modes) |
| Windows support | **Shipping** — Zed for Windows released; Win32 windowing, DirectWrite, no extra feature flags; windows 0.61 deps unconditional | **First-class** — Windows is a documented supported platform with its own text pipeline (DirectWrite + subpixel ClearType + LCD), vendored winit/wgpu-hal patches tuned for Windows |
| Custom GPU drawing | `Canvas` element (low-level paint API), `paint_quad`, `Path`/lyon tessellation, `Surface` element, custom elements with total paint control | `canvas(shape)`/`shape()` vector strokes+fills in the element's own points (analytic capsules, no atlas — changes every frame are fine), **replaceable `Pixels` RGBA buffers** (CPU-rendered per-frame images, exactly what the dithered bar needs), radial/linear gradients, blur, cached subtrees |
| Virtualized lists | `uniform_list`, `list`, `table` in Zed; battle-tested at editor scale | `virtualize`/`virtual_rows` — index-built rows, only mounted window has layout/text/scene nodes; measurement preserves visible anchor; `scroll_into_view` by key |
| Input | Actions/keymap system, focus handles, IME (Windows supported) | Full chord system (platform-aware `mod`), IME, key routing order, double-click pairing, drag with click counts, middle/back buttons, canvas wheel/pinch capture |
| Drag & drop | Internal typed drags; `ExternalPaths` for platform file drops | `.on_drag` per-gesture scratch, drag-out with `DragImage`/`DragItem`, `.on_drop_files`/`.on_drop_hover` receiving |
| Accessibility | macOS mature; **Windows screen-reader support missing** (issue #41138); AccessKit integration landing (PR #56065, "core primitives in place") | AccessKit built-in + vendored `accesskit_windows` patch (headings/landmarks reach UIA — Narrator/NVDA read them); roles/labels built into the element API; headless a11y dumps |
| Headless/testing | `[gpui::test]` macro + TestAppContext | **Headless by design**: render to PNG, dump element tree (`--tree` JSON), scripted input — the prototype's own logic was verified this way |
| Animation | `Animation` element, ad hoc | First-class: analytic springs per property, presence transitions, per-edge layout animation, `live_only()` handlers that skip view/layout entirely (2.0 ms/frame at 6,000 elements) |
| Dependency weight (prototype) | gpui + taffy + resvg + usvg + image + seahash + smol + windows 0.61 (builds green on this machine) | rune (+ vendored wgpu-hal/winit/parley_engine/accesskit patches that apps must carry) |
| Component ecosystem | Zed codebase as reference; gpui-component / gpui-kit community libs | `rune-kit` components (buttons, diff views, markdown), examples incl. a full Settings app |
| Risks | Pre-1.0 breaking changes; single corporate agenda (Zed first); Windows a11y behind an in-flight PR; docs thin | Pre-release: "pin a dev rev"; **no pull requests accepted** (maintainers' agents turn issues around — bus factor is Anomaly's own tooling, OpenCode ships on it); small ecosystem |

## Recommendation

**Rune** — by a wide margin, for this specific product:

1. **Windows-first quality.** Rune ships Windows-specific text rendering (DirectWrite, ClearType
   subpixel, its own outline pipeline), a vendored winit patch fixing Windows-specific
   taskbar/scale bugs, and an FXC-precompiled DX12 path. GPUI's Windows story is younger: the
   editor shipped for Windows in 2025, and accessibility — for a file manager the single most
   user-visible OS integration — is **entirely missing on Windows** today, while Rune's Windows
   a11y path (UIA with headings/landmarks) is already in its vendored AccessKit.
2. **The dithered bar maps to a first-class primitive.** Rune's `Pixels` (replaceable CPU-rendered
   RGBA buffers uploaded by dirty rows) is precisely the port of `StorageBarRenderer` — the
   identity survives without bending the framework. In GPUI the same result requires PNG-blob
   images (`Image::from_bytes`) with a cache-management dance.
3. **Animation model matches Clove's motion vocabulary** (springs, presence transitions,
   settle-and-stop), including the bar's "stop animating once settled" contract — and
   `Tx::live_only()` gives pointer-driven bar animation at ~2 ms CPU/frame without re-running
   the view.
4. **Virtualization semantics fit a file manager**: index-built `virtual_rows`, stable keys,
   scroll-into-view by key for keyboard navigation; measurement anchors keep the reader's place
   when directories load in the background.
5. **Headless tooling** fits this project's benchmarking (Phase 3): scripted input + PNG/JSON
   output make measurable, reproducible UI benchmarks without a GUI session.

GPUI remains a strong fallback: it is more proven at "editor-shaped app" scale, has more
community components, and if Rune's pre-release volatility proves painful mid-project, the
`clove-render`/`clove-fs` architecture keeps the migration cost bounded (only the `clove-ui`
layer would change).

Build/perf measurements from the two prototypes: see `04-prototypes-and-benchmarks.md`.
