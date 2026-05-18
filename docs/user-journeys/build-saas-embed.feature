Feature: SaaS embed of mycelium API

  As a SaaS founder
  I want to use mycelium as the AI backend behind my own product
  So that I ship AI features without exposing mycelium to my customers

  Background:
    Given mycelium-api is running and reachable from Liz's backend
    And Liz's backend enforces auth and tenant prefixing

  Scenario: Tenant-scoped agent creation
    When Liz's backend POSTs an agent with id "acme/draft"
    Then the KV key "agent/acme/draft" is created
    And the bare "draft" agent is NOT created

  Scenario: Tenant isolation
    Given tenants "acme" and "globex" both have an agent "draft"
    When acme's user triggers a step
    Then only acme/draft is invoked
    And no log line cross-references globex

  Scenario Outline: Per-tenant model choice
    Given tenant "<tenant>" prefers model "<model>"
    When that tenant runs a step
    Then the LLM endpoint and model used are "<model>"

    Examples:
      | tenant | model                |
      | acme   | qwen2.5:0.5b         |
      | globex | gpt-4o-mini          |
      | bigco  | claude-sonnet-latest |

  Scenario: Quota enforcement
    Given tenant "acme" has a 10k/month plan and has used 9999
    When the 10000th request is attempted via Liz's gateway
    Then Liz's gateway responds 200 (allowed)
    When the 10001st request is attempted
    Then Liz's gateway responds 402 before reaching mycelium

  Scenario: Webhook fan-out
    Given tenant "acme" has a webhook URL configured
    When a mycelium.event.* matching acme's prefix fires
    Then Liz's gateway POSTs the event to acme's webhook
    And retries up to 3 times on failure

  Scenario: Self-hosted appliance
    Given Liz ships a docker-compose pinned at mycelium v0.x
    When a customer brings it up on-prem
    Then GET /health returns 200
    And no outbound network is required if a local LLM is configured
