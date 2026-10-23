# Installs the Sortypix AI beta into a test folder, runs it from there, checks its Explorer
# commands and shortcuts, uninstalls it, and proves Sortypix (installed or not) was not touched:
# same files, same registry entries, same settings.json.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$version = [regex]::Match([IO.File]::ReadAllText((Join-Path $root 'Cargo.toml')), 'version = "([^"]+)"').Groups[1].Value
$setup = Join-Path $PSScriptRoot "out/Sortypix-AI-Setup-$version.exe"
$appId = '{A6C7B8C5-53FC-4328-9F7A-47FD7275B225}_is1'
$uninstallRoot = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
$key = 'HKCU:\Software\Classes\Directory\shell\SortypixAI'
$backgroundKey = 'HKCU:\Software\Classes\Directory\Background\shell\SortypixAI'
if ((Test-Path (Join-Path $env:LOCALAPPDATA 'Programs/Sortypix AI')) -or (Test-Path $key) -or (Test-Path (Join-Path $uninstallRoot $appId))) {
    throw 'Isolated first-install test requires no existing Sortypix AI installation.'
}

# Everything of Sortypix this installer must leave alone.
function Get-SortypixState {
    $state = New-Object Collections.Generic.List[string]
    $app = Join-Path $env:LOCALAPPDATA 'Programs/Sortypix'
    if (Test-Path $app) {
        Get-ChildItem -LiteralPath $app -Recurse -File | Sort-Object FullName | ForEach-Object {
            $state.Add("file $($_.FullName) $((Get-FileHash -LiteralPath $_.FullName).Hash)")
        }
    }
    $keys = 'HKCU:\Software\Classes\Directory\shell\Sortypix', 'HKCU:\Software\Classes\Directory\shell\Sortypix\command',
        'HKCU:\Software\Classes\Directory\Background\shell\Sortypix', 'HKCU:\Software\Classes\Directory\Background\shell\Sortypix\command',
        "$uninstallRoot\{738617A0-A20C-4305-8CF2-7DB1491952AB}_is1"
    foreach ($k in $keys) {
        if (-not (Test-Path $k)) { $state.Add("key $k absent"); continue }
        $item = Get-Item $k
        foreach ($name in ($item.GetValueNames() | Sort-Object)) { $state.Add("key $k [$name] $($item.GetValue($name))") }
    }
    $settings = Join-Path $env:LOCALAPPDATA 'Sortypix/settings.json'
    if (Test-Path $settings) { $state.Add("settings $((Get-FileHash -LiteralPath $settings).Hash)") }
    foreach ($link in (Join-Path ([Environment]::GetFolderPath('Programs')) 'Sortypix.lnk'), (Join-Path ([Environment]::GetFolderPath('Desktop')) 'Sortypix.lnk')) {
        if (Test-Path $link) { $state.Add("link $link $((Get-FileHash -LiteralPath $link).Hash)") }
    }
    $identity = Get-AppxPackage -Name 'MickaelMancini.Sortypix.Explorer'
    if ($identity) { $state.Add("identity $($identity.Version) $($identity.Status)") }
    return $state
}
$before = Get-SortypixState

$testDir = Join-Path $root ('target/installer-test/Sortypix AI espace ' + [char]0xE9)
$p = Start-Process -FilePath $setup -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', ('/DIR="' + $testDir + '"')) -WindowStyle Hidden -PassThru -Wait
if ($p.ExitCode -ne 0) { throw "Install failed: $($p.ExitCode)" }
$exe = Join-Path $testDir 'sortypix-ai.exe'
if ((Get-FileHash $exe).Hash -ne (Get-FileHash (Join-Path $root 'target/release/sortypix-ai.exe')).Hash) { throw 'Installed binary mismatch' }
$models = Join-Path $testDir 'models'
$bundled = @('aesthetic-l14.bin', 'onnxruntime.dll', 'textual-int8.onnx', 'tokenizer.json', 'visual-int8.onnx')
$found = @(Get-ChildItem -LiteralPath $models -File | Sort-Object Name | Select-Object -ExpandProperty Name)
if (Compare-Object $bundled $found) { throw "Unexpected model files: $($found -join ', ')" }
foreach ($name in 'LICENSE', 'THIRD-PARTY-NOTICES.txt') {
    if (-not (Test-Path (Join-Path $testDir $name))) { throw "Missing $name" }
}
$notices = [IO.File]::ReadAllText((Join-Path $testDir 'THIRD-PARTY-NOTICES.txt'))
foreach ($expected in '=== ort ', 'OpenAI CLIP', 'improved aesthetic predictor', 'ONNX Runtime 1.31.0: ThirdPartyNotices.txt') {
    if (-not $notices.Contains($expected)) { throw "Notices lack: $expected" }
}
if ((Get-Item ($key + '\command')).GetValue('') -cne ('"' + $exe + '" "%1"')) { throw 'Explorer quoting mismatch' }
if ((Get-Item ($backgroundKey + '\command')).GetValue('') -cne ('"' + $exe + '" "%V"')) { throw 'Explorer background quoting mismatch' }
if ((Get-Item $key).GetValue('') -cne 'Open with Sortypix AI') { throw 'Explorer label mismatch' }
$links = (Join-Path ([Environment]::GetFolderPath('Programs')) 'Sortypix AI.lnk'), (Join-Path ([Environment]::GetFolderPath('Desktop')) 'Sortypix AI.lnk')
foreach ($link in $links) { if (-not (Test-Path $link)) { throw "Missing shortcut $link" } }

