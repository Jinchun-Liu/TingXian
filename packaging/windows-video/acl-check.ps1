# Read-only helper shared by CI and target-machine preflight. Display names for
# application-package groups are not reliably round-trippable through LookupAccountName.
function Assert-VideoComponentPathProtected([string]$Path) {
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Component path is redirected; preserve it and stop.' }
    $acl = Get-Acl -LiteralPath $Path
    # Windows TrustedInstaller service SID, alongside SYSTEM and Administrators.
    # https://learn.microsoft.com/en-us/archive/msdn-magazine/2008/november/access-control-understanding-windows-file-and-registry-permissions
    $allowedWriters = @('S-1-5-18','S-1-5-32-544','S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464')
    $writeRights = [Security.AccessControl.FileSystemRights]::Write -bor [Security.AccessControl.FileSystemRights]::Delete -bor [Security.AccessControl.FileSystemRights]::ChangePermissions -bor [Security.AccessControl.FileSystemRights]::TakeOwnership -bor [Security.AccessControl.FileSystemRights]::DeleteSubdirectoriesAndFiles
    # Include unmapped GENERIC_WRITE and GENERIC_ALL, not just concrete file bits.
    $writeMask = [long]$writeRights -bor 0x50000000
    foreach ($rule in $acl.GetAccessRules($true,$true,[Security.Principal.SecurityIdentifier])) {
        if ($rule.AccessControlType -ne 'Allow' -or ($rule.PropagationFlags -band [Security.AccessControl.PropagationFlags]::InheritOnly)) { continue }
        if (([long][int]$rule.FileSystemRights -band $writeMask) -and $rule.IdentityReference.Value -notin $allowedWriters) {
            throw 'Component path grants write rights to a non-system/admin identity.'
        }
    }
}
