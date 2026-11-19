//! Player settings: units, mouse sensitivity, key bindings, crosshair and
//! aircraft presets. Saved to `settings.json` in your user settings folder
//! (on Windows, `%APPDATA%\\WT Aim Trainer`), so they're remembered no matter
//! where the game is started from.

use std::collections::BTreeMap;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::flight_model::AircraftCard;
use crate::weapons::{GunCard, is_builtin_gun};

const SETTINGS_FILE: &str = "settings.json";
const APP_FOLDER: &str = "WT Aim Trainer";

/// Where settings live: the per-user config folder, falling back to the
/// current folder if none can be found.
pub fn settings_path() -> std::path::PathBuf {
    let base = std::env::var_os("WTAT_CONFIG_DIR")
        .or_else(|| std::env::var_os("APPDATA"))
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")));
    match base {
        Some(dir) => dir.join(APP_FOLDER).join(SETTINGS_FILE),
        None => std::path::PathBuf::from(SETTINGS_FILE),
    }
}

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

    /// Format a target range given in meters (km, mi or nmi).
    pub fn range(self, m: f32) -> String {
        match self {
            Units::Metric => format!("{:.2} km", m / 1000.0),
            Units::Imperial => format!("{:.2} mi", m / 1609.344),
            Units::Nautical => format!("{:.2} nmi", m / 1852.0),
        }
    }

    /// Speed unit for editing, with the factor from m/s.
    pub fn speed_unit(self) -> (&'static str, f32) {
        match self {
            Units::Metric => ("km/h", 3.6),
            Units::Imperial => ("mph", 2.236_936),
            Units::Nautical => ("kt", 1.943_844),
        }
    }

    /// Climb rate unit for editing, with the factor from m/s.
    pub fn climb_unit(self) -> (&'static str, f32) {
        match self {
            Units::Metric => ("m/s", 1.0),
            Units::Imperial | Units::Nautical => ("ft/min", 196.850_4),
        }
    }

    /// Altitude unit for editing, with the factor from meters.
    pub fn altitude_unit(self) -> (&'static str, f32) {
        match self {
            Units::Metric => ("m", 1.0),
            Units::Imperial | Units::Nautical => ("ft", 3.280_84),
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
            Action::Reset => "Reset scenario",
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

/// Gun reticle styles, matching War Thunder's options.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Crosshair {
    #[default]
    Cross,
    Circle,
    CircleDot,
    Dot,
    SmallCross,
}

impl Crosshair {
    pub const ALL: [Crosshair; 5] = [
        Crosshair::Cross,
        Crosshair::Circle,
        Crosshair::CircleDot,
        Crosshair::Dot,
        Crosshair::SmallCross,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Crosshair::Cross => "Cross",
            Crosshair::Circle => "Circle",
            Crosshair::CircleDot => "Circle with dot",
            Crosshair::Dot => "Dot",
            Crosshair::SmallCross => "Small cross",
        }
    }
}

/// Crosshair color. `Default` uses each style's own color like the game.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrosshairColor {
    #[default]
    Default,
    White,
    Gold,
    Green,
    Cyan,
    Yellow,
    Red,
    Magenta,
}

impl CrosshairColor {
    pub const ALL: [CrosshairColor; 8] = [
        CrosshairColor::Default,
        CrosshairColor::White,
        CrosshairColor::Gold,
        CrosshairColor::Green,
        CrosshairColor::Cyan,
        CrosshairColor::Yellow,
        CrosshairColor::Red,
        CrosshairColor::Magenta,
    ];

    pub fn label(self) -> &'static str {
        match self {
            CrosshairColor::Default => "Default color",
            CrosshairColor::White => "White",
            CrosshairColor::Gold => "Gold",
            CrosshairColor::Green => "Green",
            CrosshairColor::Cyan => "Cyan",
            CrosshairColor::Yellow => "Yellow",
            CrosshairColor::Red => "Red",
            CrosshairColor::Magenta => "Magenta",
        }
    }

    /// The chosen color, or `None` to use the style's own.
    pub fn color(self) -> Option<Color> {
        Some(match self {
            CrosshairColor::Default => return None,
            CrosshairColor::White => Color::srgb(0.95, 0.95, 0.95),
            CrosshairColor::Gold => Color::srgb(0.85, 0.72, 0.45),
            CrosshairColor::Green => Color::srgb(0.30, 1.0, 0.40),
            CrosshairColor::Cyan => Color::srgb(0.30, 0.95, 1.0),
            CrosshairColor::Yellow => Color::srgb(1.0, 0.95, 0.25),
            CrosshairColor::Red => Color::srgb(1.0, 0.25, 0.2),
            CrosshairColor::Magenta => Color::srgb(1.0, 0.35, 0.95),
        })
    }
}

