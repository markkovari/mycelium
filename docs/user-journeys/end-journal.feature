Feature: Daily journal

  As a person who wants a low-friction reflection habit
  I want a nightly prompt and weekly digest
  So that I notice patterns without manual effort

  Background:
    Given a "journal-bot" agent exists
    And a "journal-digest" agent exists
    And the cron-scheduler is RUNNING

  Scenario: Nightly prompt fires
    Given a cron job at 22:00 local time is registered for Domi
    When the local time reaches 22:00
    Then exactly one prompt is sent to Domi
    And the prompt is short (under 100 characters)

  Scenario: Entry is persisted with mood tag
    When Domi replies "tired, run, tense meeting"
    Then memory/domi/journal/<date> stores the entry text
    And memory/domi/journal/<date>/mood holds a sentiment label

  Scenario: Sunday digest summarises the week
    Given 5 entries exist for the past 7 days
    When the Sunday 09:00 cron fires
    Then a digest message is sent referencing those 5 entries
    And the digest includes a mood trend

  Scenario: Skip a day
    When Domi sends "/skip today"
    Then no nightly prompt is sent today
    And the streak counter does not reset

  Scenario: Export to PDF
    When Domi sends "/export pdf"
    Then a batch is dispatched to produce a PDF
    And within 60 seconds Domi receives a downloadable link

  Scenario: Encrypted-at-rest mode
    Given memory/domi/journal/encryption is enabled
    When Domi posts an entry
    Then the KV value is opaque encrypted bytes
    And the bot cannot summarise without the user's passphrase

  Scenario: Catch-up after scheduler downtime
    Given the scheduler was offline at 22:00
    And the catch-up policy is "latest"
    When the scheduler restarts at 22:30
    Then exactly one prompt is sent for today
