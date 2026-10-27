//! Combat content: the starter combat. Static data assembled from the
//! definition types; no mechanics live here.

use crate::effect::{Atom, Instruction, PotionDef, TargetSpec};
use crate::state::{Card, CardDef, EnemyDef, OnPlay, PlayerRunState};
use crate::wire::Event;
use crate::{CombatState, LOCAL_PLAYER};

/// The step-2 combat (DESIGN.md): 4× stepping strike (6-7-8) and 4× stepping
/// defend (5-6-7) — reusable-toggle potions with hand-written instruction
/// vectors — plus two true potions authored via `PotionDef` lowering. One
/// inert dummy, 30 HP.
pub fn starter_combat(seed: u64) -> (CombatState, Vec<Event>) {
    let strike = |id: u32| {
        Card::new(
            id,
            CardDef {
                name: "Strike Draught".to_string(),
                on_play: OnPlay::Reusable,
                effects: vec![
                    Instruction::apply(Atom::Damage(6), 1),
                    Instruction::apply(Atom::Damage(7), 1),
                    Instruction::apply(Atom::Damage(8), 1),
                ],
                unwrap: 0,
            },
        )
    };
    let defend = |id: u32| {
        Card::new(
            id,
            CardDef {
                name: "Ward Draught".to_string(),
                on_play: OnPlay::Reusable,
                effects: vec![
                    Instruction::bind(TargetSpec::Caster, 0),
                    Instruction::apply(Atom::Block(5), 1),
                    Instruction::apply(Atom::Block(6), 1),
                    Instruction::apply(Atom::Block(7), 1),
                ],
                unwrap: 0,
            },
        )
    };
    let potion = |id: u32, name: &str, def: PotionDef| {
        Card::new(
            id,
            CardDef {
                name: name.to_string(),
                on_play: OnPlay::Consume,
                effects: def.lower(),
                unwrap: 0,
            },
        )
    };
    let deck: Vec<Card> = (0..4)
        .map(strike)
        .chain((4..8).map(defend))
        .chain([potion(
            8,
            "Firebomb Flask",
            PotionDef {
                target: TargetSpec::Chosen,
                effects: vec![Atom::Damage(20)],
            },
        )])
        .chain([potion(
            9,
            "Stoneskin Flask",
            PotionDef {
                target: TargetSpec::Caster,
                effects: vec![Atom::Block(15)],
            },
        )])
        .collect();
    let player = PlayerRunState {
        id: LOCAL_PLAYER,
        name: "Alchemist".to_string(),
        hp: 80,
        max_hp: 80,
        deck,
    };
    let enemies = [EnemyDef {
        name: "Training Dummy".to_string(),
        hp: 30,
    }];
    CombatState::start(&[player], &enemies, seed)
}
