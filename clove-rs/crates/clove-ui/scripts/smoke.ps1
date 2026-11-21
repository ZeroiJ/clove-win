# Headless UI smoke test: drives the real app with Rune's `--script`, no window.
#
# Proves the wiring that the engine unit tests cannot: that typing reaches the filter and
# that Delete reaches the trash handler. The move itself is proven by
# `clove-shell`'s recycle round-trip test.
#
# Run from `clove-rs`:  powershell -File crates/clove-ui/scripts/smoke.ps1

$ErrorActionPreference = "Continue"
$root = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))
$exe = Join-Path $root "target\debug\clove.exe"
if (-not (Test-Path $exe)) { throw "build first: cargo build -p clove-ui" }

$sandbox = Join-Path $env:TEMP "clove-ui-smoke"
if (Test-Path $sandbox) { Remove-Item $sandbox -Recurse -Force }
New-Item -ItemType Directory -Path $sandbox | Out-Null
foreach ($name in @("alpha.txt", "beta.txt", "gamma.txt", "delta.txt")) {
    Set-Content -Path (Join-Path $sandbox $name) -Value $name -NoNewline
}
New-Item -ItemType Directory -Path (Join-Path $sandbox "zulu") | Out-Null

$env:CLOVE_START_DIR = $sandbox
$fail = 0

function Run-Script($script, $at, $tree) {
    & $exe --script $script --at $at --tree $tree --scale 1 2>$null | Out-Null
    if (-not (Test-Path $tree)) { throw "no tree from $script" }
    Get-Content $tree -Raw
}

# 1. Typing filters: "alp" matches only alpha.txt, so the header reads "1 of 5".
$tree = Join-Path $sandbox "filter-tree.json"
$text = Run-Script (Join-Path $PSScriptRoot "filter.json") "2.2" $tree
if ($text -match '1 of 5') {
    "PASS  typing reaches the filter field (1 of 5)"
} else {
    "FAIL  filter: expected '1 of 5'"
    $fail = 1
}

# 2. Delete reaches the trash handler: its status line names the Recycle Bin. The worker
#    thread may or may not have finished inside the headless run, so either the intent
#    ("moving …") or the result ("moved …") is accepted.
$tree = Join-Path $sandbox "trash-tree.json"
$text = Run-Script (Join-Path $PSScriptRoot "trash.json") "2.2" $tree
if ($text -match 'Recycle Bin') {
    "PASS  Delete reaches the trash handler"
} else {
    "FAIL  delete: no Recycle Bin status in the tree"
    $fail = 1
}

Remove-Item $sandbox -Recurse -Force -ErrorAction SilentlyContinue
exit $fail
