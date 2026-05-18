Feature: Recipe helper

  As a hungry user with a pantry of leftovers
  I want a fast recipe suggestion that respects my diet
  So that I can cook in 20 minutes without scrolling food sites

  Background:
    Given a "recipe-chef" agent exists with tools: [web-search]
    And the tool-runner workload is RUNNING

  Scenario: Quick suggestion from ingredient list
    When Janos sends "eggs rice kimchi 20 min" to the bot
    Then within 8 seconds the bot proposes a recipe by name
    And the proposal asks "want the full recipe?"

  Scenario: Confirmation expands to full steps with citation
    Given Janos has been offered "kimchi fried rice"
    When Janos replies "yes"
    Then the bot replies with at least 3 numbered steps
    And the reply contains at least one source URL
    And the reply mentions estimated time ≤ 20 minutes

  Scenario: Allergy from memory is honoured
    Given memory/janos/diet contains {"avoid": ["peanut"]}
    When Janos asks "snack ideas"
    Then no suggested item contains "peanut"

  Scenario: Conversation-aware substitution
    Given the bot suggested a recipe containing rice
    When Janos says "I don't have rice"
    Then the bot proposes a rice substitution in the same conversation
    And the bot does not ask Janos to re-list ingredients

  Scenario: Save items to shopping list
    Given the bot suggested a recipe missing 3 ingredients
    When Janos says "/save"
    Then memory/janos/shopping is updated with those 3 items
    And the bot confirms with the saved count

  Scenario Outline: Different cuisines
    When Janos asks "<query>"
    Then the bot returns a recipe of "<cuisine>" within 10 seconds

    Examples:
      | query                          | cuisine  |
      | something Hungarian for dinner | hungarian|
      | quick Thai noodle bowl         | thai     |
      | vegan Italian pasta            | italian  |
