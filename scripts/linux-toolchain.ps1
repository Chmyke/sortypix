# Dot-sourced by the Linux scripts: Zig for cargo-zigbuild, and the MSVC linker that the
# build scripts need since they run on Windows (scripts/toolchain.ps1).
if (-not $env:CARGO_ZIGBUILD_ZIG_PATH) {
    $zig = python -c "import ziglang, os; print(os.path.join(os.path.dirname(ziglang.__file__), 'zig.exe'))"
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path $zig)) { throw 'Zig not found: python -m pip install --user ziglang' }
    $env:CARGO_ZIGBUILD_ZIG_PATH = $zig
}
. (Join-Path $PSScriptRoot 'toolchain.ps1')
