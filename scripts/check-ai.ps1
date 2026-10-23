# Sortypix AI probes: model integrity, speed and quality on the real models, plus the guard that
# the normal application never links ONNX Runtime. Run from check.ps1 after the release build.
# -GuardOnly runs the dependency and binary guards alone (needs no models); -Binary names the
# executable to scan (default target/release/sortypix.exe, skipped while it is not built).
# SORTYPIX_AI_DIR names the ai-dev root (models/, corpus/, calibration/); default
# %LOCALAPPDATA%\Sortypix\ai-dev on Windows.
param([switch]$GuardOnly, [string]$Binary)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    . (Join-Path $PSScriptRoot 'toolchain.ps1')
    # Normal Sortypix must stay free of the AI runtime (plan 1, task 9).
    $tree = cargo tree -p sortypix-ui --locked
    if ($LASTEXITCODE -ne 0) { Write-Output 'FAIL cargo tree -p sortypix-ui'; exit 1 }
    if ($tree | Select-String ' ort ') {
        Write-Output 'FAIL sortypix-ui depends on ort (ONNX Runtime): the normal build must not.'
        exit 1
    }
    Write-Output 'AI guard: sortypix-ui has no ort dependency'
    # The released binary itself: no ONNX Runtime name, in ASCII or UTF-16 (plan 2 Review Focus 5).
    # $Binary lets the guard be proven against sortypix-ai.exe, which must be refused.
    if (-not $Binary) { $Binary = Join-Path $root 'target/release/sortypix.exe' }
    if (Test-Path -LiteralPath $Binary) {
        $bytes = [IO.File]::ReadAllBytes($Binary)
        $latin = [Text.Encoding]::GetEncoding(28591).GetString($bytes)
        $wide = [Text.Encoding]::Unicode.GetString($bytes) + [Text.Encoding]::Unicode.GetString($bytes, 1, $bytes.Length - 1)
        if ($latin.IndexOf('onnxruntime', [StringComparison]::OrdinalIgnoreCase) -ge 0 -or
            $wide.IndexOf('onnxruntime', [StringComparison]::OrdinalIgnoreCase) -ge 0) {
            Write-Output "FAIL $Binary contains 'onnxruntime': the normal build must not."
            exit 1
        }
        Write-Output "AI guard: $(Split-Path -Leaf $Binary) contains no 'onnxruntime'"
    } elseif ($PSBoundParameters.ContainsKey('Binary')) { Write-Output "FAIL missing $Binary"; exit 1 }
    if ($GuardOnly) { exit 0 }

    if (-not $env:SORTYPIX_AI_DIR) { $env:SORTYPIX_AI_DIR = Join-Path $env:LOCALAPPDATA 'Sortypix\ai-dev' }
    python -I scripts/ai/fetch-models.py --check --dir $env:SORTYPIX_AI_DIR
    if ($LASTEXITCODE -ne 0) { Write-Output 'FAIL fetch-models.py --check'; exit 1 }
    # A stale report must not satisfy the existence check below.
    foreach ($report in 'ai-speed.txt', 'ai-quality.txt') {
        Remove-Item (Join-Path $root "target/reports/$report") -ErrorAction SilentlyContinue
    }
    foreach ($probe in 'ai_speed', 'ai_quality') {
        & (Join-Path $root "target/release/examples/$probe.exe")
        if ($LASTEXITCODE -ne 0) { Write-Output "FAIL $probe"; exit 1 }
    }
    foreach ($report in 'ai-speed.txt', 'ai-quality.txt') {
        if (-not (Test-Path (Join-Path $root "target/reports/$report"))) {
            Write-Output "FAIL missing report $report"; exit 1
        }
    }
    Write-Output 'AI probes: OK'
} finally { Pop-Location }
