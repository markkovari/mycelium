Feature: Support team operations

  As a support team lead
  I want the bot to handle tier-1 and hand off cleanly
  So that humans only touch the hard cases

  Background:
    Given a "tier1" agent exists with order-lookup tool
    And a router rule fans mycelium.event.handoff to "support-queue"
    And the human dashboard subscribes to "support-queue"

  Scenario: Tier-1 self-resolves
    When a customer asks "where's my order"
    Then the agent calls order-lookup
    And the agent replies with the shipping status
    And mycelium.event.ticket.resolved is published on confirmation

  Scenario: Auto handoff after 3 turns
    Given a conversation has 3 turns without resolution
    When the customer sends a 4th message
    Then mycelium.event.handoff is published
    And the bot stops posting further replies

  Scenario: Sentiment routing
    Given the customer's sentiment is below -0.5
    When the customer's first message arrives
    Then mycelium.event.handoff is published immediately
    And tier-1 does not attempt a reply

  Scenario: Macro expansion for human reply
    Given a human-typed message contains "/macro greeting"
    When the message is sent
    Then the macro is expanded from memory/team/macros
    And the customer sees the expanded text

  Scenario: SLA breach warning
    Given a ticket has SLA "1h" and is 50 minutes old without resolution
    When the SLA cron checks
    Then a warning event mycelium.event.sla.warning is published
    And the team channel receives a Telegram notice

  Scenario: Daily reporting digest
    When the daily 18:00 cron fires
    Then a digest message is sent to the team channel
    And it includes resolved count, avg response time, top 3 topics

  Scenario: Privacy / data subject access
    When a customer requests data deletion
    Then the request is routed to a human in the privacy queue
    And the deletion is performed only after human approval
