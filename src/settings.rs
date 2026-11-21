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

/// A kind of quantity whose display unit the player can pick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnitKind {
    Speed,
    Altitude,
    Distance,
    Climb,
    Power,
    Thrust,
    Mass,
    Area,
}

/// One display unit: its name, how many of it make one stored (SI-ish)
/// unit, and how many decimals to show.
pub struct UnitDef {
    pub name: &'static str,
    pub per_base: f32,
    pub decimals: usize,
}

const fn u(name: &'static str, per_base: f32, decimals: usize) -> UnitDef {
    UnitDef {
        name,
        per_base,
        decimals,
    }
}

const FT: f32 = 3.280_84;
const SPEED_UNITS: [UnitDef; 4] = [
    u("km/h", 3.6, 0),
    u("mph", 2.236_936, 0),
    u("kn", 1.943_844, 0),
    u("m/s", 1.0, 0),
];
const ALTITUDE_UNITS: [UnitDef; 2] = [u("m", 1.0, 0), u("ft", FT, 0)];
const DISTANCE_UNITS: [UnitDef; 6] = [
    u("km", 0.001, 2),
    u("mi", 1.0 / 1609.344, 2),
    u("nmi", 1.0 / 1852.0, 2),
    u("m", 1.0, 0),
    u("ft", FT, 0),
    u("yd", FT / 3.0, 0),
];
const CLIMB_UNITS: [UnitDef; 3] = [
    u("m/s", 1.0, 1),
    u("ft/min", FT * 60.0, 0),
    u("ft/s", FT, 1),
];
/// Stored in horsepower.
const POWER_UNITS: [UnitDef; 2] = [u("hp", 1.0, 0), u("kW", 0.745_7, 0)];
/// Stored in kgf.
const THRUST_UNITS: [UnitDef; 3] = [
    u("kgf", 1.0, 0),
    u("lbf", 2.204_623, 0),
    u("kN", 0.009_806_65, 2),
];
const MASS_UNITS: [UnitDef; 2] = [u("kg", 1.0, 0), u("lb", 2.204_623, 0)];
const AREA_UNITS: [UnitDef; 2] = [u("sq m", 1.0, 2), u("sq ft", 10.763_91, 1)];

impl UnitKind {
    pub const ALL: [UnitKind; 8] = [
        UnitKind::Speed,
        UnitKind::Altitude,
        UnitKind::Distance,
        UnitKind::Climb,
        UnitKind::Power,
        UnitKind::Thrust,
        UnitKind::Mass,
        UnitKind::Area,
    ];

    pub fn label(self) -> &'static str {
        match self {
            UnitKind::Speed => "Airspeed",
            UnitKind::Altitude => "Altitude",
            UnitKind::Distance => "Distance and range",
            UnitKind::Climb => "Climb rate",
            UnitKind::Power => "Engine power",
            UnitKind::Thrust => "Engine thrust",
            UnitKind::Mass => "Mass",
            UnitKind::Area => "Wing area",
        }
    }

    pub fn options(self) -> &'static [UnitDef] {
        match self {
            UnitKind::Speed => &SPEED_UNITS,
            UnitKind::Altitude => &ALTITUDE_UNITS,
            UnitKind::Distance => &DISTANCE_UNITS,
            UnitKind::Climb => &CLIMB_UNITS,
            UnitKind::Power => &POWER_UNITS,
            UnitKind::Thrust => &THRUST_UNITS,
            UnitKind::Mass => &MASS_UNITS,
            UnitKind::Area => &AREA_UNITS,
        }
    }

    fn index(self) -> usize {
        UnitKind::ALL.iter().position(|k| *k == self).unwrap()
    }
}

/// The display unit picked for each kind of quantity. Everything is stored
/// in fixed units (m, m/s, hp, kgf, kg, sq m), so changing a unit only
/// changes what's shown and typed: presets convert by themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Units {
    choice: [u8; 8],
}

impl Default for Units {
    fn default() -> Self {
        Units::METRIC
    }
}

impl Units {
    pub const METRIC: Units = Units {
        choice: [0, 0, 0, 0, 0, 0, 0, 0],
    };
    pub const IMPERIAL: Units = Units {
        choice: [1, 1, 1, 1, 0, 1, 1, 1],
    };
    pub const NAUTICAL: Units = Units {
        choice: [2, 1, 2, 1, 0, 1, 1, 1],
    };

