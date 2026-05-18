Feature: Journalist toolkit

  As an investigative journalist
  I want audio in, fact-checked drafts out
  So that my time goes to investigation, not transcription

  Background:
    Given a "journalist-writer" agent exists
    And transcriber, entity-extractor, source-checker workloads are RUNNING
    And public-records and cite-format tools are wired

  Scenario: Interview to draft happy path
    When Reka uploads a 30-minute interview to /interviews
    Then within 10 minutes a transcript is produced
    And entities are extracted
    And source-checker cross-references at least 3 claims
    And the writer produces a draft with footnoted citations

  Scenario: Off-the-record segment is excluded
    Given the transcript contains "off the record" at 12:33 and "back on record" at 14:01
    When the writer composes the draft
    Then content from 12:33 to 14:01 is omitted
    And a notice is logged

  Scenario: Speaker diarisation
    Given the audio has 3 distinct speakers
    When the transcript renders
    Then each segment is labelled A, B, or C
    And the UI lets Reka rename labels to real names

  Scenario: Verification needed callout
    Given two sources disagree on a date
    When source-checker processes them
    Then the draft includes a "verification needed" callout for that fact

  Scenario: Negative claim about a living person needs 2 sources
    Given a claim is detected as negative + about a living person
    When source-checker finds only 1 corroborating source
    Then the writer flags it and excludes it from the draft body
    And a sidebar lists it as "needs more sources"

  Scenario: Translation while preserving quotes
    Given the source audio is Hungarian
    And Reka asks for an English draft
    When the writer outputs the draft
    Then direct quotes appear in Hungarian with English translation in brackets

  Scenario: Legal hold prevents deletion
    Given Reka marks an interview as "legal-hold"
    When the retention sweep runs
    Then no segments from that interview are deleted
