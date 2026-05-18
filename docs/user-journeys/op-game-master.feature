Feature: Game master with persistent NPCs

  As a GM running a long campaign
  I want each NPC to be its own agent with shared lore
  So that interactions feel real and stay consistent

  Background:
    Given multiple NPC agents exist (e.g. "npc-elara", "npc-graff")
    And the lore-lookup tool is available
    And memory/campaign/* holds canonical lore

  Scenario: Player DMs a specific NPC
    When a player sends "/npc elara" followed by a question
    Then the response is in elara's voice
    And it references lore from memory/campaign/<topic>

  Scenario: GM updates lore
    When the GM sends "/lore add seal forged by Threll"
    Then memory/campaign/seal is updated
    And subsequent NPC answers reflect that lore

  Scenario: Cross-NPC consistency
    Given memory/campaign/seal mentions "echoes in dreams"
    When player asks both npc-elara and npc-graff about the seal
    Then both replies are consistent on the "echoes in dreams" detail

  Scenario: Per-NPC relations with each player
    Given memory/npc-elara/player-tom/relations = "owes a favour"
    When Tom asks Elara for help
    Then the reply tone reflects that favour

  Scenario Outline: Dice tool integration
    When a player attempts a contested action
    Then a dice tool call is published with modifier "<mod>"
    And the NPC narrates the outcome consistent with total "<total>"

    Examples:
      | mod | total |
      | +2  | 16    |
      | -1  | 7     |
      | +4  | 19    |

  Scenario: Session quiet hours
    Given mycelium.event.session.starts has been published
    When any player DMs an NPC
    Then the NPC remains silent
    And the bot replies "the gods are speaking, return later"

  Scenario: NPC archive
    When the GM sends "/npc-archive elara"
    Then memory/agent/npc-elara is tagged dormant
    And subsequent DMs to npc-elara say "they are gone"
