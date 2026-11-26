// Scheme toggle, copy actions, hero cobalt-dither stripe canvas, and
// direct JS port of Clove's Bayer 4x4 bar renderer (clove-render/src/bar.rs).

const KIND_META = [
  { name: "Apps", rgb: [0.349, 0.612, 1.0], hex: "#599cff" },
  { name: "Code", rgb: [0.436, 0.855, 0.453], hex: "#6fda73" },
  { name: "Photos", rgb: [0.946, 0.404, 0.691], hex: "#f167b0" },
  { name: "Movies", rgb: [0.639, 0.449, 1.0], hex: "#a372ff" },
  { name: "Documents", rgb: [0.332, 0.826, 0.946], hex: "#55d3f1" },
  { name: "Music", rgb: [0.969, 0.818, 0.265], hex: "#f7d144" },
  { name: "System", rgb: [0.698, 0.729, 0.784], hex: "#b2bac8" },
  { name: "Caches", rgb: [0.982, 0.558, 0.233], hex: "#fa8e3b" },
  { name: "Trash", rgb: [0.941, 0.328, 0.284], hex: "#f05448" },
];

const PROFILES = {
  workstation: {
    used: "227.1 GB",
    total: "509.9 GB",
    pct: "45%",
    free: "282.8 GB",
    sizes: ["168.8 GB", "26.1 GB", "5.7 MB", "0 GB", "360.4 KB", "0 GB", "22.6 GB", "9.6 GB", "0 GB"],
    fractions: [0.331, 0.051, 0.008, 0.0, 0.006, 0.0, 0.044, 0.019, 0.0],
  },
  rustdev: {
    used: "372.4 GB",
    total: "1.0 TB",
    pct: "37%",
    free: "627.6 GB",
    sizes: ["64.2 GB", "198.4 GB", "4.1 GB", "12.0 GB", "2.8 GB", "1.4 GB", "31.5 GB", "54.2 GB", "3.8 GB"],
    fractions: [0.064, 0.198, 0.004, 0.012, 0.003, 0.002, 0.032, 0.054, 0.004],
  },
  media: {
    used: "1.48 TB",
    total: "2.0 TB",
    pct: "74%",
    free: "520.0 GB",
    sizes: ["92.0 GB", "48.0 GB", "310.0 GB", "760.0 GB", "18.0 GB", "164.0 GB", "42.0 GB", "36.0 GB", "10.0 GB"],
    fractions: [0.046, 0.024, 0.155, 0.38, 0.009, 0.082, 0.021, 0.018, 0.005],
  },
};

const BAYER = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5].map(
  (m) => (m + 0.5) / 16.0
);

function smoothstep(t) {
  return t * t * (3.0 - 2.0 * t);
}

function clamp01(v) {
  return v < 0 ? 0 : v > 1 ? 1 : v;
}

function hash2d(x, y) {
  let h = (Math.imul(x | 0, 374761393) + Math.imul(y | 0, 668265263)) >>> 0;
  h = Math.imul(h ^ (h >>> 13), 1274126177) >>> 0;
  h = (h ^ (h >>> 16)) >>> 0;
  return (h & 0xffffff) / 16777216.0;
}

function valueNoise(x, y) {
  const xi = Math.floor(x);
  const yi = Math.floor(y);
  const tx = x - xi;
  const ty = y - yi;
  const sx = smoothstep(tx);
  const sy = smoothstep(ty);
  const a = hash2d(xi, yi);
  const b = hash2d(xi + 1, yi);
  const c = hash2d(xi, yi + 1);
  const d = hash2d(xi + 1, yi + 1);
  const top = a + (b - a) * sx;
  const bottom = c + (d - c) * sx;
  return top + (bottom - top) * sy;
}

