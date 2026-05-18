Feature: Photo sharing

  As a user who finds it easier to show than to type
  I want to send photos and get useful, grounded replies
  So that the bot helps with visual context

  Background:
    Given an "image-helper" agent exists with a multimodal model
    And an image-handler workload is RUNNING

  Scenario: Photo with caption
    When Reka sends a photo with caption "is this good?"
    Then within 12 seconds the bot replies referencing the image content
    And the reply is non-generic

  Scenario: OCR-only mode
    When the caption is "/ocr"
    Then the bot extracts text from the image only
    And the reply contains no opinion or commentary

  Scenario: Pantry inventory
    Given the caption is "/pantry"
    When the photo shows visible food items
    Then memory/<user>/pantry is updated with parsed items
    And the bot confirms the count of items added

  Scenario: Privacy opt-out
    Given the user's profile has do_not_store_images = true
    When the user sends a photo
    Then the photo bytes are not persisted in mycelium-images KV
    And the inference result IS still returned to the user

  Scenario: Album of photos
    When the user sends 3 photos in one message
    Then a batch is created internally
    And a single coalesced reply summarises all 3

  Scenario: Large image is downscaled
    Given the photo is 4096x3072
    When the image-handler processes it
    Then the LLM call uses a max edge of 1280
    And the round-trip stays under 8 seconds
