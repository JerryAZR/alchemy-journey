//! Bevy UI layer: renders [`CombatState`] and forwards player input to it.
//! Contains no game rules — all mutations go through `combat::CombatState`.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button, observe};

use crate::AppState;
use crate::combat::{self, CombatState};

/// Bevy resource wrapper so `CombatState` itself stays Bevy-free.
#[derive(Resource)]
pub struct Combat(pub CombatState);

#[derive(Component)]
struct MenuRoot;

#[derive(Component)]
struct CombatUiRoot;

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
        app.add_systems(OnEnter(AppState::Menu), setup_menu)
            .add_systems(OnExit(AppState::Menu), despawn_all::<MenuRoot>)
            .add_systems(OnEnter(AppState::Combat), setup_combat)
            .add_systems(OnExit(AppState::Combat), teardown_combat)
            .add_systems(
                Update,
                (refresh_combat_ui, update_button_tints, print_new_log_entries),
            );
    }
}

fn despawn_all<T: Component>(mut commands: Commands, query: Query<Entity, With<T>>) {
    for entity in &query {
        commands.entity(entity).despawn();
    }
}

fn seed_from_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
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
// Combat
// ---------------------------------------------------------------------------

fn setup_combat(mut commands: Commands) {
    commands.insert_resource(Combat(combat::starter_combat(seed_from_time())));
}

fn teardown_combat(mut commands: Commands, roots: Query<Entity, With<CombatUiRoot>>) {
    commands.remove_resource::<Combat>();
    for entity in &roots {
        commands.entity(entity).despawn();
    }
}

/// Rebuilds the whole combat UI whenever the combat state changes.
/// Cheap at this scale (≤ 11 cards) and keeps rendering trivially in sync.
fn refresh_combat_ui(
    mut commands: Commands,
    combat: Option<Res<Combat>>,
    roots: Query<Entity, With<CombatUiRoot>>,
) {
    let Some(combat) = combat else { return };
    if !combat.is_changed() {
        return;
    }
    for entity in &roots {
        commands.entity(entity).despawn();
    }
    spawn_combat_ui(&mut commands, &combat.0);
}

/// Prints combat log entries to the terminal as they appear.
fn print_new_log_entries(combat: Option<Res<Combat>>, mut printed: Local<usize>) {
    let Some(combat) = combat else {
        *printed = 0;
        return;
    };
    if combat.is_added() {
        *printed = 0;
    }
    let log = &combat.0.log;
    if *printed > log.len() {
        *printed = 0; // New combat reusing the resource.
    }
    for entry in &log[*printed..] {
        println!("[combat] {entry}");
    }
    *printed = log.len();
}

fn spawn_combat_ui(commands: &mut Commands, state: &CombatState) {
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
            spawn_top_bar(parent, state);
            spawn_arena(parent, state);
            spawn_bottom_bar(parent, state);
        });
}

fn spawn_top_bar(parent: &mut ChildSpawnerCommands, state: &CombatState) {
    parent
        .spawn(Node {
            width: percent(100),
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|bar| {
            let phase = match state.phase {
                combat::Phase::PlayerTurn => "Player Turn",
                combat::Phase::EnemyTurn => "Enemy Turn",
            };
            bar.spawn(label(
                format!("Turn {} — {phase}   Energy: {}", state.turn, state.energy),
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
                        state.deck.len(),
                        state.discard.len(),
                        state.consumed.len()
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

fn spawn_arena(parent: &mut ChildSpawnerCommands, state: &CombatState) {
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
            spawn_unit_panel(arena, &state.player, Color::srgb(0.2, 0.35, 0.6));
            spawn_unit_panel(arena, &state.enemy, Color::srgb(0.6, 0.25, 0.25));
        });
}

fn spawn_unit_panel(parent: &mut ChildSpawnerCommands, unit: &combat::Unit, color: Color) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: px(8),
            ..default()
        })
        .with_children(|panel| {
            panel.spawn((
                Node {
                    width: px(160),
                    height: px(110),
                    border: UiRect::all(px(3)),
                    ..default()
                },
                BackgroundColor(color),
                BorderColor::all(Color::srgb(0.85, 0.85, 0.85)),
            ));
            panel.spawn(label(unit.name.clone(), 18.0, Color::srgb(0.9, 0.9, 0.9)));
            panel.spawn(label(
                format!("HP {}/{}", unit.hp, unit.max_hp),
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

fn spawn_bottom_bar(parent: &mut ChildSpawnerCommands, state: &CombatState) {
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
                for (index, card) in state.hand.iter().enumerate() {
                    hand.spawn((
                        card_button(&card.def.name, card.def.cost),
                        observe(
                            move |_: On<Activate>, mut combat: ResMut<Combat>| {
                                let _ = combat.0.play_card(index);
                            },
                        ),
                    ));
                }
            });
            // End turn.
            bar.spawn((
                text_button("End Turn", 130.0, 56.0, Color::srgb(0.5, 0.4, 0.15)),
                observe(|_: On<Activate>, mut combat: ResMut<Combat>| {
                    combat.0.end_player_turn();
                    combat.0.run_enemy_turn();
                }),
            ));
        });
}

// ---------------------------------------------------------------------------
// Widget helpers
// ---------------------------------------------------------------------------

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

fn card_button(name: &str, cost: u32) -> impl Bundle {
    (
        Button,
        ButtonTint::new(Color::srgb(0.55, 0.4, 0.15)),
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
        BackgroundColor(Color::srgb(0.55, 0.4, 0.15)),
        children![
            label(format!("Cost {cost}"), 13.0, Color::srgb(0.9, 0.9, 0.7)),
            label(name, 15.0, Color::srgb(0.95, 0.95, 0.95)),
            label("Potion", 12.0, Color::srgb(0.8, 0.7, 0.5)),
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
