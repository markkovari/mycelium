Feature: Build the cron-scheduler component

  As a builder extending mycelium
  I want a NATS-driven cron scheduler with persistence and catch-up
  So that end-user schedule features (journey 14) have a real backend

  Background:
    Given the mycelium-cron-jobs KV bucket exists
    And the mycelium-cron-history KV bucket exists
    And the cron-scheduler workload is RUNNING

  Scenario: Register and fire a minute-cadence job
    When a job with schedule "* * * * *" is registered
    Then within 90 seconds the configured subject receives a publish
    And mycelium-cron-history records the fire

  Scenario: Pause stops fires; resume restarts them
    Given a registered minute-cadence job
    When the operator pauses it
    Then no further publishes happen
    When the operator resumes it
    Then the next minute boundary triggers a fire

  Scenario: Skip-next skips exactly one occurrence
    Given a job is about to fire in 30 seconds
    When skip-next is called for that job
    Then the upcoming fire is skipped
    And the fire after that happens at the normal cadence

  Scenario: Idempotency on scheduler restart
    Given a job's next fire is at T
    When the scheduler is restarted between T-5s and T+5s
    Then exactly one publish is recorded for time T in mycelium-cron-history

  Scenario Outline: Catch-up policy
    Given the scheduler was offline for <downtime> minutes
    And the policy is "<policy>"
    When it restarts
    Then "<fires>" fires occur

    Examples:
      | downtime | policy  | fires |
      | 5        | all     | 5     |
      | 5        | latest  | 1     |
      | 5        | none    | 0     |

  Scenario: One-shot @once schedule auto-deletes
    Given a job with schedule "@once 2026-06-01T09:00:00Z"
    When the time arrives and the publish succeeds
    Then the job is removed from mycelium-cron-jobs

  Scenario: Webhook variant
    Given a job has webhook URL "https://api.example/notify" and null subject
    When the job fires
    Then a POST is issued to the webhook URL
    And the response status is logged in mycelium-cron-history

  Scenario: Invalid expression rejected at register
    When a register call uses expression "0 9 * *"
    Then the registration fails with a clear error
    And nothing is stored in mycelium-cron-jobs
