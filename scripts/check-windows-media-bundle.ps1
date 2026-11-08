# Run only against the isolated CI install, not existing applications/user data.
param([Parameter(Mandatory)][string]$InstallDirectory,[Parameter(Mandatory)][string]$EvidenceDirectory)
$ErrorActionPreference='Stop'
$install=[IO.Path]::GetFullPath($InstallDirectory)
$evidence=[IO.Path]::GetFullPath($EvidenceDirectory)
$ciRoot=[IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\')+'\'
if(!$env:GITHUB_ACTIONS -or !$install.StartsWith($ciRoot,[StringComparison]::OrdinalIgnoreCase) -or !$evidence.StartsWith($ciRoot,[StringComparison]::OrdinalIgnoreCase)) {throw 'Media tamper checks require isolated CI directories'}
if(Test-Path -LiteralPath $evidence){throw 'Use a new media evidence directory'}
New-Item -ItemType Directory -Path $evidence | Out-Null
$exe=Join-Path $install 'meetily.exe'
$media=Join-Path $install 'video-worker/media'
$manifest=Join-Path $media 'media-sha256.txt'
function Check-Installed([string]$Label,[bool]$Expected) {
    $stdout=Join-Path $evidence ($Label+'.stdout')
    $stderr=Join-Path $evidence ($Label+'.stderr')
    $p=Start-Process -FilePath $exe -ArgumentList '--video-components-check' -WindowStyle Hidden -Wait -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    $report=Get-Content -LiteralPath $stdout -Raw|ConvertFrom-Json
    if($null -eq $report.media_started -or $report.media_started -ne $false){throw 'Component check reported media start or missing status'}
    if($Expected) {
        if($p.ExitCode -ne 0 -or $report.state -ne 'integrity_verified' -or $report.verified_components -ne 4){throw "Installed component verification failed: $Label"}
    } elseif($p.ExitCode -eq 0 -or $report.state -ne 'integrity_failure' -or $report.verified_components -ne 0) {throw "Untrusted component accepted: $Label"}
}
Check-Installed 'intact' $true
$names=@('TingXianCameraInput.dll','TingXianAudioCapture.dll','TingXianAudioOutput.dll','TingXianVideoOutput.dll')
foreach($name in $names) {
    $path=Join-Path $media $name
    $original=Join-Path $evidence ($name+'.original')
    $savedManifest=Join-Path $evidence ($name+'.manifest-original.txt')
    Copy-Item -LiteralPath $manifest -Destination $savedManifest
    Move-Item -LiteralPath $path -Destination $original
    try {
        Check-Installed ($name+'-missing') $false
        $bytes=[IO.File]::ReadAllBytes($original); $bytes[$bytes.Length-1]=$bytes[$bytes.Length-1] -bxor 1
        [IO.File]::WriteAllBytes($path,$bytes)
        Check-Installed ($name+'-changed') $false
        $old=(Get-FileHash -Algorithm SHA256 -LiteralPath $original).Hash.ToLowerInvariant()
        $new=(Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash.ToLowerInvariant()
        $text=[IO.File]::ReadAllText($savedManifest).Replace($old,$new)
        [IO.File]::WriteAllText($manifest,$text,[Text.Encoding]::ASCII)
        Check-Installed ($name+'-mutable-manifest') $false
    } finally {
        if(Test-Path -LiteralPath $path){Move-Item -LiteralPath $path -Destination (Join-Path $evidence ($name+'.changed-preserved'))}
        Move-Item -LiteralPath $original -Destination $path
        Move-Item -LiteralPath $manifest -Destination (Join-Path $evidence ($name+'.manifest-changed.txt'))
        Copy-Item -LiteralPath $savedManifest -Destination $manifest
    }
    Check-Installed ($name+'-restored') $true
}
Write-Output 'PASS installed media bundle: four real x64 DLLs, 12 missing/tamper rejections and four preserved recoveries; no DLL load or media start'
