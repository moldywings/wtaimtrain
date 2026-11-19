//! Player settings: units, mouse sensitivity and key bindings. Saved to
//! `settings.json` in the folder the game is run from.

use std::collections::BTreeMap;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

const SETTINGS_FILE: &str = "settings.json";

/// Radians of aim movement per pixel at 1.0x sensitivity.
pub const BASE_SENSITIVITY: f32 = 0.0015;
pub const MIN_SENSITIVITY: f32 = 0.1;
pub const MAX_SENSITIVITY: f32 = 5.0;

pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Settings::load())
            .add_systems(Update, save_on_change);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Units {
    #[default]
    Metric,
    Imperial,
    Nautical,
}

impl Units {
    pub fn next(self) -> Self {
        match self {
            Units::Metric => Units::Imperial,
            Units::Imperial => Units::Nautical,
            Units::Nautical => Units::Metric,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Units::Metric => "Metric (km/h, m)",
            Units::Imperial => "Imperial (mph, ft)",
            Units::Nautical => "Nautical (kn, ft)",
        }
    }

    /// Format a speed given in m/s.
    pub fn speed(self, mps: f32) -> String {
        match self {
            Units::Metric => format!("{:.0} km/h", mps * 3.6),
            Units::Imperial => format!("{:.0} mph", mps * 2.236_936),
            Units::Nautical => format!("{:.0} kn", mps * 1.943_844),
        }
    }

    /// Format a distance or altitude given in meters.
    pub fn distance(self, m: f32) -> String {
        match self {
            Units::Metric => format!("{:.0} m", m),
            Units::Imperial | Units::Nautical => format!("{:.0} ft", m * 3.280_84),
        }
    }
}

/// Things the player can bind to a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Action {
    ThrottleUp,
    ThrottleDown,
    PitchUp,
    PitchDown,
    RollLeft,
    RollRight,
    YawLeft,
    YawRight,
    FreeLook,
    Reset,
}

impl Action {
    pub const ALL: [Action; 10] = [
        Action::ThrottleUp,
        Action::ThrottleDown,
        Action::PitchUp,
        Action::PitchDown,
        Action::RollLeft,
        Action::RollRight,
        Action::YawLeft,
        Action::YawRight,
        Action::FreeLook,
        Action::Reset,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Action::ThrottleUp => "Throttle up",
            Action::ThrottleDown => "Throttle down",
            Action::PitchUp => "Pitch up (nose up)",
            Action::PitchDown => "Pitch down (nose down)",
            Action::RollLeft => "Roll left",
            Action::RollRight => "Roll right",
            Action::YawLeft => "Yaw left",
            Action::YawRight => "Yaw right",
            Action::FreeLook => "Free look (hold)",
            Action::Reset => "Reset position",
        }
    }

    pub fn default_key(self) -> KeyCode {
        match self {
            Action::ThrottleUp => KeyCode::ShiftLeft,
            Action::ThrottleDown => KeyCode::ControlLeft,
            // Like a stick: W pushes the nose down, S pulls it up.
            Action::PitchUp => KeyCode::KeyS,
            Action::PitchDown => KeyCode::KeyW,
            Action::RollLeft => KeyCode::KeyA,
            Action::RollRight => KeyCode::KeyD,
            Action::YawLeft => KeyCode::KeyQ,
            Action::YawRight => KeyCode::KeyE,
            Action::FreeLook => KeyCode::AltLeft,
            Action::Reset => KeyCode::KeyR,
        }
    }
}

#[derive(Resource, Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub units: Units,
    /// Multiplier on [`BASE_SENSITIVITY`].
    pub sensitivity: f32,
    pub bindings: BTreeMap<Action, KeyCode>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            units: Units::Metric,
            sensitivity: 1.0,
            bindings: default_bindings(),
        }
    }
}

pub fn default_bindings() -> BTreeMap<Action, KeyCode> {
    Action::ALL.iter().map(|a| (*a, a.default_key())).collect()
}

impl Settings {
    pub fn key(&self, action: Action) -> KeyCode {
        self.bindings
            .get(&action)
            .copied()
            .unwrap_or_else(|| action.default_key())
    }

    pub fn pressed(&self, keys: &ButtonInput<KeyCode>, action: Action) -> bool {
        keys.pressed(self.key(action))
    }

    pub fn just_pressed(&self, keys: &ButtonInput<KeyCode>, action: Action) -> bool {
        keys.just_pressed(self.key(action))
    }

    /// Radians of aim movement per pixel of mouse movement.
    pub fn aim_per_pixel(&self) -> f32 {
        BASE_SENSITIVITY * self.sensitivity
    }

    /// Bind `key` to `action`. Any other action already on that key swaps to
    /// the action's old key, so no two actions share a key.
    pub fn rebind(&mut self, action: Action, key: KeyCode) {
        let old = self.key(action);
        for a in Action::ALL {
            if a != action && self.key(a) == key {
                self.bindings.insert(a, old);
            }
        }
        self.bindings.insert(action, key);
    }

    fn load() -> Self {
        match std::fs::read_to_string(SETTINGS_FILE) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                warn!("Ignoring unreadable {SETTINGS_FILE}: {e}");
                Settings::default()
            }),
            Err(_) => Settings::default(),
        }
    }

    fn save(&self) {
        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                if let Err(e) = std::fs::write(SETTINGS_FILE, text) {
                    warn!("Could not save {SETTINGS_FILE}: {e}");
                }
            }
            Err(e) => warn!("Could not serialize settings: {e}"),
        }
    }
}

fn save_on_change(settings: Res<Settings>) {
    if settings.is_changed() && !settings.is_added() {
        settings.save();
    }
}

/// Short, readable name for a key, e.g. `KeyW` -> `W`, `ShiftLeft` -> `L Shift`.
pub fn key_name(key: KeyCode) -> String {
    let raw = format!("{key:?}");
    if let Some(rest) = raw.strip_prefix("Key") {
        return rest.to_string();
    }
    if let Some(rest) = raw.strip_prefix("Digit") {
        return rest.to_string();
    }
    for (side, short) in [("Left", "L "), ("Right", "R ")] {
        if let Some(base) = raw.strip_suffix(side) {
            return format!("{short}{base}");
        }
    }
    if let Some(rest) = raw.strip_prefix("Arrow") {
        return format!("{rest} Arrow");
    }
    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebinding_to_a_used_key_swaps() {
        let mut s = Settings::default();
        s.rebind(Action::RollLeft, KeyCode::KeyD);
        assert_eq!(s.key(Action::RollLeft), KeyCode::KeyD);
        assert_eq!(s.key(Action::RollRight), KeyCode::KeyA);
    }

    #[test]
    fn settings_round_trip_through_json() {
        let mut s = Settings {
            units: Units::Nautical,
            sensitivity: 1.7,
            ..Default::default()
        };
        s.rebind(Action::FreeLook, KeyCode::KeyC);
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.units, Units::Nautical);
        assert_eq!(back.key(Action::FreeLook), KeyCode::KeyC);
        assert!((back.sensitivity - 1.7).abs() < 1e-6);
    }

    #[test]
    fn key_names_are_short() {
        assert_eq!(key_name(KeyCode::KeyW), "W");
        assert_eq!(key_name(KeyCode::ShiftLeft), "L Shift");
        assert_eq!(key_name(KeyCode::AltLeft), "L Alt");
    }
}
