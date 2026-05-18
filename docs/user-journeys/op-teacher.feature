Feature: Teacher running per-student tutors

  As a teacher with 30 students
  I want each student to have their own tutor agent
  So that pace and content adapt without me manually intervening

  Background:
    Given a "tutor-math-template" agent exists
    And per-student clones tutor-math-1001 through tutor-math-1030 exist
    And cron-scheduler is RUNNING

  Scenario: Per-student conversation isolation
    Given student 1003 has chatted with tutor-math-1003
    When student 1005 queries tutor-math-1005
    Then student 1003's conversation is not visible
    And memory keys are namespaced by student id

  Scenario: Adaptive difficulty bumps level
    Given student/1003/level is 1
    When student 1003 solves 3 problems quickly
    Then student/1003/level becomes 2
    And the next problem is harder

  Scenario: Stuck detection alert
    Given student 1004 has been on one problem for 20 minutes
    When the stuck-check cron runs
    Then a mycelium.event.student.stuck message is published
    And the teacher dashboard receives a Telegram notice

  Scenario: Parental access is restricted
    Given a parent's chat_id is linked to student 1006
    When the parent queries GET /conversations?student_id=1006
    Then teacher-only annotations are not returned
    And the parent sees timestamps and topics but not raw teacher notes

  Scenario: Weekly progress digest
    When the Sunday 09:00 cron fires
    Then each student receives a digest of last 7 days
    And each parent receives a summary version

  Scenario: Batch homework grading
    When the teacher uploads a CSV of 30 student answers
    Then a batch is dispatched with 30 items
    And within 5 minutes per-student feedback messages are delivered

  Scenario: Anti-cheat flagging
    Given student 1009 answers Q3 in under 5 seconds
    When the flagging job processes the response
    Then mycelium.event.anti-cheat.flag is published
    And the teacher dashboard highlights this answer for review
