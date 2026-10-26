//! The UI frontend: renders a projection of [`CombatState`] and submits
//! player intents. It never mutates combat state (COMBAT.md Ruling 4).

use alchemy_combat::{
    Atom, Card, CombatOutcome, CombatPlayer, CombatState, EnemyId, Instruction, Intent,
    LOCAL_PLAYER, Unit, UnitRef, cast_needs_target,
};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button, observe};

use crate::AppState;
use crate::driver::{Combat, IntentQueue};

#[derive(Component)]
struct MenuRoot;

#[derive(Component)]
struct CombatUiRoot;

/// The hand index awaiting a chosen target, if any. UI policy: click a
/// target-needing card to arm it, then click a living enemy to submit.
/// Cleared whenever combat state changes (a play may shift hand indices).
#[derive(Resource, Default)]
struct Targeting(Option<usize>);

/// Base/hover/press colors for a button so the styling system can restore it.
#[derive(Component, Clone, Copy)]
struct ButtonTint {
    normal: Color,
    hovered: Color,
    pressed: Color,
}

impl ButtonTint {
    fn new(normal: Color) -> Self {
        Self {
            normal,
            hovered: normal.lighter(0.15),
            pressed: normal.lighter(0.35),
        }
    }
}

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Targeting>()
            .add_systems(OnEnter(AppState::Menu), setup_menu)
            .add_systems(OnExit(AppState::Menu), despawn_all::<MenuRoot>)
            .add_systems(OnExit(AppState::Combat), despawn_all::<CombatUiRoot>)
            .add_systems(Update, (refresh_combat_ui, update_button_tints));
    }
}

fn despawn_all<T: Component>(mut commands: Commands, query: Query<Entity, With<T>>) {
    for entity in &query {
        commands.entity(entity).despawn();
    }
}

// ---------------------------------------------------------------------------
// Menu
// ---------------------------------------------------------------------------

fn setup_menu(mut commands: Commands) {
    commands
        .spawn((
            MenuRoot,
            Node {
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: px(30),
                ..default()
            },
            BackgroundColor(Color::srgb(0.08, 0.08, 0.12)),
        ))
        .with_children(|parent| {
            parent.spawn(label("ALCHEMY JOURNEY", 48.0, Color::srgb(0.9, 0.8, 0.5)));
            parent.spawn((
                text_button("Start Combat", 180.0, 60.0, Color::srgb(0.2, 0.45, 0.3)),
                observe(|_: On<Activate>, mut next: ResMut<NextState<AppState>>| {
                    next.set(AppState::Combat);
                }),
            ));
        });
}

// ---------------------------------------------------------------------------
// Combat projection
// ---------------------------------------------------------------------------

/// Rebuilds the whole combat UI whenever the combat state or the targeting
/// mode changes. Cheap at this scale (≤ 10 cards) and keeps the projection
/// trivially in sync — the UI is rebuildable from state at any time, by
/// construction.
fn refresh_combat_ui(
    mut commands: Commands,
    combat: Option<Res<Combat>>,
    mut targeting: ResMut<Targeting>,
    roots: Query<Entity, With<CombatUiRoot>>,
) {
    let Some(combat) = combat else { return };
    if !combat.is_changed() && !targeting.is_changed() {
        return;
    }
    if combat.is_changed() {
        // A state change may shift hand indices; drop any pending target arm.
        targeting.bypass_change_detection().0 = None;
    }
    for entity in &roots {
        commands.entity(entity).despawn();
    }
    spawn_combat_ui(&mut commands, &combat.0, targeting.0);
}

fn spawn_combat_ui(commands: &mut Commands, state: &CombatState, targeting: Option<usize>) {
    let local = state
        .player(LOCAL_PLAYER)
        .expect("single-player combat always has the local player");
    commands
        .spawn((
            CombatUiRoot,
            Node {
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::SpaceBetween,
                padding: UiRect::all(px(12)),
                row_gap: px(10),
                ..default()
            },
            BackgroundColor(Color::srgb(0.08, 0.08, 0.12)),
        ))
        .with_children(|parent| {
            spawn_top_bar(parent, state, local);
            spawn_arena(parent, state, targeting);
            spawn_bottom_bar(parent, local, state.outcome, targeting);
        });
}

