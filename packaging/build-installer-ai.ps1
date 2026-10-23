# Builds the Sortypix AI installer (packaging/sortypix-ai.iss) beside the Sortypix one: same
# version, same changelog entry (refused without it). The binary must be the one check.ps1
# verified, and every bundled model file must match the hashes recorded by
# scripts/ai/fetch-models.py, themselves built from the sources pinned in packaging/ai/models.json.
# SORTYPIX_AI_DIR names the ai-dev root (default %LOCALAPPDATA%\Sortypix\ai-dev).
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$utf8 = New-Object Text.UTF8Encoding($false)
if (git -C $root status --porcelain) { throw 'Commit source changes before building the deliverable.' }
$version = [regex]::Match([IO.File]::ReadAllText((Join-Path $root 'Cargo.toml')), 'version = "([^"]+)"').Groups[1].Value
$iss = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'sortypix-ai.iss'))
if ([regex]::Match($iss, 'AppVersion "([^"]+)"').Groups[1].Value -ne $version) { throw 'Installer version mismatch' }
if ($iss -notmatch '(?m)^OutputBaseFilename=Sortypix-AI-Setup-\{#AppVersion\}\r?$') { throw 'Unexpected installer output name' }
python -S (Join-Path $root 'scripts/changelog.py') --notes $version | Out-Null
if ($LASTEXITCODE -ne 0) { throw "No release without its changelog.json entry: $version" }
$exe = Join-Path $root 'target/release/sortypix-ai.exe'
$verified = Join-Path $root 'target/reports/verified-ai-sha256.txt'
if (-not (Test-Path $verified) -or (Get-FileHash $exe).Hash -ne ([IO.File]::ReadAllText($verified)).Trim()) {
    throw 'Run check.ps1 (without -SkipAI, models present) on this release before packaging.'
}

$aiRoot = $env:SORTYPIX_AI_DIR
if (-not $aiRoot) { $aiRoot = Join-Path $env:LOCALAPPDATA 'Sortypix\ai-dev' }
$models = Join-Path $aiRoot 'models'
$manifest = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'ai/models.json'), $utf8) | ConvertFrom-Json
$record = [IO.File]::ReadAllText((Join-Path $models 'SHA256SUMS.json'), $utf8) | ConvertFrom-Json
foreach ($source in $manifest.sources) {
    $built = $record.sources.($source.name)
    if ($built -ne $source.sha256) { throw "$($source.name): the models were built from a source that differs from the manifest" }
}
$bundled = 'onnxruntime.dll', 'visual-int8.onnx', 'textual-int8.onnx', 'tokenizer.json', 'aesthetic-l14.bin'
foreach ($name in $bundled) {
    $want = $record.files.$name
    if (-not $want) { throw "${name}: no recorded SHA256 in $models\SHA256SUMS.json" }
    $path = Join-Path $models $name
    if ((Get-Item -LiteralPath $path).Length -ne [int64]$want.bytes) { throw "${name}: size mismatch" }
    if ((Get-FileHash -LiteralPath $path).Hash -ne $want.sha256.ToUpperInvariant()) { throw "${name}: SHA256 mismatch" }
}
Write-Output "Model files verified: $($bundled -join ', ')"

# Notices: the crates of the AI binary, the model notices, then ONNX Runtime's own license and
# third-party notices taken from the pinned archive the DLL was extracted from.
$ortName = 'onnxruntime-win-x64-1.31.0.zip'
$ortZip = Join-Path $aiRoot "downloads/$ortName"
$ortPin = ($manifest.sources | Where-Object { $_.name -eq $ortName }).sha256
if ((Get-FileHash -LiteralPath $ortZip).Hash -ne $ortPin.ToUpperInvariant()) { throw "$ortName does not match the manifest" }
$stage = Join-Path $PSScriptRoot 'out/ai'
[IO.Directory]::CreateDirectory($stage) | Out-Null
$crates = Join-Path $stage 'crate-notices.txt'
& (Join-Path $root 'scripts/notices.ps1') -Features ai -Output $crates
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::OpenRead($ortZip)
try {
    $ortText = foreach ($member in 'LICENSE', 'ThirdPartyNotices.txt') {
        $entry = $archive.GetEntry("onnxruntime-win-x64-1.31.0/$member")
        if (-not $entry) { throw "$ortName has no $member" }
        $reader = New-Object IO.StreamReader($entry.Open(), $utf8)
        try { "`n=== ONNX Runtime 1.31.0: $member ===`n" + $reader.ReadToEnd() } finally { $reader.Dispose() }
    }
} finally { $archive.Dispose() }
$notices = Join-Path $stage 'THIRD-PARTY-NOTICES.txt'
$text = [IO.File]::ReadAllText($crates, $utf8) + "`n" +
    [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'ai/MODEL-NOTICES.txt'), $utf8) + ($ortText -join '')
[IO.File]::WriteAllText($notices, $text, $utf8)

$compiler = Join-Path $env:LOCALAPPDATA 'Programs/Inno Setup 6/ISCC.exe'
if (-not (Test-Path $compiler)) { $compiler = (Get-Command ISCC.exe -ErrorAction Stop).Source }
& $compiler /Q "/DModels=$models" "/DNotices=$notices" (Join-Path $PSScriptRoot 'sortypix-ai.iss')
if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed' }
$name = "Sortypix-AI-Setup-$version.exe"
$setup = Join-Path $PSScriptRoot "out/$name"
$hash = (Get-FileHash $setup).Hash
[IO.File]::WriteAllText((Join-Path $PSScriptRoot 'out/SHA256-AI.txt'), "$hash  $name`n", $utf8)
Write-Output "Installer: $setup ($((Get-Item $setup).Length) bytes)"
Write-Output "SHA256: $hash"
