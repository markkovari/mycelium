Feature: Raspberry Pi edge deployment

  As a homelab operator on a Raspberry Pi 5
  I want the full mycelium stack to run offline with a local LLM
  So that my home automation works without cloud

  Background:
    Given the Pi has nats-server, wash host, and Ollama installed
    And the OCI cache is pre-populated from CI
    And the local LLM endpoint is reachable at 127.0.0.1:11434

  Scenario: Stack starts offline
    Given the Pi has no internet
    When wash host is started with --oci-cache-dir set
    Then all 10 workloads reach WORKLOAD_STATE_RUNNING
    And no outbound network call is made

  Scenario: Agent uses local LLM
    Given an agent "home" has endpoint 127.0.0.1:11434
    When a step is triggered
    Then the LLM call goes to 127.0.0.1:11434
    And no external endpoint is contacted

  Scenario: Zigbee tool integration
    Given a home-tools component is deployed with the zigbee tool
    When the user says "turn off the bedroom light"
    Then a mycelium.tool.call with tool_id "light.off" is published
    And the zigbee bridge actuates the device within 2 seconds

  Scenario: Local Whisper STT
    Given a transcriber workload uses local whisper.cpp HTTP
    When a voice memo arrives
    Then the transcription happens locally
    And no audio is sent off-device

  Scenario: SD-card wear protection
    Given nats-server is configured with --store-dir on a USB SSD
    When the stack runs continuously for 30 days
    Then no writes occur on the SD card from JetStream

  Scenario: Resource caps respected
    Given the Pi has 8GB RAM
    And pool_size is 1 across all workloads
    When 5 concurrent requests arrive
    Then memory usage stays under 6GB
    And requests queue rather than fail

  Scenario: Failover to spare Pi
    Given two Pis are configured as primary and warm spare
    When the primary becomes unreachable
    Then within 30 seconds the spare takes over
    And clients reconnect transparently