# Runs from the installed folder only: ai-dev must not be a fallback.
$previousAiDir = $env:SORTYPIX_AI_DIR
$env:SORTYPIX_AI_DIR = Join-Path $root 'target/installer-test/no-ai-dev'
try {
    $photos = Join-Path $root ('target/installer-test/photos espace ' + [char]0xE9)
    $report = Join-Path $root ('target/installer-test/ui-' + [Guid]::NewGuid().ToString('N'))
    & (Join-Path $root 'target/release/examples/fixtures.exe') $photos
    if ($LASTEXITCODE -ne 0) { throw 'Installed diagnostic fixture generation failed' }
    $original = @(Get-ChildItem -LiteralPath $photos -File | Sort-Object Name | Get-FileHash | Select-Object -ExpandProperty Hash)
    $p = Start-Process -FilePath $exe -ArgumentList @('--diagnostic', ('"' + $photos + '"'), ('"' + $report + '"')) -WindowStyle Hidden -PassThru
    if (-not $p.WaitForExit(55000)) { Stop-Process -Id $p.Id; throw 'Installed UI diagnostic timeout' }
    if ($p.ExitCode -ne 0 -or -not (Test-Path (Join-Path $report 'PASS.txt')) -or (Test-Path (Join-Path $report 'FAIL.txt'))) { throw 'Installed UI diagnostic failed' }
    $restored = @(Get-ChildItem -LiteralPath $photos -File | Sort-Object Name | Get-FileHash | Select-Object -ExpandProperty Hash)
    if (Compare-Object $original $restored) { throw 'Installed diagnostic changed source bytes' }
    Write-Output 'Installed UI diagnostic: PASS'

    # The benchmark waits for the analysis, so it proves the bundled models load and run.
    $large = Join-Path $root 'target/ai-benchmark-photos'
    & (Join-Path $root 'target/release/examples/fixtures.exe') $large '--large'
    if ($LASTEXITCODE -ne 0) { throw 'Large fixture generation failed' }
    $bench = Join-Path $root ('target/installer-test/ai-' + [Guid]::NewGuid().ToString('N'))
    $p = Start-Process -FilePath $exe -ArgumentList @('--benchmark', ('"' + $large + '"'), ('"' + $bench + '"')) -WindowStyle Hidden -PassThru
    if (-not $p.WaitForExit(60000)) { Stop-Process -Id $p.Id; throw 'Installed AI benchmark timeout' }
    $result = @('PASS.txt', 'FAIL.txt') | ForEach-Object { Join-Path $bench $_ } | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $result) { throw 'Installed AI benchmark wrote no report' }
    $text = [IO.File]::ReadAllText($result)
    Write-Output "Installed AI benchmark ($(Split-Path -Leaf $result)):"
    Write-Output $text
    $used = [regex]::Match($text, '(?m)^AI models: (.+?)\r?$').Groups[1].Value
    if (-not $used -or [IO.Path]::GetFullPath($used) -ne [IO.Path]::GetFullPath($models)) {
        throw "The installed application did not use its bundled models: '$used'"
    }
    if ($text -notmatch 'AI analysis at start: ready') { throw 'The bundled models did not analyse any photo' }
} finally { $env:SORTYPIX_AI_DIR = $previousAiDir }

$p = Start-Process -FilePath (Join-Path $testDir 'unins000.exe') -ArgumentList '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART' -WindowStyle Hidden -PassThru -Wait
if ($p.ExitCode -ne 0) { throw "Uninstall failed: $($p.ExitCode)" }
# The uninstaller removes its own files from a copy that exits after it: wait for it briefly.
$deadline = (Get-Date).AddSeconds(20)
while ((Test-Path $exe) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 200 }
if ((Test-Path $key) -or (Test-Path $backgroundKey)) { throw 'Explorer entries remain after uninstall' }
if ((Test-Path $exe) -or (Test-Path $models)) { throw 'Application remains after uninstall' }
if (Test-Path (Join-Path $uninstallRoot $appId)) { throw 'Uninstall entry remains after uninstall' }
foreach ($link in $links) { if (Test-Path $link) { throw "Shortcut remains after uninstall: $link" } }

$after = Get-SortypixState
if (Compare-Object @($before) @($after)) { throw "Sortypix was changed:`n$((Compare-Object @($before) @($after) | Out-String))" }
Write-Output "Sortypix untouched: $($before.Count) files, keys and settings compared"
Write-Output 'AI installer, file hashes, bundled models, Explorer commands, shortcuts, Unicode/spaced paths, installed UI, uninstall and Sortypix intact: PASS'
