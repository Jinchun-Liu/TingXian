param([string]$BuildDirectory = (Join-Path $PSScriptRoot '../target/video-worker-build'))
$ErrorActionPreference='Stop'
$source=Join-Path $PSScriptRoot '../video/windows'
$stage=Join-Path $PSScriptRoot '../frontend/src-tauri/binaries/video-worker'
cmake -S $source -B $BuildDirectory -A x64
if ($LASTEXITCODE) {throw 'Video worker configure failed'}
cmake --build $BuildDirectory --config Release --target tingxian-video-worker --parallel 2
if ($LASTEXITCODE) {throw 'Video worker build failed'}
New-Item -ItemType Directory -Path $stage -Force | Out-Null
$worker=Join-Path $BuildDirectory 'Release/tingxian-video-worker.exe'
Copy-Item -LiteralPath $worker -Destination (Join-Path $stage 'tingxian-video-worker.exe')
Copy-Item -LiteralPath (Join-Path $source 'LICENSE-Microsoft.txt') -Destination (Join-Path $stage 'LICENSE-Microsoft.txt')
Copy-Item -LiteralPath (Join-Path $source 'THIRD-PARTY.md') -Destination (Join-Path $stage 'THIRD-PARTY.md')
(Get-FileHash -Algorithm SHA256 -LiteralPath $worker).Hash.ToLowerInvariant() | Set-Content -LiteralPath (Join-Path $stage 'worker-sha256.txt') -Encoding ascii
Write-Output 'Staged local inspection worker; no SDK or device registration included'
