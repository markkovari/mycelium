Feature: Privacy controls for the end user

  As an EU end user
  I want to see, export, and delete my data
  So that I have meaningful control under GDPR-style expectations

  Background:
    Given a "/privacy" command is wired in the bot
    And a mycelium-privacy-audit KV bucket exists

  Scenario: List what's stored
    When Sara sends "/privacy"
    Then the bot replies with counts per data class (conversations, messages, memory, journal)
    And the counts match the actual KV contents

  Scenario: Export everything
    When Sara sends "/privacy export"
    Then an export job is dispatched
    And within 15 minutes a download link is sent to Sara
    And the export contains all data classes she has

  Scenario: Per-class deletion
    When Sara sends "/privacy delete journal"
    Then every memory/<sara>/journal/* KV entry is removed
    And a tombstone is written to mycelium-privacy-audit
    And other data classes (conversations, memory non-journal) are untouched

  Scenario: Hard delete (right to be forgotten)
    When Sara sends "/privacy forget-me"
    Then all KV buckets that reference her chat_id are purged
    And only an immutable audit stub remains
    And the stub contains {chat_id, deleted_at, reason}

  Scenario: Consent toggle
    When Sara toggles "voice transcripts" off
    Then subsequent voice memos are transcribed but not persisted
    And existing voice transcripts are not auto-deleted

  Scenario: Retention sweep
    Given a TTL of 90 days for journal entries
    When the nightly retention sweep runs
    Then entries older than 90 days are removed
    And each removal is logged with reason "retention-sweep"

  Scenario: Local-only mode forbids external network
    Given the deployment is configured in local-only mode
    When any component attempts wasi:http/outgoing-handler to a non-local host
    Then the call is rejected
    And the event is logged
