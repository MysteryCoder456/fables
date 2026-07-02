//! Screen-space HUD: hull, speed, cargo manifest, mining progress and the
//! pause overlay. Client-only; reads simulation state, never writes it.

use bevy::prelude::*;

use crate::components::{
    CentralStar, GameState, Hull, MiningRig, Planet, PlayerShip, RenderSet, ResourceDeposit,
    ShipStats, SimPosition, Velocity,
};
use crate::config::GameConfig;
use crate::logic::cargo::Cargo;
use crate::resource_types::ResourceType;

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_hud)
            // PostStartup: minimap dots reference world entities spawned in Startup.
            .add_systems(PostStartup, spawn_minimap)
            .add_systems(
                Update,
                (
                    update_hull_bar,
                    update_speed_text,
                    update_cargo_text,
                    update_mining_bar,
                    update_minimap,
                )
                    .in_set(RenderSet::Decor),
            )
            .add_systems(OnEnter(GameState::Paused), show_pause_overlay)
            .add_systems(OnExit(GameState::Paused), hide_pause_overlay);
    }
}

#[derive(Component)]
struct HullBarFill;
#[derive(Component)]
struct SpeedText;
#[derive(Component)]
struct CargoText;
#[derive(Component)]
struct MiningBar;
#[derive(Component)]
struct MiningBarFill;
#[derive(Component)]
struct MiningLabel;
#[derive(Component)]
struct PauseOverlay;

const PANEL_BG: Color = Color::srgba(0.05, 0.08, 0.12, 0.75);
const BAR_BG: Color = Color::srgba(0.15, 0.18, 0.22, 0.9);

fn spawn_hud(mut commands: Commands) {
    // --- Top-left: hull + speed ---
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
                TextFont {
                    font_size: FontSize::Px(12.0),
                    ..default()
                },
                TextColor(Color::srgb(0.7, 0.8, 0.9)),
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
                SpeedText,
                Text::new("SPD 0"),
                TextFont {
                    font_size: FontSize::Px(12.0),
                    ..default()
                },
                TextColor(Color::srgb(0.7, 0.8, 0.9)),
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
                TextFont {
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.9, 0.95)),
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
                TextFont {
                    font_size: FontSize::Px(12.0),
                    ..default()
                },
                TextColor(Color::srgb(0.9, 0.85, 0.5)),
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

    // --- Center: pause overlay ---
    commands.spawn((
        Name::new("HUD Pause Overlay"),
        PauseOverlay,
        Text::new("PAUSED - press P to resume"),
        TextFont {
            font_size: FontSize::Px(28.0),
            ..default()
        },
        TextColor(Color::srgb(0.9, 0.9, 1.0)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(45.0),
            left: Val::Percent(50.0),
            margin: UiRect::left(Val::Px(-220.0)),
            ..default()
        },
        Visibility::Hidden,
    ));
}

fn update_hull_bar(
    ships: Query<(&Hull, &ShipStats), With<PlayerShip>>,
    mut fills: Query<(&mut Node, &mut BackgroundColor), With<HullBarFill>>,
) {
    let Ok((hull, stats)) = ships.single() else {
        return;
    };
    let fraction = (hull.0 / stats.max_hull).clamp(0.0, 1.0);
    for (mut node, mut color) in &mut fills {
        node.width = Val::Percent(fraction * 100.0);
        // Green when healthy, red when critical.
        color.0 = Color::srgb(0.9 - 0.6 * fraction, 0.2 + 0.7 * fraction, 0.3);
    }
}

fn update_speed_text(
    ships: Query<&Velocity, With<PlayerShip>>,
    mut texts: Query<&mut Text, With<SpeedText>>,
) {
    let Ok(velocity) = ships.single() else {
        return;
    };
    for mut text in &mut texts {
        text.0 = format!("SPD {:>4.0}", velocity.0.length());
    }
}

