//! The extensible resource taxonomy.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Every mineable resource in the game. Extending the game with a new
/// resource means adding a variant here (plus config entries); cargo, mining,
/// HUD and persistence all iterate over this enum generically.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Reflect,
)]
pub enum ResourceType {
    Iron,
    Ice,
    Crystal,
    Gas,
}

impl ResourceType {
    pub const ALL: [ResourceType; 4] = [
        ResourceType::Iron,
        ResourceType::Ice,
        ResourceType::Crystal,
        ResourceType::Gas,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ResourceType::Iron => "Iron",
            ResourceType::Ice => "Ice",
            ResourceType::Crystal => "Crystal",
            ResourceType::Gas => "Gas",
        }
    }

    /// Display color used for asteroids tinting, HUD text and particles.
    pub fn color(self) -> Color {
        match self {
            ResourceType::Iron => Color::srgb(0.78, 0.6, 0.5),
            ResourceType::Ice => Color::srgb(0.65, 0.85, 0.95),
            ResourceType::Crystal => Color::srgb(0.85, 0.55, 0.9),
            ResourceType::Gas => Color::srgb(0.55, 0.9, 0.6),
        }
    }
}
