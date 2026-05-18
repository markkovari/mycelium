Feature: Batched action over many items

  As a user with a list of similar items
  I want to dispatch one batch and get all results back
  So that I don't manually paste 200 prompts

  Background:
    Given a "rewrite-friendly" agent exists
    And a batch-dispatcher worker is RUNNING and subscribed to mycelium.batch.submit
    And the mycelium-batch-results KV bucket exists

  Scenario: Submit a batch and get a tracking id
    When Anett POSTs a batch of 200 items with agent_id "rewrite-friendly"
    Then the response is a batch id within 1 second
    And 200 mycelium.task.submit messages are published with that batch tag
    And the API returns an estimated completion time

  Scenario: Check status of a running batch
    Given a batch "XK3F9" is in progress with 50/200 done
    When the user asks "/batch status XK3F9"
    Then the bot replies with "50/200 done, ~2 minutes left"

  Scenario: Complete batch emits a single done event
    When all 200 items have produced a step.result
    Then exactly one mycelium.batch.XK3F9.done event is published
    And the event payload includes 198 succeeded and 2 failed item indices

  Scenario: Cancel a batch in flight
    Given batch "XK3F9" has dispatched 150/200 items
    When the user issues "/batch cancel XK3F9"
    Then no further mycelium.task.submit messages are published for this batch
    And in-flight items finish naturally
    And the user is notified on completion

  Scenario: Retry specific failures
    Given batch "XK3F9" finished with items 47 and 132 failed
    When the user issues "/batch retry XK3F9 47 132"
    Then exactly 2 mycelium.task.submit messages are published
    And only those two item indices are retried

  Scenario Outline: Per-item agent override
    Given a batch with items that each carry their own agent_id
    When the dispatcher fans out tasks
    Then item "<i>" routes to agent "<agent>"

    Examples:
      | i | agent          |
      | 0 | translator-hu  |
      | 1 | translator-de  |
      | 2 | translator-en  |
