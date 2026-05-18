Feature: Multi-model A/B comparison

  As a PM evaluating model options
  I want to run the same prompts through multiple variants
  So that I can pick the best on quality, cost, and latency

  Background:
    Given 4 agent variants exist differing only in model/endpoint/api_key
    And a scoring-agent and reporter-agent exist
    And the batch pipeline is RUNNING

  Scenario: Submit 4 parallel batches
    When 4 batches of 200 items each are submitted
    Then 800 mycelium.task.submit messages are published
    And each batch is tagged with its variant id

  Scenario: All step.results carry cost telemetry
    Given each variant agent enriches its step.result
    When step.results land
    Then each contains tokens_in, tokens_out, cost_usd

  Scenario: Blind scoring
    Given scorer is shuffled before reading outputs
    When the scorer rates 100 outputs
    Then no output preserves variant attribution to the scorer
    And the report joins back to variants by id only at report time

  Scenario Outline: Stratified reporting
    Given prompts have category labels
    When the reporter aggregates results
    Then a category-by-variant matrix is produced

    Examples:
      | category   |
      | formal     |
      | casual     |
      | complaint  |
      | sales      |

  Scenario: Drift detection
    Given last-month's scores are stored
    When this month's run completes
    Then variants whose score dropped > 10% raise mycelium.event.drift.alert
    And the reporter highlights them

  Scenario: Cost dry-run before submit
    When a 200-item batch is submitted with ?dry_run_cost=true
    Then no LLM calls are made
    And an estimated cost is returned within 1 second
