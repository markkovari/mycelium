Feature: Multilingual end user

  As a user not writing in English
  I want the bot to reply in my language naturally
  So that I don't have to translate twice

  Background:
    Given a multilingual-capable agent "polyglot" is configured

  Scenario: Auto-detect on first message
    Given memory/<chat>/lang is unset
    When the user writes "mit főzzek vacsorára?"
    Then a language-id tool is invoked
    And memory/<chat>/lang is set to "hu"
    And the reply is in Hungarian

  Scenario: Override with /lang
    Given memory/<chat>/lang is "hu"
    When the user sends "/lang en"
    Then memory/<chat>/lang becomes "en"
    And the next reply is in English

  Scenario: Auto re-detect each message
    Given memory/<chat>/lang is "auto"
    When user sends "wie geht's"
    Then the bot replies in German for that turn
    And memory/<chat>/lang is not modified

  Scenario Outline: Common languages handled
    When the user writes "<input>"
    Then the bot's first reply is in language "<expected>"

    Examples:
      | input                | expected |
      | mit főzzek           | hu       |
      | wie wird das Wetter  | de       |
      | how is the weather   | en       |
      | qué tiempo hace      | es       |

  Scenario: Bridging between members in a group
    Given a family group with Pal (de) and Anna (hu)
    When Pal sends "/translate-to anna heute spät"
    Then Anna sees a Hungarian translation in the group
    And no original German is repeated to her

  Scenario: Locale-aware date formatting
    Given memory/<chat>/lang is "hu"
    When the bot mentions a date
    Then the date format is "YYYY. MM. DD."
