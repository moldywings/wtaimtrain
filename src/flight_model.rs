//! Aircraft flight model.
//!
//! An [`AircraftCard`] holds the figures you'd read off a stat card: engine
//! power (or thrust), engine count, climb rate, sustained turn time and top
//! speed, plus a few optional extras. [`FlightModel::from_card`] fills in the
//! blanks (drag, induced drag and propeller efficiency) by solving for the
//! values that reproduce those figures, and [`step`] flies the result with
//! lift, drag, thrust, gravity and stalls.
//!
//! Axes follow Bevy: forward is -Z, up is +Y, right is +X.

use bevy::math::{Quat, Vec3};
use serde::{Deserialize, Serialize};

pub const G: f32 = 9.81;
const KGF: f32 = 9.80665;
const HP: f32 = 745.7;
/// One knot in m/s.
pub const KT: f32 = 0.514_444;

// ---------------------------------------------------------------------------
// Stat card
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineKind {
    /// Piston engine and propeller; `power` is horsepower per engine.
    Prop,
    /// Jet engine; `power` is thrust in kgf per engine.
    Jet,
}

/// The figures a player enters. Everything is stored in SI units.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AircraftCard {
    pub name: String,
    pub engine: EngineKind,
    /// Horsepower (prop) or kgf of thrust (jet), per engine.
    pub power: f32,
    pub engines: u32,
    /// Best climb rate at sea level, m/s.
    pub climb_rate: f32,
    /// Max sustained turn rate at sea level, degrees per second, flown at
    /// `sustained_turn_speed` (m/s).
    pub sustained_turn_rate: f32,
    pub sustained_turn_speed: f32,
    /// Max instantaneous turn rate at sea level, degrees per second, at
    /// `instant_turn_speed` (m/s). Sets the max lift and the G limit.
    pub instant_turn_rate: f32,
    pub instant_turn_speed: f32,
    /// Top speed, m/s, reached at `top_speed_altitude`.
    pub top_speed: f32,
    // Optional extras. Leave them out (None) and they're estimated from
    // the rest of the card; see [`AircraftCard::extras`].
    /// Altitude of the top speed figure, m.
    pub top_speed_altitude: Option<f32>,
    /// Typical combat mass, kg.
    pub mass: Option<f32>,
    /// Wing area, m².
    pub wing_area: Option<f32>,
    /// Maximum lift coefficient (sets stall speed).
    pub cl_max: Option<f32>,
    /// Best full-stick roll rate, degrees per second.
    pub roll_rate: Option<f32>,
    /// Airspeed of that best roll rate, m/s. Below it the ailerons have less
    /// bite; above it they slowly stiffen.
    pub roll_speed: Option<f32>,
    /// Guns and where they're mounted.
    pub weapons: crate::weapons::Loadout,
}

/// The optional figures with every blank filled in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extras {
    pub top_speed_altitude: f32,
    pub mass: f32,
    pub wing_area: f32,
    pub cl_max: f32,
    pub roll_rate: f32,
    pub roll_speed: f32,
}

impl Default for AircraftCard {
    fn default() -> Self {
        AircraftCard::yak3()
    }
}

impl AircraftCard {
    /// The default flight model, from War Thunder's Yak-3 figures.
    pub fn yak3() -> Self {
        AircraftCard {
            name: "Yak-3".into(),
            engine: EngineKind::Prop,
            power: 1320.0,
            engines: 1,
            climb_rate: 4193.0 * 0.3048 / 60.0, // 4193 ft/min
            sustained_turn_rate: 23.59,
            sustained_turn_speed: 175.0 * KT,
            instant_turn_rate: 41.1,
            instant_turn_speed: 302.0 * KT,
            top_speed: 352.0 * 0.514_444, // 352 kt
            top_speed_altitude: Some(4000.0),
            mass: Some(2464.0),
            wing_area: Some(14.83), // from 166.17 kg/m² wing loading
            // Max lift comes from the instantaneous turn instead.
            cl_max: None,
            roll_rate: Some(165.0),
            roll_speed: Some(480.0 / 3.6),
            weapons: crate::weapons::Loadout::default(),
        }
    }

    /// Fill in any blank optional figures with typical values for the
    /// aircraft's type and power, based on WWII fighters and early jets.
    pub fn extras(&self) -> Extras {
        let engines = self.engines.max(1) as f32;
        let output = self.power.max(1.0) * engines;
        let (kg_per_unit, wing_loading, cl_max, roll, alt) = match self.engine {
            // kg per horsepower; twins carry more airframe per hp.
            EngineKind::Prop => (
                if engines > 1.0 { 2.4 } else { 1.9 },
                175.0,
                1.35,
                120.0,
                5000.0,
            ),
            // kg per kgf of thrust.
            EngineKind::Jet => (1.85, 260.0, 1.15, 150.0, 0.0),
        };
        let mass = self.mass.unwrap_or(output * kg_per_unit).max(100.0);
        let wing_area = self.wing_area.unwrap_or(mass / wing_loading).max(1.0);
        // The instantaneous turn is a max-lift (or G-limited) pull, so it
        // tells us the max lift coefficient.
        // The sustained turn needs at least that much lift too (it's often
        // flown right at the edge of the stall).
        let cl_needed = |rate_deg: f32, v: f32| {
            let q = 0.5 * air_density(0.0) * v.max(10.0).powi(2);
            load_factor(rate_deg.max(0.0).to_radians(), v) * mass * G / (q * wing_area)
        };
        let cl_from_turn = if self.instant_turn_rate > 0.0 && self.instant_turn_speed > 10.0 {
            Some(
                cl_needed(self.instant_turn_rate, self.instant_turn_speed)
                    .max(1.02 * cl_needed(self.sustained_turn_rate, self.sustained_turn_speed)),
            )
        } else {
            None
        };
        Extras {
            top_speed_altitude: self.top_speed_altitude.unwrap_or(alt).max(0.0),
            mass,
            wing_area,
            cl_max: self
                .cl_max
                .or(cl_from_turn)
                .unwrap_or(cl_max)
                .clamp(0.6, 2.5),
            roll_rate: self.roll_rate.unwrap_or(roll).max(10.0),
            // Typically a bit over half the top speed.
            roll_speed: self.roll_speed.unwrap_or(0.6 * self.top_speed).max(20.0),
        }
    }

    /// Load factor in the card's instantaneous turn.
    pub fn instant_turn_g(&self) -> f32 {
        load_factor(self.instant_turn_rate.to_radians(), self.instant_turn_speed)
    }
}

/// Load factor for a level turn at `rate` (rad/s) and speed `v` (m/s).
pub fn load_factor(rate: f32, v: f32) -> f32 {
    (1.0 + (v * rate / G).powi(2)).sqrt()
}

// ---------------------------------------------------------------------------
// Atmosphere
// ---------------------------------------------------------------------------

/// Air density (kg/m³) at altitude `h` meters, International Standard Atmosphere.
pub fn air_density(h: f32) -> f32 {
    let h = h.clamp(0.0, 11_000.0);
    let t = 288.15 - 0.0065 * h;
    1.225 * (t / 288.15).powf(4.2559)
}

// ---------------------------------------------------------------------------
// Derived model
// ---------------------------------------------------------------------------

/// Everything the physics needs, derived from a card.
#[derive(Clone, Debug)]
pub struct FlightModel {
    pub name: String,
    pub engine: EngineKind,
    pub mass: f32,
    pub wing_area: f32,
    /// Total shaft power (W) for props, total thrust (N) for jets.
    pub power: f32,
    /// Propeller peak efficiency (props) or thrust multiplier (jets). Solved.
    pub efficiency: f32,
    /// Static thrust cap for props, N.
    pub static_thrust: f32,
    /// Altitude up to which engine output holds (supercharger), m.
    pub critical_altitude: f32,
    /// Zero-lift drag coefficient. Solved.
    pub cd0: f32,
    /// Induced drag factor (CD = cd0 + k·CL²). Solved.
    pub k: f32,
    /// Lift curve slope, per radian.
    pub cl_alpha: f32,
    pub cl_max: f32,
    /// Angle of attack at which the wing stalls, radians.
    pub alpha_stall: f32,
    /// Best full-stick roll rate, rad/s.
    pub roll_rate: f32,
    /// Airspeed of the best roll rate, m/s.
    pub roll_speed: f32,
    /// Structural/elevator G limit.
    pub g_limit: f32,
    /// How well the solved model matches the card (predicted figures).
    pub check: Performance,
}

