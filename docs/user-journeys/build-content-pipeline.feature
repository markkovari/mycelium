Feature: Content pipeline (editor → fact-check → translate)

  As a marketing builder
  I want a multi-stage pipeline driven by events
  So that one draft becomes a fully-checked multi-language bundle

  Background:
    Given editor-agent, fact-checker-agent, and translator agents exist for en/de/hu/es
    And the pipeline definition exists in mycelium-pipelines/marketing-default
    And a chain-driver workload is RUNNING

  Scenario: Happy path through all stages
    When a draft is POSTed to /content/draft
    Then editor-agent processes it
    And fact-checker-agent verifies its claims
    And translator-batch produces 4 language outputs
    And a final-bundle event is published when all 4 are ready

  Scenario: Human approval gate
    Given pipeline has a human-approval stage after fact-check
    When the human sends "/approve"
    Then translation kicks off
    When the human sends "/reject too aggressive"
    Then the draft returns to editor with the reason

  Scenario: Short content skips fact-check
    Given the draft is under 280 characters
    When the pipeline runs
    Then fact-checker is skipped
    And translation begins right after editor

  Scenario: Translation memory hit
    Given a phrase has been translated previously
    When the translator processes the same source phrase
    Then memory/translations/<hash>/<lang> is read
    And no new LLM call is made for that phrase

  Scenario: Brand voice enforcement
    Given memory/brand/voice contains "avoid: synergy, leverage"
    When editor produces a draft containing those words
    Then editor returns with those terms replaced
    And the final bundle is free of disallowed terms

  Scenario: Quality scoring loop
    Given the scorer threshold is 0.7
    When a translation scores 0.5
    Then it routes back to translator with feedback
    And retries at most 2 times before flagging for human review
