Feature: Build the batch pipeline components

  As a builder extending mycelium
  I want a batch-dispatcher and batch-coalescer that turn one ask into N tasks
  So that end-user batch features (journey 13) have a real backend

  Background:
    Given the mycelium-batches KV bucket exists
    And the mycelium-batch-state KV bucket exists
    And the mycelium-batch-dlq KV bucket exists

  Scenario: Submit triggers fan-out
    When a batch with 100 items is registered via /batches
    And the dispatcher receives mycelium.batch.submit
    Then exactly 100 mycelium.task.submit messages are published
    And each carries batch_meta {id, index}

  Scenario: Coalescer counts to done
    Given a batch of 5 items is in flight
    When 5 mycelium.step.result messages arrive matching the batch id
    Then exactly one mycelium.batch.<id>.done event is emitted
    And it contains 5 succeeded items if none failed

  Scenario: Failed item lands in DLQ
    Given an item produces a step.result with an error
    When the coalescer processes it
    Then the failure is written to mycelium-batch-dlq KV
    And it does not count as succeeded in the done event

  Scenario: Sharded dispatcher load balances
    Given 2 batch-dispatcher instances on a NATS queue group "batch-dispatcher"
    When a 1000-item batch arrives
    Then the items are split roughly evenly across the two instances
    And no item is processed twice

  Scenario: Replay failed items
    Given batch BX has 3 items in DLQ
    When the operator hits POST /batches/BX/replay-failed
    Then exactly 3 mycelium.task.submit messages are republished for those indices
    And the DLQ entries are cleared on success

  Scenario: Dispatcher survives restart
    Given a batch of 200 items has dispatched 80
    When the batch-dispatcher workload is stopped and restarted
    Then it resumes from item 81
    And no item 0-80 is re-dispatched