function initThemeToggle() {
  const btn = document.getElementById("theme-toggle");
  const root = document.documentElement;
  if (!btn) return;

  function apply(scheme) {
    root.dataset.scheme = scheme;
    btn.textContent = scheme === "dark" ? "LIGHT" : "DARK";
  }

  let saved = null;
  try {
    saved = localStorage.getItem("clove-scheme");
  } catch {}
  const qp = new URLSearchParams(window.location.search).get("scheme");
  const initial = qp === "dark" || qp === "light" ? qp : saved === "dark" ? "dark" : "light";
  apply(initial);

  btn.addEventListener("click", () => {
    const next = root.dataset.scheme === "dark" ? "light" : "dark";
    try {
      localStorage.setItem("clove-scheme", next);
    } catch {}
    apply(next);
    drawHeroStripes();
  });
}

function initCopyButtons() {
  const live = document.getElementById("copy-live");
  const aurBtn = document.getElementById("copy-aur-btn");
  const aurLabel = document.getElementById("copy-aur-label");
  const paruBtn = document.getElementById("copy-paru-btn");

  async function copy(cmd, onDone) {
    try {
      await navigator.clipboard.writeText(cmd);
      if (live) live.textContent = `Copied ${cmd}`;
      onDone();
    } catch {
      if (live) live.textContent = cmd;
    }
  }

  if (aurBtn && aurLabel) {
    aurBtn.addEventListener("click", () => {
      const cmd = aurBtn.dataset.cmd || "yay -S clove-linux";
      copy(cmd, () => {
        aurLabel.textContent = "Copied yay -S clove-linux";
        setTimeout(() => {
          aurLabel.textContent = cmd;
        }, 1800);
      });
    });
  }

  if (paruBtn) {
    paruBtn.addEventListener("click", () => {
      const cmd = paruBtn.dataset.cmd || "paru -S clove-linux";
      const orig = paruBtn.textContent;
      copy(cmd, () => {
        paruBtn.textContent = "copied paru -S clove-linux";
        setTimeout(() => {
          paruBtn.textContent = orig;
        }, 1800);
      });
    });
  }
}

function drawHeroStripes() {
  const canvas = document.getElementById("hero-stripes");
  if (!canvas) return;
  const ctx = canvas.getContext("2d");
  const rect = canvas.getBoundingClientRect();
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  const w = Math.max(320, Math.round(rect.width * dpr));
  const h = Math.max(120, Math.round(rect.height * dpr));
  if (canvas.width !== w || canvas.height !== h) {
    canvas.width = w;
    canvas.height = h;
  }

  ctx.clearRect(0, 0, w, h);

  // Ordered architectural cobalt stripe grid aligned strictly to crisp rows (no jitter).
  const isDark = document.documentElement.dataset.scheme === "dark";
  const barW = Math.max(2, Math.round(2.5 * dpr));
  const pitch = barW * 2;
  const cols = Math.ceil(w / pitch);
  const tierH = Math.round(56 * dpr);
  const tiers = [
    { y: 0, h: tierH, washAlpha: isDark ? 0.2 : 0.14, flankSpan: 0.16, solidAlpha: isDark ? 0.88 : 0.92 },
    { y: tierH + Math.round(4 * dpr), h: tierH, washAlpha: isDark ? 0.1 : 0.07, flankSpan: 0.12, solidAlpha: isDark ? 0.82 : 0.85 },
    { y: (tierH + Math.round(4 * dpr)) * 2, h: tierH, washAlpha: 0, flankSpan: 0.08, solidAlpha: isDark ? 0.65 : 0.7 },
    { y: (tierH + Math.round(4 * dpr)) * 3, h: tierH, washAlpha: 0, flankSpan: 0.045, solidAlpha: isDark ? 0.42 : 0.45 },
  ];

  // Align cobalt wings cleanly outside the 1020px centered app frame.
  const winPx = Math.min(1020 * dpr, w - 32 * dpr);
  const winLeft = (w - winPx) / 2;
  const winRight = winLeft + winPx;

  for (const tier of tiers) {
    if (tier.y >= h) break;
    const drawH = Math.min(tier.h, h - tier.y);
    const wingPx = w * tier.flankSpan;

    for (let c = 0; c < cols; c++) {
      const x = c * pitch;
      const onLeftWing = x >= winLeft - wingPx && x < winLeft - 10 * dpr;
      const onRightWing = x > winRight + 10 * dpr && x <= winRight + wingPx;

      let alpha = 0;
      if (onLeftWing || onRightWing) {
        alpha = tier.solidAlpha;
      } else if (tier.washAlpha > 0) {
        alpha = tier.washAlpha;
      }

      if (alpha <= 0) continue;
      ctx.fillStyle = `rgba(13, 37, 255, ${alpha})`;
      ctx.fillRect(x, tier.y, barW, drawH);
    }
  }
}

