Feature: Solo developer local iteration

  As a solo developer
  I want a fast local inner loop on agent + prompt changes
  So that I can ship iterations to ghcr in minutes

  Background:
    Given the mycelium stack is running locally on wash host v2
    And the OCI registry is reachable at localhost:5001
    And NATS JetStream is up at localhost:4222
    And all 10 workloads are in WORKLOAD_STATE_RUNNING

  Scenario: Register a new agent via REST
    When I POST a new AgentConfig {id: "draft", model: "qwen2.5:0.5b"} to /agents
    Then the response status is 201
    And "agent/draft" exists in the mycelium-agent-config KV bucket
    And GET /agents/draft returns the same config

  Scenario: Iterate prompt without redeploy
    Given an agent "draft" already exists
    When I PUT a new system_prompt for "draft"
    And I publish to mycelium.task.submit with agent_id="draft"
    Then the next mycelium.step.result reflects the updated prompt
    And no workload restart occurred

  Scenario Outline: Swap LLM model via KV
    Given an agent "draft" exists with model "<old>"
    When I overwrite the KV value with model "<new>"
    And I trigger a step
    Then the step calls the "<new>" model endpoint
    And the previous model is not invoked

    Examples:
      | old           | new                          |
      | qwen2.5:0.5b  | llama3:8b                    |
      | llama3:8b     | claude-3-5-sonnet-latest     |
      | qwen2.5:0.5b  | gpt-4o-mini                  |

  Scenario: Hot-rebuild a single component
    When I edit components/gateway/src/lib.rs
    And I run "just build-one gateway"
    And I push the new image to localhost:5001
    And I redeploy with PULL_POLICY=IMAGE_PULL_POLICY_ALWAYS
    Then the new gateway version is RUNNING within 10 seconds
    And HTTP requests succeed against the updated code

  Scenario: Ship to ghcr.io via CI
    When I push the branch to origin
    Then the release.yaml workflow builds 13 component images
    And each image is tagged with the commit SHA
    And each image is also tagged with "latest" on main

  Scenario: Auto-task submission is documented as blocked
    Given gateway is the HTTP exporter in mycelium-api
    When I POST a user message to /conversations/:id/messages
    Then the message is appended successfully
    But no mycelium.task.submit message is published automatically
    And the docs instruct me to publish via "nats pub mycelium.task.submit"

  Scenario: Stack survives NATS restart
    Given several agents and conversations exist
    When I kill the nats-server process
    And restart it pointing at the same --store_dir
    Then GET /agents still lists every previously registered agent
    And GET /conversations/:id still returns the prior history
