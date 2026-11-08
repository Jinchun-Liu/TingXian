param([string]$BuildDirectory = (Join-Path $PSScriptRoot '../target/video-worker-build'))
$ErrorActionPreference='Stop'
$source=Join-Path $PSScriptRoot '../video/windows'
$stage=Join-Path $PSScriptRoot '../frontend/src-tauri/binaries/video-worker'
cmake -S $source -B $BuildDirectory -A x64
if ($LASTEXITCODE) {throw 'Video worker configure failed'}
cmake --build $BuildDirectory --config Release --target tingxian-video-worker --parallel 2
if ($LASTEXITCODE) {throw 'Video worker build failed'}
cmake --build $BuildDirectory --config Release --target TingXianCameraInput TingXianVideoOutput --parallel 2
if ($LASTEXITCODE) {throw 'Video media adapters build failed'}
$audioBuild=Join-Path $BuildDirectory 'audio'
cmake -S (Join-Path $PSScriptRoot '../audio/windows') -B $audioBuild -A x64
if ($LASTEXITCODE) {throw 'Audio adapters configure failed'}
cmake --build $audioBuild --config Release --target TingXianAudioCapture TingXianAudioOutput --parallel 2
if ($LASTEXITCODE) {throw 'Audio adapters build failed'}
New-Item -ItemType Directory -Path $stage -Force | Out-Null
$worker=Join-Path $BuildDirectory 'Release/tingxian-video-worker.exe'
Copy-Item -LiteralPath $worker -Destination (Join-Path $stage 'tingxian-video-worker.exe')
Copy-Item -LiteralPath (Join-Path $source 'LICENSE-Microsoft.txt') -Destination (Join-Path $stage 'LICENSE-Microsoft.txt')
Copy-Item -LiteralPath (Join-Path $source 'THIRD-PARTY.md') -Destination (Join-Path $stage 'THIRD-PARTY.md')
(Get-FileHash -Algorithm SHA256 -LiteralPath $worker).Hash.ToLowerInvariant() | Set-Content -LiteralPath (Join-Path $stage 'worker-sha256.txt') -Encoding ascii
$media=Join-Path $stage 'media'
$names=@('TingXianCameraInput.dll','TingXianAudioCapture.dll','TingXianAudioOutput.dll','TingXianVideoOutput.dll')
if (Test-Path -LiteralPath $media) {
    foreach($item in Get-ChildItem -LiteralPath $media -Force) {
        if($item.PSIsContainer -or $item.Name -notin ($names + 'media-sha256.txt') -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {throw 'Unexpected media stage entry; preserve and inspect it'}
    }
} else { New-Item -ItemType Directory -Path $media | Out-Null }
$lines=@('tingxian-media-v1')
foreach($name in $names) {
    $built=if($name -like 'TingXianAudio*'){Join-Path $audioBuild "Release/$name"}else{Join-Path $BuildDirectory "Release/$name"}
    Copy-Item -LiteralPath $built -Destination (Join-Path $media $name)
    $lines += (Get-FileHash -Algorithm SHA256 -LiteralPath $built).Hash.ToLowerInvariant() + '  ' + $name
}
[IO.File]::WriteAllText((Join-Path $media 'media-sha256.txt'),($lines -join "`n")+"`n",[Text.Encoding]::ASCII)
Write-Output 'Staged inspection worker and four native media adapters; no SDK, models, audio driver or device registration included'
