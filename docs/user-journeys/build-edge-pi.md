# 21 — Edge Pi operator

**Persona**: Tibor. Raspberry Pi 5, 8GB RAM. Wants home automation + privacy.
**Goal**: Full mycelium stack offline. Local LLM. No cloud.
**Primitives**: ARM64 builds + local Ollama + pre-warmed OCI cache + small models.

## Architecture

```
Pi 5
├── nats-server (file storage)
├── wash host (--allow-insecure-registries)
├── ollama (qwen2.5:0.5b or phi3:mini, ARM-quantised)
├── OCI cache populated from CI ghcr.io once
└── 10 workloads (mycelium-*)
```

No external traffic at runtime.

## Setup

```bash
# On Pi
mkdir -p /var/mycelium/{nats,oci-cache}
# Pre-pull on dev laptop, rsync to Pi
rsync -avz ~/oci-cache/ tibor@pi.local:/var/mycelium/oci-cache/

# On Pi
nats-server -js -sd /var/mycelium/nats -p 4222 &
wash host \
  --scheduler-nats-url nats://127.0.0.1:4222 \
  --data-nats-url nats://127.0.0.1:4222 \
  --http-addr 0.0.0.0:8080 \
  --oci-cache-dir /var/mycelium/oci-cache \
  --allow-insecure-registries &

# Configure LLM to local ollama
nats kv put mycelium-agent-config agent/home '{
  "id":"home","name":"Home","system_prompt":"Voice-control hub. Be terse.",
  "model":"qwen2.5:0.5b","tools":["zigbee","weather-local"],"max_steps":3,
  "endpoint":"http://127.0.0.1:11434/v1/chat/completions"
}'
```

## Branches

### A. Resource caps
- `pool_size=1` everywhere on Pi
- Pi 5 4GB → drop event-logger workload, write events to a file via custom component

### B. Zigbee / MQTT bridge
- Custom `home-tools` component imports `wasi:sockets` and bridges zigbee2mqtt
- Tool calls: `light.on`, `door.lock`, `temp.read`

### C. Voice control via local Whisper
- whisper.cpp running as native service on Pi
- Custom transcriber workload calls its local HTTP endpoint

### D. Periodic check-in
- cron: hourly sensor snapshot stored to KV
- Daily digest: which devices acted, anomalies

### E. Failover
- Two Pis, one primary, one warm spare
- NATS clustering between them; KV replicated
- Heartbeat via cron — primary updates `system/primary-heartbeat` every 15s; spare watches

### F. SD-card wear-out protection
- KV file storage moved to USB SSD
- nats-server `--store-dir` points there

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| Pi OOM | concurrent step + big model | drop pool_size to 1, smaller model |
| Slow LLM (>30s) | quantised model on small core | smaller model OR cloud fallback off-toggle |
| Stale OCI cache | new component version | rsync from laptop on update day |
| Pi died, data gone | SD corruption | SSD + rsync to NAS nightly |

## Connects to

- [End: privacy](end-privacy.md) — local-only mode
- [End: family group](end-family-group.md) — home users
- [End: voice memo](end-voice-memo.md) — local STT
