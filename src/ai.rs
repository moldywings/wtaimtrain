//! A dogfighting AI pilot, used by the practice target and by multiplayer
//! targets. It hunts one enemy: leads its shots for its biggest gun, keeps
//! its speed between its best sustained turn speed and its corner speed
//! (unloading or easing the throttle as needed), breaks off close passes and
//! pulls up well before the ground.

use bevy::prelude::*;

use crate::flight_model::{self, AircraftCard, Body, Controls, FlightModel};
use crate::weapons::{GunCard, lead_direction};

/// Lowest it's happy to fly, m; below this it climbs back up.
const FLOOR: f32 = 250.0;
/// Opens fire inside this range, m.
const FIRE_RANGE: f32 = 900.0;

/// What a pilot remembers between frames.
#[derive(Clone, Debug, Default)]
pub struct Pilot {
    /// Seconds left of breaking away after a close pass.
    pub break_off: f32,
    /// Which way it breaks (+1 right, -1 left).
    pub break_side: f32,
}

/// The plane it's after.
#[derive(Clone, Copy, Debug)]
pub struct Foe {
    pub position: Vec3,
    pub velocity: Vec3,
}

/// The biggest gun on a card (what the AI aims with), looked up by name.
pub fn biggest_gun<'a>(
    card: &AircraftCard,
    lookup: impl Fn(&str) -> &'a GunCard,
) -> Option<GunCard> {
    card.weapons
        .guns()
        .iter()
        .map(|(_, g, _)| lookup(g))
        .max_by(|a, b| a.caliber_mm().total_cmp(&b.caliber_mm()))
        .cloned()
}

/// How long until the plane would hit the ground on its current path, s
/// (infinite when climbing).
fn time_to_ground(body: &Body) -> f32 {
    let vy = body.velocity.y;
    if vy >= -1.0 {
        f32::INFINITY
    } else {
        (body.position.y - FLOOR * 0.4).max(0.0) / -vy
    }
}

/// When the plane is about to hit the ground: where to point to pull out.
pub fn ground_escape(body: &Body) -> Option<Vec3> {
    let pull_time = 2.5 + body.speed() / 60.0;
    if time_to_ground(body) < pull_time || body.position.y < FLOOR * 0.5 {
        let along = body.velocity.normalize_or(body.forward());
        let flat = along.with_y(0.0).normalize_or(Vec3::NEG_Z);
        Some((flat + Vec3::Y * 0.9).normalize())
    } else {
        None
    }
}

/// Where to point, how much throttle, and whether to shoot.
pub struct Decision {
    pub aim: Vec3,
    pub throttle: f32,
    pub firing: bool,
}

/// Decide what to do this frame.
pub fn decide(
    card: &AircraftCard,
    model: &FlightModel,
    body: &Body,
    gun: Option<&GunCard>,
    foe: Option<Foe>,
    pilot: &mut Pilot,
    dt: f32,
) -> Decision {
    pilot.break_off = (pilot.break_off - dt).max(0.0);
    let pos = body.position;
    let speed = body.speed();
    let along = body.velocity.normalize_or(body.forward());
    let max_throttle = model.max_throttle();

    // The ground comes first: pull up, wings level, full power.
    if let Some(aim) = ground_escape(body) {
        return Decision {
            aim,
            throttle: max_throttle,
            firing: false,
        };
    }

    let Some(foe) = foe else {
        // Nobody to fight: cruise level, holding height above the floor.
        let flat = along.with_y(0.0).normalize_or(Vec3::NEG_Z);
        let climb = ((FLOOR * 3.0 - pos.y) * 0.002).clamp(-0.15, 0.25);
        return Decision {
            aim: (flat + Vec3::Y * climb).normalize(),
            throttle: 0.85,
            firing: false,
        };
    };

    let rel = foe.position - pos;
    let range = rel.length();
    let lead = gun.and_then(|g| lead_direction(g, pos, body.velocity, foe.position, foe.velocity));
    let closing_speed = -(foe.velocity - body.velocity).dot(rel.normalize_or_zero());

    // Too close and closing fast: break away to one side, then turn back in.
    if range < 160.0 && closing_speed > 20.0 && pilot.break_off <= 0.0 {
        pilot.break_off = 1.8;
        let side = rel.cross(Vec3::Y).dot(body.velocity);
        pilot.break_side = if side >= 0.0 { 1.0 } else { -1.0 };
    }
    let mut aim = if pilot.break_off > 0.0 {
        let away = (-rel).with_y(0.0).normalize_or(Vec3::X);
        let side = away.cross(Vec3::Y) * pilot.break_side;
        (away + side * 0.8 + Vec3::Y * 0.3).normalize()
    } else if range < 1500.0 {
        // Close in: point the guns where the enemy will be.
        lead.unwrap_or_else(|| rel.normalize_or(along))
    } else {
        // Far off: lead pursuit, cutting the corner toward where it's going.
        let t = (range / speed.max(60.0)).min(5.0);
        (foe.position + foe.velocity * t * 0.5 - pos).normalize_or(along)
    };

    // Energy: below the best sustained turn speed a hard pull only bleeds
    // speed, so ease off the turn (and drop the nose a little if there's
    // height to spare) until it builds back up.
    let sustained = card
        .sustained_turn_speed
        .max(model.stall_speed(pos.y) * 1.3);
    let corner = card.instant_turn_speed.max(sustained);
    let wanted_turn = along.angle_between(aim);
    if speed < sustained * 0.92 && pilot.break_off <= 0.0 {
        let k = ((speed / sustained - 0.7) / 0.22).clamp(0.35, 1.0);
        aim = along.slerp(aim, k).normalize();
        if pos.y > FLOOR * 2.0 && wanted_turn > 0.35 {
            aim = (aim - Vec3::Y * 0.12).normalize();
        }
    }
    // Keep clear of the floor while fighting.
    if pos.y < FLOOR {
        aim = (aim.with_y(aim.y.max(0.15))).normalize();
    }

    // Throttle: full power, unless it's fast, behind the enemy and about to
    // overshoot, when it eases off to stay behind (and near corner speed).
    let behind = rel.dot(foe.velocity) > 0.0;
    let mut throttle = max_throttle;
    if behind && range < 600.0 && closing_speed > 15.0 && speed > sustained {
        throttle = (1.0 - (closing_speed - 15.0) / 60.0).clamp(0.15, 1.0);
    }
    if speed > corner * 1.15 && wanted_turn > 0.5 {
        throttle = throttle.min(0.4);
    }
    let throttle = throttle.min(max_throttle);

    // Shoot when the guns are roughly on target: like a real pilot it opens
    // fire a little before the solution is perfect.
    let forward = body.forward();
    let cone = (10.0 / range.max(1.0)).atan().max(2.0f32.to_radians());
    let firing = range < FIRE_RANGE
        && pilot.break_off <= 0.0
        && lead.is_some_and(|d| d.angle_between(forward) < cone);

    Decision {
        aim,
        throttle,
        firing,
    }
}

