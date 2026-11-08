param(
    [Parameter(Mandatory)][string]$BaselineDirectory,
    [Parameter(Mandatory)][string]$PackageDirectory,
    [Parameter(Mandatory)][string]$BinaryDirectory,
    [Parameter(Mandatory)][string]$EvidenceDirectory
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'acl-check.ps1')
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
    throw 'Run only on a disposable GitHub-hosted Windows runner.'
}
# Decision: can installed 0.1.2 be safely replaced by the pinned v2 source? Stop on unexpected failure, data change, or incomplete rollback.
# No device is created; all retained data is synthetic.
$baseline = (Resolve-Path -LiteralPath $BaselineDirectory).Path
$package = (Resolve-Path -LiteralPath $PackageDirectory).Path
$bin = (Resolve-Path -LiteralPath $BinaryDirectory).Path
$evidence = [IO.Path]::GetFullPath($EvidenceDirectory)
if (Test-Path -LiteralPath $evidence) { throw 'Preserve existing evidence.' }
New-Item -ItemType Directory -Path $evidence | Out-Null
$oldProduct = '{CDA85B35-A484-4930-B288-07F4BB9A5D8C}'
$newProduct = '{B7682B8D-467E-47DD-A3E2-B0F0E58EF864}'
$install = Join-Path $env:ProgramW6432 'TingXian\VideoSource'
$serverKey = 'SOFTWARE\Classes\CLSID\{44D20DA7-55EC-4B4A-A767-99E97D6E5A90}\InprocServer32'
$machine = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::LocalMachine,[Microsoft.Win32.RegistryView]::Registry64)
$installer = New-Object -ComObject WindowsInstaller.Installer
$checks = [Collections.Generic.List[string]]::new()
$success = $false
function Assert-True([bool]$Condition, [string]$Message) { if (!$Condition) { throw $Message } }
function Passed([string]$Name) { $checks.Add($Name); Write-Output "PASS $Name" }
function Run-Msi([string]$Msi, [string]$Name, [switch]$Uninstall) {
    $verb = if ($Uninstall) { '/x' } else { '/i' }
    $arguments = @($verb, ('"'+$Msi+'"'), '/qn', '/norestart', '/l*v', ('"'+(Join-Path $evidence ($Name+'.log'))+'"'))
    $process = Start-Process -FilePath (Join-Path $env:SystemRoot 'System32\msiexec.exe') -ArgumentList $arguments -PassThru -WindowStyle Hidden
    try {
        if (!$process.WaitForExit(120000)) { throw 'MSI still running; preserve it and do not launch another.' }
        return $process.ExitCode
    } finally { $process.Dispose() }
}
function Verify-Payload([string]$ExpectedDirectory) {
    $expectedManifest = Join-Path $ExpectedDirectory 'payload\component-manifest.json'
    Assert-True ((Get-FileHash -LiteralPath (Join-Path $install 'component-manifest.json')).Hash -ceq (Get-FileHash -LiteralPath $expectedManifest).Hash) 'Manifest differs.'
    $manifest = Get-Content -LiteralPath $expectedManifest -Raw | ConvertFrom-Json
    foreach ($file in $manifest.files) {
        $path = Join-Path $install $file.path
        Assert-True ((Get-Item -LiteralPath $path).Length -eq $file.bytes) 'Installed size differs.'
        Assert-True ((Get-FileHash -LiteralPath $path).Hash.ToLowerInvariant() -ceq $file.sha256) 'Installed digest differs.'
    }
    $key = $machine.OpenSubKey($serverKey)
    try {
        Assert-True ($key -and $key.GetValue('') -ieq (Join-Path $install 'TingXianVideoSource.dll')) 'COM registration differs.'
        Assert-True ($key.GetValue('ThreadingModel') -ceq 'Both') 'COM threading differs.'
        Assert-True ($key.GetValue('SyntheticUpgradeSetting') -ceq 'keep') 'Unknown registry value lost.'
    } finally { if ($key) { $key.Dispose() } }
    Assert-True ((Get-FileHash -LiteralPath $syntheticFile).Hash -ceq $syntheticHash) 'Unknown synthetic file changed.'
    foreach ($path in @((Join-Path $install 'TingXianVideoSource.dll'),$install,(Split-Path -Parent $install),$env:ProgramW6432)) { Assert-VideoComponentPathProtected $path }
}
try {
    Assert-True ($installer.ProductState($oldProduct) -eq -1 -and $installer.ProductState($newProduct) -eq -1) 'An installed product must be preserved.'
    foreach ($folder in @($baseline,$package)) {
        $manifest = Get-Content -LiteralPath (Join-Path $folder 'build-manifest.json') -Raw | ConvertFrom-Json
        foreach ($item in $manifest.packages) {
            Assert-True ((Get-FileHash -LiteralPath (Join-Path $folder $item.file)).Hash.ToLowerInvariant() -ceq $item.sha256) 'Package digest differs.'
        }
    }
    $old = Join-Path $baseline 'TingXian-VideoSource-0.1.2-experimental-x64.msi'
    $new = Join-Path $package 'TingXian-VideoSource-0.2.0-experimental-x64.msi'
    $rollback = Join-Path $package 'TingXian-VideoSource-0.2.0-ROLLBACK-FIXTURE-x64.msi'
    Assert-True ((Run-Msi $old 'baseline-install') -eq 0) 'Baseline install failed.'
    Assert-True ($installer.ProductState($oldProduct) -eq 5) 'Baseline not installed.'
    $syntheticFile = Join-Path $install 'synthetic-upgrade-preserve.json'
    Assert-True (!(Test-Path -LiteralPath $syntheticFile)) 'Synthetic filename already exists.'
    [IO.File]::WriteAllText($syntheticFile,'{"synthetic":true,"text":"升级保留"}',[Text.UTF8Encoding]::new($false))
    $syntheticHash = (Get-FileHash -LiteralPath $syntheticFile).Hash
    $key = $machine.OpenSubKey($serverKey,$true)
    try { $key.SetValue('SyntheticUpgradeSetting','keep') } finally { $key.Dispose() }
    Verify-Payload $baseline
    # Current production probe must reject the registered old bytes without loading them.
    & (Join-Path $bin 'tingxian-video-device-probe.exe') --probe | Set-Content -LiteralPath (Join-Path $evidence 'baseline-pin-probe.json') -Encoding utf8
    Assert-True ($LASTEXITCODE -eq 0) 'Old-source read-only probe did not complete.'
    $probe = Get-Content -Raw -LiteralPath (Join-Path $evidence 'baseline-pin-probe.json') | ConvertFrom-Json
    Assert-True ($probe.machine_registration_present -and !$probe.source_integrity_verified -and $probe.registration_error -eq '0x80070017' -and !$probe.device_created) 'Old source was not rejected for byte mismatch.'
    Passed 'registered 0.1.2 is rejected by the current native byte pin without source activation'
    & (Join-Path $bin 'tingxian-video-registration-test.exe') --registered-no-device 1> (Join-Path $evidence 'baseline-regression.stdout.log') 2> (Join-Path $evidence 'baseline-regression.stderr.log')
    Assert-True ($LASTEXITCODE -eq 1) 'Old DLL unexpectedly has timing v2.'
    Assert-True ((Get-Content -LiteralPath (Join-Path $evidence 'baseline-regression.stderr.log') -Raw) -match 'registered COM activation failed: 0x80004002') 'Old source failed for an unexpected reason.'
    Passed 'real old COM activation proves v2 interface is absent'

    Assert-True ((Run-Msi $rollback 'upgrade-rollback') -eq 1603) 'Upgrade failure fixture did not fail.'
    Assert-True ($installer.ProductState($oldProduct) -eq 5 -and $installer.ProductState($newProduct) -eq -1) 'Upgrade rollback did not restore old product.'
    Verify-Payload $baseline
    Passed 'failed upgrade restores old payload, registration and synthetic data'

    Assert-True ((Run-Msi $new 'upgrade') -eq 0) 'Actual upgrade failed.'
    Assert-True ($installer.ProductState($oldProduct) -eq -1 -and $installer.ProductState($newProduct) -eq 5) 'Upgrade product states differ.'
    Verify-Payload $package
    & (Join-Path $bin 'tingxian-video-device-probe.exe') --probe | Set-Content -LiteralPath (Join-Path $evidence 'upgraded-pin-probe.json') -Encoding utf8
    Assert-True ($LASTEXITCODE -eq 0) 'Upgraded read-only probe failed.'
    $probe = Get-Content -Raw -LiteralPath (Join-Path $evidence 'upgraded-pin-probe.json') | ConvertFrom-Json
    Assert-True ($probe.source_integrity_verified -and $probe.registration_error -eq '0x00000000' -and !$probe.device_created) 'Upgraded source not pinned.'
    & (Join-Path $bin 'tingxian-video-registration-test.exe') --registered-no-device | Tee-Object -FilePath (Join-Path $evidence 'upgraded-com.log')
    Assert-True ($LASTEXITCODE -eq 0) 'Upgraded COM lifetime regression failed.'
    & (Join-Path $bin 'tingxian-video-source-test.exe') (Join-Path $install 'TingXianVideoSource.dll') | Set-Content -LiteralPath (Join-Path $evidence 'upgraded-source.log') -Encoding utf8
    Assert-True ($LASTEXITCODE -eq 0) 'Upgraded DLL source regressions failed.'
    & (Join-Path $bin 'tingxian-video-ipc-test.exe') (Join-Path $install 'TingXianVideoSource.dll') | Set-Content -LiteralPath (Join-Path $evidence 'upgraded-ipc.log') -Encoding utf8
    Assert-True ($LASTEXITCODE -eq 0) 'Upgraded DLL IPC regressions failed.'
    Passed 'upgrade preserves data and passes installed v2 source/IPC'

    Assert-True ((Run-Msi $old 'downgrade-rejected') -eq 1603) 'Downgrade was not rejected.'
    Verify-Payload $package
    Assert-True ($installer.ProductState($newProduct) -eq 5 -and $installer.ProductState($oldProduct) -eq -1) 'Downgrade changed product state.'
    Passed 'downgrade rejected without modifying installed component'

    Assert-True ((Run-Msi $new 'upgraded-uninstall' -Uninstall) -eq 0) 'Upgraded uninstall failed.'
    Assert-True ($installer.ProductState($newProduct) -eq -1) 'Product survived uninstall.'
    Assert-True (!(Test-Path -LiteralPath (Join-Path $install 'TingXianVideoSource.dll'))) 'DLL survived uninstall.'
    Assert-True ((Get-FileHash -LiteralPath $syntheticFile).Hash -ceq $syntheticHash) 'Uninstall removed synthetic data.'
    $key = $machine.OpenSubKey($serverKey)
    try {
        Assert-True ($key -and $key.GetValue('SyntheticUpgradeSetting') -ceq 'keep') 'Unknown registry value removed.'
        Assert-True ($null -eq $key.GetValue('')) 'COM server survived uninstall.'
    } finally { if ($key) { $key.Dispose() } }
    Passed 'upgraded uninstall preserves unknown data'
    $success = $true
} finally {
    [ordered]@{result=$(if($success){'pass'}else{'fail'});checks=$checks;baseline_commit='084dae59e3ee150571e979211e4e4282826881b7';device_created=$false;physical_camera_opened=$false} |
        ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $evidence 'result.json') -Encoding utf8
    $machine.Dispose()
    [Runtime.InteropServices.Marshal]::FinalReleaseComObject($installer) | Out-Null
}