fn spawn_top_bar(parent: &mut ChildSpawnerCommands, state: &CombatState, local: &CombatPlayer) {
    parent
        .spawn(Node {
            width: percent(100),
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|bar| {
            let status = match state.outcome {
                Some(CombatOutcome::Victory) => "Victory!",
                None if local.has_ended_turn => "Turn ended",
                None => "Your move",
            };
            bar.spawn(label(
                format!("Turn {} — {status}   Energy: {}", state.turn, local.energy),
                20.0,
                Color::srgb(0.9, 0.9, 0.9),
            ));
            bar.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(16),
                ..default()
            })
            .with_children(|right| {
                right.spawn(label(
                    format!(
                        "Deck {}  Discard {}  Consumed {}",
                        local.deck.len(),
                        local.discard.len(),
                        local.consumed.len()
                    ),
                    16.0,
                    Color::srgb(0.7, 0.7, 0.7),
                ));
                right.spawn((
                    text_button("Quit to Menu", 140.0, 40.0, Color::srgb(0.45, 0.2, 0.2)),
                    observe(|_: On<Activate>, mut next: ResMut<NextState<AppState>>| {
                        next.set(AppState::Menu);
                    }),
                ));
            });
        });
}

fn spawn_arena(parent: &mut ChildSpawnerCommands, state: &CombatState, targeting: Option<usize>) {
    parent
        .spawn(Node {
            width: percent(100),
            flex_grow: 1.0,
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            padding: UiRect::horizontal(px(60)),
            ..default()
        })
        .with_children(|arena| {
            arena
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: px(20),
                    ..default()
                })
                .with_children(|side| {
                    for player in &state.players {
                        spawn_unit_panel(side, &player.unit, Color::srgb(0.2, 0.35, 0.6), None);
                    }
                });
            if state.outcome.is_some() {
                arena.spawn(label(
                    match state.outcome {
                        Some(CombatOutcome::Victory) => "VICTORY",
                        None => unreachable!(),
                    },
                    56.0,
                    Color::srgb(0.9, 0.8, 0.4),
                ));
            }
            arena
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: px(20),
                    ..default()
                })
                .with_children(|side| {
                    for (index, enemy) in state.enemies.iter().enumerate() {
                        // While a card is armed, living enemies become
                        // target buttons (UI policy; the core validates).
                        let target = targeting
                            .map(|hand_index| (hand_index, UnitRef::Enemy(EnemyId(index as u32))));
                        spawn_unit_panel(
                            side,
                            enemy,
                            Color::srgb(0.6, 0.25, 0.25),
                            enemy.is_alive().then_some(target).flatten(),
                        );
                    }
                });
        });
}

/// A unit panel; when `target` is `Some((hand_index, unit))` the panel is a
/// button submitting that play with the chosen target.
fn spawn_unit_panel(
    parent: &mut ChildSpawnerCommands,
    unit: &Unit,
    color: Color,
    target: Option<(usize, UnitRef)>,
) {
    let panel_color = if unit.is_alive() {
        color
    } else {
        Color::srgb(0.25, 0.25, 0.28)
    };
    let mut entity = parent.spawn((
        Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: px(8),
            ..default()
        },
        // Targetable units light up gold.
        BorderColor::all(if target.is_some() {
            Color::srgb(0.95, 0.85, 0.4)
        } else {
            Color::srgb(0.85, 0.85, 0.85)
        }),
    ));
    entity.with_children(|panel| {
        let mut box_entity = panel.spawn((
            Node {
                width: px(160),
                height: px(110),
                border: UiRect::all(px(3)),
                ..default()
            },
            BackgroundColor(panel_color),
            BorderColor::all(Color::srgb(0.85, 0.85, 0.85)),
        ));
        if let Some((hand_index, unit_ref)) = target {
            box_entity.insert((Button, ButtonTint::new(panel_color), Hovered::default()));
            box_entity.observe(
                move |_: On<Activate>,
                      mut queue: ResMut<IntentQueue>,
                      mut targeting: ResMut<Targeting>| {
                    targeting.0 = None;
                    queue.submit(Intent::PlayCard {
                        hand_index,
                        target: Some(unit_ref),
                    });
                },
            );
        }
        panel.spawn(label(unit.name.clone(), 18.0, Color::srgb(0.9, 0.9, 0.9)));
        panel.spawn(label(
            if unit.is_alive() {
                format!("HP {}/{}", unit.hp, unit.max_hp)
            } else {
                "DEAD".to_string()
            },
            16.0,
            Color::srgb(0.6, 0.9, 0.6),
        ));
        panel.spawn(label(
            format!("Block {}", unit.block),
            16.0,
            Color::srgb(0.6, 0.7, 0.9),
        ));
    });
}

