param(
    [Parameter(Mandatory)][string]$AppDirectory,
    [Parameter(Mandatory)][string]$SourceDirectory,
    [Parameter(Mandatory)][string]$EvidenceDirectory,
    [Parameter(Mandatory)][string]$ExpectedCommit
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') { throw 'Disposable hosted runner only.' }
# Hypothesis: the installed app's own pinned worker accepts its paired system
# source and propagates mismatches. Decision: deployment wiring, not media readiness.
# Stop on provenance/hash mismatch, unexpected acceptance or synthetic data loss.
$evidence=(Resolve-Path -LiteralPath $EvidenceDirectory).Path
if (Test-Path -LiteralPath (Join-Path $evidence 'result.json')) { throw 'Preserve previous evidence.' }
$appFiles=@(Get-ChildItem -LiteralPath $AppDirectory -Filter '*-setup.exe' -File -Recurse)
$sums=@(Get-ChildItem -LiteralPath $AppDirectory -Filter 'SHA256SUMS.txt' -File -Recurse)
$builds=@(Get-ChildItem -LiteralPath $SourceDirectory -Filter 'build-manifest.json' -File -Recurse)
$manifests=@(Get-ChildItem -LiteralPath $SourceDirectory -Filter 'component-manifest.json' -File -Recurse)
if ($appFiles.Count -ne 1 -or $sums.Count -ne 1 -or $builds.Count -ne 1 -or $manifests.Count -ne 1) { throw 'Ambiguous artifact contents.' }
$appHash=(Get-FileHash -LiteralPath $appFiles[0].FullName).Hash.ToLowerInvariant()
if ((Get-Content -Raw -LiteralPath $sums[0].FullName).Trim() -cne ($appHash+'  '+$appFiles[0].Name)) { throw 'App digest differs.' }
$build=Get-Content -Raw -LiteralPath $builds[0].FullName | ConvertFrom-Json
$manifest=Get-Content -Raw -LiteralPath $manifests[0].FullName | ConvertFrom-Json
if ($build.commit -cne $ExpectedCommit -or $manifest.commit -cne $ExpectedCommit -or $manifest.timing_protocol -ne 2) { throw 'Source provenance differs.' }
$packages=@($build.packages | Where-Object { !$_.rollback_fixture })
if ($packages.Count -ne 1) { throw 'Expected one normal source package.' }
$package=$packages[0]
$msis=@(Get-ChildItem -LiteralPath $SourceDirectory -Filter $package.file -File -Recurse)
if ($msis.Count -ne 1 -or $msis[0].Length -ne $package.bytes -or (Get-FileHash -LiteralPath $msis[0].FullName).Hash.ToLowerInvariant() -cne $package.sha256) { throw 'Source MSI digest differs.' }
$location=Join-Path $env:RUNNER_TEMP 'TingXian-paired-install'
$source=Join-Path $env:ProgramW6432 'TingXian\VideoSource\TingXianVideoSource.dll'
if ((Test-Path -LiteralPath $location) -or (Test-Path -LiteralPath $source)) { throw 'Existing installation must be preserved.' }
$exe=Join-Path $location 'meetily.exe'
$checks=[Collections.Generic.List[string]]::new()
$ok=$false
function Run([string]$File,[string[]]$Arguments,[string]$Name) {
    $p=Start-Process -FilePath $File -ArgumentList $Arguments -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $evidence ($Name+'.stdout')) -RedirectStandardError (Join-Path $evidence ($Name+'.stderr'))
    try {
        if (!$p.WaitForExit(180000)) { throw "Process still running; preserve state: $Name" }
        if ($p.ExitCode -ne 0) { throw "Process failed: $Name ($($p.ExitCode))" }
    } finally { $p.Dispose() }
}
function Inspect([string]$Name,[uint32]$ErrorCode,[bool]$Present) {
    Run $exe @('--video-worker-check') $Name
    $p=Get-Content -Raw -LiteralPath (Join-Path $evidence ($Name+'.stdout')) | ConvertFrom-Json
    if ($p.phase -ne 'unavailable' -or $p.reason -ne 'runtime_not_pinned' -or $p.inspection.registration_error -ne $ErrorCode -or $p.inspection.machine_registration_present -ne $Present -or $p.inspection.media_started -or $null -ne $p.eye -or $null -ne $p.stream) { throw "Wrong installed app inspection: $Name" }
    $checks.Add($Name)
}
try {
    Run $appFiles[0].FullName @('/S',"/D=$location") 'app-install'
    Run $exe @('--video-components-check') 'media-components'
    $media=Get-Content -Raw -LiteralPath (Join-Path $evidence 'media-components.stdout') | ConvertFrom-Json
    if ($media.state -ne 'integrity_verified' -or $media.verified_components -ne 4 -or $media.media_started) { throw 'Installed app media bundle differs.' }
    $checks.Add('installed four-adapter integrity')
    Inspect 'source-absent' 2147746132 $false
    Run (Join-Path $env:SystemRoot 'System32\msiexec.exe') @('/i',('"'+$msis[0].FullName+'"'),'/qn','/norestart','/l*v',('"'+(Join-Path $evidence 'source-install.log')+'"')) 'source-install'
    $sourceEntry=@($manifest.files | Where-Object { $_.path -eq 'TingXianVideoSource.dll' })
    if ($sourceEntry.Count -ne 1 -or (Get-FileHash -LiteralPath $source).Hash.ToLowerInvariant() -cne $sourceEntry[0].sha256) { throw 'Installed source differs.' }
    Inspect 'source-paired' 0 $true
    # Read/hash only: the corrupted DLL is never loaded or activated by inspection.
    $original=[IO.File]::ReadAllBytes($source)
    try {
        $changed=[byte[]]$original.Clone(); $changed[$changed.Length-1]=$changed[$changed.Length-1] -bxor 1
        [IO.File]::WriteAllBytes($source,$changed)
        Inspect 'source-mismatch' 2147942423 $true
    } finally { [IO.File]::WriteAllBytes($source,$original) }
    Inspect 'source-restored' 0 $true
    $sentinel=Join-Path $env:APPDATA 'io.github.jinchunliu.meetilyinterview\paired-ci-preserve.txt'
    if (Test-Path -LiteralPath $sentinel) { throw 'Preserve existing sentinel.' }
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $sentinel) | Out-Null
    [IO.File]::WriteAllText($sentinel,'Synthetic paired installer check only.')
    $sentinelHash=(Get-FileHash -LiteralPath $sentinel).Hash
    Run (Join-Path $env:SystemRoot 'System32\msiexec.exe') @('/x',('"'+$msis[0].FullName+'"'),'/qn','/norestart','/l*v',('"'+(Join-Path $evidence 'source-uninstall.log')+'"')) 'source-uninstall'
    Inspect 'source-removed' 2147746132 $false
    Run (Join-Path $location 'uninstall.exe') @('/S') 'app-uninstall'
    $deadline=[DateTime]::UtcNow.AddSeconds(15)
    while ((Test-Path -LiteralPath $exe) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 100 }
    if ((Test-Path -LiteralPath $source) -or (Test-Path -LiteralPath $exe) -or (Get-FileHash -LiteralPath $sentinel).Hash -cne $sentinelHash) { throw 'Uninstall or data preservation differs.' }
    $checks.Add('both uninstalls preserve synthetic app data')
    $ok=$true
} finally {
    [ordered]@{result=$(if($ok){'pass'}else{'fail'});app_commit=$ExpectedCommit;app_sha256=$appHash;source_msi_sha256=$package.sha256;checks=$checks;device_created=$false;capture_started=$false;sdk_loaded=$false;local_target_machine_test=$false} | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $evidence 'result.json') -Encoding utf8
}