fn update_cargo_text(
    ships: Query<&Cargo, With<PlayerShip>>,
    mut texts: Query<&mut Text, With<CargoText>>,
) {
    let Ok(cargo) = ships.single() else {
        return;
    };
    let mut lines = format!("CARGO {}/{}", cargo.total(), cargo.capacity());
    if cargo.is_full() {
        lines.push_str("  [FULL]");
    }
    // Full manifest (zeroes included) keeps the panel layout stable.
    for kind in ResourceType::ALL {
        lines.push_str(&format!("\n{:<8} {:>3}", kind.name(), cargo.amount(kind)));
    }
    for mut text in &mut texts {
        text.0.clone_from(&lines);
    }
}

fn update_mining_bar(
    ships: Query<&MiningRig, With<PlayerShip>>,
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

// ---------------------------------------------------------------------------
// Minimap
// ---------------------------------------------------------------------------

const MINIMAP_SIZE: f32 = 160.0;

/// A minimap marker tracking one world entity's `SimPosition`.
#[derive(Component)]
struct MinimapDot {
    target: Entity,
    dot_size: f32,
}

/// World units from the star that map to the minimap edge.
#[derive(Resource)]
struct MinimapExtent(f32);

fn spawn_minimap(
    mut commands: Commands,
    config: Res<GameConfig>,
    stars: Query<Entity, With<CentralStar>>,
    planets: Query<(Entity, &Planet)>,
    ships: Query<Entity, With<PlayerShip>>,
) {
    // Everything of interest must fit: widest orbit or outermost belt.
    let system = &config.system;
    let extent = system
        .planets
        .iter()
        .map(|planet| planet.orbit_radius)
        .chain(system.belts.iter().map(|belt| belt.outer_radius))
        .fold(1000.0_f32, f32::max)
        * 1.08;
    commands.insert_resource(MinimapExtent(extent));

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

    let mut spawn_dot = |target: Entity, size: f32, color: Color| {
        commands.spawn((
            MinimapDot {
                target,
                dot_size: size,
            },
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(size),
                height: Val::Px(size),
                ..default()
            },
            BackgroundColor(color),
            ChildOf(map_box),
        ));
    };

    for star in &stars {
        let color = system.star.color;
        spawn_dot(star, 7.0, Color::srgb(color.0, color.1, color.2));
    }
    for (entity, planet) in &planets {
        if let Some(cfg) = system.planets.get(planet.config_index) {
            spawn_dot(
                entity,
                4.0,
                Color::srgb(cfg.color.0, cfg.color.1, cfg.color.2),
            );
        }
    }
    for ship in &ships {
        spawn_dot(ship, 3.0, Color::WHITE);
    }

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
                            TextFont {
                                font_size: FontSize::Px(10.0),
                                ..default()
                            },
                            TextColor(Color::srgb(0.7, 0.75, 0.8)),
                        ));
                    });
            }
        });
}

fn update_minimap(
    extent: Option<Res<MinimapExtent>>,
    positions: Query<&SimPosition>,
    mut dots: Query<(&MinimapDot, &mut Node, &mut Visibility)>,
) {
    let Some(extent) = extent else {
        return;
    };
    for (dot, mut node, mut visibility) in &mut dots {
        // Hide markers whose entity is gone (e.g. a despawned target).
        let Ok(pos) = positions.get(dot.target) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        *visibility = Visibility::Inherited;
        let normalized = (pos.current / extent.0 + Vec2::ONE) / 2.0;
        let range = MINIMAP_SIZE - dot.dot_size;
        node.left = Val::Px((normalized.x.clamp(0.0, 1.0) * range).round());
        // World +Y is up; UI +Y is down.
        node.top = Val::Px(((1.0 - normalized.y.clamp(0.0, 1.0)) * range).round());
    }
}

fn show_pause_overlay(mut overlays: Query<&mut Visibility, With<PauseOverlay>>) {
    for mut visibility in &mut overlays {
        *visibility = Visibility::Visible;
    }
}

fn hide_pause_overlay(mut overlays: Query<&mut Visibility, With<PauseOverlay>>) {
    for mut visibility in &mut overlays {
        *visibility = Visibility::Hidden;
    }
}
