Feature: Scheduled-digest subscriber

  As a user who wants periodic summaries
  I want to register a cron-like schedule once
  So that the bot pushes me digests without me asking each time

  Background:
    Given a "digest-bot" agent exists
    And a cron-scheduler component is RUNNING
    And the mycelium-cron-jobs KV bucket exists

  Scenario: Register a weekly schedule
    When Tomas sends "/schedule weekly Monday 8:00 summarize my journal last 7 days"
    Then a new cron job is stored in mycelium-cron-jobs with a unique id
    And the bot replies with the next fire time in Tomas's time zone
    And no message is sent before that time

  Scenario: Schedule fires and produces a digest
    Given a registered cron job for Tomas at Monday 8:00
    When the local time matches the cron expression
    Then exactly one mycelium.task.submit is published with Tomas's payload
    And within 30 seconds telegram-out delivers a digest to Tomas
    And the digest references the last 7 days of journal entries

  Scenario: Empty-data digest is handled gracefully
    Given Tomas has no journal entries in the last 7 days
    When the schedule fires
    Then the digest message reads "no entries this week" (or operator-defined)
    And no error is shown to the user

  Scenario: Pause and resume a schedule
    When Tomas sends "/schedule pause MN-08"
    Then no fires occur until resumed
    When Tomas sends "/schedule resume MN-08"
    Then the next scheduled fire happens at the normal time

  Scenario: Skip a single occurrence
    Given the next fire is tomorrow 8:00
    When Tomas sends "/schedule skip-next MN-08"
    Then tomorrow's fire is skipped
    And the fire after tomorrow happens normally

  Scenario: Time zone awareness
    Given memory/tomas/tz is "Europe/Budapest"
    When a "Monday 8:00" schedule is registered
    Then the cron-scheduler computes the next fire in UTC correctly
    And accounts for DST transitions

  Scenario: Idempotent on scheduler restart
    Given a cron job is about to fire at 08:00:00
    When the cron-scheduler crashes and restarts at 08:00:01
    Then at most one task is submitted for that fire time
