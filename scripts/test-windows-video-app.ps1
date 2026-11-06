# Hypothesis: Tauri's lib-test executable lacks the v6 Common Controls manifest
# linked into the production binary. Preserve imports/resources, then attach it
# to a copy of this test executable only. Fail on compilation, embed or test error.
# Decision: test main-window IPC/quit wiring without media or application data.
[CmdletBinding()]
param([Parameter(Mandatory)][string]$EvidenceDirectory)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (Test-Path -LiteralPath $EvidenceDirectory) { throw 'Use a new evidence directory' }
$evidence = (New-Item -ItemType Directory -Path $EvidenceDirectory).FullName
Push-Location $repo
try {
    $env:CMAKE_PROJECT_INCLUDE = (Resolve-Path .github/force-portable-ggml.cmake).Path.Replace('\','/')
    & cargo test --release -p meetily --lib --locked --no-run --message-format=json 1> "$evidence/compile.jsonl" 2> "$evidence/compile.stderr.log"
    if ($LASTEXITCODE) { Get-Content "$evidence/compile.stderr.log" -Tail 60; throw 'Application test compilation failed' }
    $built = @(Get-Content "$evidence/compile.jsonl" | ForEach-Object {
        $entry = $_ | ConvertFrom-Json
        if ($entry.reason -eq 'compiler-artifact' -and $entry.profile.test -and $entry.target.name -eq 'app_lib' -and $entry.executable) { $entry.executable }
    })
    if ($built.Count -ne 1) { throw 'Expected one application library test executable' }
    $original = (Resolve-Path -LiteralPath $built[0]).Path
    & dumpbin /IMPORTS $original > "$evidence/imports.txt"
    if ($LASTEXITCODE) { throw 'Could not inspect test imports' }
    & mt.exe -nologo "-inputresource:$original;#1" "-out:$evidence/original-manifest.xml" *> "$evidence/manifest-probe.log"
    $hasManifest = $LASTEXITCODE -eq 0
    # Use the same directory for loader DLL lookup; never overwrite the original
    # test/cache artifact or patch the production application.
    $copy = Join-Path (Split-Path $original) ('video-ipc-test-' + [guid]::NewGuid().ToString('N') + '.exe')
    Copy-Item -LiteralPath $original -Destination $copy
    $originalHash = (Get-FileHash -LiteralPath $original -Algorithm SHA256).Hash.ToLower()
    if (!$hasManifest) {
        & mt.exe -nologo -manifest "$PSScriptRoot/windows-test-controls.xml" "-outputresource:$copy;#1" *> "$evidence/manifest-embed.log"
        if ($LASTEXITCODE) { throw 'Could not embed test Common Controls manifest' }
    }
    & mt.exe -nologo "-inputresource:$copy;#1" "-out:$evidence/test-manifest.xml" *> "$evidence/manifest-verify.log"
    if ($LASTEXITCODE) { throw 'Test manifest is missing after preparation' }
    $manifest = Get-Content "$evidence/test-manifest.xml" -Raw
    if ($manifest -notmatch 'Microsoft.Windows.Common-Controls' -or $manifest -notmatch '6.0.0.0') { throw 'Test manifest lacks Common Controls v6' }
    $testExit = 0
    foreach ($suite in @('interview_video::tests','interview_windows::tests')) {
        & $copy $suite --nocapture 2>&1 | Tee-Object "$evidence/test.log" -Append
        if ($LASTEXITCODE) { $testExit = $LASTEXITCODE; break }
    }
    if ((Get-FileHash -LiteralPath $original -Algorithm SHA256).Hash.ToLower() -ne $originalHash) { throw 'Original test artifact changed' }
    @{original_manifest_present=$hasManifest;original_sha256=$originalHash;prepared_sha256=(Get-FileHash -LiteralPath $copy -Algorithm SHA256).Hash.ToLower();test_exit=$testExit;production_manifest_changed=$false} |
        ConvertTo-Json | Set-Content "$evidence/result.json" -Encoding utf8
    if ($testExit) { throw "Application video IPC tests failed: $testExit" }
} finally { Pop-Location }
