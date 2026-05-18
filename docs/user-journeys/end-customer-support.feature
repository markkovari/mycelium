Feature: Customer support seeker

  As a customer with a problem
  I want a bot that triages and escalates fast
  So that I either self-serve or get a human within minutes

  Background:
    Given a "support-tier1" agent exists with max_steps 3
    And a router rule fans out mycelium.event.handoff to "support-queue"
    And a human operator is subscribed to "support-queue"

  Scenario: Self-serve fix succeeds
    When Karl describes "sync stopped yesterday"
    Then tier1 asks at most 2 clarifying questions
    And tier1 proposes a fix link
    And on Karl saying "yes!" the conversation is marked resolved

  Scenario: Immediate human handoff
    When Karl says "human"
    Then a mycelium.event.handoff message is published with the conversation id
    And the conversation memory is tagged "handoff=requested"
    And tier1 stops sending further replies in that conversation

  Scenario: Auto-escalation after max_steps
    Given the conversation has 3 prior turns with no resolution
    When Karl sends a 4th message
    Then the bot escalates with a handoff event
    And the bot replies "passing you to a human, sit tight"

  Scenario: Human reply reaches the user
    Given a handoff is active for conversation CID
    When the human POSTs an assistant message to /conversations/CID/messages
    Then telegram-out delivers that text to Karl
    And the message role is preserved as "assistant"

  Scenario: Returning customer is recognised
    Given Karl had a sync issue 14 days ago
    When Karl opens a new conversation with the same chat_id
    Then the bot greets him with "welcome back"
    And references the prior issue category