    pub fn get(self, kind: UnitKind) -> &'static UnitDef {
        let options = kind.options();
        &options[(self.choice[kind.index()] as usize).min(options.len() - 1)]
    }

    /// Pick a kind's unit by its place in `kind.options()`.
    pub fn set(&mut self, kind: UnitKind, i: u8) {
        self.choice[kind.index()] = i.min(kind.options().len() as u8 - 1);
    }

    /// The place of a kind's unit in `kind.options()`.
    pub fn index(self, kind: UnitKind) -> usize {
        (self.choice[kind.index()] as usize).min(kind.options().len() - 1)
    }

    /// Unit name and factor from the stored unit, for editing.
    pub fn unit(self, kind: UnitKind) -> (&'static str, f32) {
        let d = self.get(kind);
        (d.name, d.per_base)
    }

    /// Format a stored value with its unit.
    pub fn fmt(self, kind: UnitKind, value: f32) -> String {
        let d = self.get(kind);
        format!("{:.*} {}", d.decimals, value * d.per_base, d.name)
    }

    /// Format a speed given in m/s.
    pub fn speed(self, mps: f32) -> String {
        self.fmt(UnitKind::Speed, mps)
    }

    /// Format a distance or range given in meters.
    pub fn range(self, m: f32) -> String {
        self.fmt(UnitKind::Distance, m)
    }

    /// Format an altitude given in meters.
    pub fn altitude(self, m: f32) -> String {
        self.fmt(UnitKind::Altitude, m)
    }

    pub fn speed_unit(self) -> (&'static str, f32) {
        self.unit(UnitKind::Speed)
    }

    pub fn climb_unit(self) -> (&'static str, f32) {
        self.unit(UnitKind::Climb)
    }

    pub fn altitude_unit(self) -> (&'static str, f32) {
        self.unit(UnitKind::Altitude)
    }
}

// Saved as {"Speed": "km/h", ...}. Older saves had one word for everything
// ("Metric", "Imperial" or "Nautical"), which still loads.
impl Serialize for Units {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let map: BTreeMap<String, &str> = UnitKind::ALL
            .iter()
            .map(|k| (format!("{k:?}"), self.get(*k).name))
            .collect();
        map.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Units {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Saved {
            Old(String),
            New(BTreeMap<String, String>),
        }
        Ok(match Saved::deserialize(d)? {
            Saved::Old(name) => match name.as_str() {
                "Imperial" => Units::IMPERIAL,
                "Nautical" => Units::NAUTICAL,
                _ => Units::METRIC,
            },
            Saved::New(map) => {
                let mut units = Units::METRIC;
                for kind in UnitKind::ALL {
                    if let Some(name) = map.get(&format!("{kind:?}"))
                        && let Some(i) = kind.options().iter().position(|o| o.name == name)
                    {
                        units.choice[kind.index()] = i as u8;
                    }
                }
                units
            }
        })
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
    Scoreboard,
}

impl Action {
    pub const ALL: [Action; 11] = [
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
        Action::Scoreboard,
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
            Action::Reset => "Reset (host holds it in multiplayer)",
            Action::Scoreboard => "Scoreboard (hold, multiplayer)",
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
            Action::Scoreboard => KeyCode::Tab,
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
    /// Extra aim sensitivity while zoomed in, multiplied on top.
    pub zoom_sensitivity: f32,
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
    /// Each scenario's start distances (random within each range).
    pub ranges: Ranges,
    /// Show where to aim to hit the target.
    pub lead_indicator: bool,
    /// The target shoots back.
    pub enemy_fires: bool,
    /// One shot from each gun per click, instead of holding to fire.
    pub single_fire: bool,
    /// Your name in multiplayer.
    pub nickname: String,
    /// Your aircraft's paint (the canopy keeps its color).
    pub plane_color: PlaneColor,
    /// Options for multiplayer games you host.
    pub host: crate::multiplayer::HostOptions,
    /// The address you last joined, and the port you last hosted on.
    pub join_address: String,
    pub host_port: u16,
    /// How many bundles of built-in presets have been added already, so a
    /// preset you delete doesn't come back.
    pub bundled_presets: u32,
}

/// Paint colors for your aircraft.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PlaneColor {
    #[default]
    Red,
    Orange,
    Yellow,
    Green,
    Olive,
    Blue,
    Navy,
    Purple,
    Gray,
    Black,
    White,
}

impl PlaneColor {
    pub const ALL: [PlaneColor; 11] = [
        PlaneColor::Red,
        PlaneColor::Orange,
        PlaneColor::Yellow,
        PlaneColor::Green,
        PlaneColor::Olive,
        PlaneColor::Blue,
        PlaneColor::Navy,
        PlaneColor::Purple,
        PlaneColor::Gray,
        PlaneColor::Black,
        PlaneColor::White,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PlaneColor::Red => "Red",
            PlaneColor::Orange => "Orange",
            PlaneColor::Yellow => "Yellow",
            PlaneColor::Green => "Green",
            PlaneColor::Olive => "Olive drab",
            PlaneColor::Blue => "Blue",
            PlaneColor::Navy => "Navy",
            PlaneColor::Purple => "Purple",
            PlaneColor::Gray => "Gray",
            PlaneColor::Black => "Black",
            PlaneColor::White => "White",
        }
    }

