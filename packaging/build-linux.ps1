# Builds packaging/out/sortypix-<version>-linux-<arch>.tar.gz for ARM64 and x86_64.
# Cross-compiles with cargo-zigbuild against glibc 2.31 (Debian 11, Ubuntu 20.04 and later)
# and ships the license notices of every crate in the Linux binary.
# The ARM64 binary must be the one scripts/check-linux.ps1 -Diagnostic just exercised.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if (git -C $root status --porcelain) { throw 'Commit source changes before building the deliverable.' }
. (Join-Path $root 'scripts/linux-toolchain.ps1')
$utf8 = New-Object System.Text.UTF8Encoding $false
$cargo = [IO.File]::ReadAllText((Join-Path $root 'Cargo.toml'), $utf8)
$version = [regex]::Match($cargo, '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"').Groups[1].Value
if (-not $version) { throw 'Cannot read the workspace version.' }
$out = Join-Path $PSScriptRoot 'out'
New-Item -ItemType Directory -Force $out | Out-Null
$proof = Join-Path $root 'target/reports/verified-linux-arm64-sha256.txt'
Push-Location $root
try {
    $lines = @()
    foreach ($arch in @(@{ Name = 'arm64'; Triple = 'aarch64-unknown-linux-gnu' }, @{ Name = 'x86_64'; Triple = 'x86_64-unknown-linux-gnu' })) {
        cargo zigbuild --target "$($arch.Triple).2.31" --release --locked -p sortypix-ui --bin sortypix
        if ($LASTEXITCODE -ne 0) { throw "Build failed for $($arch.Name)." }
        $binary = Join-Path $root "target/$($arch.Triple)/release/sortypix"
        if ($arch.Name -eq 'arm64' -and (-not (Test-Path $proof) -or
                (Get-FileHash $binary).Hash -ne ([IO.File]::ReadAllText($proof)).Trim())) {
            throw 'Run scripts/check-linux.ps1 -Diagnostic (or check.ps1) on this release before packaging.'
        }
        $name = "sortypix-$version-linux-$($arch.Name)"
        $stage = Join-Path $out "linux-stage/$name"
        if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
        New-Item -ItemType Directory -Force (Join-Path $stage 'icons') | Out-Null
        Copy-Item $binary $stage
        Copy-Item (Join-Path $PSScriptRoot 'linux/*') $stage
        Copy-Item (Join-Path $root 'LICENSE') (Join-Path $stage 'LICENSE.txt')
        Copy-Item (Join-Path $root 'assets/sortypix.png'), (Join-Path $root 'assets/sortypix.svg') (Join-Path $stage 'icons')
        & (Join-Path $root 'scripts/notices.ps1') -Triple $arch.Triple -Platform 'Linux' -Output (Join-Path $stage 'THIRD-PARTY-NOTICES.txt')
        $archive = Join-Path $out "$name.tar.gz"
        tar -czf $archive -C (Split-Path $stage) $name
        if ($LASTEXITCODE -ne 0) { throw "Archive failed for $($arch.Name)." }
        # The license and the pointer to the source code travel with the binary: read the
        # archive back and refuse one that lacks one of them.
        $listed = tar -tzf $archive
        foreach ($required in 'sortypix', 'install.sh', 'LICENSE.txt', 'ABOUT.txt', 'THIRD-PARTY-NOTICES.txt', 'icons/sortypix.png') {
            if ($listed -notcontains "$name/$required") {
                Remove-Item $archive
                throw "The $($arch.Name) archive does not carry $required. Nothing was kept in packaging/out."
            }
        }
        $hash = (Get-FileHash $archive -Algorithm SHA256).Hash
        $lines += "$hash  $name.tar.gz"
        Write-Output ("{0}  {1} ({2} bytes)" -f $hash, (Split-Path $archive -Leaf), (Get-Item $archive).Length)
    }
    [IO.File]::WriteAllText((Join-Path $out 'SHA256-linux.txt'), (($lines -join "`n") + "`n"), $utf8)
} finally { Pop-Location }
