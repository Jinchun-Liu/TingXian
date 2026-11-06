param([Parameter(Mandatory=$true)][string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
# Test fixtures only. SpVoice writes to SpFileStream, never to the audio device.
# Microsoft SAPI AudioOutputStream / SpeechAudioFormatType documentation:
# https://learn.microsoft.com/en-us/previous-versions/windows/desktop/ms723597(v=vs.85)
# https://learn.microsoft.com/en-us/previous-versions/windows/desktop/ms720595(v=vs.85)
$destination = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $destination) { throw 'Use a new evidence directory; existing artifacts are preserved.' }
[void][IO.Directory]::CreateDirectory($destination)
$cases = @(
  @{id='zh';voice='Microsoft Huihui Desktop - Chinese (Simplified)';text='欢迎参加模拟面试。我们讨论 StockMarketAgent 的契约测试。JSON Schema 中，null、零和缺失字段有什么区别？事务提交后超时，如何使用幂等键安全重试？请说明验证依据，不要把未知结果当作失败。'},
  @{id='en';voice='Microsoft Zira Desktop - English (United States)';text='Welcome to interview practice. Please explain contract testing for StockMarketAgent. How do null, zero, and a missing field differ in JSON Schema? If a transaction commits but the response times out, how would an idempotency key make retries safe?'}
)
$records = @()
foreach ($case in $cases) {
  $voice = $null; $stream = $null; $format = $null; $opened = $false
  try {
    $voice = New-Object -ComObject SAPI.SpVoice
    $token = @($voice.GetVoices() | Where-Object { $_.GetDescription() -eq $case.voice })
    if ($token.Count -ne 1) { throw "Required installed offline voice unavailable: $($case.voice)" }
    $voice.Voice = $token[0]
    $voice.AllowAudioOutputFormatChangesOnNextSet = $false
    $format = New-Object -ComObject SAPI.SpAudioFormat
    $format.Type = 18 # SAFT16kHz16BitMono
    $stream = New-Object -ComObject SAPI.SpFileStream
    $stream.Format = $format
    $path = Join-Path $destination ($case.id + '.wav')
    $stream.Open($path,3,$false) # SSFMCreateForWrite; new directory only
    $opened = $true
    $voice.AudioOutputStream = $stream
    if ($null -eq $voice.AudioOutputStream) { throw 'File output stream was not assigned; no speech will be requested.' }
    [void]$voice.Speak($case.text,0)
    $stream.Close(); $opened = $false
    $records += @{id=$case.id;voice=$case.voice;text=$case.text;path=$path;sha256=(Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant();destination='file_stream';audio_playback=$false;capture=$false}
  } finally {
    if ($opened) { $stream.Close() }
    foreach ($object in @($voice,$stream,$format)) { if ($null -ne $object) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($object) } }
  }
}
$records | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $destination 'manifest.json') -Encoding utf8
Write-Output "Created $($records.Count) local synthetic WAV fixtures without audio playback."