    pub fn color(self) -> Color {
        match self {
            PlaneColor::Red => Color::srgb(0.85, 0.16, 0.12),
            PlaneColor::Orange => Color::srgb(0.92, 0.45, 0.1),
            PlaneColor::Yellow => Color::srgb(0.9, 0.78, 0.15),
            PlaneColor::Green => Color::srgb(0.2, 0.55, 0.22),
            PlaneColor::Olive => Color::srgb(0.33, 0.36, 0.2),
            PlaneColor::Blue => Color::srgb(0.2, 0.42, 0.85),
            PlaneColor::Navy => Color::srgb(0.1, 0.16, 0.38),
            PlaneColor::Purple => Color::srgb(0.48, 0.2, 0.7),
            PlaneColor::Gray => Color::srgb(0.5, 0.52, 0.55),
            PlaneColor::Black => Color::srgb(0.07, 0.07, 0.08),
            PlaneColor::White => Color::srgb(0.9, 0.9, 0.9),
        }
    }
}

/// Practice scenarios.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scenario {
    HeadOn,
    #[default]
    Chase,
    Stall,
    Dogfight,
    Banking,
    Diving,
    /// A random one of the others each time.
    AllInOne,
}

impl Scenario {
    pub const ALL: [Scenario; 7] = [
        Scenario::HeadOn,
        Scenario::Chase,
        Scenario::Stall,
        Scenario::Dogfight,
        Scenario::Banking,
        Scenario::Diving,
        Scenario::AllInOne,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Scenario::HeadOn => "Head on",
            Scenario::Chase => "Chase",
            Scenario::Stall => "Stall",
            Scenario::Dogfight => "Dogfight",
            Scenario::Banking => "Banking",
            Scenario::Diving => "Diving",
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
            Scenario::Banking => {
                "The target starts ahead of you, off to one side and above, below or \
                 level, then pulls a hard banked turn: usually across your nose, \
                 sometimes away. Each run takes a different path across the screen: \
                 low to high, high to low, or straight across."
            }
            Scenario::Diving => {
                "You start above the target and dive on it. Some targets haven't seen \
                 you and fly straight until you open fire; others spot you as you dive \
                 in. Once alerted it dives away and runs, jinking, rolling or \
                 corkscrewing."
            }
            Scenario::AllInOne => {
                "A random scenario every time it resets, using each scenario's own \
                 start distances."
            }
        }
    }
}

impl Scenario {
    /// The start distances you can set for this scenario.
    pub fn ranges(self) -> &'static [RangeId] {
        match self {
            Scenario::HeadOn => &[RangeId::HeadOn],
            Scenario::Chase => &[RangeId::Chase],
            Scenario::Stall => &[RangeId::Stall],
            Scenario::Dogfight => &[RangeId::Dogfight],
            Scenario::Banking => &[RangeId::Banking],
            Scenario::Diving => &[RangeId::Diving, RangeId::DivingHeight],
            Scenario::AllInOne => &[],
        }
    }
}

/// A min..max range, m.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Span {
    pub min: f32,
    pub max: f32,
}

const fn span(min: f32, max: f32) -> Span {
    Span { min, max }
}

/// Each scenario's start distance range (and the diving height).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ranges {
    pub head_on: Span,
    pub chase: Span,
    pub stall: Span,
    pub dogfight: Span,
    pub banking: Span,
    pub diving: Span,
    pub diving_height: Span,
}

impl Default for Ranges {
    fn default() -> Self {
        Ranges {
            head_on: span(1800.0, 2600.0),
            chase: span(300.0, 800.0),
            stall: span(100.0, 200.0),
            dogfight: span(400.0, 1000.0),
            banking: span(500.0, 1300.0),
            diving: span(800.0, 2000.0),
            diving_height: span(300.0, 1200.0),
        }
    }
}

/// One settable range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeId {
    HeadOn,
    Chase,
    Stall,
    Dogfight,
    Banking,
    Diving,
    DivingHeight,
}

/// Limits and step for start distances, m.
pub const RANGE_MIN: f32 = 50.0;
pub const RANGE_MAX: f32 = 5000.0;
pub const RANGE_STEP: f32 = 50.0;

impl RangeId {
    pub fn label(self) -> &'static str {
        match self {
            RangeId::HeadOn => "Target starts this far ahead",
            RangeId::Chase => "Target starts this far ahead",
            RangeId::Stall => "Target starts this far ahead",
            RangeId::Dogfight => "Target starts this far away",
            RangeId::Banking => "Target starts this far ahead",
            RangeId::Diving => "Target starts this far ahead",
            RangeId::DivingHeight => "You start this high above it",
        }
    }

    /// Whether this is a height (shown in altitude units).
    pub fn is_height(self) -> bool {
        self == RangeId::DivingHeight
    }
}

