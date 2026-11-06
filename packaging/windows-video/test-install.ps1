param(
    [Parameter(Mandatory)][string]$PackageDirectory,
    [Parameter(Mandatory)][string]$BinaryDirectory,
    [Parameter(Mandatory)][string]$EvidenceDirectory
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'acl-check.ps1')
# This script deliberately injects failures into a synthetic installation. It is
# not a maintenance/cleanup script for user PCs, and never touches app databases.
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
    throw 'Run only on a disposable GitHub-hosted Windows runner.'
}
$principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Disposable runner must permit machine installation.' }
$package = (Resolve-Path -LiteralPath $PackageDirectory).Path
$bin = (Resolve-Path -LiteralPath $BinaryDirectory).Path
$evidence = [IO.Path]::GetFullPath($EvidenceDirectory)
if (Test-Path -LiteralPath $evidence) { throw 'Evidence directory already exists.' }
New-Item -ItemType Directory -Path $evidence | Out-Null
$install = Join-Path $env:ProgramW6432 'TingXian\VideoSource'
$dll = Join-Path $install 'TingXianVideoSource.dll'
$sourceKey = 'SOFTWARE\Classes\CLSID\{44D20DA7-55EC-4B4A-A767-99E97D6E5A90}'
$serverKey = $sourceKey + '\InprocServer32'
$product = '{2265B958-40D3-48B6-8670-6402EB4F03EB}'
$machine = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::LocalMachine,[Microsoft.Win32.RegistryView]::Registry64)
$installer = New-Object -ComObject WindowsInstaller.Installer
$checks = [Collections.Generic.List[object]]::new()
$success = $false
function Assert-True([bool]$Condition, [string]$Message) { if (!$Condition) { throw $Message } }
function Passed([string]$Name) { $checks.Add([ordered]@{name=$Name;result='pass'}); Write-Output "PASS $Name" }
function Read-Server {
    $key = $machine.OpenSubKey($serverKey)
    if (!$key) { return $null }
    try { return $key.GetValue('') } finally { $key.Dispose() }
}
function Run-Msi([string]$Msi, [string]$Name, [switch]$Uninstall) {
    $log = Join-Path $evidence ($Name+'.log')
    $verb = if ($Uninstall) { '/x' } else { '/i' }
    $args = @($verb, ('"'+$Msi+'"'), '/qn', '/norestart', '/l*v', ('"'+$log+'"'))
    $process = Start-Process -FilePath (Join-Path $env:SystemRoot 'System32\msiexec.exe') -ArgumentList $args -PassThru -WindowStyle Hidden
    try {
        if (!$process.WaitForExit(120000)) { throw 'MSI did not finish; preserve the running installer and investigate. Do not start another.' }
        return $process.ExitCode
    } finally { $process.Dispose() }
}
function Assert-Absent {
    Assert-True ($null -eq (Read-Server)) 'Source registration survived a failed install/uninstall.'
    Assert-True ($installer.ProductState($product) -eq -1) 'Product remains installed.'
    Assert-True (!(Test-Path -LiteralPath (Join-Path $install 'tingxian-video-device-probe.exe'))) 'Installed probe survived rollback/uninstall.'
    Assert-True (!(Test-Path -LiteralPath (Join-Path $install 'component-manifest.json'))) 'Installed manifest survived rollback/uninstall.'
}
try {
    $existing = $machine.OpenSubKey($sourceKey)
    if ($existing) { $existing.Dispose(); throw 'Existing source key: preserve it and use a fresh runner.' }
    Assert-True (!(Test-Path -LiteralPath $install)) 'Existing component directory: preserve it and use a fresh runner.'
    Assert-True ($installer.ProductState($product) -eq -1) 'Existing component product: use a fresh runner.'
    $build = Get-Content -LiteralPath (Join-Path $package 'build-manifest.json') -Raw | ConvertFrom-Json
    foreach ($item in $build.packages) {
        $path = Join-Path $package $item.file
        Assert-True ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -eq $item.sha256) 'Package hash mismatch.'
    }
    $msi = Join-Path $package 'TingXian-VideoSource-0.1.0-experimental-x64.msi'
    $fixtureMsi = Join-Path $package 'TingXian-VideoSource-0.1.0-ROLLBACK-FIXTURE-x64.msi'

    # A fresh install must not overwrite an unrelated/stale registration.
    $foreign = 'C:\TingXianSyntheticFixture\preserve-this.dll'
    $key = $machine.CreateSubKey($serverKey)
    try { $key.SetValue('', $foreign, [Microsoft.Win32.RegistryValueKind]::String) } finally { $key.Dispose() }
    [ordered]@{key=$serverKey;default_value=$foreign;synthetic_only=$true} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $evidence 'registration-fixture.json') -Encoding utf8
    Assert-True ((Run-Msi $msi 'registration-conflict') -eq 1603) 'Conflicting registration was not rejected.'
    Assert-True ((Read-Server) -ceq $foreign) 'Conflicting registration was overwritten.'
    Assert-True (!(Test-Path -LiteralPath $dll)) 'Conflict installed files despite rejection.'
    # Remove only the exact synthetic key whose previous absence and current
    # value were just verified; its reconstructable value is saved in evidence.
    $machine.DeleteSubKeyTree($sourceKey)
    Passed 'foreign source registration rejected and preserved'

    # Occupy the DLL path with a synthetic directory to force a real file failure.
    New-Item -ItemType Directory -Path $dll | Out-Null
    [IO.File]::WriteAllText((Join-Path $dll 'synthetic-blocker.txt'),'Preserve this synthetic fixture.')
    Assert-True ((Run-Msi $msi 'dll-write-failure') -eq 1603) 'DLL write failure was reported as success.'
    Assert-Absent
    $blocker = (Resolve-Path -LiteralPath $dll).Path
    $archive = Join-Path $evidence 'blocked-dll-directory'
    Assert-True ($blocker -ceq [IO.Path]::GetFullPath((Join-Path $install 'TingXianVideoSource.dll'))) 'Unexpected blocker path.'
    Assert-True ([IO.Path]::GetFullPath($archive).StartsWith($evidence+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) 'Archive leaves evidence directory.'
    Assert-True (!((Get-Item -LiteralPath $blocker).Attributes -band [IO.FileAttributes]::ReparsePoint)) 'Unexpected redirected blocker.'
    Move-Item -LiteralPath $blocker -Destination $archive
    Passed 'DLL write failure returns nonzero and rolls back, blocker archived'

    # The separate fixture fails after InstallExecute applied files and registry.
    Assert-True ((Run-Msi $fixtureMsi 'transaction-rollback') -eq 1603) 'Rollback fixture unexpectedly succeeded.'
    $rollback = Get-Content -LiteralPath (Join-Path $evidence 'transaction-rollback.log') -Raw
    Assert-True ($rollback -match 'RegAddValue\(Name=ThreadingModel' -and $rollback -match 'FailAfterAppliedScript') 'Fixture did not reach applied registration and its injected failure.'
    Assert-Absent
    Assert-True (!(Test-Path -LiteralPath $dll)) 'DLL survived transaction rollback.'
    Passed 'transaction failure after applied registration restores absence'

    # Unknown files and values are not part of this MSI's ownership.
    if (!(Test-Path -LiteralPath $install)) { New-Item -ItemType Directory -Path $install | Out-Null }
    $userFile = Join-Path $install 'synthetic-unowned-record.json'
    [IO.File]::WriteAllText($userFile,'{"synthetic":true,"text":"保留这条合成资料"}',[Text.UTF8Encoding]::new($false))
    $userHash = (Get-FileHash -LiteralPath $userFile -Algorithm SHA256).Hash
    for ($cycle = 1; $cycle -le 2; $cycle++) {
        Assert-True ((Run-Msi $msi "install-$cycle") -eq 0) 'Normal component install failed.'
        Assert-True ($installer.ProductState($product) -eq 5) 'Product not installed locally.'
        Assert-True ((Read-Server) -ieq $dll) 'COM path does not identify the installed DLL.'
        $key = $machine.OpenSubKey($serverKey)
        try { Assert-True ($key.GetValue('ThreadingModel') -ceq 'Both') 'Wrong threading model.' } finally { $key.Dispose() }
        $manifest = Get-Content -LiteralPath (Join-Path $install 'component-manifest.json') -Raw | ConvertFrom-Json
        Assert-True ($manifest.commit -ceq $build.commit) 'Installed manifest source commit differs.'
        foreach ($file in $manifest.files) {
            $installed = Join-Path $install $file.path
            Assert-True ((Get-Item -LiteralPath $installed).Length -eq $file.bytes) 'Installed size differs.'
            Assert-True ((Get-FileHash -LiteralPath $installed -Algorithm SHA256).Hash.ToLowerInvariant() -ceq $file.sha256) 'Installed file hash differs.'
        }
        foreach ($path in @($dll, $install, (Split-Path -Parent $install), $env:ProgramW6432)) { Assert-VideoComponentPathProtected $path }
        & (Join-Path $install 'tingxian-video-device-probe.exe') --probe | Set-Content -LiteralPath (Join-Path $evidence "installed-probe-$cycle.json") -Encoding utf8
        Assert-True ($LASTEXITCODE -eq 0) 'Installed diagnostic failed.'
        $probe = Get-Content -LiteralPath (Join-Path $evidence "installed-probe-$cycle.json") -Raw | ConvertFrom-Json
        Assert-True ($probe.machine_registration_present -and !$probe.user_registration_conflict -and !$probe.device_created) 'Installed registration probe differs.'
        & (Join-Path $bin 'tingxian-video-registration-test.exe') --registered-no-device | Tee-Object -FilePath (Join-Path $evidence "registered-com-$cycle.log")
        Assert-True ($LASTEXITCODE -eq 0) 'Registered COM class did not activate.'
        & (Join-Path $bin 'tingxian-video-source-test.exe') $dll | Set-Content -LiteralPath (Join-Path $evidence "installed-source-$cycle.log") -Encoding utf8
        Assert-True ($LASTEXITCODE -eq 0) 'Installed media-source tests failed.'
        Passed "installed cycle ${cycle}: hashes, protected ACL, diagnostic and real COM/MF"

        $key = $machine.CreateSubKey($sourceKey)
        try { $key.SetValue('SyntheticUnownedSetting','keep') } finally { $key.Dispose() }
        Assert-True ((Run-Msi $msi "uninstall-$cycle" -Uninstall) -eq 0) 'Component uninstall failed.'
        Assert-Absent
        Assert-True (!(Test-Path -LiteralPath $dll)) 'Owned DLL survived uninstall.'
        Assert-True ((Get-FileHash -LiteralPath $userFile -Algorithm SHA256).Hash -ceq $userHash) 'Unknown synthetic record changed.'
        $key = $machine.OpenSubKey($sourceKey)
        try { Assert-True ($key -and $key.GetValue('SyntheticUnownedSetting') -ceq 'keep') 'Unknown registry setting removed.' } finally { if ($key) { $key.Dispose() } }
        Passed "uninstall cycle $cycle preserves unknown file and registry value"
    }
    $success = $true
} finally {
    [ordered]@{result=$(if ($success) {'pass'} else {'fail'});commit=$(& git -C (Join-Path $PSScriptRoot '..\..') rev-parse HEAD);windows=[Environment]::OSVersion.Version.ToString();checks=$checks;synthetic_only=$true;device_created=$false;physical_camera_opened=$false} |
        ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $evidence 'result.json') -Encoding utf8
    $machine.Dispose()
    [Runtime.InteropServices.Marshal]::FinalReleaseComObject($installer) | Out-Null
}
