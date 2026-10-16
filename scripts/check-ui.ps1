$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$run = Join-Path $root ('target/check-' + [Guid]::NewGuid().ToString('N'))
$photos = Join-Path $run 'photos'
$screens = Join-Path $run 'ui'
$benchmark = Join-Path $run 'benchmark'
& (Join-Path $root 'target/release/examples/fixtures.exe') $photos
if ($LASTEXITCODE -ne 0) { throw 'Fixture generation failed' }
$before = @(Get-ChildItem -LiteralPath $photos -File | Sort-Object Name | Get-FileHash | Select-Object -ExpandProperty Hash)
$exe = Join-Path $root 'target/release/sortypix.exe'
foreach ($mode in @(@('--diagnostic', $screens), @('--benchmark', $benchmark))) {
    $arguments = @($mode[0], ('"' + $photos + '"'), ('"' + $mode[1] + '"'))
    $process = Start-Process -FilePath $exe -ArgumentList $arguments -WindowStyle Hidden -PassThru
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $peakBytes = [long]0
    while (-not $process.WaitForExit(100)) {
        $process.Refresh()
        if (-not $process.HasExited) { $peakBytes = [Math]::Max($peakBytes, $process.PeakWorkingSet64) }
        if ($timer.Elapsed.TotalSeconds -gt 55) { Stop-Process -Id $process.Id; throw 'UI diagnostic timeout' }
    }
    if ($process.ExitCode -ne 0) { throw "UI exited with $($process.ExitCode)" }
    $failure = Join-Path $mode[1] 'FAIL.txt'
    if (Test-Path $failure) { throw ([IO.File]::ReadAllText($failure)) }
    $pass = Join-Path $mode[1] 'PASS.txt'
    if (-not (Test-Path $pass)) { throw "No successful UI report in $($mode[1])" }
    Write-Output ([IO.File]::ReadAllText($pass))
    $memory = ('Peak working set ({0}): {1:N1} MiB' -f $mode[0], ($peakBytes / 1MB))
    [IO.File]::WriteAllText((Join-Path $mode[1] 'memory.txt'), $memory)
    Write-Output $memory
}
$session = Join-Path $run 'session'
$other = Join-Path $run 'other'
& (Join-Path $root 'target/release/examples/fixtures.exe') $other
if ($LASTEXITCODE -ne 0) { throw 'Session fixture generation failed' }
foreach ($mode in @('--session-store', '--session-restore', '--session-other')) {
    $source = if ($mode -eq '--session-other') { $other } else { $photos }
    $process = Start-Process -FilePath $exe -ArgumentList @($mode, ('"' + $source + '"'), ('"' + $session + '"')) -WindowStyle Hidden -PassThru
    if (-not $process.WaitForExit(20000)) { Stop-Process -Id $process.Id; throw 'Session probe timeout' }
    if ($process.ExitCode -ne 0) { throw "Session probe exited with $($process.ExitCode)" }
    if (Test-Path (Join-Path $session 'FAIL.txt')) { throw ([IO.File]::ReadAllText((Join-Path $session 'FAIL.txt'))) }
    $report = Join-Path $session ($mode.TrimStart('-') + '.PASS.txt')
    if (-not (Test-Path $report)) { throw "Session report missing: $mode" }
    Write-Output ([IO.File]::ReadAllText($report))
}
$after = @(Get-ChildItem -LiteralPath $photos -File | Sort-Object Name | Get-FileHash | Select-Object -ExpandProperty Hash)
if (Compare-Object $before $after) { throw 'UI actions and undo changed fixture bytes' }
if ((Get-ChildItem -LiteralPath $photos -Directory).Count -ne 0) { throw 'Undo left a sorting directory behind' }
$reports = Join-Path $root 'target/reports'
[IO.Directory]::CreateDirectory($reports) | Out-Null
[IO.File]::WriteAllText((Join-Path $reports 'latest-ui.txt'), $run, (New-Object Text.UTF8Encoding($false)))
Copy-Item -LiteralPath (Join-Path $benchmark 'performance.txt') -Destination (Join-Path $reports 'ui-performance.txt')
Write-Output "UI and byte restoration: PASS ($run)"