impl Ranges {
    pub fn get(&self, id: RangeId) -> Span {
        *self.get_ref(id)
    }

    fn get_ref(&self, id: RangeId) -> &Span {
        match id {
            RangeId::HeadOn => &self.head_on,
            RangeId::Chase => &self.chase,
            RangeId::Stall => &self.stall,
            RangeId::Dogfight => &self.dogfight,
            RangeId::Banking => &self.banking,
            RangeId::Diving => &self.diving,
            RangeId::DivingHeight => &self.diving_height,
        }
    }

    /// Move one end (0 near, 1 far) by some steps, keeping min <= max.
    pub fn step(&mut self, id: RangeId, end: u8, steps: i32) {
        let r = match id {
            RangeId::HeadOn => &mut self.head_on,
            RangeId::Chase => &mut self.chase,
            RangeId::Stall => &mut self.stall,
            RangeId::Dogfight => &mut self.dogfight,
            RangeId::Banking => &mut self.banking,
            RangeId::Diving => &mut self.diving,
            RangeId::DivingHeight => &mut self.diving_height,
        };
        let d = steps as f32 * RANGE_STEP;
        if end == 0 {
            r.min = (r.min + d).clamp(RANGE_MIN, RANGE_MAX);
            r.max = r.max.max(r.min);
        } else {
            r.max = (r.max + d).clamp(RANGE_MIN, RANGE_MAX);
            r.min = r.min.min(r.max);
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            units: Units::METRIC,
            sensitivity: 1.0,
            look_sensitivity: 1.0,
            zoom_sensitivity: 1.0,
            bindings: default_bindings(),
            crosshair: Crosshair::Cross,
            crosshair_color: CrosshairColor::Default,
            aircraft: vec![AircraftCard::yak3()],
            player_aircraft: "Yak-3".into(),
            target_aircraft: "Yak-3".into(),
            guns: vec![GunCard::shvak(), GunCard::ubs()],
            scenario: Scenario::Chase,
            ranges: Ranges::default(),
            lead_indicator: false,
            enemy_fires: false,
            single_fire: false,
            nickname: "Pilot".into(),
            plane_color: PlaneColor::Red,
            host: Default::default(),
            join_address: String::new(),
            host_port: crate::multiplayer::DEFAULT_PORT,
            bundled_presets: 0,
        }
    }
}

/// Gun and aircraft presets that come with the game (beyond the built-in
/// Yak-3 and ShVAK). Bump [`BUNDLE_VERSION`] when adding more.
const BUNDLED_PRESETS: &str = include_str!("../assets/presets.json");
const BUNDLE_VERSION: u32 = 1;

#[derive(Deserialize)]
struct Bundle {
    guns: Vec<GunCard>,
    aircraft: Vec<AircraftCard>,
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
        settings.add_bundled_presets();
        settings.ensure_aircraft();
        settings
    }

    /// Add the presets that ship with the game, once: any you already have
    /// by that name stay as they are.
    pub fn add_bundled_presets(&mut self) {
        if self.bundled_presets >= BUNDLE_VERSION {
            return;
        }
        match serde_json::from_str::<Bundle>(BUNDLED_PRESETS) {
            Ok(bundle) => {
                for g in bundle.guns {
                    if !self.guns.iter().any(|x| x.name == g.name) {
                        self.guns.push(g);
                    }
                }
                for a in bundle.aircraft {
                    if !self.aircraft.iter().any(|x| x.name == a.name) {
                        self.aircraft.push(a);
                    }
                }
            }
            Err(e) => warn!("Bundled presets are unreadable: {e}"),
        }
        self.bundled_presets = BUNDLE_VERSION;
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
            units: Units::NAUTICAL,
            sensitivity: 1.7,
            ..Default::default()
        };
        s.rebind(Action::FreeLook, KeyCode::KeyC);
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.units, Units::NAUTICAL);
        assert_eq!(back.key(Action::FreeLook), KeyCode::KeyC);
        assert!((back.sensitivity - 1.7).abs() < 1e-6);
    }

    #[test]
    fn units_load_old_and_new_saves() {
        let old: Settings = serde_json::from_str(r#"{"units": "Imperial"}"#).unwrap();
        assert_eq!(old.units.speed(100.0), "224 mph");
        let mut u = Units::METRIC;
        u.set(UnitKind::Distance, 3);
        let s = Settings {
            units: u,
            ..Default::default()
        };
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.units.range(1234.0), "1234 m");
        assert_eq!(back.units.speed(100.0), "360 km/h");
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
