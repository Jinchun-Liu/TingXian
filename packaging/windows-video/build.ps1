param(
    [Parameter(Mandatory)][string]$BinaryDirectory,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [switch]$BuildRollbackFixture
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).Path
$bin = (Resolve-Path -LiteralPath $BinaryDirectory).Path
$output = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $output) { throw 'Use a new output directory; existing artifacts are preserved.' }
$candidates = @()
if ($env:WIX) { $candidates += Join-Path $env:WIX 'bin' }
$candidates += Join-Path ${env:ProgramFiles(x86)} 'WiX Toolset v3.14\bin'
$wix = $candidates | Where-Object { Test-Path -LiteralPath (Join-Path $_ 'candle.exe') } | Select-Object -First 1
if (!$wix) { throw 'WiX 3.14.1 build tools are required for developers; ordinary users only need the MSI.' }
$candle = Join-Path $wix 'candle.exe'
$light = Join-Path $wix 'light.exe'
foreach ($tool in @($candle, $light)) {
    if ((Get-Item -LiteralPath $tool).VersionInfo.FileVersion -notlike '3.14.1.*') { throw 'Unexpected WiX tool version.' }
}
$commit = (& git -C $root rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $commit -notmatch '^[0-9a-f]{40}$') { throw 'Cannot determine build commit.' }
if (& git -C $root status --porcelain --untracked-files=no) { throw 'Build from committed sources so the manifest identifies the actual code.' }
New-Item -ItemType Directory -Path $output | Out-Null
$payload = Join-Path $output 'payload'
New-Item -ItemType Directory -Path $payload | Out-Null
foreach ($name in @('TingXianVideoSource.dll','tingxian-video-device-probe.exe')) {
    Copy-Item -LiteralPath (Join-Path $bin $name) -Destination (Join-Path $payload $name)
}
foreach ($name in @('LICENSE-Microsoft.txt','THIRD-PARTY.md')) {
    Copy-Item -LiteralPath (Join-Path $root "video\windows\$name") -Destination (Join-Path $payload $name)
}
foreach ($name in @('LICENSE.md','NOTICE.md')) {
    Copy-Item -LiteralPath (Join-Path $root $name) -Destination (Join-Path $payload $name)
}
$files = @(Get-ChildItem -LiteralPath $payload -File | Sort-Object Name | ForEach-Object {
    [ordered]@{path=$_.Name;bytes=$_.Length;sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}
})
[ordered]@{schema_version=1;component='tingxian_windows_video_source';version='0.1.2';channel='experimental';commit=$commit;files=$files} |
    ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $payload 'component-manifest.json') -Encoding utf8
$packages = @()
foreach ($fixture in @($false, $true)) {
    if ($fixture -and !$BuildRollbackFixture) { continue }
    $name = if ($fixture) { 'TingXian-VideoSource-0.1.2-ROLLBACK-FIXTURE-x64' } else { 'TingXian-VideoSource-0.1.2-experimental-x64' }
    $obj = Join-Path $output ($name+'.wixobj')
    $msi = Join-Path $output ($name+'.msi')
    $arguments = @('-nologo','-arch','x64',"-dPayloadDir=$payload",'-out',$obj)
    if ($fixture) { $arguments += '-dRollbackFixture=1' }
    $arguments += Join-Path $PSScriptRoot 'Component.wxs'
    & $candle @arguments
    if ($LASTEXITCODE -ne 0) { throw 'WiX compilation failed.' }
    & $light -nologo -out $msi $obj
    if ($LASTEXITCODE -ne 0) { throw 'MSI linking or validation failed.' }
    $packages += [ordered]@{file=[IO.Path]::GetFileName($msi);rollback_fixture=$fixture;bytes=(Get-Item -LiteralPath $msi).Length;sha256=(Get-FileHash -LiteralPath $msi -Algorithm SHA256).Hash.ToLowerInvariant()}
}
[ordered]@{commit=$commit;wix_version=(Get-Item -LiteralPath $candle).VersionInfo.FileVersion;packages=$packages} |
    ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $output 'build-manifest.json') -Encoding utf8
$packages | ConvertTo-Json -Depth 4
