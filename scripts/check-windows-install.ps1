$ErrorActionPreference = 'Stop'
$installers = @(Get-ChildItem "$PSScriptRoot/../target/release/bundle/nsis/*-setup.exe")
if ($installers.Count -ne 1) { throw 'Expected one installer' }
# A synthetic directory occupying a required DLL path forces a real File error.
# This must abort before uninstall registration instead of silently ignoring it.
# Retain the failed directory as evidence; never use an existing installation.
$blockedLocation = Join-Path $env:RUNNER_TEMP '听弦 Windows 安装失败验收'
if (Test-Path -LiteralPath $blockedLocation) { throw "Test destination already exists: $blockedLocation" }
New-Item -ItemType Directory -Path (Join-Path $blockedLocation 'onnxruntime.dll') | Out-Null
$blocked = Start-Process -FilePath $installers[0].FullName -ArgumentList @('/S', "/D=$blockedLocation") -Wait -PassThru
if ($blocked.ExitCode -eq 0) { throw 'Installer reported success when a required DLL could not be written' }
if (Test-Path -LiteralPath (Join-Path $blockedLocation 'uninstall.exe')) { throw 'Failed installation continued to create an uninstaller' }
Write-Host "PASS installer stops on a required-file write failure (exit $($blocked.ExitCode))"
$location = Join-Path $env:RUNNER_TEMP '听弦 Windows 安装验收'
if (Test-Path $location) { throw "Test destination already exists: $location" }
$process = Start-Process -FilePath $installers[0].FullName -ArgumentList @('/S', "/D=$location") -Wait -PassThru
if ($process.ExitCode -ne 0) { throw "Install failed: $($process.ExitCode)" }
$exe = Join-Path $location 'meetily.exe'
if (!(Test-Path $exe)) { throw 'Installed application missing' }
python "$PSScriptRoot/verify-windows-runtime.py" $location
if ($LASTEXITCODE) { throw 'Installed runtime hashes differ' }
# Deliberately remove the developer Python/FFmpeg from PATH for this subprocess.
$savedPath = $env:PATH
try {
    $env:PATH = "$env:SystemRoot\System32;$env:SystemRoot;$location"
    & "$PSScriptRoot/check-windows-media-bundle.ps1" -InstallDirectory $location -EvidenceDirectory (Join-Path $env:RUNNER_TEMP 'media-bundle-install')
    if ($LASTEXITCODE) { throw 'Installed native media bundle verification failed' }
    $video = Start-Process -FilePath $exe -ArgumentList '--video-worker-check' -WindowStyle Hidden -Wait -PassThru -RedirectStandardOutput (Join-Path $env:RUNNER_TEMP 'tingxian-video-check.stdout') -RedirectStandardError (Join-Path $env:RUNNER_TEMP 'tingxian-video-check.stderr')
    if ($video.ExitCode -ne 0) { Get-Content (Join-Path $env:RUNNER_TEMP 'tingxian-video-check.stderr'); throw 'Installed video inspection worker check failed' }
    # Disposable install only: a changed EXE plus matching mutable text digest
    # must still fail against the pin embedded in the installed main executable.
    $workerPath=Join-Path $location 'video-worker/tingxian-video-worker.exe'
    $pinPath=Join-Path $location 'video-worker/worker-sha256.txt'
    $savedWorker=Join-Path $location 'video-worker/worker-original-preserved.exe'
    $savedPin=Join-Path $location 'video-worker/pin-original-preserved.txt'
    if ((Test-Path -LiteralPath $savedWorker) -or (Test-Path -LiteralPath $savedPin)) {throw 'Worker integrity fixture already exists'}
    Move-Item -LiteralPath $workerPath -Destination $savedWorker
    Move-Item -LiteralPath $pinPath -Destination $savedPin
    try {
        $corrupt=[IO.File]::ReadAllBytes($savedWorker)
        $corrupt[$corrupt.Length-1]=$corrupt[$corrupt.Length-1] -bxor 1
        [IO.File]::WriteAllBytes($workerPath,$corrupt)
        (Get-FileHash -Algorithm SHA256 -LiteralPath $workerPath).Hash.ToLowerInvariant() | Set-Content -LiteralPath $pinPath -Encoding ascii
        $badVideo=Start-Process -FilePath $exe -ArgumentList '--video-worker-check' -WindowStyle Hidden -Wait -PassThru -RedirectStandardError (Join-Path $env:RUNNER_TEMP 'tingxian-video-corrupt.stderr')
        if ($badVideo.ExitCode -eq 0 -or !(Select-String -LiteralPath (Join-Path $env:RUNNER_TEMP 'tingxian-video-corrupt.stderr') -Pattern 'Integrity' -Quiet)) {throw 'Modified worker was not rejected by embedded integrity pin'}
    } finally {
        if (Test-Path -LiteralPath $workerPath) {Move-Item -LiteralPath $workerPath -Destination (Join-Path $location 'video-worker/worker-corrupt-preserved.exe')}
        if (Test-Path -LiteralPath $pinPath) {Move-Item -LiteralPath $pinPath -Destination (Join-Path $location 'video-worker/pin-corrupt-preserved.txt')}
        Move-Item -LiteralPath $savedWorker -Destination $workerPath
        Move-Item -LiteralPath $savedPin -Destination $pinPath
    }
    Write-Host 'PASS installed local worker inspection and modified executable rejection; no media started'
    $check = Start-Process -FilePath $exe -ArgumentList '--installation-check' -Wait -PassThru -RedirectStandardError (Join-Path $env:RUNNER_TEMP 'tingxian-install-check.stderr')
    if ($check.ExitCode -ne 0) { Get-Content (Join-Path $env:RUNNER_TEMP 'tingxian-install-check.stderr'); throw "Offline installation check failed: $($check.ExitCode)" }
    $platform = Start-Process -FilePath $exe -ArgumentList '--windows-platform-check' -Wait -PassThru -RedirectStandardError (Join-Path $env:RUNNER_TEMP 'tingxian-platform-check.stderr')
    if ($platform.ExitCode -ne 0) { Get-Content (Join-Path $env:RUNNER_TEMP 'tingxian-platform-check.stderr'); throw "Windows screenshot/recycle check failed: $($platform.ExitCode)" }
} finally { $env:PATH = $savedPath }
# Synthetic sentinel outside the install folder verifies uninstall keeps app data.
$data = Join-Path $env:APPDATA 'io.github.jinchunliu.meetilyinterview'
New-Item -ItemType Directory -Force $data | Out-Null
$sentinel = Join-Path $data 'ci-preserve-user-data.txt'
if (Test-Path $sentinel) { throw 'Refusing to overwrite existing sentinel' }
'Synthetic CI preservation check' | Set-Content $sentinel
$uninstaller = Join-Path $location 'uninstall.exe'
if (!(Test-Path $uninstaller)) { throw 'Uninstall entry missing' }
$uninstall = Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait -PassThru
if ($uninstall.ExitCode -ne 0) { throw "Uninstall failed: $($uninstall.ExitCode)" }
if (!(Test-Path $sentinel)) { throw 'Uninstall deleted user data' }
Write-Host 'PASS installed resources, offline ASR/documents/media/database, uninstall preserves app data'