/// Fly one frame toward a decision.
pub fn fly(model: &FlightModel, body: &mut Body, decision: &Decision, dt: f32) -> Controls {
    let mut controls = flight_model::instructor(model, body, decision.aim, dt);
    controls.throttle = decision.throttle;
    flight_model::step(model, body, &controls, dt);
    controls
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yak() -> (AircraftCard, FlightModel) {
        let card = AircraftCard::yak3();
        let model = FlightModel::from_card(&card);
        (card, model)
    }

    /// Diving straight at the ground, it pulls out instead of crashing.
    #[test]
    fn pulls_up_before_the_ground() {
        let (card, model) = yak();
        let rot = Quat::from_rotation_x(-60f32.to_radians());
        let mut body = Body::new(Vec3::new(0.0, 900.0, 0.0), rot, 150.0);
        let mut pilot = Pilot::default();
        // An enemy right below, tempting it to dive.
        let foe = Foe {
            position: Vec3::new(0.0, 50.0, -300.0),
            velocity: Vec3::new(0.0, 0.0, -100.0),
        };
        let mut lowest = f32::MAX;
        for _ in 0..(20 * 60) {
            let f = Foe {
                position: foe.position.with_y(50.0),
                ..foe
            };
            let d = decide(&card, &model, &body, None, Some(f), &mut pilot, 1.0 / 60.0);
            fly(&model, &mut body, &d, 1.0 / 60.0);
            lowest = lowest.min(body.position.y);
        }
        assert!(lowest > 20.0, "flew down to {lowest} m");
    }

    /// Two AI pilots dogfighting: neither crashes, and they get shots in.
    #[test]
    fn dogfight_between_two_pilots() {
        let (card, model) = yak();
        let gun = GunCard::shvak();
        let mut a = Body::new(Vec3::new(0.0, 2000.0, 700.0), Quat::IDENTITY, 130.0);
        let mut b = Body::new(
            Vec3::new(0.0, 2000.0, -700.0),
            Quat::from_rotation_y(std::f32::consts::PI),
            130.0,
        );
        let (mut pa, mut pb) = (Pilot::default(), Pilot::default());
        let dt = 1.0 / 60.0;
        let (mut fire_a, mut fire_b, mut lowest) = (0, 0, f32::MAX);
        for _ in 0..(120 * 60) {
            let fa = Foe {
                position: b.position,
                velocity: b.velocity,
            };
            let fb = Foe {
                position: a.position,
                velocity: a.velocity,
            };
            let da = decide(&card, &model, &a, Some(&gun), Some(fa), &mut pa, dt);
            let db = decide(&card, &model, &b, Some(&gun), Some(fb), &mut pb, dt);
            fly(&model, &mut a, &da, dt);
            fly(&model, &mut b, &db, dt);
            fire_a += da.firing as u32;
            fire_b += db.firing as u32;
            lowest = lowest.min(a.position.y).min(b.position.y);
        }
        println!("firing frames: {fire_a} {fire_b}, lowest {lowest:.0} m");
        assert!(lowest > 30.0, "got down to {lowest} m");
        assert!(
            fire_a + fire_b > 60,
            "only {fire_a} + {fire_b} frames of shooting"
        );
    }

    /// Behind a slower enemy, it eases the throttle instead of overshooting.
    #[test]
    fn eases_off_to_stay_behind() {
        let (card, model) = yak();
        let body = Body::new(Vec3::new(0.0, 2000.0, 0.0), Quat::IDENTITY, 160.0);
        let foe = Foe {
            position: Vec3::new(0.0, 2000.0, -250.0),
            velocity: Vec3::new(0.0, 0.0, -90.0),
        };
        let d = decide(
            &card,
            &model,
            &body,
            None,
            Some(foe),
            &mut Pilot::default(),
            0.016,
        );
        assert!(d.throttle < 0.8, "throttle {}", d.throttle);
    }
}