/// Predicted performance figures, for comparing against the card.
#[derive(Clone, Copy, Debug, Default)]
pub struct Performance {
    /// Top speed at the card's altitude, m/s.
    pub top_speed: f32,
    /// Top speed at sea level, m/s.
    pub top_speed_sea_level: f32,
    /// Best sea-level climb rate, m/s.
    pub climb_rate: f32,
    /// Sustained turn rate at the card's speed, sea level, deg/s.
    pub sustained_turn_rate: f32,
    /// Instantaneous turn rate at the card's speed, sea level, deg/s.
    pub instant_turn_rate: f32,
    /// Best sustained 360° turn time at sea level, s.
    pub turn_time: f32,
    /// 1 G stall speed at sea level, m/s.
    pub stall_speed: f32,
}

/// Largest pull (and push) on the stick that stays just short of the stall,
/// as stick fractions (both positive). Used to keep keyboard pitch inside
/// what the instructor allows.
pub fn safe_pitch_stick(m: &FlightModel, b: &Body) -> (f32, f32) {
    let v = b.speed();
    let q = 0.5 * air_density(b.position.y.max(0.0)) * v * v;
    let alpha_full = ELEVATOR_ALPHA * m.alpha_stall;
    let alpha_g_limit = if q > 1.0 {
        m.g_limit * m.mass * G / (q * m.wing_area * m.cl_alpha)
    } else {
        alpha_full
    };
    let alpha_max = alpha_full.min(alpha_g_limit).max(1e-4);
    let safe = 0.85 * m.alpha_stall;
    (
        (safe / alpha_max).min(1.0),
        (safe / (0.6 * alpha_max)).min(1.0),
    )
}

/// Speed at which pitch and yaw controls reach full crispness, m/s.
const ROLL_REF_SPEED: f32 = 400.0 / 3.6;

/// Full-stick roll rate (rad/s) at airspeed `v`: it builds up with speed
/// to the card's best rate at its roll speed, then falls off gently (to
/// about three quarters at 1.6x that speed) as the ailerons stiffen.
pub fn roll_authority(m: &FlightModel, v: f32) -> f32 {
    let r = v / m.roll_speed.max(1.0);
    let build = r.min(1.0);
    let over = (r - 1.0).max(0.0);
    m.roll_rate * build / (1.0 + 0.9 * over * over)
}
const ASPECT_RATIO: f32 = 6.0;

impl FlightModel {
    pub fn from_card(card: &AircraftCard) -> FlightModel {
        let extras = card.extras();
        let engines = card.engines.max(1) as f32;
        let (power, static_thrust, critical_altitude) = match card.engine {
            EngineKind::Prop => {
                let hp = card.power.max(1.0) * engines;
                // Roughly 0.8 kgf of static thrust per horsepower.
                (hp * HP, 0.8 * KGF * hp, extras.top_speed_altitude)
            }
            EngineKind::Jet => {
                let t = card.power.max(1.0) * engines * KGF;
                (t, t, 0.0)
            }
        };
        let cl_alpha =
            std::f32::consts::TAU * ASPECT_RATIO / (2.0 + (ASPECT_RATIO.powi(2) + 4.0).sqrt());
        let cl_max = extras.cl_max;

        let mut m = FlightModel {
            name: card.name.clone(),
            engine: card.engine,
            mass: extras.mass,
            wing_area: extras.wing_area,
            power,
            efficiency: 0.8,
            static_thrust,
            critical_altitude,
            cd0: 0.02,
            k: 1.0 / (std::f32::consts::PI * 0.8 * ASPECT_RATIO),
            cl_alpha,
            cl_max,
            alpha_stall: cl_max / cl_alpha,
            roll_rate: extras.roll_rate.to_radians(),
            roll_speed: extras.roll_speed,
            // The instantaneous turn's G, but no less than a typical
            // fighter's limit... unless the card's pull is gentler.
            g_limit: card.instant_turn_g().clamp(4.0, 15.0),
            check: Performance::default(),
        };
        m.solve(card);
        m.check = m.performance(card);
        m
    }

    /// Find efficiency, cd0 and k so the model hits the card's top speed,
    /// climb rate and turn time.
    fn solve(&mut self, card: &AircraftCard) {
        let target_climb = card.climb_rate.max(0.5);
        let target_turn = card.sustained_turn_rate.max(1.0).to_radians();
        let turn_speed = card.sustained_turn_speed.max(30.0);
        for _ in 0..30 {
            // Efficiency from climb rate (cd0 follows from top speed).
            self.efficiency = bisect(0.2, 1.5, |e| {
                let mut m = self.clone();
                m.efficiency = e;
                m.fit_cd0(card);
                m.best_climb(0.0) - target_climb
            });
            self.fit_cd0(card);
            // Induced drag from the sustained turn: at that speed and G,
            // thrust just balances drag.
            let n = load_factor(target_turn, turn_speed);
            self.k = bisect(0.01, 0.6, |k| {
                let mut m = self.clone();
                m.k = k;
                m.fit_cd0(card);
                m.drag(turn_speed, 0.0, n) - m.thrust(turn_speed, 0.0, 1.0)
            });
            self.fit_cd0(card);
        }
    }

    /// Pick cd0 so thrust equals drag at the card's top speed and altitude.
    fn fit_cd0(&mut self, card: &AircraftCard) {
        let h = card.extras().top_speed_altitude;
        let v = card.top_speed.max(30.0);
        let q = 0.5 * air_density(h) * v * v;
        let s = self.wing_area;
        let w = self.mass * G;
        let induced = self.k * (w / (q * s)).powi(2);
        self.cd0 = (self.thrust(v, h, 1.0) / (q * s) - induced).max(0.004);
    }

    /// Engine output at altitude: constant up to the critical altitude, then
    /// falling with air density.
    fn altitude_factor(&self, h: f32) -> f32 {
        match self.engine {
            EngineKind::Prop => {
                if h <= self.critical_altitude {
                    1.0
                } else {
                    air_density(h) / air_density(self.critical_altitude)
                }
            }
            EngineKind::Jet => (air_density(h) / 1.225).powf(0.8),
        }
    }

    /// Thrust in newtons at airspeed `v`, altitude `h`, throttle 0..=1.
    pub fn thrust(&self, v: f32, h: f32, throttle: f32) -> f32 {
        let f = self.altitude_factor(h) * throttle.clamp(0.0, 1.0);
        match self.engine {
            EngineKind::Prop => {
                let v = v.max(1.0);
                // Propellers are poor at low speed and approach peak efficiency fast.
                let eta = self.efficiency * (1.0 - (-v / 35.0).exp());
                let from_power = eta * self.power * f / v;
                let cap = self.static_thrust * f * (1.0 + v / 120.0);
                from_power.min(cap)
            }
            EngineKind::Jet => self.efficiency * self.power * f,
        }
    }

    /// Drag in newtons for level flight at load factor `n`.
    fn drag(&self, v: f32, h: f32, n: f32) -> f32 {
        let q = 0.5 * air_density(h) * v * v;
        let cl = n * self.mass * G / (q * self.wing_area);
        q * self.wing_area * (self.cd0 + self.k * cl * cl)
    }

    pub fn stall_speed(&self, h: f32) -> f32 {
        (2.0 * self.mass * G / (air_density(h) * self.wing_area * self.cl_max)).sqrt()
    }

    /// Best climb rate (m/s) at altitude `h`.
    pub fn best_climb(&self, h: f32) -> f32 {
        let vs = self.stall_speed(h);
        let mut best = f32::MIN;
        let mut v = vs * 1.1;
        while v < 320.0 {
            let excess = (self.thrust(v, h, 1.0) - self.drag(v, h, 1.0)) * v;
            best = best.max(excess / (self.mass * G));
            v += 2.0;
        }
        best
    }

    /// Sustained turn rate (rad/s) at speed `v` and altitude `h`: the
    /// tightest turn where thrust still equals drag.
    pub fn sustained_turn_rate_at(&self, v: f32, h: f32) -> f32 {
        let q = 0.5 * air_density(h) * v * v;
        let s = self.wing_area;
        let t = self.thrust(v, h, 1.0);
        let cl_thrust = ((t / (q * s) - self.cd0) / self.k).max(0.0).sqrt();
        let n = (cl_thrust.min(self.cl_max) * q * s / (self.mass * G)).min(self.g_limit);
        if n > 1.0 {
            G * (n * n - 1.0).sqrt() / v
        } else {
            0.0
        }
    }

    /// Instantaneous turn rate (rad/s) at speed `v` and altitude `h`: max
    /// lift or the G limit, whichever comes first.
    pub fn instant_turn_rate_at(&self, v: f32, h: f32) -> f32 {
        let q = 0.5 * air_density(h) * v * v;
        let n = (self.cl_max * q * self.wing_area / (self.mass * G)).min(self.g_limit);
        if n > 1.0 {
            G * (n * n - 1.0).sqrt() / v
        } else {
            0.0
        }
    }

