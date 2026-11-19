$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)

function Send-Message($message) {
    [Console]::WriteLine((ConvertTo-Json -InputObject $message -Depth 5 -Compress))
}

$engine = $null
try {
    Add-Type -AssemblyName System.Speech
    $installed = [System.Speech.Recognition.SpeechRecognitionEngine]::InstalledRecognizers()
    $language = $env:VOICETAG_SPEECH_LANGUAGE
    if ($language -eq 'auto') {
        $language = [System.Globalization.CultureInfo]::CurrentUICulture.TwoLetterISOLanguageName
    }
    $recognizer = $installed | Where-Object { $_.Culture.TwoLetterISOLanguageName -eq $language } | Select-Object -First 1
    if (-not $recognizer -and $env:VOICETAG_SPEECH_LANGUAGE -eq 'auto') {
        $recognizer = $installed | Select-Object -First 1
    }
    if (-not $recognizer) {
        throw 'Windows 未安装所选语言的桌面语音识别组件。请安装相应组件，或直接选择手动听写。'
    }

    $engine = [System.Speech.Recognition.SpeechRecognitionEngine]::new($recognizer)
    $engine.SetInputToWaveFile($env:VOICETAG_SPEECH_WAV)
    $engine.LoadGrammar([System.Speech.Recognition.DictationGrammar]::new())
    $null = Register-ObjectEvent -InputObject $engine -EventName SpeechRecognized -SourceIdentifier VoicetagSpeech
    $null = Register-ObjectEvent -InputObject $engine -EventName RecognizeCompleted -SourceIdentifier VoicetagDone
    $segments = [System.Collections.Generic.List[object]]::new()
    Send-Message @{ type = 'stage'; message = 'Windows 正在离线识别' }
    $engine.RecognizeAsync([System.Speech.Recognition.RecognizeMode]::Multiple)
    while ($true) {
        $event = Wait-Event
        Remove-Event -EventIdentifier $event.EventIdentifier
        if ($event.SourceIdentifier -eq 'VoicetagDone') {
            if ($event.SourceEventArgs.Error) { throw $event.SourceEventArgs.Error }
            break
        }
        if ($event.SourceIdentifier -ne 'VoicetagSpeech') { continue }
        $result = $event.SourceEventArgs.Result
        if (-not $result.Text -or -not $result.Audio) { continue }
        $start = $result.Audio.AudioPosition.TotalSeconds
        $segments.Add(@{
            id = ''; start = $start; end = $start + $result.Audio.Duration.TotalSeconds
            text = $result.Text; speakerId = 'unknown'
        })
    }
    if ($segments.Count -eq 0) { throw 'Windows 未识别出文字。可以回听音频并手动听写。' }
    Send-Message @{
        type = 'result'
        speakers = @(@{ id = 'unknown'; name = '待确认' })
        segments = $segments.ToArray()
    }
} catch {
    Send-Message @{ type = 'error'; message = $_.Exception.Message }
    exit 1
} finally {
    if ($engine) { $engine.Dispose() }
}
