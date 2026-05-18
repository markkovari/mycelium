Feature: Accessibility-first usage

  As a user relying on screen reader or voice-only interaction
  I want clean, audible, well-described content
  So that the bot is fully usable without sight

  Background:
    Given an "a11y" agent exists with accessibility-aware system_prompt
    And a TTS workload is RUNNING
    And memory/<user>/voice_reply is "on"

  Scenario: Voice-in to voice-out round trip
    When Jakub sends a voice memo
    Then within 12 seconds an audio reply is delivered
    And a plain-text version accompanies the audio

  Scenario: No emoji in any reply
    When the agent's draft reply contains emoji
    Then a post-processor strips them before delivery
    And the cleaned reply is delivered

  Scenario: No markdown in any reply
    When the agent's draft reply contains "*bold*" or "_italic_"
    Then the markers are removed
    And the words remain plain text

  Scenario: Links carry alt-text descriptions
    Given the agent reply includes a URL
    When the reply is finalised
    Then the URL is preceded by a human-readable description
    And the URL itself is also present (so screen reader can read both)

  Scenario: TTS rate honoured
    Given memory/<user>/tts_rate is 0.85
    When an audio reply is synthesised
    Then the audio playback rate is 0.85x

  Scenario Outline: Voice commands intercepted
    When the user says "<command>"
    Then the bot does NOT submit a new task
    And the bot performs the local action "<action>"

    Examples:
      | command       | action            |
      | stop reading  | stop current TTS  |
      | repeat last   | resend last reply |
      | louder        | volume up         |
      | slower        | TTS rate * 0.9    |
