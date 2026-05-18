Feature: Power-user agent switching

  As a power user with many specialised agents
  I want to switch agents instantly inside the same chat
  So that I match the right tool to the right turn

  Background:
    Given agents "researcher", "draft", "code", "coach" exist
    And the bot accepts /agent <id> commands

  Scenario: Switch to a different agent
    Given the active agent for the user is "researcher"
    When the user sends "/agent draft"
    Then memory/<user>/active-agent becomes "draft"
    And subsequent messages route to "draft"

  Scenario: Per-agent conversation continuity
    Given the user has separate conversations with "researcher" and "draft"
    When the user switches between them
    Then each agent's conversation history is preserved independently

  Scenario: Share context across agents
    Given the user is in "draft" mode
    When the user sends "/share-context researcher"
    Then the last 20 messages from researcher's conversation are appended to draft's context
    And the next draft reply references that context

  Scenario: Hotkey aliases
    Given memory/<user>/aliases maps "r"="researcher" and "d"="draft"
    When the user sends "/r"
    Then the active agent becomes "researcher"

  Scenario: Cost-aware override
    When the user prefixes "/cheap" before a question
    Then the next step uses the cheap model regardless of agent default

  Scenario: Chained agents
    When the user sends "/chain research-then-draft what's the EU AI Act"
    Then researcher answers first
    And draft post-processes researcher's answer
    And only the final draft reply is visible to the user
