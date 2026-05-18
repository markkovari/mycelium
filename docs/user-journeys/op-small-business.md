# 18 — Small business owner

**Persona**: Hanna. Hair salon, 4 staff, ~120 bookings/week.
**Goal**: Bot answers FAQ, books appointments, sends reminders, asks for review post-visit.
**Primitives**: Booking tool + calendar memory + cron reminders + review prompt.

## Architecture

```
Telegram customer → bot (book / FAQ / cancel)
                      │
                      ├── booking tool writes to memory/salon/calendar
                      ├── cron reminder T-24h, T-2h
                      └── post-visit cron at T+2h → review prompt
Staff Telegram group → bot pings new bookings
```

## Happy path

```
Customer: "haircut tomorrow 3pm with Eszter?"
Bot:      "Eszter is free 3pm. Confirm? Y/N"
Customer: "Y"
Bot:      "Booked. Reminder will come tomorrow 1pm. Address: ..."

T-24h    Bot: "Reminder: haircut tomorrow 3pm. Reply 'cancel' if needed."
T+2h     Bot: "Hope it went well! 1-5 stars, and any feedback?"
```

## Branches

### A. Multi-staff calendar
- `memory/salon/calendar/<staff>/<date>/<slot>` holds bookings
- Booking tool checks availability across staff

### B. Walk-in mode
- `/walkin` from customer → bot picks earliest free slot today across staff

### C. Cancellation handling
- "cancel" → finds the booking, frees the slot, sends a "got it" + survey for cancel reason
- If late cancel (<2h) → flag in memory, optional fee message

### D. Loyalty
- `memory/customer/<id>/visits` counter
- 10th visit → coupon code

### E. Multi-channel review push
- Configured review platforms (Google, Yelp) — bot sends the link after a positive (>=4 star) reply

### F. Staff handoff
- "speak to Eszter" → handoff event routed to staff group only when she's working

### G. No-show policy
- If T+10min after appointment and no check-in: post a "still coming?" + after 30 min, mark no-show

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| Double-booking | slot lookup race | atomic check+write on memory key |
| Reminder spam | duplicate cron | idempotency key on (booking_id, fire_ts) |
| Bot books wrong staff | weak intent parsing | confirmation step before write |
| Reviews not landing | wrong region's platform | configurable platform per locale |

## Connects to

- [End: scheduled digest](end-scheduled-digest.md) — weekly business summary
- [Op: first bot](op-first-bot.md) — Hanna's bot was set up by the wizard
- [Op: cron scheduler](op-cron-scheduler.md) — reminders engine
