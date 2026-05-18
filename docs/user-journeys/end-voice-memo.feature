Feature: Voice memo to text reply

  As a hands-busy user
  I want to send voice memos and get useful text replies
  So that I can interact while driving or cooking

  Background:
    Given a "voice-helper" agent exists
    And a transcriber workload is RUNNING
    And telegram-out is RUNNING

  Scenario: Short voice memo
    When Robi sends a 5-second voice note "remind me to call mum tonight"
    Then within 8 seconds the bot replies in text
    And the reply references "mum" and "tonight"

  Scenario: Voice-reply mode
    Given memory/robi/voice_reply is "on"
    When Robi sends a voice memo
    Then the bot reply is delivered as an audio file
    And the audio duration is under 30 seconds

  Scenario: Low-confidence transcription
    Given the STT response confidence is below 0.6
    When Robi sends a voice memo
    Then the bot quotes its transcript and asks for confirmation
    And no task is submitted until Robi confirms

  Scenario: Local STT (offline)
    Given the deployment is on a Raspberry Pi with whisper.cpp wired in
    When Robi sends a voice memo on a slow link
    Then transcription happens locally without an external HTTP call

  Scenario: Long memo handled in chunks
    When Robi sends a 4-minute voice memo
    Then within 10 seconds the bot sends "still listening..."
    And the final reply arrives within 60 seconds
