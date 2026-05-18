Feature: Student tutor

  As a student doing homework
  I want a tutor that shows me steps, not answers
  So that I actually learn the material

  Background:
    Given a "tutor-math" agent exists with a Socratic system_prompt
    And a "tutor-bio" agent exists with citation requirements
    And tool-runner is RUNNING with "calculator" available

  Scenario: Step-by-step problem walkthrough
    When Lila asks "what is 7/12 + 5/8"
    Then the first reply asks Lila for the LCM of 12 and 8
    And the reply does NOT contain "35/24"

  Scenario: Bait-and-switch attempt is resisted
    Given the bot is mid-walkthrough
    When Lila says "just give me the answer"
    Then the bot offers a hint instead of the answer
    And the bot does not reveal the final numerical answer

  Scenario: Switching subjects via /agent
    Given Lila has been chatting with tutor-math
    When Lila sends "/agent tutor-bio"
    Then the next message routes to tutor-bio
    And the math context remains saved for resume

  Scenario: Calculator tool is used silently
    Given the walkthrough requires an arithmetic intermediate value
    When the agent needs that value
    Then a mycelium.tool.call is published for "calculator"
    And the tool result lands on mycelium.tool.result within 2 seconds
    And the agent uses the result without revealing the final answer

  Scenario: Resume progress next session
    Given memory/lila/math/topic was "fractions" yesterday
    When Lila starts a new conversation today
    Then the bot opens with "still on fractions?"

  Scenario: Parent reads the log
    When the parent calls GET /conversations?agent_id=tutor-math
    Then the response lists every conversation Lila had with tutor-math
