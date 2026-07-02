//! Screen-space HUD: hull, speed, credits, cargo manifest, mining progress,
//! dock trading panel, chat + event feed, death overlay, minimap and the
//! connection overlay. Client-only; reads simulation state and emits
//! [`SendChat`]/[`SendAction`] messages for the network layer.

use std::collections::HashMap;

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;

use crate::components::{
    BodyRadius, CentralStar, ChatTyping, Credits, DeathFlash, Feed, GameState, Gate, Hull,
    LocalShip, MiningRig, Planet, PlayerShip, RenderSet, ResourceDeposit, SendAction, SendChat,
    ShipStats, SimPosition, Velocity,
};
use crate::config::GameConfig;
use crate::logic::cargo::Cargo;
use crate::logic::economy::{upgrade_cost, UpgradeKind, Upgrades};
use crate::plugins::economy::docked_planet;
use crate::protocol::PlayerAction;
use crate::resource_types::ResourceType;

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ChatDraft>()
            .add_systems(Startup, (spawn_hud, spawn_minimap))
            .add_systems(
                Update,
                (
                    chat_input,
                    dock_keybinds,
                    (
                        update_hull_bar,
                        update_status_text,
                        update_cargo_text,
                        update_mining_bar,
                        update_dock_panel,
                        update_feed_panel,
                        update_death_overlay,
                        ensure_minimap_dots,
                        update_minimap,
                    ),
                )
                    .chain()
                    .in_set(RenderSet::Decor),
            )
            .add_systems(OnEnter(GameState::Playing), hide_connecting_overlay);
    }
}

#[derive(Component)]
struct HullBarFill;
#[derive(Component)]
struct StatusText;
#[derive(Component)]
struct CargoText;
#[derive(Component)]
struct MiningBar;
#[derive(Component)]
struct MiningBarFill;
#[derive(Component)]
struct MiningLabel;
#[derive(Component)]
struct ConnectingOverlay;
#[derive(Component)]
struct DockPanel;
#[derive(Component)]
struct DockText;
#[derive(Component)]
struct FeedText;
#[derive(Component)]
struct ChatPrompt;
#[derive(Component)]
struct DeathOverlay;

/// The chat line being typed.
#[derive(Resource, Default)]
struct ChatDraft(String);

const PANEL_BG: Color = Color::srgba(0.05, 0.08, 0.12, 0.75);
const BAR_BG: Color = Color::srgba(0.15, 0.18, 0.22, 0.9);

fn small_text(size: f32, color: Color) -> (TextFont, TextColor) {
    (
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
    )
}

