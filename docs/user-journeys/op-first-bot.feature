Feature: Non-developer first-bot setup

  As a non-technical bot owner
  I want a guided setup that hides REST and KV details
  So that I can launch a working Telegram bot without reading code

  Background:
    Given a `just wizard` command exists and is documented
    And the user has a Telegram bot token from BotFather

  Scenario: Happy-path wizard run
    When Klara runs the wizard with valid inputs
    Then a new agent is registered via POST /agents
    And telegram.bot_token is stored via wasi:config
    And the Telegram setWebhook call succeeds
    And the wizard prints a success summary with one test instruction

  Scenario: Token validation up front
    When Klara enters an invalid token
    Then the wizard rejects the token before writing anything
    And no agent is created
    And the wizard re-prompts

  Scenario: Template selection
    When Klara picks the "class FAQ" template
    Then the resulting agent has the template's system_prompt
    And the tools list matches the template
    And the agent's name is editable in a follow-up step

  Scenario: Onboarding nudge after silence
    Given the wizard completed 24 hours ago
    And no Telegram traffic has hit the bot
    When the daily check runs
    Then Klara receives a private DM with troubleshooting tips

  Scenario: Tool toggle without rebuild
    Given the bot is running
    When Klara adds the "calendar" tool via the wizard
    Then the agent's KV record is updated
    And no WASM rebuild or workload restart is required

  Scenario: Pause the bot
    When Klara runs `just bot-pause yogabot`
    Then the bot's KV flag "paused" is true
    And the next user message receives an "offline today" reply