#[derive(Resource, Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub units: Units,
    /// Multiplier on [`BASE_SENSITIVITY`].
    pub sensitivity: f32,
    /// Multiplier on [`BASE_SENSITIVITY`] for free look.
    pub look_sensitivity: f32,
    pub bindings: BTreeMap<Action, KeyCode>,
    pub crosshair: Crosshair,
    pub crosshair_color: CrosshairColor,
    /// Saved flight model presets.
    pub aircraft: Vec<AircraftCard>,
    /// Preset names flown by the player and the target.
    pub player_aircraft: String,
    pub target_aircraft: String,
    /// Saved gun presets.
    pub guns: Vec<GunCard>,
    /// Which practice scenario runs.
    pub scenario: Scenario,
    /// Chase scenario: the target starts this far ahead, m (random
    /// between the two).
    pub chase_min: f32,
    pub chase_max: f32,
    /// Show where to aim to hit the target.
    pub lead_indicator: bool,
    /// The target shoots back.
    pub enemy_fires: bool,
}

/// Practice scenarios.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scenario {
    HeadOn,
    #[default]
    Chase,
    Stall,
    Dogfight,
    /// A random one of the others each time.
    AllInOne,
}

impl Scenario {
    pub const ALL: [Scenario; 5] = [
        Scenario::HeadOn,
        Scenario::Chase,
        Scenario::Stall,
        Scenario::Dogfight,
        Scenario::AllInOne,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Scenario::HeadOn => "Head on",
            Scenario::Chase => "Chase",
            Scenario::Stall => "Stall",
            Scenario::Dogfight => "Dogfight",
            Scenario::AllInOne => "All in one",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Scenario::HeadOn => {
                "You fly straight at each other. The target holds a straight line at a \
                 random speed, from a slightly different height and toward a different \
                 point near you each time."
            }
            Scenario::Chase => {
                "You start behind the target. It keeps its heading but weaves and \
                 dodges: some runs hard, some gently, some not at all."
            }
            Scenario::Stall => {
                "The target starts just above and ahead of you and pulls straight up, \
                 rolling and dodging as it climbs."
            }
            Scenario::Dogfight => {
                "The target starts close by, at a random spot, height, heading and \
                 speed. It tries to ram you, or with enemy fire on, to shoot you down."
            }
            Scenario::AllInOne => "A random scenario every time it resets.",
        }
    }
}

/// Limits and step for the chase start distance, m.
pub const CHASE_MIN: f32 = 100.0;
pub const CHASE_MAX: f32 = 3000.0;
pub const CHASE_STEP: f32 = 100.0;

impl Default for Settings {
    fn default() -> Self {
        Self {
            units: Units::Metric,
            sensitivity: 1.0,
            look_sensitivity: 1.0,
            bindings: default_bindings(),
            crosshair: Crosshair::Cross,
            crosshair_color: CrosshairColor::Default,
            aircraft: vec![AircraftCard::yak3()],
            player_aircraft: "Yak-3".into(),
            target_aircraft: "Yak-3".into(),
            guns: vec![GunCard::shvak(), GunCard::ubs()],
            scenario: Scenario::Chase,
            chase_min: 300.0,
            chase_max: 800.0,
            lead_indicator: false,
            enemy_fires: false,
        }
    }
}

pub fn default_bindings() -> BTreeMap<Action, KeyCode> {
    Action::ALL.iter().map(|a| (*a, a.default_key())).collect()
}

impl Settings {
    /// The preset with this name, or the first one (there's always at least one).
    pub fn card(&self, name: &str) -> &AircraftCard {
        self.aircraft
            .iter()
            .find(|c| c.name == name)
            .unwrap_or(&self.aircraft[0])
    }

    pub fn player_card(&self) -> &AircraftCard {
        self.card(&self.player_aircraft)
    }

    pub fn target_card(&self) -> &AircraftCard {
        self.card(&self.target_aircraft)
    }

    /// Add or replace a preset by name.
    /// Store a preset. The built-in Yak-3 can't be overwritten.
    pub fn save_card(&mut self, card: AircraftCard) -> Result<(), String> {
        if is_builtin(&card.name) {
            return Err(format!(
                "\"{}\" is the built-in preset and can't be changed. Give it a new name to save a copy.",
                card.name
            ));
        }
        match self.aircraft.iter_mut().find(|c| c.name == card.name) {
            Some(existing) => *existing = card,
            None => self.aircraft.push(card),
        }
        Ok(())
    }

    /// The gun preset with this name, or the built-in ShVAK.
    pub fn gun(&self, name: &str) -> &GunCard {
        self.guns
            .iter()
            .find(|g| g.name == name)
            .unwrap_or(&self.guns[0])
    }

