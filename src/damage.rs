//! Damage models: how many hits, and where, bring a plane down.
//!
//! Fighter: hits add up until the plane is destroyed, and a direct hit on
//! the cockpit destroys it at once. Bomber: only a cockpit hit brings it
//! down (or, with no fixed guns, losing every turret). Turrets take their
//! own damage either way and are knocked out one by one.

use serde::{Deserialize, Serialize};

use crate::flight_model::AircraftCard;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DamageModel {
    #[default]
    Fighter,
    Bomber,
}

impl DamageModel {
    pub fn label(self) -> &'static str {
        match self {
            DamageModel::Fighter => "Fighter",
            DamageModel::Bomber => "Bomber",
        }
    }

    pub fn next(self) -> DamageModel {
        match self {
            DamageModel::Fighter => DamageModel::Bomber,
            DamageModel::Bomber => DamageModel::Fighter,
        }
    }
}

/// What a round struck.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum HitPart {
    #[default]
    Airframe,
    Cockpit,
    /// A turret, by its place in the loadout's turret list.
    Turret(u8),
}

/// A turret takes this many times the damage the airframe would, so it
/// goes after about a third of the hits.
pub const TURRET_FRAGILITY: f32 = 3.0;

/// The damage a plane has taken.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Damage {
    /// Share of the airframe shot away (fighter damage model; 1 = down).
    pub airframe: f32,
    /// Each turret's damage (1 = knocked out).
    #[serde(default)]
    pub turrets: Vec<f32>,
    /// The cockpit took a direct hit.
    #[serde(default)]
    pub cockpit: bool,
}

impl Damage {
    pub fn turret_lost(&self, i: usize) -> bool {
        self.turrets.get(i).is_some_and(|d| *d >= 1.0)
    }

    /// Knocked out turrets as bits (turret 0 = bit 0).
    pub fn lost_mask(&self) -> u8 {
        (0..self.turrets.len().min(8))
            .filter(|i| self.turret_lost(*i))
            .fold(0, |m, i| m | (1 << i))
    }

    pub fn destroyed(&self, card: &AircraftCard) -> bool {
        if self.cockpit {
            return true;
        }
        match card.damage_model() {
            DamageModel::Fighter => self.airframe >= 1.0,
            DamageModel::Bomber => {
                // Nothing left to shoot back with.
                let turrets = card.weapons.turrets.len();
                card.weapons.guns().is_empty()
                    && turrets > 0
                    && (0..turrets).all(|i| self.turret_lost(i))
            }
        }
    }

    /// Take a hit. Returns the turret it knocked out, if any.
    pub fn hit(&mut self, card: &AircraftCard, part: HitPart, damage: f32) -> Option<u8> {
        let damage = damage.clamp(0.0, 1.0);
        match part {
            HitPart::Cockpit => {
                self.cockpit = true;
                None
            }
            HitPart::Turret(i) if (i as usize) < card.weapons.turrets.len() => {
                let n = card.weapons.turrets.len();
                if self.turrets.len() < n {
                    self.turrets.resize(n, 0.0);
                }
                let was = self.turret_lost(i as usize);
                self.turrets[i as usize] += damage * TURRET_FRAGILITY;
                (!was && self.turret_lost(i as usize)).then_some(i)
            }
            _ => {
                if card.damage_model() == DamageModel::Fighter {
                    self.airframe += damage;
                }
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turrets::{Turret, TurretPlace};

    #[test]
    fn fighters_add_up_and_die_to_a_cockpit_hit() {
        let card = AircraftCard::yak3();
        let mut d = Damage::default();
        d.hit(&card, HitPart::Airframe, 0.4);
        assert!(!d.destroyed(&card));
        d.hit(&card, HitPart::Airframe, 0.7);
        assert!(d.destroyed(&card));
        let mut d = Damage::default();
        d.hit(&card, HitPart::Cockpit, 0.01);
        assert!(d.destroyed(&card));
    }

    #[test]
    fn bombers_only_die_to_the_cockpit_or_losing_every_turret() {
        let mut card = AircraftCard::yak3();
        card.damage = Some(DamageModel::Bomber);
        card.weapons.turrets = vec![
            Turret::new(TurretPlace::Top),
            Turret::new(TurretPlace::Rear),
        ];
        let mut d = Damage::default();
        for _ in 0..50 {
            d.hit(&card, HitPart::Airframe, 0.2);
        }
        assert!(!d.destroyed(&card), "airframe hits don't count");
        // It still has its fixed guns, so losing both turrets isn't enough.
        assert_eq!(d.hit(&card, HitPart::Turret(0), 0.4), Some(0));
        assert_eq!(d.hit(&card, HitPart::Turret(1), 0.4), Some(1));
        assert!(!d.destroyed(&card));
        // Turrets only: losing them all brings it down.
        card.weapons.nose_center.count = 0;
        card.weapons.nose_ring.count = 0;
        assert!(d.destroyed(&card));
        assert_eq!(d.lost_mask(), 0b11);
    }
}
