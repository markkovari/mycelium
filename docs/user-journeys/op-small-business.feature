Feature: Small business booking bot

  As a small business owner
  I want a bot that books, reminds, and asks for reviews
  So that I capture more bookings and reduce no-shows

  Background:
    Given a "salon-bot" agent exists with booking + calendar tools
    And memory/salon/calendar holds existing bookings
    And cron-scheduler is RUNNING

  Scenario: Booking happy path
    When the customer asks "haircut tomorrow 3pm with Eszter"
    Then the bot confirms availability
    And on customer confirmation a booking is written to memory/salon/calendar/eszter/<date>/15:00
    And the customer receives a confirmation message

  Scenario: Reminder T-24h fires once
    Given a booking exists for tomorrow 15:00
    When the T-24h cron runs at 15:00 today
    Then exactly one reminder is sent to the customer

  Scenario: Late cancellation handling
    Given a booking is in 1 hour
    When the customer sends "cancel"
    Then the slot is freed
    And the booking is tagged "late-cancel" in memory
    And the customer is informed about the cancellation policy

  Scenario: Loyalty milestone
    Given memory/customer/<id>/visits = 9
    When the 10th booking is completed
    Then a coupon-code message is sent to the customer
    And the visits counter increments to 10

  Scenario: Post-visit review prompt
    Given a booking ended 2 hours ago
    When the T+2h cron fires
    Then the customer is asked for a 1-5 star rating
    On rating >= 4
    Then the bot sends a link to the configured review platform

  Scenario: No-show detection
    Given an appointment was due 40 minutes ago
    And no check-in occurred
    When the no-show cron fires
    Then the booking is marked no-show
    And the staff group receives a notice

  Scenario: Walk-in earliest slot
    When the customer sends "/walkin haircut"
    Then the bot scans all staff calendars for today
    And replies with the earliest free slot across staff
