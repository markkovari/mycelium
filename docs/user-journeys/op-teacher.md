# 17 — Teacher

**Persona**: Mr Nagy. 30 students, 9th grade math.
**Goal**: Per-student tutor that never gives the answer. Track engagement. Spot stuck students.
**Primitives**: One agent template + per-student instance via KV + parental visibility.

## Setup

```bash
# Template agent
curl -X POST http://localhost:8080/agents -H 'host: localhost' -d '{
  "id":"tutor-math-template","name":"Math Tutor",
  "system_prompt":"Socratic style. Never reveal final answer. Adjust to student level. Use diagrams in text.",
  "model":"qwen2.5:0.5b","tools":["calculator","graph-tool"],"max_steps":5
}'

# Per-student clones (script-driven)
for student in 1001..1030; do
  curl -X POST http://localhost:8080/agents -H 'host: localhost' -d "{
    \"id\":\"tutor-math-$student\",
    \"name\":\"$student tutor\",
    \"system_prompt\":\"[clone from template] Student level: 9. Strengths: ... Weaknesses: ...\",
    \"model\":\"qwen2.5:0.5b\",\"tools\":[\"calculator\",\"graph-tool\"],\"max_steps\":5
  }"
done
```

## Branches

### A. Adaptive difficulty
- Memory `student/<id>/level` tracks current. If 3 problems in a row solved fast → bump level. Slow → drop.

### B. Stuck detection
- If a student spends >15 min on one problem without progress, ping teacher's dashboard

### C. Parental visibility
- Per-conversation auth: parent can `GET /conversations?student_id=<id>`
- Stripped of teacher-only metadata

### D. Weekly progress report
- cron: each student gets a Sunday digest of topics covered, accuracy, time spent

### E. Anti-cheating
- Hash + timestamp every problem set per student
- Teacher dashboard shows "Tom answered Q3 in 4 seconds" — flag

### F. Homework batch grading
- Upload a CSV of 30 student responses → batch dispatcher scores them
- Per-student feedback delivered via Telegram

### G. Group lesson mode
- All 30 see a shared agent in a class group; questions get short answers
- Per-student deep tutoring stays in DM

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| Tutor gives answer | system_prompt drift | strong negative example in prompt |
| Wrong difficulty | level not updated | nightly recalibration cron |
| Parent sees teacher notes | leaky GET | strict field allowlist |
| Cheating undetected | naive flagging | timing + answer similarity + plagiarism tool |

## Connects to

- [End: student](end-student.md)
- [Op: batch jobs](op-batch-jobs.md) — homework grading
- [End: scheduled digest](end-scheduled-digest.md) — weekly reports
