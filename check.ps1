param([switch]$Fast, [switch]$Installer, [switch]$SkipLinux)
# Single authoritative verification command for Sortypix.
# Stage 1 (always): project context integrity. Stage 2 (once code exists): Rust pipeline.
$ErrorActionPreference = 'Stop'
if ($Fast -and $Installer) { throw '-Installer requires the full verification pipeline.' }
Push-Location $PSScriptRoot
try {
    $failures = @()
    $utf8 = New-Object System.Text.UTF8Encoding($false)

    # Every doc referenced from the entry points must exist. Maintainer only: the project
    # context is not published, so a public clone has no entry point to check.
    $entries = @('CLAUDE.md', 'AGENTS.md', 'docs/contexte/INDEX.md')
    if (-not (Test-Path 'CLAUDE.md')) { $entries = @() }
    foreach ($entry in $entries) {
        if (-not (Test-Path $entry)) { $failures += "Missing entry point: $entry"; continue }
        $text = [IO.File]::ReadAllText((Resolve-Path $entry), $utf8)
        foreach ($match in [regex]::Matches($text, '(?<![\w/.-])(docs/[\w./-]+\.md)')) {
            $target = $match.Groups[1].Value
            if (-not (Test-Path $target)) { $failures += "$entry references missing $target" }
        }
    }

    # Markdown stays UTF-8 without BOM (method rule).
    Get-ChildItem -Recurse -Filter *.md -File |
        Where-Object { $_.FullName -notmatch '\\target\\' } |
        ForEach-Object {
            $bytes = [IO.File]::ReadAllBytes($_.FullName)
            if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) {
                $failures += "BOM found: $($_.FullName)"
            }
        }

    if ($failures.Count -gt 0) { $failures | ForEach-Object { Write-Output "FAIL $_" }; exit 1 }
    Write-Output 'Context: OK'

    # Interface text carries no long dash (owner rule, 2026-10-21). Maintainer only until the
    # checker is listed in publish/public-files.txt.
    $dashes = Join-Path $PSScriptRoot 'scripts/check-dashes.ps1'
    if (Test-Path $dashes) {
        & powershell -NoProfile -ExecutionPolicy Bypass -File $dashes
        if ($LASTEXITCODE -ne 0) { exit 1 }
    }

    # -S: no installed package is visible, so these checks pass under any account
    # (per-user packages are invisible to a sandbox user).
    python -S scripts/build-icon.py --check
    if ($LASTEXITCODE -ne 0) { exit 1 }
    # The current version has its user-facing entry and CHANGELOG.md matches changelog.json.
    python -S scripts/changelog.py --check
    if ($LASTEXITCODE -ne 0) { exit 1 }
    # Maintainer only, the demo media and the publishing tooling are not published.
    if (Test-Path 'scripts/create-demos.py') {
        python -S scripts/create-demos.py --check
        if ($LASTEXITCODE -ne 0) { exit 1 }
    }
    # Nothing private may reach the public tree (publish/private-terms.txt).
    $export = Join-Path $PSScriptRoot 'publish/export-public.ps1'
    if (Test-Path $export) { & $export -ScanOnly }

    if (-not (Test-Path 'Cargo.toml')) {
        Write-Output 'Rust: skipped (no Cargo.toml yet)'
        Write-Output 'PASS'
        exit 0
    }

    # PITFALL: never run cargo from Git Bash on Windows; its link.exe shadows MSVC.
    . (Join-Path $PSScriptRoot 'scripts/toolchain.ps1')
    cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { exit 1 }
    cargo clippy --workspace --all-targets --locked -- -D warnings
    if ($LASTEXITCODE -ne 0) { exit 1 }
    cargo test --workspace --locked
    if ($LASTEXITCODE -ne 0) { exit 1 }
    # Linux builds from the same code: it must lint clean for its target too (when installed).
    if ((rustup target list --installed) -contains 'x86_64-unknown-linux-gnu') {
        cargo clippy --workspace --all-targets --locked --target x86_64-unknown-linux-gnu -- -D warnings
        if ($LASTEXITCODE -ne 0) { exit 1 }
    } elseif (-not $Fast) { throw 'rustup target add x86_64-unknown-linux-gnu' }
    if (-not $Fast) {
        cargo build --workspace --release --examples --bins --lib --locked
        if ($LASTEXITCODE -ne 0) { exit 1 }
        & (Join-Path $PSScriptRoot 'packaging/build-explorer-package.ps1')
        & (Join-Path $PSScriptRoot 'scripts/check-explorer.ps1')
        & (Join-Path $PSScriptRoot 'target/release/examples/performance.exe')
        if ($LASTEXITCODE -ne 0) { exit 1 }
        & (Join-Path $PSScriptRoot 'scripts/check-ui.ps1')
        # Tests, the native UI probe and the install scripts on a real Linux desktop (ARM64).
        if ($SkipLinux) { Write-Output 'Linux: SKIPPED (-SkipLinux)' }
        else { & (Join-Path $PSScriptRoot 'scripts/check-linux.ps1') -Diagnostic }
        $hash = (Get-FileHash (Join-Path $PSScriptRoot 'target/release/sortypix.exe') -Algorithm SHA256).Hash
        [IO.File]::WriteAllText((Join-Path $PSScriptRoot 'target/reports/verified-sha256.txt'), $hash, $utf8)
        $shellHash = (Get-FileHash (Join-Path $PSScriptRoot 'target/release/sortypix_shell.dll')).Hash
        [IO.File]::WriteAllText((Join-Path $PSScriptRoot 'target/reports/verified-shell-sha256.txt'), $shellHash, $utf8)
        if ($Installer) {
            & (Join-Path $PSScriptRoot 'packaging/build-installer.ps1')
            & (Join-Path $PSScriptRoot 'packaging/test-installer.ps1')
        }
    }
    Write-Output 'PASS'
} finally { Pop-Location }
