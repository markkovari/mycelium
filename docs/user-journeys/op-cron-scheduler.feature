Feature: Cron-style scheduling admin

  As an operator running many recurring jobs
  I want a CRUD surface and audit trail for schedules
  So that I can debug missed fires without grepping logs

  Background:
    Given the cron-scheduler component is RUNNING
    And the mycelium-cron-jobs KV bucket exists
    And the mycelium-cron-history KV bucket exists

  Scenario: Register a weekday job
    When the operator POSTs a cron with expression "0 9 * * 1-5"
    Then the job is stored in mycelium-cron-jobs
    And the next fire is computed in the configured timezone
    And a history entry "registered" is written

  Scenario: List all jobs filtered by team
    Given 40 jobs exist across 5 teams
    When the operator queries GET /cron?team=eng
    Then only jobs with key prefix "cron/eng/" are returned

  Scenario: Reject invalid cron expression
    When the operator POSTs a cron with expression "0 9 * *"
    Then the response status is 400
    And the error message references the expected 5 fields

  Scenario: Dry-run preview without saving
    When the operator POSTs a cron with ?dry_run=true
    Then the response lists the next 5 fire times
    And no entry is written to mycelium-cron-jobs

  Scenario Outline: Catch-up policy after downtime
    Given the scheduler was down for 2 hours
    And the policy is "<policy>"
    When the scheduler restarts
    Then "<fires>" fires are dispatched

    Examples:
      | policy   | fires |
      | all      | 3     |
      | latest   | 1     |
      | none     | 0     |

  Scenario: One-shot reminder
    When the operator POSTs schedule "@once 2026-06-01T09:00:00Z"
    Then the job fires once at that moment
    And is auto-deleted from mycelium-cron-jobs after success

  Scenario: Dead-letter on repeated failure
    Given a job has a payload its agent always rejects
    And retry max is 3
    When the job fires
    Then it retries up to 3 times
    And a mycelium.event.cron.dead-letter event is emitted with the failure details

  Scenario: Webhook-out variant
    Given a job's subject is null and webhook is "https://api.example/notify"
    When the job fires
    Then the scheduler issues a POST to the webhook URL via wasi:http/outgoing-handler
    And response status is recorded in cron history
