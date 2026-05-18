Feature: Long-running researcher with tool loop

  As a researcher
  I want an agent that plans, fetches, cites, and writes
  So that I get a citable report from one ask

  Background:
    Given a "researcher" agent exists with max_steps 30
    And tools web-search, arxiv-fetch, pdf-extract, cite-format are available
    And a cost cap is configured

  Scenario: Multi-step research run
    When Dora asks "compare CRDT performance in 4 papers"
    Then the agent issues multiple mycelium.tool.call messages
    And the final reply contains at least 4 citations

  Scenario: Budget cap stops the run
    Given cost_usd_max is $1.00
    When the running cost reaches $1.00
    Then the agent stops and emits a "stopped on budget" notice
    And the partial report is delivered

  Scenario: Resume across sessions
    Given Dora left the conversation mid-research yesterday
    When Dora returns and says "continue"
    Then the agent reads the prior conversation
    And resumes from the last unfinished sub-question

  Scenario: Live progress events
    Given a research is in flight
    When each tool call completes
    Then a mycelium.event.research.progress is published
    And it carries step number, tool id, status

  Scenario: Cross-check claims
    Given the agent has issued a claim
    When the claim was sourced from a single tool result
    Then a second independent search is performed
    And disagreements are flagged in the final report

  Scenario: Hand-off to writer
    Given researcher's final report is ready
    When the chain rule applies
    Then writer-agent receives the report and polishes prose
    And only the polished version is shown to Dora