fn spawn_hud(mut commands: Commands) {
    // --- Top-left: hull + speed + credits ---
    commands
        .spawn((
            Name::new("HUD Status Panel"),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(12.0),
                left: Val::Px(12.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                padding: UiRect::all(Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(PANEL_BG),
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new("HULL"),
                small_text(12.0, Color::srgb(0.7, 0.8, 0.9)),
            ));
            panel
                .spawn((
                    Node {
                        width: Val::Px(200.0),
                        height: Val::Px(12.0),
                        ..default()
                    },
                    BackgroundColor(BAR_BG),
                ))
                .with_children(|bar| {
                    bar.spawn((
                        HullBarFill,
                        Node {
                            width: Val::Percent(100.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.3, 0.9, 0.4)),
                    ));
                });
            panel.spawn((
                StatusText,
                Text::new("SPD 0"),
                small_text(12.0, Color::srgb(0.7, 0.8, 0.9)),
            ));
        });

    // --- Bottom-left: cargo manifest ---
    commands
        .spawn((
            Name::new("HUD Cargo Panel"),
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(12.0),
                left: Val::Px(12.0),
                padding: UiRect::all(Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(PANEL_BG),
        ))
        .with_children(|panel| {
            panel.spawn((
                CargoText,
                Text::new("CARGO 0/0"),
                small_text(13.0, Color::srgb(0.85, 0.9, 0.95)),
            ));
        });

    // --- Bottom-center: mining progress ---
    commands
        .spawn((
            Name::new("HUD Mining Bar"),
            MiningBar,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(48.0),
                left: Val::Percent(50.0),
                margin: UiRect::left(Val::Px(-120.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                align_items: AlignItems::Center,
                ..default()
            },
            Visibility::Hidden,
        ))
        .with_children(|root| {
            root.spawn((
                MiningLabel,
                Text::new("MINING"),
                small_text(12.0, Color::srgb(0.9, 0.85, 0.5)),
            ));
            root.spawn((
                Node {
                    width: Val::Px(240.0),
                    height: Val::Px(8.0),
                    ..default()
                },
                BackgroundColor(BAR_BG),
            ))
            .with_children(|bar| {
                bar.spawn((
                    MiningBarFill,
                    Node {
                        width: Val::Percent(0.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.95, 0.8, 0.3)),
                ));
            });
        });

    // --- Right-center: dock/trade panel (visible while docked) ---
    commands
        .spawn((
            Name::new("HUD Dock Panel"),
            DockPanel,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(28.0),
                right: Val::Px(12.0),
                padding: UiRect::all(Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.06, 0.1, 0.08, 0.85)),
            Visibility::Hidden,
        ))
        .with_children(|panel| {
            panel.spawn((
                DockText,
                Text::new(""),
                small_text(12.0, Color::srgb(0.8, 0.95, 0.85)),
            ));
        });

    // --- Bottom-right: chat + event feed ---
    commands
        .spawn((
            Name::new("HUD Feed Panel"),
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(12.0),
                right: Val::Px(12.0),
                width: Val::Px(360.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                padding: UiRect::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(PANEL_BG),
        ))
        .with_children(|panel| {
            panel.spawn((
                FeedText,
                Text::new(""),
                small_text(11.0, Color::srgb(0.75, 0.82, 0.9)),
            ));
            panel.spawn((
                ChatPrompt,
                Text::new("[Enter] to chat"),
                small_text(11.0, Color::srgb(0.5, 0.6, 0.7)),
            ));
        });

    // --- Center: death overlay ---
    commands.spawn((
        Name::new("HUD Death Overlay"),
        DeathOverlay,
        Text::new("SHIP DESTROYED - CARGO LOST"),
        small_text(30.0, Color::srgb(1.0, 0.35, 0.3)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(40.0),
            left: Val::Percent(50.0),
            margin: UiRect::left(Val::Px(-240.0)),
            ..default()
        },
        Visibility::Hidden,
    ));

    // --- Center: connection overlay (shown until the server welcomes us) ---
    commands.spawn((
        Name::new("HUD Connecting Overlay"),
        ConnectingOverlay,
        Text::new("CONNECTING TO SERVER..."),
        small_text(28.0, Color::srgb(0.9, 0.9, 1.0)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(45.0),
            left: Val::Percent(50.0),
            margin: UiRect::left(Val::Px(-200.0)),
            ..default()
        },
    ));
}

fn hide_connecting_overlay(
    mut commands: Commands,
    overlays: Query<Entity, With<ConnectingOverlay>>,
) {
    for entity in &overlays {
        commands.entity(entity).despawn();
    }
}

// ---------------------------------------------------------------------------
// Input: chat line + dock keybinds
// ---------------------------------------------------------------------------

fn chat_input(
    mut typing: ResMut<ChatTyping>,
    mut draft: ResMut<ChatDraft>,
    mut keys: MessageReader<KeyboardInput>,
    mut send: MessageWriter<SendChat>,
) {
    for event in keys.read() {
        if !event.state.is_pressed() {
            continue;
        }
        match &event.logical_key {
            Key::Enter => {
                if typing.0 {
                    let text = draft.0.trim().to_string();
                    if !text.is_empty() {
                        send.write(SendChat(text));
                    }
                    draft.0.clear();
                    typing.0 = false;
                } else {
                    typing.0 = true;
                }
            }
            Key::Escape if typing.0 => {
                draft.0.clear();
                typing.0 = false;
            }
            Key::Backspace if typing.0 => {
                draft.0.pop();
            }
            Key::Space if typing.0 => {
                draft.0.push(' ');
            }
            Key::Character(input) if typing.0 && draft.0.len() < 160 => {
                draft.0.push_str(input.as_str());
            }
            _ => {}
        }
    }
}

/// While docked: 1-4 sell a resource stack, 5-9 buy the next upgrade tier.
fn dock_keybinds(
    keys: Res<ButtonInput<KeyCode>>,
    typing: Res<ChatTyping>,
    config: Res<GameConfig>,
    ships: Query<&SimPosition, With<LocalShip>>,
    planets: Query<(&Planet, &SimPosition, &BodyRadius)>,
    mut actions: MessageWriter<SendAction>,
) {
    if typing.0 {
        return;
    }
    let Ok(ship_pos) = ships.single() else {
        return;
    };
    if docked_planet(ship_pos.current, config.ship.docking_range, planets.iter()).is_none() {
        return;
    }

    const SELL_KEYS: [KeyCode; 4] = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
    ];
    for (key, kind) in SELL_KEYS.iter().zip(ResourceType::ALL) {
        if keys.just_pressed(*key) {
            actions.write(SendAction(PlayerAction::Sell(kind)));
        }
    }
    const BUY_KEYS: [KeyCode; 5] = [
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ];
    for (key, kind) in BUY_KEYS.iter().zip(UpgradeKind::ALL) {
        if keys.just_pressed(*key) {
            actions.write(SendAction(PlayerAction::BuyUpgrade(kind)));
        }
    }
}

// ---------------------------------------------------------------------------
// Panels
// ---------------------------------------------------------------------------

fn update_hull_bar(
    ships: Query<(&Hull, &ShipStats), With<LocalShip>>,
    mut fills: Query<(&mut Node, &mut BackgroundColor), With<HullBarFill>>,
) {
    let Ok((hull, stats)) = ships.single() else {
        return;
    };
    let fraction = (hull.0 / stats.max_hull).clamp(0.0, 1.0);
    for (mut node, mut color) in &mut fills {
        node.width = Val::Percent(fraction * 100.0);
        color.0 = Color::srgb(0.9 - 0.6 * fraction, 0.2 + 0.7 * fraction, 0.3);
    }
}

fn update_status_text(
    ships: Query<(&Velocity, &Credits, &Upgrades), With<LocalShip>>,
    mut texts: Query<&mut Text, With<StatusText>>,
) {
    let Ok((velocity, credits, upgrades)) = ships.single() else {
        return;
    };
    for mut text in &mut texts {
        text.0 = format!(
            "SPD {:>4.0}   CR {:>6}   LVL {}",
            velocity.0.length(),
            credits.0,
            upgrades.total_tiers()
        );
    }
}

fn update_cargo_text(
    ships: Query<&Cargo, With<LocalShip>>,
    mut texts: Query<&mut Text, With<CargoText>>,
) {
    let Ok(cargo) = ships.single() else {
        return;
    };
    let mut lines = format!("CARGO {}/{}", cargo.total(), cargo.capacity());
    if cargo.is_full() {
        lines.push_str("  [FULL]");
    }
    for kind in ResourceType::ALL {
        lines.push_str(&format!("\n{:<8} {:>3}", kind.name(), cargo.amount(kind)));
    }
    for mut text in &mut texts {
        text.0.clone_from(&lines);
    }
}

fn update_mining_bar(
    ships: Query<&MiningRig, With<LocalShip>>,
    deposits: Query<&ResourceDeposit>,
    mut bars: Query<&mut Visibility, With<MiningBar>>,
    mut fills: Query<&mut Node, With<MiningBarFill>>,
    mut labels: Query<&mut Text, With<MiningLabel>>,
) {
    let Ok(rig) = ships.single() else {
        return;
    };
    let target_deposit = rig.target.and_then(|entity| deposits.get(entity).ok());

    for mut visibility in &mut bars {
        *visibility = if target_deposit.is_some() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    let Some(deposit) = target_deposit else {
        return;
    };
    for mut node in &mut fills {
        node.width = Val::Percent(rig.progress.clamp(0.0, 1.0) * 100.0);
    }
    for mut label in &mut labels {
        label.0 = format!(
            "MINING {} ({:.0} left)",
            deposit.kind.name(),
            deposit.amount
        );
    }
}

fn update_dock_panel(
    config: Res<GameConfig>,
    ships: Query<(&SimPosition, &Cargo, &Credits, &Upgrades), With<LocalShip>>,
    planets: Query<(&Planet, &SimPosition, &BodyRadius)>,
    mut panels: Query<&mut Visibility, With<DockPanel>>,
    mut texts: Query<&mut Text, With<DockText>>,
) {
    let Ok((ship_pos, cargo, credits, upgrades)) = ships.single() else {
        return;
    };
    let docked = docked_planet(ship_pos.current, config.ship.docking_range, planets.iter())
        .and_then(|planet| {
            config
                .systems
                .get(planet.system_index)
                .and_then(|system| system.planets.get(planet.config_index))
        });

    for mut visibility in &mut panels {
        *visibility = if docked.is_some() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    let Some(planet_cfg) = docked else {
        return;
    };

    let mut lines = format!("DOCKED: {}\n\nSELL", planet_cfg.name);
    for (i, kind) in ResourceType::ALL.iter().enumerate() {
        let price = planet_cfg
            .market
            .iter()
            .find(|entry| entry.kind == *kind)
            .map(|entry| entry.price)
            .unwrap_or(0);
        lines.push_str(&format!(
            "\n[{}] {:<8} {:>2} cr  (have {})",
            i + 1,
            kind.name(),
            price,
            cargo.amount(*kind)
        ));
    }
    lines.push_str(&format!("\n\nUPGRADES (CR {})", credits.0));
    for (i, kind) in UpgradeKind::ALL.iter().enumerate() {
        let tier = upgrades.tier(*kind);
        match upgrade_cost(*kind, tier) {
            Some(cost) => lines.push_str(&format!(
                "\n[{}] {:<12} T{} -> {:>5} cr",
                i + 5,
                kind.name(),
                tier,
                cost
            )),
            None => lines.push_str(&format!("\n[{}] {:<12} MAXED", i + 5, kind.name())),
        }
    }
    for mut text in &mut texts {
        text.0.clone_from(&lines);
    }
}

fn update_feed_panel(
    feed: Res<Feed>,
    typing: Res<ChatTyping>,
    draft: Res<ChatDraft>,
    mut feeds: Query<&mut Text, (With<FeedText>, Without<ChatPrompt>)>,
    mut prompts: Query<&mut Text, With<ChatPrompt>>,
) {
    for mut text in &mut feeds {
        let joined: Vec<&str> = feed.0.iter().map(String::as_str).collect();
        text.0 = joined.join("\n");
    }
    for mut prompt in &mut prompts {
        prompt.0 = if typing.0 {
            format!("> {}_", draft.0)
        } else {
            "[Enter] to chat".into()
        };
    }
}

fn update_death_overlay(
    time: Res<Time>,
    mut flash: ResMut<DeathFlash>,
    mut overlays: Query<&mut Visibility, With<DeathOverlay>>,
) {
    flash.0 = (flash.0 - time.delta_secs()).max(0.0);
    for mut visibility in &mut overlays {
        *visibility = if flash.0 > 0.0 {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

// ---------------------------------------------------------------------------
// Minimap (shows the system the local ship is currently in)
// ---------------------------------------------------------------------------

const MINIMAP_SIZE: f32 = 160.0;

#[derive(Component)]
struct MinimapDot {
    target: Entity,
    dot_size: f32,
}

#[derive(Resource)]
struct MinimapBox(Entity);

fn spawn_minimap(mut commands: Commands) {
    let map_box = commands
        .spawn((
            Name::new("HUD Minimap"),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(12.0),
                right: Val::Px(12.0),
                width: Val::Px(MINIMAP_SIZE),
                height: Val::Px(MINIMAP_SIZE),
                ..default()
            },
            BackgroundColor(PANEL_BG),
        ))
        .id();
    commands.insert_resource(MinimapBox(map_box));

    // Resource color legend below the map.
    commands
        .spawn((
            Name::new("HUD Minimap Legend"),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(12.0 + MINIMAP_SIZE + 6.0),
                right: Val::Px(12.0),
                width: Val::Px(MINIMAP_SIZE),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(2.0),
                padding: UiRect::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(PANEL_BG),
        ))
        .with_children(|legend| {
            for kind in ResourceType::ALL {
                legend
                    .spawn(Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(6.0),
                        ..default()
                    })
                    .with_children(|row| {
                        row.spawn((
                            Node {
                                width: Val::Px(8.0),
                                height: Val::Px(8.0),
                                ..default()
                            },
                            BackgroundColor(kind.color()),
                        ));
                        row.spawn((
                            Text::new(kind.name()),
                            small_text(10.0, Color::srgb(0.7, 0.75, 0.8)),
                        ));
                    });
            }
        });
}

/// Keep one dot per tracked world entity (ships, planets, stars, gates come
/// and go at runtime).
fn ensure_minimap_dots(
    mut commands: Commands,
    config: Res<GameConfig>,
    map_box: Option<Res<MinimapBox>>,
    tracked: Query<
        (
            Entity,
            Option<&Planet>,
            Has<CentralStar>,
            Has<Gate>,
            Has<PlayerShip>,
            Has<LocalShip>,
        ),
        Or<(
            With<Planet>,
            With<CentralStar>,
            With<PlayerShip>,
            With<Gate>,
        )>,
    >,
    dots: Query<(Entity, &MinimapDot)>,
) {
    let Some(map_box) = map_box else {
        return;
    };
    let mut dots_by_target: HashMap<Entity, Entity> = HashMap::new();
    for (dot_entity, dot) in &dots {
        dots_by_target.insert(dot.target, dot_entity);
    }

    for (entity, planet, is_star, is_gate, is_ship, is_local) in &tracked {
        if dots_by_target.remove(&entity).is_some() {
            continue;
        }
        let (size, color) = if is_star {
            (7.0, Color::srgb(1.0, 0.9, 0.5))
        } else if is_gate {
            (5.0, Color::srgb(0.4, 0.9, 1.0))
        } else if let Some(planet) = planet {
            let Some(cfg) = config
                .systems
                .get(planet.system_index)
                .and_then(|system| system.planets.get(planet.config_index))
            else {
                continue;
            };
            (4.0, Color::srgb(cfg.color.0, cfg.color.1, cfg.color.2))
        } else if is_ship {
            if is_local {
                (3.0, Color::WHITE)
            } else {
                (3.0, Color::srgb(1.0, 0.75, 0.4))
            }
        } else {
            continue;
        };
        commands.spawn((
            MinimapDot {
                target: entity,
                dot_size: size,
            },
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(size),
                height: Val::Px(size),
                ..default()
            },
            BackgroundColor(color),
            ChildOf(map_box.0),
        ));
    }

    for dot_entity in dots_by_target.into_values() {
        commands.entity(dot_entity).despawn();
    }
}

fn update_minimap(
    config: Res<GameConfig>,
    local: Query<&SimPosition, With<LocalShip>>,
    positions: Query<&SimPosition>,
    mut dots: Query<(&MinimapDot, &mut Node, &mut Visibility)>,
) {
    // The map shows whichever system the local ship is nearest to.
    let ship_pos = local.single().map(|pos| pos.current).unwrap_or(Vec2::ZERO);
    let Some(system) = config.systems.iter().min_by(|a, b| {
        let da = ship_pos.distance(Vec2::new(a.center.0, a.center.1));
        let db = ship_pos.distance(Vec2::new(b.center.0, b.center.1));
        da.total_cmp(&db)
    }) else {
        return;
    };
    let center = Vec2::new(system.center.0, system.center.1);
    let extent = system
        .planets
        .iter()
        .map(|planet| planet.orbit_radius)
        .chain(system.belts.iter().map(|belt| belt.outer_radius))
        .chain(
            system
                .gates
                .iter()
                .map(|gate| Vec2::new(gate.position.0, gate.position.1).length()),
        )
        .fold(1000.0_f32, f32::max)
        * 1.1;

    for (dot, mut node, mut visibility) in &mut dots {
        let Ok(pos) = positions.get(dot.target) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        let relative = pos.current - center;
        // Things in other systems don't belong on this map.
        if relative.length() > extent * 1.05 {
            *visibility = Visibility::Hidden;
            continue;
        }
        *visibility = Visibility::Inherited;
        let normalized = (relative / extent + Vec2::ONE) / 2.0;
        let range = MINIMAP_SIZE - dot.dot_size;
        node.left = Val::Px((normalized.x.clamp(0.0, 1.0) * range).round());
        node.top = Val::Px(((1.0 - normalized.y.clamp(0.0, 1.0)) * range).round());
    }
}
