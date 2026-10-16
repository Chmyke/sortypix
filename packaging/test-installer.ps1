$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$version = [regex]::Match([IO.File]::ReadAllText((Join-Path $root 'Cargo.toml')), 'version = "([^"]+)"').Groups[1].Value
$setup = Join-Path $PSScriptRoot "out/Sortypix-Setup-$version.exe"
$installed = Join-Path $env:LOCALAPPDATA 'Programs/Sortypix/sortypix.exe'
$key = 'HKCU:\Software\Classes\Directory\shell\Sortypix'
if ((Test-Path $installed) -or (Test-Path $key)) { throw 'Isolated first-install test requires no existing Sortypix installation.' }
$testDir = Join-Path $root ('target/installer-test/Sortypix espace ' + [char]0xE9)
$args = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', ('/DIR="' + $testDir + '"'))
$p = Start-Process -FilePath $setup -ArgumentList $args -WindowStyle Hidden -PassThru -Wait
if ($p.ExitCode -ne 0) { throw "Install failed: $($p.ExitCode)" }
if ([Environment]::OSVersion.Version.Build -ge 22000) {
    $identity = Get-AppxPackage -Name 'MickaelMancini.Sortypix.Explorer'
    if (-not $identity -or $identity.Version -ne "$version.0") { throw 'Modern Explorer identity registration failed.' }
}
$exe = Join-Path $testDir 'sortypix.exe'
if ((Get-FileHash $exe).Hash -ne (Get-FileHash (Join-Path $root 'target/release/sortypix.exe')).Hash) { throw 'Installed binary mismatch' }
$actual = (Get-Item ($key + '\command')).GetValue('')
$expected = '"' + $exe + '" "%1"'
if ($actual -cne $expected) { throw 'Explorer quoting mismatch' }
$background = (Get-Item 'HKCU:\Software\Classes\Directory\Background\shell\Sortypix\command').GetValue('')
if ($background -cne ('"' + $exe + '" "%V"')) { throw 'Explorer background quoting mismatch' }
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
$p = Start-Process -FilePath (Join-Path $testDir 'unins000.exe') -ArgumentList '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART' -WindowStyle Hidden -PassThru -Wait
if ($p.ExitCode -ne 0) { throw "Uninstall failed: $($p.ExitCode)" }
if ((Test-Path $key) -or (Test-Path 'HKCU:\Software\Classes\Directory\Background\shell\Sortypix')) { throw 'Explorer entries remain after uninstall' }
if (Test-Path $exe) { throw 'Application remains after uninstall' }
if (Get-AppxPackage -Name 'MickaelMancini.Sortypix.Explorer') { throw 'Modern Explorer identity remains after uninstall.' }
Write-Output 'Installer, file hash, Explorer commands, Unicode/spaced paths, installed UI and uninstall: PASS'
