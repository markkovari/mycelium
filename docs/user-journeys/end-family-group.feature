Feature: Family group assistant

  As a member of a family Telegram group
  I want a bot that's helpful but doesn't dominate the chat
  So that the bot is welcome long-term

  Background:
    Given a "family" agent exists with mention-only system_prompt
    And telegram-out is RUNNING

  Scenario: Bot answers only when mentioned
    When Pal sends "anyone seen the keys" without mentioning the bot
    Then no Telegram message is sent by telegram-out for that chat

  Scenario: Bot answers when mentioned
    When Anna sends "@fambot grocery list?"
    Then a reply is posted to the group within 6 seconds
    And the reply addresses Anna by name

  Scenario: Per-member memory
    Given memory/family/anna/grocery-history contains items
    When Anna asks "what did I buy last week"
    Then the bot lists items only from anna's history
    And the bot does not leak Pal's history

  Scenario: Quiet hours suppress fires
    Given memory/family/quiet-hours is "22:00-07:00"
    When a scheduled cron tries to send a message at 02:00
    Then no Telegram message is delivered
    And the fire is logged as "suppressed-quiet-hours"

  Scenario Outline: Permission tiers
    Given memory/family/roles maps "<member>" to "<tier>"
    When "<member>" asks "spend 50€ on flowers"
    Then the bot responds with "<reaction>"

    Examples:
      | member | tier  | reaction                              |
      | anna   | adult | confirms and books the spend          |
      | lili   | child | refuses with "ask a parent" message    |

  Scenario: Private DM stays private
    Given Anna privately DMs the bot about a surprise
    When Anna later opens the group
    Then no group message references the surprise
    And memory/family/anna/private/surprise holds the note
