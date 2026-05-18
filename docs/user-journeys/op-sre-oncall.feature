Feature: SRE oncall responder

  As the oncall engineer
  I want alerts triaged and escalated automatically
  So that I only get paged when I really need to act

  Background:
    Given an "oncall-agent" exists with runbook system_prompt
    And POST /alerts is wired into mycelium-api
    And cron-scheduler is RUNNING for escalation timers

  Scenario: High-severity alert pages oncall
    When an alert with severity "high" hits POST /alerts
    Then a mycelium.task.submit is published for oncall-agent
    And within 8 seconds Marek receives a Telegram message
    And the message offers /ack /escalate /silence options

  Scenario: Ack stops paging
    Given an active alert
    When Marek replies "/ack"
    Then mycelium-events-journal records ack with timestamp and chat_id
    And no further pages for that alert are sent

  Scenario: Auto-escalation if no ack
    Given an alert was paged 5 minutes ago without ack
    When the 5-minute escalation cron fires
    Then the next oncall in rotation is paged
    And the original page is marked "escalated"

  Scenario: Silence with reason
    When Marek sends "/silence 30m known bug PR-1234"
    Then the alert is silenced for 30 minutes
    And the reason is stored in the alert's event record
    And the silence auto-expires after 30 minutes

  Scenario: Runbook lookup
    Given memory/runbook/api-prod contains steps
    When an alert for "api-prod" fires
    Then the agent's reply references at least one step from the runbook

  Scenario: Multi-step diagnostic batch
    When Marek sends "/diag api-prod"
    Then a batch is dispatched with metrics, logs, and trace tool calls
    And within 30 seconds Marek receives a single coalesced summary

  Scenario: Handoff to next oncall
    When Marek sends "/handoff @sasha"
    Then memory/oncall/current becomes "sasha"
    And subsequent pages go to Sasha
