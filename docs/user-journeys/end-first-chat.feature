Feature: First-time Telegram chatter

  As a brand-new Telegram user added to a mycelium bot
  I want to send my first message and get a clear, useful reply
  So that I trust the bot enough to keep using it

  Background:
    Given a configured Telegram bot is wired to mycelium-telegram-in
    And an onboarding agent "welcome-bot" exists with a friendly system_prompt
    And telegram-out is RUNNING with telegram.bot_token configured

  Scenario: First /start triggers a welcome
    When Mara sends "/start" from chat_id 1001
    Then telegram-gateway acknowledges with HTTP 200 within 1 second
    And telegram-out sends a welcome message to chat 1001 within 4 seconds
    And the welcome message contains at least 2 suggested prompts

  Scenario: First free-text message gets a real reply
    Given Mara has already received the welcome
    When Mara sends "help me plan dinner"
    Then a mycelium.task.submit message is published with agent_id "welcome-bot"
    And within 8 seconds telegram-out posts a reply to chat 1001
    And the reply text is non-empty

  Scenario: Re-engagement after silence
    Given Mara's last message was more than 7 days ago
    When the daily cron job runs
    Then a re-engagement message is sent to chat 1001
    And only one re-engagement per 14-day window is sent

  Scenario: /reset starts a new conversation
    Given Mara has an active conversation
    When Mara sends "/reset"
    Then a new conversation row is created in mycelium-conversations
    And the previous conversation is preserved in the KV for audit
    And the bot confirms with "Fresh start — what's up?"

  Scenario: Bot fails gracefully when the LLM endpoint is unreachable
    Given the agent's configured llm.endpoint is unreachable
    When Mara sends any user message
    Then the bot replies with an apology and "try again in a minute"
    And no Telegram error code is surfaced to Mara
