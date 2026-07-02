//! Ship cargo hold: capacity-limited, typed resource storage.

use std::collections::BTreeMap;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::resource_types::ResourceType;

/// A capacity-limited cargo hold. Invariant: `total() <= capacity`.
/// `BTreeMap` keeps iteration order stable for the HUD and save files.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cargo {
    capacity: u32,
    contents: BTreeMap<ResourceType, u32>,
}

impl Cargo {
    pub fn new(capacity: u32) -> Self {
        Self {
            capacity,
            contents: BTreeMap::new(),
        }
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    pub fn total(&self) -> u32 {
        self.contents.values().sum()
    }

    pub fn free_space(&self) -> u32 {
        self.capacity.saturating_sub(self.total())
    }

    pub fn is_full(&self) -> bool {
        self.free_space() == 0
    }

    pub fn amount(&self, kind: ResourceType) -> u32 {
        self.contents.get(&kind).copied().unwrap_or(0)
    }

    /// Store up to `amount` units, respecting capacity.
    /// Returns how many units were actually stored.
    pub fn add(&mut self, kind: ResourceType, amount: u32) -> u32 {
        let stored = amount.min(self.free_space());
        if stored > 0 {
            *self.contents.entry(kind).or_insert(0) += stored;
        }
        stored
    }

    /// Remove up to `amount` units. Returns how many were actually removed.
    /// Not called by gameplay yet: reserved for trading/refining mechanics.
    #[allow(dead_code)]
    pub fn remove(&mut self, kind: ResourceType, amount: u32) -> u32 {
        let Some(stored) = self.contents.get_mut(&kind) else {
            return 0;
        };
        let removed = amount.min(*stored);
        *stored -= removed;
        if *stored == 0 {
            self.contents.remove(&kind);
        }
        removed
    }

    /// Non-empty stacks in stable (enum) order.
    /// Not called by gameplay yet: reserved for trading/station UIs.
    #[allow(dead_code)]
    pub fn iter(&self) -> impl Iterator<Item = (ResourceType, u32)> + '_ {
        self.contents.iter().map(|(kind, amount)| (*kind, *amount))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_respects_capacity() {
        let mut cargo = Cargo::new(10);
        assert_eq!(cargo.add(ResourceType::Iron, 7), 7);
        assert_eq!(cargo.add(ResourceType::Ice, 7), 3, "only 3 slots left");
        assert_eq!(cargo.total(), 10);
        assert!(cargo.is_full());
        assert_eq!(cargo.add(ResourceType::Gas, 1), 0);
    }

    #[test]
    fn remove_caps_at_stored_amount() {
        let mut cargo = Cargo::new(10);
        cargo.add(ResourceType::Crystal, 4);
        assert_eq!(cargo.remove(ResourceType::Crystal, 10), 4);
        assert_eq!(cargo.amount(ResourceType::Crystal), 0);
        assert_eq!(cargo.remove(ResourceType::Crystal, 1), 0);
        assert_eq!(cargo.remove(ResourceType::Iron, 1), 0, "never stored");
    }

    #[test]
    fn free_space_tracks_mixed_contents() {
        let mut cargo = Cargo::new(20);
        cargo.add(ResourceType::Iron, 5);
        cargo.add(ResourceType::Gas, 3);
        assert_eq!(cargo.free_space(), 12);
        cargo.remove(ResourceType::Iron, 2);
        assert_eq!(cargo.free_space(), 14);
    }

    #[test]
    fn iter_skips_emptied_stacks() {
        let mut cargo = Cargo::new(10);
        cargo.add(ResourceType::Iron, 2);
        cargo.add(ResourceType::Ice, 3);
        cargo.remove(ResourceType::Iron, 2);
        let kinds: Vec<_> = cargo.iter().map(|(kind, _)| kind).collect();
        assert_eq!(kinds, vec![ResourceType::Ice]);
    }

    #[test]
    fn zero_capacity_stores_nothing() {
        let mut cargo = Cargo::new(0);
        assert_eq!(cargo.add(ResourceType::Iron, 5), 0);
        assert!(cargo.is_full());
    }
}
