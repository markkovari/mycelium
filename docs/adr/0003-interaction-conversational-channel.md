# ADR-0003: Conversational Channel Interaction Mode

**Status:** Accepted

## Context

Users need a real-time, back-and-forth interface with the agent. This covers the primary use case: send a message, get a reply, maintain conversation history. Entry points include a CLI REPL and a Telegram bot, with more channels possible (WhatsApp, Discord, etc.). The agent must be reachable from any entry point without per-channel logic in the agent itself.

## Decision

All conversational input flows through a **channel abstraction** over NATS:

- **Inbound**: any channel gateway normalises an incoming message and publishes to `mycelium.channel.in` as a `ChannelMessage`.
- **Outbound**: the agent publishes replies to `mycelium.channel.<transport>.out.<session_id>`. The originating gateway subscribes to its own subject and delivers the reply to the user.
- **Session bridge** manages pairing between a session ID (CLI) and a Telegram chat ID via `mycelium.pair.*` subjects.
- **Executor** picks up `ChannelMessage` → submits a task → drives the agent step loop → publishes result back to the channel out subject.

```
User (CLI / Telegram)
  │  send
  ▼
Gateway / Telegram-gateway  ──publish──▶  mycelium.channel.in
                                              │
                                          Executor
                                              │ agent steps
                                          Agent + LLM
                                              │ reply
                                          mycelium.channel.<transport>.out.<session>
                                              │
  User  ◀──deliver──  Gateway / Telegram-gateway
```

Channel gateways are independent components. Adding a new channel (e.g., Discord) requires only a new gateway component — no changes to executor, agent, or router.

## Consequences

- **+** Agent is fully channel-agnostic; new entry points require no agent changes.
- **+** Multiple channels can serve the same session (CLI + Telegram simultaneously).
- **+** NATS request/reply gives the CLI sub-second round-trip in local dev.
- **-** Session pairing adds a handshake step before first use on new channels.
- **-** Out-of-order delivery possible if NATS core (non-JetStream) subjects are used for high-volume channels; JetStream consumers should be used for channels requiring ordering guarantees.
