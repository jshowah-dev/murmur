# Regenerates the synthetic test fixtures: powershell -File tests/fixtures/make_fixtures.ps1 -Out tests/fixtures
param([string]$Out, [string]$Voice = 'Microsoft Zira Desktop')
Add-Type -AssemblyName System.Speech
$clips = [ordered]@{
  'the_hawb_is_late.wav' = 'The hob is late.'
  'link_the_mawb_to_the_hawb.wav' = 'Link the mob to the hob.'
  'bol_shows_the_shipment_as_delivered.wav' = 'B O L shows the shipment as delivered.'
  'open_the_jira_ticket_for_the_customer.wav' = 'Open the Jira ticket for the customer.'
  'deploy_to_uat_after_the_build_finishes.wav' = 'Deploy to U A T after the build finishes.'
  'thanks_new_line_jeff.wav' = 'Thanks, new line, Jeff.'
  'the_ear_build_passed_on_the_first_try.wav' = 'The ear build passed on the first try.'
}
$fmt = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono)
foreach ($k in $clips.Keys) {
  $s = New-Object System.Speech.Synthesis.SpeechSynthesizer
  $s.SelectVoice($Voice)
  $s.SetOutputToWaveFile((Join-Path $Out $k), $fmt)
  $s.Speak($clips[$k])
  $s.Dispose()
}
