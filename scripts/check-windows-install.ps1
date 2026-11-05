$ErrorActionPreference = 'Stop'
$installers = @(Get-ChildItem "$PSScriptRoot/../target/release/bundle/nsis/*-setup.exe")
if ($installers.Count -ne 1) { throw 'Expected one installer' }
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
    $check = Start-Process -FilePath $exe -ArgumentList '--installation-check' -Wait -PassThru
    if ($check.ExitCode -ne 0) { throw "Offline installation check failed: $($check.ExitCode)" }
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
