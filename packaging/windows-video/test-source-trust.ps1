param([Parameter(Mandatory)][string]$BinaryDirectory,[Parameter(Mandatory)][string]$EvidenceDirectory)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') { throw 'Disposable hosted runner only.' }
$evidence=[IO.Path]::GetFullPath($EvidenceDirectory)
if (Test-Path -LiteralPath $evidence) { throw 'Preserve evidence.' }
New-Item -ItemType Directory -Path $evidence | Out-Null
$dll=Join-Path $env:ProgramW6432 'TingXian\VideoSource\TingXianVideoSource.dll'
$probe=Join-Path $BinaryDirectory 'tingxian-video-device-probe.exe'
$sourceKey='SOFTWARE\Classes\CLSID\{44D20DA7-55EC-4B4A-A767-99E97D6E5A90}'
$machine=[Microsoft.Win32.RegistryKey]::OpenBaseKey('LocalMachine','Registry64')
$user=[Microsoft.Win32.RegistryKey]::OpenBaseKey('CurrentUser','Registry64')
$server=$machine.OpenSubKey($sourceKey+'\InprocServer32',$true)
$original=$server.GetValue('')
$backup=Join-Path $evidence 'source-before-synthetic-tamper.dll'
Copy-Item -LiteralPath $dll -Destination $backup
$originalHash=(Get-FileHash -LiteralPath $dll).Hash
$originalAcl=Get-Acl -LiteralPath $dll
$results=[Collections.Generic.List[object]]::new()
function Probe([string]$Name,[string]$Expected) {
    $text=& $probe --probe
    if ($LASTEXITCODE) { throw 'Probe did not complete.' }
    $text | Set-Content -LiteralPath (Join-Path $evidence ($Name+'.json')) -Encoding utf8
    $p=$text | ConvertFrom-Json
    if ($p.registration_error -ne $Expected -or $p.source_integrity_verified -ne ($Expected -eq '0x00000000') -or $p.device_created) { throw "Unexpected trust decision: $Name" }
    $results.Add([ordered]@{case=$Name;error=$p.registration_error;verified=$p.source_integrity_verified})
}
try {
    Probe 'intact' '0x00000000'
    & (Join-Path $BinaryDirectory 'tingxian-source-trust-test.exe') --installed-lease $dll | Set-Content -LiteralPath (Join-Path $evidence 'lease.log') -Encoding utf8
    if ($LASTEXITCODE) { throw 'Installed lease regression failed.' }
    try {
        $bytes=[IO.File]::ReadAllBytes($dll)
        $bytes[$bytes.Length-1]=$bytes[$bytes.Length-1] -bxor 1
        [IO.File]::WriteAllBytes($dll,$bytes)
        Probe 'byte-tamper' '0x80070017'
    } finally { Copy-Item -LiteralPath $backup -Destination $dll }
    Probe 'byte-restored' '0x00000000'
    try {
        $server.SetValue('',$backup)
        Probe 'outside-system-install' '0x800700a1'
    } finally { $server.SetValue('',$original) }
    Probe 'path-restored' '0x00000000'
    $existing=$user.OpenSubKey($sourceKey)
    if ($existing) { $existing.Dispose(); throw 'Preserve existing user registration.' }
    try {
        $shadow=$user.CreateSubKey($sourceKey)
        $shadow.SetValue('SyntheticOnly','created-by-this-check'); $shadow.Dispose()
        Probe 'user-shadow' '0x80070005'
    } finally {
        # Exact synthetic key, proven absent before creation; preserve its value in evidence.
        '{"SyntheticOnly":"created-by-this-check"}' | Set-Content -LiteralPath (Join-Path $evidence 'user-shadow-fixture.json')
        $user.DeleteSubKey($sourceKey)
    }
    Probe 'shadow-restored' '0x00000000'
    try {
        $mutable=Get-Acl -LiteralPath $dll
        $users=[Security.Principal.SecurityIdentifier]::new('S-1-5-32-545')
        $mutable.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($users,[Security.AccessControl.FileSystemRights]::Write,[Security.AccessControl.AccessControlType]::Allow))
        Set-Acl -LiteralPath $dll -AclObject $mutable
        Probe 'unsafe-write-permission' '0x80070005'
    } finally { Set-Acl -LiteralPath $dll -AclObject $originalAcl }
    Probe 'acl-restored' '0x00000000'
    if ((Get-FileHash -LiteralPath $dll).Hash -cne $originalHash) { throw 'Restored bytes differ.' }
} finally {
    $results | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $evidence 'results.json') -Encoding utf8
    $server.Dispose(); $machine.Dispose(); $user.Dispose()
}