    /// Best sustained turn rate (rad/s) at altitude `h`.
    pub fn best_sustained_turn_rate(&self, h: f32) -> f32 {
        let rho = air_density(h);
        let s = self.wing_area;
        let w = self.mass * G;
        let mut best = 0.0f32;
        let mut v = self.stall_speed(h);
        while v < 320.0 {
            let q = 0.5 * rho * v * v;
            let t = self.thrust(v, h, 1.0);
            let cl_thrust = ((t / (q * s) - self.cd0) / self.k).max(0.0).sqrt();
            let cl = cl_thrust.min(self.cl_max);
            let n = (cl * q * s / w).min(self.g_limit);
            if n > 1.0 {
                best = best.max(G * (n * n - 1.0).sqrt() / v);
            }
            v += 1.0;
        }
        best
    }

    /// Level top speed (m/s) at altitude `h`.
    pub fn top_speed(&self, h: f32) -> f32 {
        let lo = self.stall_speed(h) * 1.3;
        bisect(lo, 400.0, |v| self.thrust(v, h, 1.0) - self.drag(v, h, 1.0))
    }

    pub fn performance(&self, card: &AircraftCard) -> Performance {
        let top_speed_altitude = card.extras().top_speed_altitude;
        let turn = self.best_sustained_turn_rate(0.0);
        Performance {
            sustained_turn_rate: self
                .sustained_turn_rate_at(card.sustained_turn_speed.max(30.0), 0.0)
                .to_degrees(),
            instant_turn_rate: self
                .instant_turn_rate_at(card.instant_turn_speed.max(30.0), 0.0)
                .to_degrees(),
            top_speed: self.top_speed(top_speed_altitude),
            top_speed_sea_level: self.top_speed(0.0),
            climb_rate: self.best_climb(0.0),
            turn_time: if turn > 0.0 {
                std::f32::consts::TAU / turn
            } else {
                f32::INFINITY
            },
            stall_speed: self.stall_speed(0.0),
        }
    }

    /// Lift coefficient at angle of attack `alpha`, with a stall.
    pub fn lift_coefficient(&self, alpha: f32) -> f32 {
        let a = alpha.abs();
        let linear = self.cl_alpha * a;
        // After the stall the wing behaves more like a flat plate.
        let post = 1.2 * (2.0 * a).sin() * 0.85;
        let blend_width = 3f32.to_radians();
        let cl = if a <= self.alpha_stall {
            linear
        } else if a <= self.alpha_stall + blend_width {
            let t = (a - self.alpha_stall) / blend_width;
            self.cl_max + (post.min(self.cl_max) - self.cl_max) * t
        } else {
            post.min(self.cl_max)
        };
        cl.copysign(alpha)
    }

    pub fn is_stalled(&self, alpha: f32) -> bool {
        alpha.abs() > self.alpha_stall
    }
}

