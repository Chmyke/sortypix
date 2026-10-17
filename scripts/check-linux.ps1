# Cross-compiles Sortypix for a Linux ARM64 machine and runs its tests there over SSH.
# Nothing is compiled on the remote host; binaries run with low priority in ~/sortypix-dev.
# -Diagnostic also runs the release window through the native UI probe on the remote
# desktop, checks that every photo comes back byte for byte, and tries the install scripts.
# Needs: rustup target aarch64-unknown-linux-gnu, cargo-zigbuild, Zig (pip install ziglang),
# and key-based SSH access to the host (default alias "pi").
param([string]$RemoteHost = 'pi', [switch]$Diagnostic)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'linux-toolchain.ps1')
# glibc 2.31, as the published archives: the probed binary is the one that ships.
$target = 'aarch64-unknown-linux-gnu'
$glibc = "$target.2.31"
Push-Location $root
try {
    $artifacts = cargo zigbuild --target $glibc --workspace --tests --locked --message-format json |
        Where-Object { $_ -like '{*' } | ForEach-Object { $_ | ConvertFrom-Json }
    if ($LASTEXITCODE -ne 0) { throw 'Cross-compilation failed.' }
    $tests = @($artifacts | Where-Object { $_.reason -eq 'compiler-artifact' -and $_.profile.test -and $_.executable } |
        ForEach-Object { $_.executable })
    $release = Join-Path $root "target/$target/release"
    if ($Diagnostic) {
        cargo zigbuild --target $glibc --release --locked -p sortypix-ui --bin sortypix --example fixtures
        if ($LASTEXITCODE -ne 0) { throw 'Release cross-compilation failed.' }
    }
    ssh -o BatchMode=yes $RemoteHost 'rm -rf ~/sortypix-dev && mkdir -p ~/sortypix-dev/bin ~/sortypix-dev/fixtures ~/sortypix-dev/package/icons'
    if ($LASTEXITCODE -ne 0) { throw "SSH to $RemoteHost failed." }
    scp -q -o BatchMode=yes @tests "${RemoteHost}:sortypix-dev/bin/"
    if ($LASTEXITCODE -ne 0) { throw 'Copy failed.' }
    $fixtures = @(Get-ChildItem (Join-Path $root 'tests/fixtures') -File | ForEach-Object { $_.FullName })
    scp -q -o BatchMode=yes @fixtures "${RemoteHost}:sortypix-dev/fixtures/"
    if ($LASTEXITCODE -ne 0) { throw 'Copy failed.' }
    if ($Diagnostic) {
        $package = @('install.sh', 'uninstall.sh' | ForEach-Object { Join-Path $root "packaging/linux/$_" }) +
            (Join-Path $root 'packaging/test-linux-install.sh') + (Join-Path $release 'sortypix')
        scp -q -o BatchMode=yes @package "${RemoteHost}:sortypix-dev/package/"
        if ($LASTEXITCODE -ne 0) { throw 'Copy failed.' }
        $icons = 'png', 'svg' | ForEach-Object { Join-Path $root "assets/sortypix.$_" }
        scp -q -o BatchMode=yes @icons "${RemoteHost}:sortypix-dev/package/icons/"
        if ($LASTEXITCODE -ne 0) { throw 'Copy failed.' }
        scp -q -o BatchMode=yes (Join-Path $release 'examples/fixtures') "${RemoteHost}:sortypix-dev/make-fixtures"
        if ($LASTEXITCODE -ne 0) { throw 'Copy failed.' }
    }
    $run = 'cd ~/sortypix-dev/bin && chmod +x * && status=0; export SORTYPIX_FIXTURES=~/sortypix-dev/fixtures; ' +
        'for t in *; do case "$t" in *.log) continue;; esac; ' +
        'nice -n 19 "./$t" -q >"$t.log" 2>&1 || { status=1; echo "FAILED: $t"; tail -n 30 "$t.log"; }; done; '
    if ($Diagnostic) {
        # The probe drives the real window on the remote desktop session, then closes it.
        $run += 'cd ~/sortypix-dev && chmod +x make-fixtures package/sortypix && ' +
            'export XDG_RUNTIME_DIR=/run/user/$(id -u) WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-wayland-0}; ' +
            'nice -n 19 ./make-fixtures photos >/dev/null && before=$(cd photos && sha256sum * | sort); ' +
            'timeout 90 nice -n 10 package/sortypix --diagnostic photos ui; code=$?; ' +
            'if [ -f ui/PASS.txt ] && [ $code -eq 0 ]; then echo "ui: $(cat ui/PASS.txt)"; ' +
            'else status=1; echo "FAILED: ui (exit $code): $(cat ui/FAIL.txt 2>/dev/null)"; fi; ' +
            'after=$(cd photos && sha256sum * | sort); ' +
            '[ "$before" = "$after" ] && [ -z "$(find photos -mindepth 1 -type d)" ] && echo "photos: restored byte for byte" ' +
            '|| { status=1; echo "FAILED: the probe changed the photos"; }; ' +
            # Speed on this machine, for the record: the budgets are set for a Windows PC.
            'timeout 90 nice -n 10 package/sortypix --benchmark photos bench >/dev/null 2>&1; ' +
            'echo "benchmark (informative): $(head -c 400 bench/PASS.txt bench/FAIL.txt 2>/dev/null | tr ''\n'' '' '')"; ' +
            # Install and removal in a throwaway home: the script prints its own verdict.
            'sh package/test-linux-install.sh ~/sortypix-dev/package ~/sortypix-dev/install-home || status=1; '
    }
    $run += 'exit $status'
    # PITFALL: Windows PowerShell drops the double quotes of native arguments; the script
    # travels base64-encoded so the remote shell receives it byte for byte.
    $encoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($run))
    ssh -o BatchMode=yes $RemoteHost "echo $encoded | base64 -d | bash"
    if ($LASTEXITCODE -ne 0) { throw "Linux checks failed on $RemoteHost." }
    if ($Diagnostic) {
        # The archive may only carry the binary this probe exercised (build-linux.ps1).
        $reports = Join-Path $root 'target/reports'
        [IO.Directory]::CreateDirectory($reports) | Out-Null
        $hash = (Get-FileHash (Join-Path $release 'sortypix') -Algorithm SHA256).Hash
        [IO.File]::WriteAllText((Join-Path $reports 'verified-linux-arm64-sha256.txt'), $hash, (New-Object Text.UTF8Encoding($false)))
    }
    Write-Output "Linux: PASS ($($tests.Count) test binaries on $RemoteHost)"
} finally { Pop-Location }