    /// Store a gun preset. The built-in ShVAK can't be overwritten.
    pub fn save_gun(&mut self, gun: GunCard) -> Result<(), String> {
        if is_builtin_gun(&gun.name) {
            return Err(format!(
                "\"{}\" is the built-in gun and can't be changed. Give it a new name to save a copy.",
                gun.name
            ));
        }
        match self.guns.iter_mut().find(|g| g.name == gun.name) {
            Some(existing) => *existing = gun,
            None => self.guns.push(gun),
        }
        Ok(())
    }

    /// Remove a gun preset. The built-in ShVAK can't be deleted. Guns
    /// mounted on aircraft that used it fall back to the ShVAK.
    pub fn delete_gun(&mut self, name: &str) -> Result<(), String> {
        if is_builtin_gun(name) {
            return Err(format!(
                "\"{name}\" is the built-in gun and can't be deleted."
            ));
        }
        self.guns.retain(|g| g.name != name);
        self.ensure_aircraft();
        Ok(())
    }

    /// Remove a preset. The built-in Yak-3 can't be deleted.
    pub fn delete_card(&mut self, name: &str) -> Result<(), String> {
        if is_builtin(name) {
            return Err(format!(
                "\"{name}\" is the built-in preset and can't be deleted."
            ));
        }
        self.aircraft.retain(|c| c.name != name);
        self.ensure_aircraft();
        Ok(())
    }

    /// Keep at least one preset and point the player and target at real ones.
    fn ensure_aircraft(&mut self) {
        // The built-in Yak-3 always comes first, with its stock numbers
        // (older saves may hold an edited copy).
        self.aircraft.retain(|c| !is_builtin(&c.name));
        self.aircraft.insert(0, AircraftCard::yak3());
        // Same for the built-in ShVAK gun.
        self.guns.retain(|g| !is_builtin_gun(&g.name));
        self.guns.insert(0, GunCard::shvak());
        let first = self.aircraft[0].name.clone();
        if !self.aircraft.iter().any(|c| c.name == self.player_aircraft) {
            self.player_aircraft = first.clone();
        }
        if !self.aircraft.iter().any(|c| c.name == self.target_aircraft) {
            self.target_aircraft = first;
        }
    }

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

    /// Radians of free look per pixel of mouse movement.
    pub fn look_per_pixel(&self) -> f32 {
        BASE_SENSITIVITY * self.look_sensitivity
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
        let mut settings = Self::load_file();
        settings.ensure_aircraft();
        settings
    }

    fn load_file() -> Self {
        let path = settings_path();
        // Earlier versions saved next to the game; pick that up once.
        let text =
            std::fs::read_to_string(&path).or_else(|_| std::fs::read_to_string(SETTINGS_FILE));
        match text {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                warn!("Ignoring unreadable {}: {e}", path.display());
                Settings::default()
            }),
            Err(_) => Settings::default(),
        }
    }

    fn save(&self) {
        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                let path = settings_path();
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                // Write to a temporary file first so a crash can't leave a
                // half-written settings file behind.
                let tmp = path.with_extension("json.tmp");
                let result = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path));
                if let Err(e) = result {
                    warn!("Could not save {}: {e}", path.display());
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

/// The stock Yak-3 preset, which can't be edited or deleted.
pub fn is_builtin(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case(&AircraftCard::yak3().name)
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
    fn many_presets_round_trip() {
        let mut s = Settings::default();
        for i in 0..40 {
            let mut card = AircraftCard::yak3();
            card.name = format!("Plane {i}");
            card.mass = None;
            s.save_card(card).unwrap();
        }
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.aircraft.len(), 41);
        assert_eq!(back.card("Plane 39").mass, None);
    }

    #[test]
    fn built_in_yak_is_locked() {
        let mut s = Settings::default();
        let mut edited = AircraftCard::yak3();
        edited.power = 5000.0;
        assert!(s.save_card(edited.clone()).is_err());
        assert!(s.delete_card("Yak-3").is_err());
        assert_eq!(s.card("Yak-3").power, 1320.0);

        // An old save with an edited Yak-3 gets the stock one back.
        s.aircraft = vec![edited];
        s.ensure_aircraft();
        assert_eq!(s.aircraft.len(), 1);
        assert_eq!(s.card("Yak-3").power, 1320.0);
    }

    #[test]
    fn key_names_are_short() {
        assert_eq!(key_name(KeyCode::KeyW), "W");
        assert_eq!(key_name(KeyCode::ShiftLeft), "L Shift");
        assert_eq!(key_name(KeyCode::AltLeft), "L Alt");
    }
}