function initStorageBar() {
  const canvas = document.getElementById("bayer-bar");
  const legendEl = document.getElementById("bar-legend");
  const summaryEl = document.getElementById("bar-summary-stat");
  const readoutEl = document.getElementById("bar-hover-readout");
  const animBtn = document.getElementById("toggle-anim");
  const profBtns = document.querySelectorAll(".prof-btn[data-profile]");
  if (!canvas || !legendEl) return;

  const ctx = canvas.getContext("2d");
  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  let animating = !reducedMotion;
  let activeProfileKey = "workstation";
  let hoveredKind = -1;
  let t = 1.25;
  let lastTs = 0;

  function syncAnimBtn() {
    if (!animBtn) return;
    animBtn.setAttribute("aria-pressed", String(animating));
    animBtn.textContent = animating ? "Pause" : "Play";
  }

  function renderLegend(profile) {
    legendEl.innerHTML = "";
    KIND_META.forEach((kind, idx) => {
      const li = document.createElement("li");
      li.className = "legend-item";
      li.innerHTML = `<span class="legend-dot" style="background:${kind.hex}"></span><span>${kind.name}</span><span class="legend-size">${profile.sizes[idx]}</span>`;
      li.addEventListener("mouseenter", () => {
        hoveredKind = idx;
        if (readoutEl) {
          const pct = Math.round(profile.fractions[idx] * 1000) / 10;
          readoutEl.textContent = `${kind.name}: ${profile.sizes[idx]} (${pct}%)`;
        }
        drawFrame();
      });
      li.addEventListener("mouseleave", () => {
        hoveredKind = -1;
        if (readoutEl) readoutEl.textContent = "Hover any kind to isolate";
        drawFrame();
      });
      legendEl.appendChild(li);
    });

    const freeLi = document.createElement("li");
    freeLi.className = "legend-item";
    freeLi.innerHTML = `<span class="legend-dot" style="background:#54545d"></span><span>Free</span><span class="legend-size">${profile.free}</span>`;
    legendEl.appendChild(freeLi);
  }

  function setProfile(key) {
    activeProfileKey = key;
    const profile = PROFILES[key];
    if (summaryEl) {
      summaryEl.textContent = `${profile.used} / ${profile.total} (${profile.pct})`;
    }
    renderLegend(profile);
    drawFrame();
  }

  function resizeCanvas() {
    const rect = canvas.getBoundingClientRect();
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = Math.max(240, Math.round(rect.width * dpr));
    const h = Math.round(44 * dpr);
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w;
      canvas.height = h;
    }
  }

  function drawFrame() {
    resizeCanvas();
    const profile = PROFILES[activeProfileKey];
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const widthPts = canvas.width / dpr;
    const heightPts = canvas.height / dpr;
    const pitch = 4.0;
    const cols = Math.max(1, Math.floor(widthPts / pitch));
    const rows = Math.max(1, Math.floor(heightPts / pitch));
    const ox = (widthPts - cols * pitch) / 2.0;
    const oy = (heightPts - rows * pitch) / 2.0;

    const edges = [0];
    let acc = 0;
    for (const frac of profile.fractions) {
      acc = Math.min(1, acc + Math.max(0, frac));
      edges.push(acc * widthPts);
    }

    ctx.clearRect(0, 0, canvas.width, canvas.height);

    for (let k = 0; k < KIND_META.length; k++) {
      const x0 = edges[k] * dpr;
      const x1 = edges[k + 1] * dpr;
      if (x1 <= x0) continue;
      const [r, g, b] = KIND_META[k].rgb.map((c) => Math.round(c * 255));
      const dimAlpha = hoveredKind >= 0 && hoveredKind !== k ? 0.04 : 0.12;
      ctx.fillStyle = `rgba(${r}, ${g}, ${b}, ${dimAlpha})`;
      ctx.fillRect(x0, 0, x1 - x0, canvas.height);
    }

    const drift = t * 0.32;
    const step = Math.floor(t * 1.6);
    const litSize = Math.max(2, Math.round(3.0 * dpr));
    const dimSize = Math.max(1, Math.round(1.5 * dpr));
    const cellPx = pitch * dpr;
    const litOff = (cellPx - litSize) / 2.0;
    const dimOff = (cellPx - dimSize) / 2.0;

    for (let j = 0; j < rows; j++) {
      const cy = oy + (j + 0.5) * pitch;
      const edgeLight = Math.exp(-(Math.min(cy, heightPts - cy) - 2.0) / 5.0);
      for (let i = 0; i < cols; i++) {
        const cx = ox + (i + 0.5) * pitch;
        let kind = -1;
        for (let k = 0; k < KIND_META.length; k++) {
          if (cx >= edges[k] && cx < edges[k + 1]) {
            kind = k;
            break;
          }
        }

        const xBase = (ox + i * pitch) * dpr;
        const yBase = (oy + j * pitch) * dpr;

        if (kind < 0) {
          ctx.fillStyle = "rgba(255, 255, 255, 0.07)";
          ctx.fillRect(xBase + dimOff, yBase + dimOff, dimSize, dimSize);
          continue;
        }

        const rgb = KIND_META[kind].rgb;
        const threshold = BAYER[(j & 3) * 4 + (i & 3)];
        const cIdx = j * cols + i;
        const seed = hash2d(i, j) * 7.0;
        const glint = hash2d(cIdx, Math.floor(step + seed)) > 0.994 ? 1.0 : 0.0;
        const cloud =
          valueNoise(i * 0.045 - drift, j * 0.11 + drift * 0.3) * 0.65 +
          valueNoise(i * 0.13 + drift * 0.6, j * 0.3) * 0.35;
        const level =
          -0.04 + 0.66 * edgeLight + 0.44 * smoothstep(clamp01((cloud - 0.42) / 0.38));

        const dimmedOut = hoveredKind >= 0 && hoveredKind !== kind;

        if (level + glint > threshold) {
          const wt = Math.min(1.0, glint * 0.6);
          const r = Math.round((rgb[0] + (1 - rgb[0]) * wt) * 255);
          const g = Math.round((rgb[1] + (1 - rgb[1]) * wt) * 255);
          const b = Math.round((rgb[2] + (1 - rgb[2]) * wt) * 255);
          const alpha = dimmedOut ? 0.25 : 0.95;
          ctx.fillStyle = `rgba(${r}, ${g}, ${b}, ${alpha})`;
          ctx.fillRect(xBase + litOff, yBase + litOff, litSize, litSize);
        } else {
          const r = Math.round(rgb[0] * 255);
          const g = Math.round(rgb[1] * 255);
          const b = Math.round(rgb[2] * 255);
          const alpha = dimmedOut ? 0.1 : 0.27;
          ctx.fillStyle = `rgba(${r}, ${g}, ${b}, ${alpha})`;
          ctx.fillRect(xBase + dimOff, yBase + dimOff, dimSize, dimSize);
        }
      }
    }
  }

  function tick(ts) {
    if (animating) {
      if (lastTs) {
        t += Math.min(0.05, (ts - lastTs) / 1000.0);
      }
      lastTs = ts;
      drawFrame();
    } else {
      lastTs = 0;
    }
    requestAnimationFrame(tick);
  }

  profBtns.forEach((btn) => {
    btn.addEventListener("click", () => {
      profBtns.forEach((b) => b.classList.remove("is-active"));
      btn.classList.add("is-active");
      setProfile(btn.dataset.profile || "workstation");
    });
  });

  if (animBtn) {
    animBtn.addEventListener("click", () => {
      animating = !animating;
      syncAnimBtn();
      drawFrame();
    });
  }

  window.addEventListener("resize", () => {
    drawHeroStripes();
    drawFrame();
  });

  syncAnimBtn();
  setProfile(activeProfileKey);
  requestAnimationFrame(tick);
}

initThemeToggle();
initCopyButtons();
drawHeroStripes();
initStorageBar();