fn spawn_bottom_bar(
    parent: &mut ChildSpawnerCommands,
    local: &CombatPlayer,
    outcome: Option<CombatOutcome>,
    targeting: Option<usize>,
) {
    parent
        .spawn(Node {
            width: percent(100),
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::FlexEnd,
            ..default()
        })
        .with_children(|bar| {
            // Hand.
            bar.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: px(8),
                ..default()
            })
            .with_children(|hand| {
                for (index, card) in local.hand.iter().enumerate() {
                    let needs_target = cast_needs_target(card);
                    let armed = targeting == Some(index);
                    let mut entity = hand.spawn(card_button(card, armed));
                    // No observers once the combat is over: the terminal
                    // state rejects everything anyway (RejectReason::CombatOver).
                    if outcome.is_none() {
                        // Intent submission: multiplayer sends this over the
                        // intent channel to the host; the UI knows no
                        // difference.
                        entity.observe(
                            move |_: On<Activate>,
                                  mut queue: ResMut<IntentQueue>,
                                  mut targeting: ResMut<Targeting>| {
                                if needs_target {
                                    // Arm (or disarm) the card; the next
                                    // enemy click submits the play.
                                    targeting.0 = if armed { None } else { Some(index) };
                                } else {
                                    queue.submit(Intent::PlayCard {
                                        hand_index: index,
                                        target: None,
                                    });
                                }
                            },
                        );
                    }
                }
            });
            // End turn.
            let mut entity = bar.spawn(text_button(
                "End Turn",
                130.0,
                56.0,
                Color::srgb(0.5, 0.4, 0.15),
            ));
            if outcome.is_none() {
                entity.observe(|_: On<Activate>, mut queue: ResMut<IntentQueue>| {
                    queue.submit(Intent::EndTurn);
                });
            }
        });
}

// ---------------------------------------------------------------------------
// Widget helpers
// ---------------------------------------------------------------------------

/// Compact effect-program readout with the cursor bracketed — the paused
/// evaluation is visible on the card face (DESIGN.md: predictability).
fn effect_line(card: &Card) -> String {
    let mut parts = Vec::new();
    for (i, instruction) in card.def.effects.iter().enumerate() {
        let text = match instruction {
            Instruction::Eval(n) => format!("all({n})"),
            Instruction::BindTarget(_) => "bind".to_string(),
            Instruction::ClearBind => "clear".to_string(),
            Instruction::Apply(Atom::Damage(n)) => format!("{n}dmg"),
            Instruction::Apply(Atom::Block(n)) => format!("{n}blk"),
        };
        if i == card.cursor {
            parts.push(format!("[{text}]"));
        } else {
            parts.push(text);
        }
    }
    parts.join(" ")
}

fn label(text: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(text.into()),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
    )
}

fn text_button(text: &str, width: f32, height: f32, color: Color) -> impl Bundle {
    (
        Button,
        ButtonTint::new(color),
        Hovered::default(),
        Node {
            width: px(width),
            height: px(height),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::all(px(6)),
            ..default()
        },
        BorderColor::all(Color::BLACK),
        BackgroundColor(color),
        children![label(text, 16.0, Color::srgb(0.95, 0.95, 0.95))],
    )
}

fn card_button(card: &Card, armed: bool) -> impl Bundle {
    let base = if armed {
        Color::srgb(0.75, 0.6, 0.2)
    } else {
        Color::srgb(0.55, 0.4, 0.15)
    };
    (
        Button,
        ButtonTint::new(base),
        Hovered::default(),
        Node {
            width: px(110),
            height: px(150),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::SpaceBetween,
            padding: UiRect::all(px(8)),
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::all(px(8)),
            ..default()
        },
        BorderColor::all(Color::srgb(0.85, 0.75, 0.4)),
        BackgroundColor(base),
        children![
            label(
                format!("Cost {}", card.def.cost),
                13.0,
                Color::srgb(0.9, 0.9, 0.7)
            ),
            label(card.def.name.clone(), 15.0, Color::srgb(0.95, 0.95, 0.95)),
            label(effect_line(card), 11.0, Color::srgb(0.85, 0.8, 0.6)),
        ],
    )
}

fn update_button_tints(
    mut buttons: Query<(&Hovered, Has<Pressed>, &ButtonTint, &mut BackgroundColor), With<Button>>,
) {
    for (hovered, pressed, tint, mut color) in &mut buttons {
        *color = if pressed {
            tint.pressed
        } else if hovered.get() {
            tint.hovered
        } else {
            tint.normal
        }
        .into();
    }
}
