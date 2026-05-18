Feature: Operator-managed nightly batch jobs

  As a batch ops admin running thousands of items per night
  I want progress, budget control, and failure containment
  So that bad batches don't burn money or block the morning report

  Background:
    Given a batch-dispatcher is RUNNING
    And mycelium-agent has AGENT_POOL_SIZE >= 8
    And the operator has access to NATS subjects mycelium.batch.>

  Scenario: Submit a 50k-item batch
    When the operator POSTs a 50000-item batch
    Then the response includes batch_id, item count, and ETA
    And dispatch begins within 5 seconds

  Scenario: Progress events stream
    Given a batch is mid-flight
    When the operator subscribes to mycelium.batch.<id>.progress
    Then a progress update is published every 30 seconds
    And each update includes done, failed, running, eta_min

  Scenario: Budget gate triggers warning
    Given the batch's projected cost exceeds the configured budget
    When 10% of items have completed
    Then a mycelium.event.batch.over-budget message is published
    And dispatching pauses until operator approval

  Scenario: Failure-burst auto-pause
    Given the failure rate exceeds 5%
    When the dispatcher checks the rolling window
    Then the batch is auto-paused
    And mycelium.event.batch.unhealthy is published with the failure rate

  Scenario: Tenant partitioning
    Given items in a batch carry tenant_id annotations
    When fan-out happens
    Then each tenant's events go to mycelium.event.tenant.<tenant_id>
    And no cross-tenant message routing occurs

  Scenario: Re-run from prior batch
    Given batch ME-2026-05-17 finished with poor quality
    When the operator submits a re-run referencing the prior batch
    Then the same input items are dispatched with the updated agent config

  Scenario: Survives wash host restart mid-batch
    Given a batch is at 30000/50000 items
    When the wash host is killed and restarted
    Then no items are double-processed
    And remaining items continue from KV-persisted state
