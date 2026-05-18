Feature: DevOps runbook engine

  As a DevOps engineer
  I want YAML runbooks that auto-triage and optionally auto-fix
  So that simple incidents resolve without paging anyone

  Background:
    Given runbook YAMLs are stored in memory/runbook/*
    And tool wrappers (kube_pods_status, etc.) are registered
    And an action-runner with permission tiers is RUNNING

  Scenario: Auto-rollback on safe condition
    Given the latency_p99 alert fires for api-prod
    And the recent deploy is rollback-safe
    When the runbook executes
    Then a "rollback" action is dispatched
    And no human approval is requested
    And mycelium.event.action.rollback.success is published

  Scenario: Human approval for risky action
    Given the runbook decides "scale_memory" with permission human-approve
    When the runbook reaches the action step
    Then a Telegram approval prompt is sent
    And no kubectl command is executed before /approve

  Scenario: Dry-run prints plan only
    When the operator runs "/runbook dry api-prod"
    Then the steps execute (diagnostics only)
    And no action is executed
    And the operator sees the chosen action and the would-be command

  Scenario: Refuse rollback without known-good
    Given no prior version is recorded as healthy
    When the runbook decides "rollback"
    Then the action is downgraded to human-approve
    And a reason "no known-good version" is included

  Scenario: Out-of-hours downgrade
    Given current time is 02:00 local
    And the runbook auto-action is allowed only during business hours
    When the alert fires
    Then the action becomes human-approve regardless of the YAML

  Scenario: Postmortem draft
    Given an incident was resolved at 03:15
    When the postmortem cron fires
    Then a draft is generated with the action timeline
    And the draft is posted to the team channel

  Scenario: Runbook linter rejects unsafe YAML
    When the operator commits a runbook without `permission:` field
    Then the linter rejects the commit
    And the KV is not updated