/// Find x in [lo, hi] where f(x) changes sign from negative to positive
/// (or the nearest end if it doesn't).
fn bisect(mut lo: f32, mut hi: f32, f: impl Fn(f32) -> f32) -> f32 {
    let flo = f(lo);
    let fhi = f(hi);
    if flo >= 0.0 && fhi >= 0.0 {
        return lo;
    }
    if flo <= 0.0 && fhi <= 0.0 {
        return hi;
    }
    let rising = fhi > flo;
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        let fm = f(mid);
        if (fm < 0.0) == rising {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

// ---------------------------------------------------------------------------
// Physics
// ---------------------------------------------------------------------------

/// Control inputs, each -1..=1. Positive pitch is nose up, positive roll is
/// right wing down, positive yaw is nose right.
#[derive(Clone, Copy, Debug, Default)]
pub struct Controls {
    pub pitch: f32,
    pub roll: f32,
    pub yaw: f32,
    pub throttle: f32,
}

/// The moving state of one aircraft.
#[derive(Clone, Copy, Debug)]
pub struct Body {
    pub position: Vec3,
    pub rotation: Quat,
    /// World velocity, m/s.
    pub velocity: Vec3,
    /// Body angular velocity (x pitch up, y yaw left, z roll left), rad/s.
    pub omega: Vec3,
    /// Readouts from the last step.
    pub alpha: f32,
    pub g_load: f32,
    /// Instructor's slowly built-up pitch correction (integral term), rad.
    pub trim: f32,
    /// The instructor's smoothed copy of the aim direction (zero until the
    /// first call), and how fast it's turning (world rad/s).
    pub aim_smoothed: Vec3,
    pub aim_rate: Vec3,
}

impl Body {
    pub fn new(position: Vec3, rotation: Quat, speed: f32) -> Body {
        Body {
            position,
            rotation,
            velocity: rotation * Vec3::NEG_Z * speed,
            omega: Vec3::ZERO,
            alpha: 0.0,
            g_load: 1.0,
            trim: 0.0,
            aim_smoothed: Vec3::ZERO,
            aim_rate: Vec3::ZERO,
        }
    }

    pub fn speed(&self) -> f32 {
        self.velocity.length()
    }

    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::NEG_Z
    }
}

/// Angle of attack and sideslip (radians) from the velocity in body axes.
pub fn alpha_beta(rotation: Quat, velocity: Vec3) -> (f32, f32) {
    let vb = rotation.inverse() * velocity;
    if vb.length_squared() < 1.0 {
        return (0.0, 0.0);
    }
    let alpha = (-vb.y).atan2(-vb.z);
    let beta = vb.x.atan2(-vb.z);
    (alpha, beta)
}

/// How far full elevator can push the angle of attack, as a multiple of the
/// stall angle. A bit over 1 so you can stall it by yanking at low speed.
const ELEVATOR_ALPHA: f32 = 1.2;
const RUDDER_BETA: f32 = 0.12;

/// Advance one aircraft by `dt` seconds. Splits large steps for stability.
pub fn step(model: &FlightModel, body: &mut Body, controls: &Controls, dt: f32) {
    let steps = (dt / (1.0 / 240.0)).ceil().max(1.0) as usize;
    let h = dt / steps as f32;
    for _ in 0..steps {
        substep(model, body, controls, h);
    }
}

fn substep(m: &FlightModel, b: &mut Body, c: &Controls, dt: f32) {
    let rho = air_density(b.position.y);
    let v = b.velocity.length();
    let q = 0.5 * rho * v * v;
    let s = m.wing_area;
    let vhat = if v > 0.1 { b.velocity / v } else { b.forward() };
    let (alpha, beta) = alpha_beta(b.rotation, b.velocity);

    let up = b.rotation * Vec3::Y;
    let right = b.rotation * Vec3::X;
    let fwd = b.rotation * Vec3::NEG_Z;

    // --- Forces ---
    let cl = m.lift_coefficient(alpha);
    let stalled = m.is_stalled(alpha);
    let mut cd = m.cd0 + m.k * cl * cl;
    if stalled {
        cd += 1.1 * alpha.sin().powi(2);
    }
    cd += 0.5 * beta.sin().powi(2);
    let lift_dir = (up - vhat * up.dot(vhat)).normalize_or_zero();
    let side_dir = (right - vhat * right.dot(vhat)).normalize_or_zero();
    let lift = lift_dir * (q * s * cl);
    let drag = -vhat * (q * s * cd);
    let side = side_dir * (-q * s * 0.8 * beta);
    let thrust = fwd * m.thrust(v, b.position.y, c.throttle);
    let gravity = Vec3::NEG_Y * (m.mass * G);
    let accel = (lift + drag + side + thrust + gravity) / m.mass;

    b.g_load = (lift + side + thrust + drag).dot(up) / (m.mass * G);
    b.alpha = alpha;

    // Rotation of the flight path, so damping acts on the nose relative to
    // where the plane is going.
    let accel_perp = accel - vhat * accel.dot(vhat);
    let path_rate_world = if v > 1.0 {
        vhat.cross(accel_perp) / v
    } else {
        Vec3::ZERO
    };
    let path_rate = b.rotation.inverse() * path_rate_world;

    // --- Rotation ---
    // Control power grows with airspeed: mushy when slow, crisp when fast.
    let speed_ratio = (v / ROLL_REF_SPEED).clamp(0.0, 2.5);
    let wn = (4.5 * speed_ratio).max(0.3); // pitch/yaw stiffness, rad/s
    let zeta = 0.75;

    // Elevator sets an angle of attack to hold, limited so a full pull
    // can't overstress the airframe at high speed.
    let alpha_full = ELEVATOR_ALPHA * m.alpha_stall;
    let alpha_g_limit = if q > 1.0 {
        m.g_limit * m.mass * G / (q * s * m.cl_alpha)
    } else {
        alpha_full
    };
    let alpha_max = alpha_full.min(alpha_g_limit);
    let alpha_cmd = if c.pitch >= 0.0 {
        c.pitch * alpha_max
    } else {
        c.pitch * alpha_max * 0.6
    };
    let pitch_acc = wn * wn * (alpha_cmd - alpha) - 2.0 * zeta * wn * (b.omega.x - path_rate.x);

    // Rudder sets a sideslip to hold; the tail weathervanes the nose.
    let beta_cmd = -c.yaw * RUDDER_BETA;
    let yaw_acc =
        -wn * wn * 0.6 * (beta - beta_cmd) - 2.0 * zeta * wn * 0.8 * (b.omega.y - path_rate.y);

    // Roll: first-order response toward a commanded rate that grows with
    // speed and stiffens at high speed. Stalled wings lose most authority
    // and tend to drop.
    let mut roll_authority = roll_authority(m, v);
    let mut wing_drop = 0.0;
    if stalled {
        let depth = ((alpha.abs() - m.alpha_stall) / m.alpha_stall).clamp(0.0, 1.0);
        roll_authority *= 1.0 - 0.7 * depth;
        // The wing on the side the plane is slipping toward drops first.
        let side = if beta.abs() > 0.005 {
            beta.signum()
        } else {
            -b.omega.y.signum()
        };
        wing_drop = -side * depth * 2.0 * speed_ratio.max(0.3);
    }
    let roll_cmd = -c.roll * roll_authority + wing_drop;
    let tau = (0.22 / speed_ratio.max(0.2)).clamp(0.12, 0.8);
    let roll_acc = (roll_cmd - b.omega.z) / tau;

    b.omega += Vec3::new(pitch_acc, yaw_acc, roll_acc) * dt;

    // --- Integrate ---
    b.velocity += accel * dt;
    b.position += b.velocity * dt;
    let w = b.omega;
    let angle = w.length() * dt;
    if angle > 1e-7 {
        b.rotation = (b.rotation * Quat::from_axis_angle(w.normalize(), angle)).normalize();
    }
}

// ---------------------------------------------------------------------------
// Instructor (mouse aim autopilot)
// ---------------------------------------------------------------------------

/// Turn toward `aim` (a world direction) the way War Thunder's instructor
/// does: roll the aim point over the canopy, pull, and level the wings when
/// the nose is on target. Keeps the angle of attack below the stall.
/// `dt` lets it build up a small trim so the nose settles exactly on the aim.
pub fn instructor(model: &FlightModel, body: &mut Body, aim: Vec3, dt: f32) -> Controls {
    instructor_tuned(model, body, aim, dt, InstructorTuning::get())
}

fn instructor_tuned(
    model: &FlightModel,
    body: &mut Body,
    aim: Vec3,
    dt: f32,
    tune: InstructorTuning,
) -> Controls {
    // Mouse input arrives in uneven steps (frames with and without
    // movement). Smooth it briefly so those steps don't kick the controls,
    // and track how fast the aim is moving so the nose can keep up.
    let aim = aim.normalize_or(Vec3::NEG_Z);
    if body.aim_smoothed == Vec3::ZERO || dt <= 0.0 {
        body.aim_smoothed = aim;
        body.aim_rate = Vec3::ZERO;
    } else {
        let k = 1.0 - (-dt / tune.aim_smooth.max(1e-4)).exp();
        let prev = body.aim_smoothed;
        let next = prev.lerp(aim, k).normalize_or(aim);
        let rate = prev.cross(next) / dt;
        let kr = 1.0 - (-dt / tune.rate_smooth.max(1e-4)).exp();
        body.aim_rate = body.aim_rate.lerp(rate, kr);
        body.aim_smoothed = next;
    }
    let aim = body.aim_smoothed;
    let local = body.rotation.inverse() * aim;
    let off_nose = local.angle_between(Vec3::NEG_Z);
    let v = body.speed();
    // Nose rate relative to the aim's own motion, in body axes.
    let aim_rate_body = body.rotation.inverse() * body.aim_rate * tune.aim_feed;
    let rel = body.omega - aim_rate_body;

    // Where the aim will be relative to the nose a moment from now, given
    // how the nose is already swinging. Acting on this instead of the
    // current error stops the nose sailing past the aim.
    let lead = tune.lead;
    let px = local.x + rel.y * lead;
    let py = local.y - rel.x * lead;
    let predicted_off = if off_nose < 0.3 {
        (px * px + py * py).sqrt()
    } else {
        off_nose
    };

    // Roll: far off the nose, bank so the aim point sits above the canopy.
    // Close to the nose, roll the wings level instead.
    let bank_to_target = if off_nose < 0.3 {
        px.atan2(py)
    } else {
        local.x.atan2(local.y)
    };
    let right = body.rotation * Vec3::X;
    // Near the nose, a shallow bank toward the aim nudges the heading over.
    let yaw_err = local.x.atan2(-local.z);
    let wanted_left_bank = (-yaw_err * tune.heading_bank).clamp(-0.3, 0.3);
    let bank_to_level = right.y.clamp(-1.0, 1.0).asin() - wanted_left_bank;
    let w = smoothstep(tune.level_below, tune.bank_above, predicted_off);
    let roll_err = w * bank_to_target + (1.0 - w) * bank_to_level;
    // Roll error -> desired roll rate -> stick, with damping on roll rate.
    let roll_rate_wanted = roll_err * tune.roll_gain;
    let roll_rate_now = -body.omega.z;
    let full = roll_authority(model, v).max(0.3);
    let roll = ((roll_rate_wanted - roll_rate_now * 0.35) / full).clamp(-1.0, 1.0);

    // Pitch: ask for angle of attack in proportion to how far the aim is
    // above the nose, easing off while still rolling into position.
    let pitch_err = local.y.atan2(-local.z);
    let alpha_max = ELEVATOR_ALPHA * model.alpha_stall;
    let roll_lag = (w * bank_to_target.abs() / 1.2).clamp(0.0, 0.85);
    // Feed-forward: the angle of attack that holds the nose up against
    // gravity, so the nose doesn't sag below the aim in level flight.
    let q = 0.5 * air_density(body.position.y) * v * v;
    let up = body.rotation * Vec3::Y;
    let alpha_hold = if q > 50.0 {
        (model.mass * G * up.y.max(0.0) / (q * model.wing_area * model.cl_alpha))
            .min(model.alpha_stall)
    } else {
        0.0
    };
    // Integral: soak up a tiny leftover error once right on target.
    if off_nose < 0.01 {
        body.trim = (body.trim + pitch_err * 1.0 * dt).clamp(-0.02, 0.02);
    } else {
        body.trim *= 1.0 - (4.0 * dt).min(1.0);
    }
    let alpha_wanted = (pitch_err * tune.pitch_gain - rel.x * tune.pitch_damp) * (1.0 - roll_lag)
        + alpha_hold * (1.0 - w)
        + body.trim;
    // Stay just under the stall (the real instructor does the same).
    let alpha_limit = 0.88 * model.alpha_stall;
    let pitch = (alpha_wanted.clamp(-0.5 * alpha_limit, alpha_limit) / alpha_max).clamp(-1.0, 1.0);

    // Rudder puts the nose right on the aim for the last few degrees.
    let yaw = ((yaw_err * tune.rudder_gain + rel.y * tune.rudder_damp)
        * (1.0 - w * tune.rudder_fade))
        .clamp(-1.0, 1.0);
    let large = Controls {
        pitch,
        roll,
        yaw,
        throttle: 0.0,
    };

    // Within a few tens of degrees, fly a coordinated turn instead: one
    // smooth "turn this way at this rate" command sets the bank, the pull
    // and the rudder together, so all three move in step.
    let fine = coordinated(model, body, aim, q, tune, dt);
    let fwd = body.forward();
    let steep = smoothstep(1.1, 1.35, fwd.y.abs().clamp(0.0, 1.0).asin());
    let wl = smoothstep(tune.big_lo, tune.big_hi, off_nose).max(steep);
    if wl <= 0.0 {
        return fine;
    }
    if wl >= 1.0 {
        return large;
    }
    Controls {
        pitch: fine.pitch + (large.pitch - fine.pitch) * wl,
        roll: fine.roll + (large.roll - fine.roll) * wl,
        yaw: fine.yaw + (large.yaw - fine.yaw) * wl,
        throttle: 0.0,
    }
}

/// Coordinated-turn mouse aim for aims near the nose. The heading and
/// climb errors become a wanted turn rate and climb rate; from those come
/// the bank angle for a coordinated turn, the angle of attack for the pull,
/// and the rudder, all as smooth functions of the same command.
fn coordinated(
    model: &FlightModel,
    body: &mut Body,
    aim: Vec3,
    q: f32,
    tune: InstructorTuning,
    dt: f32,
) -> Controls {
    let v = body.speed().max(20.0);
    let fwd = body.forward();
    let right = body.rotation * Vec3::X;
    let up = body.rotation * Vec3::Y;
    let local = body.rotation.inverse() * aim;
    let heading = |d: Vec3| d.x.atan2(-d.z);
    let elev = |d: Vec3| d.y.clamp(-1.0, 1.0).asin();
    let mut psi_err = heading(aim) - heading(fwd);
    if psi_err > std::f32::consts::PI {
        psi_err -= std::f32::consts::TAU;
    } else if psi_err < -std::f32::consts::PI {
        psi_err += std::f32::consts::TAU;
    }
    let theta = elev(fwd);
    let th_err = elev(aim) - theta;

    // How fast the aim itself is moving (right and up).
    let flat = Vec3::new(aim.x, 0.0, aim.z).normalize_or(Vec3::NEG_Z);
    let psi_rate_aim = -body.aim_rate.y * tune.aim_feed;
    let th_rate_aim = body.aim_rate.dot(flat.cross(Vec3::Y)) * tune.aim_feed;

    // The tightest turn available without stalling.
    let w = model.mass * G;
    let n_avail = (0.85 * model.cl_max * q * model.wing_area / w)
        .min(model.g_limit)
        .max(1.0);
    let w_max = G * (n_avail * n_avail - 1.0).sqrt() / v;
    let turn = (tune.kh * psi_err + psi_rate_aim).clamp(-w_max, w_max);
    let climb = (tune.kv * th_err + th_rate_aim).clamp(-w_max, w_max);

    // Bank for a coordinated turn at that rate (right bank positive).
    let phi = (-right.y).atan2(up.y);
    let cos_t = theta.cos().max(0.2);
    // Small corrections need only a little bank; the rudder does the rest.
    // (The bank that follows the aim's own motion isn't capped.)
    let phi_follow = (v * psi_rate_aim.clamp(-w_max, w_max) * cos_t / G).atan();
    let bank_cap = (tune.k_phi_cap * psi_err.abs()).min(1.4);
    let phi_wanted = (phi_follow
        + ((v * turn * cos_t / G).atan() - phi_follow).clamp(-bank_cap, bank_cap))
    .clamp(-1.4, 1.4);
    let roll_rate_wanted = tune.k_bank * (phi_wanted - phi);
    let full = roll_authority(model, v).max(0.3);
    let roll_rate_now = -body.omega.z;
    let roll = ((roll_rate_wanted - roll_rate_now * tune.roll_damp) / full).clamp(-1.0, 1.0);

    // Body pitch and yaw rates that make that turn and climb at the
    // current bank.
    let (sp, cp) = phi.sin_cos();
    let q_body = climb * cp + turn * cos_t * sp;
    let r_body = turn * cos_t * cp - climb * sp;

    // Pull: the angle of attack whose lift gives that pitch rate and holds
    // the nose up against gravity, plus a small correction on the error.
    let pitch_err = local.y.atan2(-local.z);
    if pitch_err.abs() < 0.01 {
        body.trim = (body.trim + pitch_err * tune.trim_gain * dt).clamp(-0.02, 0.02);
    } else {
        body.trim *= 1.0 - (4.0 * dt).min(1.0);
    }
    let alpha_ff = if q > 50.0 {
        model.mass * (v * q_body + G * theta.cos() * cp) / (q * model.wing_area * model.cl_alpha)
    } else {
        0.0
    };
    let alpha_wanted =
        alpha_ff + tune.kq * (q_body - body.omega.x) + tune.k_pitch * pitch_err + body.trim;
    let alpha_limit = 0.88 * model.alpha_stall;
    let alpha_max = ELEVATOR_ALPHA * model.alpha_stall;
    let pitch = (alpha_wanted.clamp(-0.5 * alpha_limit, alpha_limit) / alpha_max).clamp(-1.0, 1.0);

    // Rudder: the yaw part of the turn, plus fine aim on what's left.
    let yaw_err = local.x.atan2(-local.z);
    let yaw_rate_now = -body.omega.y;
    let yaw =
        (tune.k_rudder * yaw_err + tune.k_rudder_rate * (r_body - yaw_rate_now)).clamp(-1.0, 1.0);

    Controls {
        pitch,
        roll,
        yaw,
        throttle: 0.0,
    }
}

/// Instructor gains, kept together so they're easy to tune.
#[derive(Clone, Copy)]
struct InstructorTuning {
    lead: f32,
    heading_bank: f32,
    level_below: f32,
    bank_above: f32,
    roll_gain: f32,
    pitch_gain: f32,
    pitch_damp: f32,
    rudder_gain: f32,
    rudder_damp: f32,
    /// How much the rudder backs off while banking toward a far-off aim.
    rudder_fade: f32,
    /// Time constant for smoothing the aim, s.
    aim_smooth: f32,
    /// Time constant for smoothing the aim's turn rate, s.
    rate_smooth: f32,
    /// How much of the aim's motion to lead (0 = none, 1 = all).
    aim_feed: f32,
    // Coordinated mode (near the nose).
    /// Below `big_lo` off the nose it's all coordinated; above `big_hi`
    /// it's all "roll the aim over the canopy" (radians).
    big_lo: f32,
    big_hi: f32,
    /// Turn and climb rate wanted per radian of error, 1/s.
    kh: f32,
    kv: f32,
    /// Roll rate wanted per radian of bank error, 1/s.
    k_bank: f32,
    roll_damp: f32,
    kq: f32,
    k_pitch: f32,
    trim_gain: f32,
    k_rudder: f32,
    k_rudder_rate: f32,
    /// Bank allowed per radian of heading error.
    k_phi_cap: f32,
}

impl InstructorTuning {
    fn get() -> Self {
        InstructorTuning {
            lead: 0.1,
            heading_bank: 35.0,
            level_below: 0.005,
            bank_above: 0.06,
            roll_gain: 3.0,
            pitch_gain: 4.0,
            pitch_damp: 0.1,
            rudder_gain: 150.0,
            rudder_damp: 8.0,
            rudder_fade: 0.5,
            aim_smooth: 0.16,
            rate_smooth: 0.1,
            aim_feed: 1.0,
            big_lo: 0.35,
            big_hi: 0.7,
            kh: 8.0,
            kv: 10.0,
            k_bank: 2.0,
            roll_damp: 0.35,
            kq: 0.1,
            k_pitch: 5.0,
            trim_gain: 1.0,
            k_rudder: 80.0,
            k_rudder_rate: 8.0,
            k_phi_cap: 16.0,
        }
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kmh(v: f32) -> f32 {
        v * 3.6
    }

    #[test]
    fn yak3_matches_its_stat_card() {
        let card = AircraftCard::yak3();
        let m = FlightModel::from_card(&card);
        let p = m.check;
        println!(
            "eff {:.3} cd0 {:.4} k {:.4} clmax {:.2} glim {:.1} | top {:.0} km/h @4000, {:.0} km/h SL | climb {:.1} m/s | sust {:.2} deg/s, inst {:.1} deg/s, best 360 {:.1} s | stall {:.0} km/h",
            m.efficiency,
            m.cd0,
            m.k,
            m.cl_max,
            m.g_limit,
            kmh(p.top_speed),
            kmh(p.top_speed_sea_level),
            p.climb_rate,
            p.sustained_turn_rate,
            p.instant_turn_rate,
            p.turn_time,
            kmh(p.stall_speed)
        );
        assert!((p.top_speed - card.top_speed).abs() < 3.0, "top speed");
        assert!((p.climb_rate - card.climb_rate).abs() < 0.5, "climb");
        assert!(
            (p.sustained_turn_rate - card.sustained_turn_rate).abs() < 0.3,
            "sustained turn"
        );
        assert!(
            (p.instant_turn_rate - card.instant_turn_rate).abs() < 0.5,
            "instant turn"
        );
        assert!(m.cd0 > 0.01 && m.cd0 < 0.05, "cd0 plausible");
    }

    /// Fly the full simulation (not just the formulas) and check the
    /// emergent numbers.
    #[test]
    fn simulated_yak3_hits_top_speed_and_climb() {
        let card = AircraftCard::yak3();
        let m = FlightModel::from_card(&card);
        let full = Controls {
            throttle: 1.0,
            ..Default::default()
        };

        // Level flight at 4000 m: hold altitude with a simple autopilot.
        let mut b = Body::new(Vec3::new(0.0, 4000.0, 0.0), Quat::IDENTITY, 150.0);
        for _ in 0..(240 * 120) {
            let aim = Vec3::new(0.0, (4000.0 - b.position.y) * 0.002, -1.0);
            let mut c = instructor(&m, &mut b, aim, 1.0 / 240.0);
            c.throttle = full.throttle;
            step(&m, &mut b, &c, 1.0 / 240.0);
        }
        println!(
            "sim top speed {:.0} km/h at {:.0} m",
            kmh(b.speed()),
            b.position.y
        );
        assert!((b.speed() - card.top_speed).abs() < 6.0, "sim top speed");

        // Climb: hold a fixed climb attitude, let speed settle, and measure
        // vertical speed. The best attitude gives the best climb.
        let vy_best = card.climb_rate;
        let mut best = 0.0f32;
        for deg in [6.0f32, 8.0, 10.0, 12.0, 14.0, 16.0] {
            let mut b = Body::new(Vec3::new(0.0, 300.0, 0.0), Quat::IDENTITY, 75.0);
            let aim = Quat::from_rotation_x(deg.to_radians()) * Vec3::NEG_Z;
            let mut climbs = vec![];
            for i in 0..(240 * 60) {
                let mut c = instructor(&m, &mut b, aim, 1.0 / 240.0);
                c.throttle = 1.0;
                step(&m, &mut b, &c, 1.0 / 240.0);
                // Measure low down, where the card's sea-level figure applies.
                if i > 240 * 40 && b.position.y < 1500.0 {
                    climbs.push(b.velocity.y);
                }
            }
            if !climbs.is_empty() {
                best = best.max(climbs.iter().sum::<f32>() / climbs.len() as f32);
            }
        }
        println!("sim best climb {:.1} m/s (card {:.1})", best, vy_best);
        assert!(best > vy_best * 0.8 && best < vy_best * 1.15, "sim climb");
    }

    #[test]
    fn instructor_turns_onto_aim_without_stalling() {
        let m = FlightModel::from_card(&AircraftCard::yak3());
        let mut b = Body::new(Vec3::new(0.0, 1500.0, 0.0), Quat::IDENTITY, 120.0);
        let aim = Quat::from_rotation_y(2.0) * Vec3::NEG_Z; // 115° left
        let mut stalled_frames = 0;
        for _ in 0..(240 * 12) {
            let mut c = instructor(&m, &mut b, aim, 1.0 / 240.0);
            c.throttle = 1.0;
            step(&m, &mut b, &c, 1.0 / 240.0);
            if m.is_stalled(b.alpha) {
                stalled_frames += 1;
            }
        }
        let err = b.forward().angle_between(aim).to_degrees();
        let l = b.rotation.inverse() * aim;
        println!(
            "nose error after 12 s: {err:.2}° (local {l:?}), stalled frames {stalled_frames}, speed {:.0}",
            b.speed()
        );
        // And in plain level flight the nose should sit on the aim, not below it.
        let mut b = Body::new(Vec3::new(0.0, 1500.0, 0.0), Quat::IDENTITY, 110.0);
        for _ in 0..(240 * 8) {
            let mut c = instructor(&m, &mut b, Vec3::NEG_Z, 1.0 / 240.0);
            c.throttle = 0.8;
            step(&m, &mut b, &c, 1.0 / 240.0);
        }
        let level_err = b.forward().angle_between(Vec3::NEG_Z).to_degrees();
        println!("level flight nose error: {level_err:.3}°");
        assert!(level_err < 0.15, "nose sits on the aim in level flight");
        assert!(err < 0.3, "nose on aim");
        assert_eq!(stalled_frames, 0, "instructor never stalls");
    }

    #[test]
    fn full_aft_stick_at_low_speed_stalls() {
        let m = FlightModel::from_card(&AircraftCard::yak3());
        let mut b = Body::new(Vec3::new(0.0, 1500.0, 0.0), Quat::IDENTITY, 60.0);
        let c = Controls {
            pitch: 1.0,
            throttle: 0.3,
            ..Default::default()
        };
        let mut stalled = false;
        for _ in 0..(240 * 3) {
            step(&m, &mut b, &c, 1.0 / 240.0);
            stalled |= m.is_stalled(b.alpha);
        }
        assert!(stalled, "yanking at 216 km/h should stall");
    }

    #[test]
    fn full_aft_stick_at_high_speed_is_g_limited() {
        let m = FlightModel::from_card(&AircraftCard::yak3());
        let mut b = Body::new(Vec3::new(0.0, 1500.0, 0.0), Quat::IDENTITY, 170.0);
        let c = Controls {
            pitch: 1.0,
            throttle: 1.0,
            ..Default::default()
        };
        let mut peak = 0.0f32;
        for _ in 0..(240 * 2) {
            step(&m, &mut b, &c, 1.0 / 240.0);
            peak = peak.max(b.g_load);
        }
        println!("peak G at 612 km/h pull: {peak:.1}");
        assert!(peak > 6.0 && peak < m.g_limit + 1.0);
    }

    #[test]
    fn roll_rate_builds_up_rather_than_snapping() {
        let m = FlightModel::from_card(&AircraftCard::yak3());
        let mut b = Body::new(Vec3::new(0.0, 1500.0, 0.0), Quat::IDENTITY, ROLL_REF_SPEED);
        let c = Controls {
            roll: 1.0,
            throttle: 0.8,
            ..Default::default()
        };
        step(&m, &mut b, &c, 0.05);
        let early = -b.omega.z;
        for _ in 0..(240 * 2) {
            step(&m, &mut b, &c, 1.0 / 240.0);
        }
        let steady = -b.omega.z;
        println!(
            "roll rate after 50 ms {:.0}°/s, steady {:.0}°/s",
            early.to_degrees(),
            steady.to_degrees()
        );
        assert!(early < steady * 0.4);
        assert!((steady.to_degrees() - 140.0).abs() < 25.0);
    }

    #[test]
    fn gravity_trades_speed_for_height() {
        let m = FlightModel::from_card(&AircraftCard::yak3());
        let idle = |pitch_deg: f32, speed: f32| {
            let rot = Quat::from_rotation_x(pitch_deg.to_radians());
            let mut b = Body::new(Vec3::new(0.0, 3000.0, 0.0), rot, speed);
            let aim = rot * Vec3::NEG_Z;
            for _ in 0..(240 * 5) {
                let mut c = instructor(&m, &mut b, aim, 1.0 / 240.0);
                c.throttle = 0.0;
                step(&m, &mut b, &c, 1.0 / 240.0);
            }
            b
        };
        let dive = idle(-45.0, 100.0);
        let zoom = idle(45.0, 140.0);
        println!(
            "45° dive: 100 -> {:.0} m/s; 45° zoom: 140 -> {:.0} m/s",
            dive.speed(),
            zoom.speed()
        );
        assert!(dive.speed() > 130.0, "diving builds speed");
        assert!(zoom.speed() < 110.0, "climbing bleeds speed");

        // With no lift (zero speed, nose straight up) it simply falls.
        let mut b = Body::new(Vec3::new(0.0, 3000.0, 0.0), Quat::IDENTITY, 0.0);
        let c = Controls::default();
        step(&m, &mut b, &c, 1.0);
        assert!(
            (b.velocity.y + G).abs() < 0.3,
            "free fall at 1 g, got {}",
            b.velocity.y
        );
    }

    #[test]
    fn blank_extras_are_filled_in() {
        let card = AircraftCard {
            name: "Mystery fighter".into(),
            top_speed_altitude: None,
            mass: None,
            wing_area: None,
            cl_max: None,
            roll_rate: None,
            roll_speed: None,
            ..AircraftCard::yak3()
        };
        let e = card.extras();
        println!("filled in: {e:?}");
        // 1320 hp single: about 2500 kg and 14 m², like a Yak-3.
        assert!(e.mass > 2000.0 && e.mass < 3000.0);
        assert!(e.wing_area > 11.0 && e.wing_area < 18.0);
        let m = FlightModel::from_card(&card);
        assert!((m.check.sustained_turn_rate - card.sustained_turn_rate).abs() < 0.5);
        assert!((m.check.instant_turn_rate - card.instant_turn_rate).abs() < 0.5);
        assert!((m.check.climb_rate - card.climb_rate).abs() < 0.6);
    }

    #[test]
    fn jet_card_solves() {
        let card = AircraftCard {
            name: "Jet".into(),
            engine: EngineKind::Jet,
            power: 2700.0,
            engines: 1,
            climb_rate: 45.0,
            sustained_turn_rate: 16.0,
            sustained_turn_speed: 450.0 / 3.6,
            instant_turn_rate: 22.0,
            instant_turn_speed: 600.0 / 3.6,
            top_speed: 1050.0 / 3.6,
            top_speed_altitude: None,
            mass: None,
            wing_area: None,
            cl_max: None,
            roll_rate: None,
            roll_speed: None,
            weapons: Default::default(),
        };
        let m = FlightModel::from_card(&card);
        println!("jet {:?}", m.check);
        assert!((m.check.top_speed - card.top_speed).abs() < 5.0);
    }
}

#[cfg(test)]
mod settle_probe {
    use super::*;

    pub fn settle_time(turn_deg: f32, pitch_deg: f32, tune: InstructorTuning) -> (f32, bool) {
        settle_time_fps(turn_deg, pitch_deg, tune, 240.0)
    }

    pub fn settle_time_fps(
        turn_deg: f32,
        pitch_deg: f32,
        tune: InstructorTuning,
        fps: f32,
    ) -> (f32, bool) {
        let m = FlightModel::from_card(&AircraftCard::yak3());
        let mut b = Body::new(Vec3::new(0.0, 1500.0, 0.0), Quat::IDENTITY, 120.0);
        let aim = Quat::from_euler(
            bevy::math::EulerRot::YXZ,
            turn_deg.to_radians(),
            pitch_deg.to_radians(),
            0.0,
        ) * Vec3::NEG_Z;
        let dt = 1.0 / fps;
        let mut last_out = 0.0;
        let mut stalled = false;
        for i in 0..((fps * 15.0) as usize) {
            let mut c = instructor_tuned(&m, &mut b, aim, dt, tune);
            c.throttle = 1.0;
            step(&m, &mut b, &c, dt);
            stalled |= m.is_stalled(b.alpha);
            if b.forward().angle_between(aim).to_degrees() > 0.3 {
                last_out = i as f32 * dt;
            }
        }
        (last_out, stalled)
    }

    fn score(t: InstructorTuning) -> (f32, String) {
        let cases = [
            (30.0, 0.0),
            (90.0, 0.0),
            (-60.0, 10.0),
            (10.0, 0.0),
            (3.0, 1.0),
            (-5.0, -3.0),
            (150.0, 5.0),
        ];
        let mut total = 0.0;
        let mut out = String::new();
        for (a, p) in cases {
            let (ts, st) = settle_time(a, p, t);
            total += ts + if st { 20.0 } else { 0.0 };
            out += &format!("{a}/{p}:{ts:.2}{} ", if st { "S" } else { "" });
        }
        (total, out)
    }

    #[test]
    fn settles_quickly_at_game_frame_rates() {
        let t = InstructorTuning::get();
        for fps in [30.0, 60.0, 144.0] {
            let mut line = format!("{fps} fps: ");
            for (a, p, limit) in [
                (10.0, 0.0, 1.5),
                (3.0, 1.0, 1.0),
                (30.0, 0.0, 3.0),
                (-60.0, 10.0, 4.5),
            ] {
                let (ts, st) = settle_time_fps(a, p, t, fps);
                line += &format!("{a}/{p}:{ts:.2} ");
                assert!(!st, "no stall at {fps} fps");
                assert!(ts < limit, "{a}° at {fps} fps took {ts:.2} s");
            }
            println!("{line}");
        }
    }

    /// Once on target the stick shouldn't chatter back and forth.
    #[test]
    fn controls_are_calm_on_target() {
        let m = FlightModel::from_card(&AircraftCard::yak3());
        let mut b = Body::new(Vec3::new(0.0, 1500.0, 0.0), Quat::IDENTITY, 120.0);
        let aim = Quat::from_rotation_y(0.2) * Vec3::NEG_Z;
        let dt = 1.0 / 60.0;
        let mut prev = Controls::default();
        let mut wiggle = 0.0f32;
        for i in 0..(60 * 10) {
            let mut c = instructor(&m, &mut b, aim, dt);
            c.throttle = 0.9;
            step(&m, &mut b, &c, dt);
            if i > 60 * 4 {
                wiggle = wiggle
                    .max((c.yaw - prev.yaw).abs())
                    .max((c.roll - prev.roll).abs());
            }
            prev = c;
        }
        println!("max frame-to-frame stick change on target: {wiggle:.3}");
        assert!(wiggle < 0.1);
    }

    #[test]
    #[ignore]
    fn sweep() {
        let base = InstructorTuning::get();
        let mut best = (score(base).0, base);
        println!("base {}", score(base).1);
        for lead in [0.1, 0.2] {
            for hb in [20.0, 35.0] {
                for lb in [0.005, 0.01] {
                    for ba in [0.06, 0.1, 0.2] {
                        for pg in [3.0, 4.0] {
                            for pd in [0.1, 0.15] {
                                for rg in [40.0, 80.0, 150.0] {
                                    for rd in [0.5, 1.0, 2.0] {
                                        for rf in [1.0, 0.5] {
                                            let t = InstructorTuning {
                                                lead,
                                                heading_bank: hb,
                                                level_below: lb,
                                                bank_above: ba,
                                                roll_gain: 3.0,
                                                pitch_gain: pg,
                                                pitch_damp: pd,
                                                rudder_gain: rg,
                                                rudder_damp: rd,
                                                rudder_fade: rf,
                                                ..base
                                            };
                                            let (sc, _) = score(t);
                                            if sc < best.0 {
                                                best = (sc, t);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let t = best.1;
        println!(
            "best {:.2}: lead {} hb {} lb {} ba {} pg {} pd {} rg {} rd {} rf {}",
            best.0,
            t.lead,
            t.heading_bank,
            t.level_below,
            t.bank_above,
            t.pitch_gain,
            t.pitch_damp,
            t.rudder_gain,
            t.rudder_damp,
            t.rudder_fade
        );
        println!("{}", score(t).1);
    }
}

#[cfg(test)]
mod jump_probe {
    use super::*;

    /// Step or sweep the aim and report the largest bank, overshoot, and
    /// how much the nose wanders sideways.
    pub fn probe(name: &str, aim_at: impl Fn(f32) -> Vec3, secs: f32) -> (f32, f32) {
        let m = FlightModel::from_card(&AircraftCard::yak3());
        let mut b = Body::new(Vec3::new(0.0, 1500.0, 0.0), Quat::IDENTITY, 150.0);
        let dt = 1.0 / 60.0;
        let mut max_bank = 0.0f32;
        let mut max_err = 0.0f32;
        let mut log = String::new();
        for i in 0..((secs / dt) as usize) {
            let t = i as f32 * dt;
            let aim = aim_at(t);
            let mut c = instructor(&m, &mut b, aim, dt);
            c.throttle = 1.0;
            step(&m, &mut b, &c, dt);
            let right = b.rotation * Vec3::X;
            let bank = right.y.asin().to_degrees();
            max_bank = max_bank.max(bank.abs());
            let err = b.forward().angle_between(aim).to_degrees();
            if t > 1.5 {
                max_err = max_err.max(err);
            }
            if i % 6 == 0 {
                log += &format!("{t:.1}:b{bank:.0},e{err:.1},y{:.2},r{:.2} ", c.yaw, c.roll);
            }
        }
        println!("{name}: max bank {max_bank:.0}°, max err after 1.5 s {max_err:.2}°\n  {log}");
        (max_bank, max_err)
    }

    pub fn dir(yaw_deg: f32, pitch_deg: f32) -> Vec3 {
        Quat::from_euler(
            bevy::math::EulerRot::YXZ,
            yaw_deg.to_radians(),
            pitch_deg.to_radians(),
            0.0,
        ) * Vec3::NEG_Z
    }

    /// A mouse-like sweep: the aim moves in uneven steps (some frames get no
    /// mouse movement). Returns (control chatter, nose jerk, mean error).
    pub fn mouse_sweep(tune: InstructorTuning, rate_dps: f32, fps: f32) -> (f32, f32, f32) {
        let m = FlightModel::from_card(&AircraftCard::yak3());
        let mut b = Body::new(Vec3::new(0.0, 1500.0, 0.0), Quat::IDENTITY, 150.0);
        let dt = 1.0 / fps;
        let mut yaw_deg = 0.0f32;
        let mut prev = Controls::default();
        let mut prev_fwd = b.forward();
        let mut prev_rate = Vec3::ZERO;
        let (mut chatter, mut jerk, mut err) = (0.0, 0.0, 0.0);
        let n = (fps * 4.0) as usize;
        for i in 0..n {
            // Every third frame has no mouse input; the others make up for it.
            if i % 3 != 2 && i < n / 2 {
                yaw_deg -= rate_dps * dt * 1.5;
            }
            let aim = dir(yaw_deg, 0.0);
            let mut c = instructor_tuned(&m, &mut b, aim, dt, tune);
            c.throttle = 1.0;
            step(&m, &mut b, &c, dt);
            let fwd = b.forward();
            let rate = (fwd - prev_fwd) / dt;
            if i > 10 {
                chatter += (c.yaw - prev.yaw).abs() + (c.roll - prev.roll).abs();
                jerk += ((rate - prev_rate) / dt).length() * dt;
                err += fwd.angle_between(aim).to_degrees() / n as f32;
            }
            prev = c;
            prev_fwd = fwd;
            prev_rate = rate;
        }
        (chatter, jerk, err)
    }

    /// Uneven mouse input mustn't make the controls (and the crosshair) jump.
    #[test]
    fn mouse_steps_dont_kick_the_controls() {
        let (chatter, jerk, err) = mouse_sweep(InstructorTuning::get(), 5.0, 60.0);
        println!("chatter {chatter:.1}, jerk {jerk:.2}, err {err:.2}");
        assert!(chatter < 12.0 && jerk < 0.35 && err < 0.5);
    }

    #[test]
    #[ignore]
    fn mouse_sweeps() {
        let t = InstructorTuning::get();
        for fps in [60.0, 144.0] {
            for rate in [2.0, 5.0, 15.0] {
                let (c, j, e) = mouse_sweep(t, rate, fps);
                println!("{fps} fps {rate}°/s: chatter {c:.1}, jerk {j:.2}, mean err {e:.2}°");
            }
        }
    }

    #[test]
    #[ignore]
    fn shallow_moves() {
        probe("step 3° right", |_| dir(-3.0, 0.0), 3.0);
        probe("step 6° right", |_| dir(-6.0, 0.0), 3.0);
        probe(
            "sweep 4°/s right for 2 s",
            |t| dir(-4.0 * t.min(2.0), 0.0),
            4.0,
        );
        probe(
            "wiggle ±2° at 0.5 Hz",
            |t| dir(2.0 * (t * std::f32::consts::PI).sin(), 0.0),
            6.0,
        );
    }
}

#[cfg(test)]
pub mod smooth_sweep_pub {
    use super::jump_probe::mouse_sweep;
    #[allow(unused_imports)]
    use super::settle_probe::settle_time_fps;
    use super::*;

    /// Step the aim and count stick reversals (a control swinging one way
    /// then back), plus the largest bank.
    pub(super) fn reversals(t: InstructorTuning, yaw_deg: f32) -> (f32, f32) {
        let m = FlightModel::from_card(&AircraftCard::yak3());
        let mut b = Body::new(Vec3::new(0.0, 1500.0, 0.0), Quat::IDENTITY, 150.0);
        let dt = 1.0 / 60.0;
        let aim = super::jump_probe::dir(yaw_deg, 0.0);
        let mut prev = Controls::default();
        let mut prev_d = [0.0f32; 3];
        let mut flips = 0.0;
        let mut max_bank = 0.0f32;
        for _ in 0..(60 * 3) {
            let mut c = instructor_tuned(&m, &mut b, aim, dt, t);
            c.throttle = 1.0;
            step(&m, &mut b, &c, dt);
            let d = [c.pitch - prev.pitch, c.roll - prev.roll, c.yaw - prev.yaw];
            for i in 0..3 {
                if d[i].abs() > 0.01
                    && prev_d[i].abs() > 0.01
                    && d[i].signum() != prev_d[i].signum()
                {
                    flips += 1.0;
                }
                if d[i].abs() > 0.01 {
                    prev_d[i] = d[i];
                }
            }
            prev = c;
            max_bank = max_bank.max((b.rotation * Vec3::X).y.abs().asin().to_degrees());
        }
        (flips, max_bank)
    }

    fn score(t: InstructorTuning) -> (f32, String) {
        let mut total = 0.0;
        let mut out = String::new();
        for a in [3.0, 8.0] {
            let (f, mb) = reversals(t, -a);
            total += f * 0.5 + (mb - a * 3.0).max(0.0) * 0.1;
            out += &format!("<{a}: flips {f} bank {mb:.0}> ");
        }
        for (a, p) in [
            (3.0, 1.0),
            (10.0, 0.0),
            (30.0, 0.0),
            (-60.0, 10.0),
            (150.0, 5.0),
        ] {
            let (ts, st) = settle_time_fps(a, p, t, 60.0);
            total += ts + if st { 20.0 } else { 0.0 };
            out += &format!("{a}:{ts:.2} ");
        }
        for rate in [2.0, 5.0, 15.0] {
            let (c, j, e) = mouse_sweep(t, rate, 60.0);
            total += c * 0.05 + j * 1.0 + e * 1.0;
            out += &format!("[{rate}: c{c:.1} j{j:.2} e{e:.2}] ");
        }
        (total, out)
    }

    #[test]
    #[ignore]
    fn smooth_sweep() {
        let base = InstructorTuning::get();
        println!("base {:.2} {}", score(base).0, score(base).1);
        let mut best = (score(base).0, base);
        for kh in [6.0, 8.0, 12.0] {
            for kb in [1.5, 2.0, 3.0] {
                for kr in [30.0, 50.0, 80.0] {
                    for krr in [4.0, 8.0, 12.0] {
                        for cap in [12.0, 16.0, 24.0] {
                            for kp in [3.0, 5.0, 8.0] {
                                let t = InstructorTuning {
                                    kh,
                                    kv: kp * 2.0,
                                    k_bank: kb,
                                    k_rudder: kr,
                                    k_rudder_rate: krr,
                                    k_phi_cap: cap,
                                    k_pitch: kp,
                                    ..base
                                };
                                let sc = score(t).0;
                                if sc < best.0 {
                                    best = (sc, t);
                                }
                            }
                        }
                    }
                }
            }
        }
        let t = best.1;
        println!(
            "best {:.2}: kh {} kv {} kb {} kr {} krr {} cap {} kp {}\n{}",
            best.0,
            t.kh,
            t.kv,
            t.k_bank,
            t.k_rudder,
            t.k_rudder_rate,
            t.k_phi_cap,
            t.k_pitch,
            score(t).1
        );
    }
}

#[cfg(test)]
mod compare_probe {
    use super::*;

    /// Pitch, roll and yaw move together without swinging back and forth.
    #[test]
    fn small_turns_are_smooth_and_coordinated() {
        let t = InstructorTuning::get();
        for a in [3.0, 8.0] {
            let (flips, bank) = super::smooth_sweep_pub::reversals(t, -a);
            println!("{a} deg step: {flips} stick reversals, max bank {bank:.0}");
            // The old controller swung back and forth 5 and 9 times.
            assert!(flips <= 5.0, "{a} deg: {flips} reversals");
            assert!(bank < a * 4.5, "{a} deg: bank {bank}");
        }
    }

    #[test]
    #[ignore]
    fn compare() {
        let new = InstructorTuning::get();
        let old = InstructorTuning {
            big_lo: -2.0,
            big_hi: -1.0,
            ..new
        };
        let hi = InstructorTuning {
            kh: 8.0,
            kv: 10.0,
            k_bank: 2.0,
            k_rudder: 80.0,
            k_rudder_rate: 8.0,
            k_phi_cap: 16.0,
            k_pitch: 5.0,
            ..new
        };
        let mid = InstructorTuning {
            kh: 5.0,
            kv: 6.0,
            k_bank: 3.0,
            k_rudder: 40.0,
            k_rudder_rate: 4.0,
            k_phi_cap: 12.0,
            k_pitch: 3.0,
            ..new
        };
        let k6 = InstructorTuning {
            kh: 6.0,
            kv: 7.5,
            ..new
        };
        let a15 = InstructorTuning {
            aim_smooth: 0.16,
            kh: 8.0,
            kv: 10.0,
            ..new
        };
        let a06 = InstructorTuning {
            aim_smooth: 0.06,
            kh: 8.0,
            kv: 10.0,
            ..new
        };
        for (name, t) in [
            ("old", old),
            ("hi", hi),
            ("cur", new),
            ("a06", a06),
            ("k6", k6),
            ("a15", a15),
            ("mid", mid),
        ] {
            let mut line = format!("{name}: ");
            for a in [3.0, 8.0, 20.0] {
                let (f, mb) = super::smooth_sweep_pub::reversals(t, -a);
                let (ts, _) = super::settle_probe::settle_time_fps(a, 0.0, t, 60.0);
                line += &format!("[{a}: flips {f} bank {mb:.0} settle {ts:.2}] ");
            }
            for rate in [5.0, 15.0] {
                let (c, j, e) = super::jump_probe::mouse_sweep(t, rate, 60.0);
                line += &format!("[{rate}/s c{c:.1} j{j:.2} e{e:.2}] ");
            }
            println!("{line}");
        }
    }
}
