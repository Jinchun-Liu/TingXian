param(
    [Parameter(Mandatory = $true)][string]$InstallDirectory,
    [Parameter(Mandatory = $true)][string]$EvidenceDirectory,
    [switch]$PlatformCheck
)

# Read installed resources, then run the existing isolated offline checks.
# Does not install/uninstall, open audio devices, or touch the user's database.
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath $InstallDirectory).Path
$evidence = [IO.Path]::GetFullPath($EvidenceDirectory)
if (Test-Path -LiteralPath $evidence) { throw 'Choose a new evidence directory; existing results are never overwritten.' }
New-Item -ItemType Directory -Path $evidence | Out-Null
$report = [ordered]@{
    checkedAt = (Get-Date).ToString('o')
    installDirectory = $root
    manifestEntries = 0
    missing = @()
    mismatched = @()
    invalidPaths = @()
    offlineExitCode = $null
    platformExitCode = $null
    passed = $false
}
function Save-Report {
    $report | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $evidence 'result.json') -Encoding UTF8
}
function Run-OfflineCheck([string]$Argument, [string]$Name) {
    $savedPath = $env:PATH
    try {
        # No developer Python, FFmpeg, Rust, Node, or VS on the child PATH.
        $env:PATH = "$env:SystemRoot\System32;$env:SystemRoot;$root"
        $process = Start-Process -FilePath (Join-Path $root 'meetily.exe') -ArgumentList $Argument -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $evidence "$Name.stdout") -RedirectStandardError (Join-Path $evidence "$Name.stderr")
        if (!$process.WaitForExit(120000)) {
            # This command only owns a synthetic self-check, never a GUI session.
            Stop-Process -Id $process.Id -ErrorAction SilentlyContinue
            throw "$Name exceeded 120 seconds; inspect the retained logs."
        }
        $process.WaitForExit()
        return $process.ExitCode
    } finally { $env:PATH = $savedPath }
}
try {
    $manifestPath = Join-Path $root 'runtime-manifest.json'
    if (!(Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        $report.missing += 'runtime-manifest.json'
    } else {
        $manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
        $entries = @($manifest.PSObject.Properties)
        $report.manifestEntries = $entries.Count
        if ($entries.Count -eq 0) { throw 'Installed runtime manifest is empty.' }
        foreach ($entry in $entries) {
            $relative = $entry.Name -replace '^media/', ''
            $path = [IO.Path]::GetFullPath((Join-Path $root $relative))
            if (!$path.StartsWith($root.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
                $report.invalidPaths += $entry.Name
                continue
            }
            if (!(Test-Path -LiteralPath $path -PathType Leaf)) {
                $report.missing += $relative
            } elseif ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $entry.Value) {
                $report.mismatched += $relative
            }
        }
    }
    # The staged manifest does not cover the app or the separately built ONNX bundle.
    foreach ($relative in @('meetily.exe', 'uninstall.exe', 'python/python.exe', 'whisper/ggml-small.bin', 'onnxruntime.dll', 'onnxruntime_providers_shared.dll')) {
        if (!(Test-Path -LiteralPath (Join-Path $root $relative) -PathType Leaf)) {
            $report.missing += $relative
        }
    }
    $report.missing = @($report.missing | Sort-Object -Unique)
    Save-Report
    if ($report.missing.Count -or $report.mismatched.Count -or $report.invalidPaths.Count) {
        throw "Installation incomplete: $($report.missing.Count) missing, $($report.mismatched.Count) modified, $($report.invalidPaths.Count) invalid paths. See result.json."
    }
    $report.offlineExitCode = Run-OfflineCheck '--installation-check' 'offline'
    if ($report.offlineExitCode -ne 0) { throw "Offline self-check failed: exit $($report.offlineExitCode)." }
    if ($PlatformCheck) {
        # Explicit opt-in: an in-memory monitor screenshot and synthetic recoverable archive.
        $report.platformExitCode = Run-OfflineCheck '--windows-platform-check' 'platform'
        if ($report.platformExitCode -ne 0) { throw "Platform self-check failed: exit $($report.platformExitCode)." }
    }
    $report.passed = $true
    Write-Output "PASS: $($report.manifestEntries) resource hashes and offline runtime; real audio/GUI/AI acceptance remains separate."
} finally { Save-Report }
