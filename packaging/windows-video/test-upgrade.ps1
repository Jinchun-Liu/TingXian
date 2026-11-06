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
# Decision: can 0.1.0 be safely replaced, and does the new DLL fix premature
# shutdown? Stop on any unexpected failure, data change, or incomplete rollback.
# No device is created; all retained data is synthetic.
$baseline = (Resolve-Path -LiteralPath $BaselineDirectory).Path
$package = (Resolve-Path -LiteralPath $PackageDirectory).Path
$bin = (Resolve-Path -LiteralPath $BinaryDirectory).Path
$evidence = [IO.Path]::GetFullPath($EvidenceDirectory)
if (Test-Path -LiteralPath $evidence) { throw 'Preserve existing evidence.' }
New-Item -ItemType Directory -Path $evidence | Out-Null
$oldProduct = '{2265B958-40D3-48B6-8670-6402EB4F03EB}'
$newProduct = '{8DA8F4D7-398C-4C94-856F-CF4482DA6D0D}'
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
    $old = Join-Path $baseline 'TingXian-VideoSource-0.1.0-experimental-x64.msi'
    $new = Join-Path $package 'TingXian-VideoSource-0.1.1-experimental-x64.msi'
    $rollback = Join-Path $package 'TingXian-VideoSource-0.1.1-ROLLBACK-FIXTURE-x64.msi'
    Assert-True ((Run-Msi $old 'baseline-install') -eq 0) 'Baseline install failed.'
    Assert-True ($installer.ProductState($oldProduct) -eq 5) 'Baseline not installed.'
    $syntheticFile = Join-Path $install 'synthetic-upgrade-preserve.json'
    Assert-True (!(Test-Path -LiteralPath $syntheticFile)) 'Synthetic filename already exists.'
    [IO.File]::WriteAllText($syntheticFile,'{"synthetic":true,"text":"升级保留"}',[Text.UTF8Encoding]::new($false))
    $syntheticHash = (Get-FileHash -LiteralPath $syntheticFile).Hash
    $key = $machine.OpenSubKey($serverKey,$true)
    try { $key.SetValue('SyntheticUpgradeSetting','keep') } finally { $key.Dispose() }
    Verify-Payload $baseline
    # A differential regression must fail for the known old bug, not any error.
    & (Join-Path $bin 'tingxian-video-registration-test.exe') --registered-no-device 1> (Join-Path $evidence 'baseline-regression.stdout.log') 2> (Join-Path $evidence 'baseline-regression.stderr.log')
    Assert-True ($LASTEXITCODE -eq 1) 'Old DLL unexpectedly passed the new lifetime regression.'
    Assert-True ((Get-Content -LiteralPath (Join-Path $evidence 'baseline-regression.stderr.log') -Raw) -match '0xc00d3e85') 'Old DLL did not reproduce MF_E_SHUTDOWN.'
    Passed 'pinned old DLL reproduces premature shutdown without a device'

    Assert-True ((Run-Msi $rollback 'upgrade-rollback') -eq 1603) 'Upgrade failure fixture did not fail.'
    Assert-True ($installer.ProductState($oldProduct) -eq 5 -and $installer.ProductState($newProduct) -eq -1) 'Upgrade rollback did not restore old product.'
    Verify-Payload $baseline
    Passed 'failed upgrade restores old payload, registration and synthetic data'

    Assert-True ((Run-Msi $new 'upgrade') -eq 0) 'Actual upgrade failed.'
    Assert-True ($installer.ProductState($oldProduct) -eq -1 -and $installer.ProductState($newProduct) -eq 5) 'Upgrade product states differ.'
    Verify-Payload $package
    & (Join-Path $bin 'tingxian-video-registration-test.exe') --registered-no-device | Tee-Object -FilePath (Join-Path $evidence 'upgraded-com.log')
    Assert-True ($LASTEXITCODE -eq 0) 'Upgraded COM lifetime regression failed.'
    & (Join-Path $bin 'tingxian-video-source-test.exe') (Join-Path $install 'TingXianVideoSource.dll') | Set-Content -LiteralPath (Join-Path $evidence 'upgraded-source.log') -Encoding utf8
    Assert-True ($LASTEXITCODE -eq 0) 'Upgraded DLL source regressions failed.'
    Passed 'upgrade preserves data and fixes installed COM lifetime'

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
    [ordered]@{result=$(if($success){'pass'}else{'fail'});checks=$checks;baseline_commit='9eb55d3db58489f5f96eadfcb311f86013b6bc9b';device_created=$false;physical_camera_opened=$false} |
        ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $evidence 'result.json') -Encoding utf8
    $machine.Dispose()
    [Runtime.InteropServices.Marshal]::FinalReleaseComObject($installer) | Out-Null
}
