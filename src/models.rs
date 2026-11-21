//! The aircraft models a preset can use, and everything the game needs to
//! know about each airframe: its size, where it rolls, the chase camera,
//! where guns and turrets go, and its hit boxes.
//!
//! All the models are drawn nose toward +Z with +Y up (Bevy's forward is
//! -Z, so they're turned round), and model space numbers below are in the
//! model's own units, before scaling.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Which 3D model an aircraft preset uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModelKind {
    /// A single engine fighter (the Yak-3).
    #[default]
    Fighter,
    /// A twin engine heavy fighter (the F7F Tigercat).
    HeavyFighter,
    /// A jet fighter (the F11F-1 Tiger).
    JetFighter,
    /// A four engine bomber (the Avro Shackleton).
    Bomber,
}

impl ModelKind {
    pub const ALL: [ModelKind; 4] = [
        ModelKind::Fighter,
        ModelKind::HeavyFighter,
        ModelKind::JetFighter,
        ModelKind::Bomber,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ModelKind::Fighter => "Fighter",
            ModelKind::HeavyFighter => "Heavy fighter",
            ModelKind::JetFighter => "Jet fighter",
            ModelKind::Bomber => "Bomber",
        }
    }

    pub fn next(self) -> ModelKind {
        let i = ModelKind::ALL.iter().position(|k| *k == self).unwrap_or(0);
        ModelKind::ALL[(i + 1) % ModelKind::ALL.len()]
    }

    pub fn spec(self) -> &'static ModelSpec {
        match self {
            ModelKind::Fighter => &FIGHTER,
            ModelKind::HeavyFighter => &HEAVY_FIGHTER,
            ModelKind::JetFighter => &JET_FIGHTER,
            ModelKind::Bomber => &BOMBER,
        }
    }

    /// The model to use for a preset that hasn't picked one: jets get the
    /// jet, twins the heavy fighter, three or more engines the bomber.
    pub fn guess(engine: crate::flight_model::EngineKind, engines: u32) -> ModelKind {
        use crate::flight_model::EngineKind;
        match (engine, engines) {
            (EngineKind::Jet, _) => ModelKind::JetFighter,
            (_, 0..=1) => ModelKind::Fighter,
            (_, 2) => ModelKind::HeavyFighter,
            _ => ModelKind::Bomber,
        }
    }
}

/// An axis aligned box in model space.
pub type Aabb = (Vec3, Vec3);

/// Chase camera placement: behind and above the plane in the view's frame,
/// and how far the view points below the aim.
#[derive(Clone, Copy, Debug)]
pub struct CameraSpec {
    pub offset: Vec3,
    pub zoom_offset: Vec3,
    pub tilt_deg: f32,
}

impl CameraSpec {
    /// Gunner view: a little lower and closer, looking straight along the
    /// gunsight.
    pub fn gunner_offset(&self) -> Vec3 {
        Vec3::new(0.0, self.offset.y * 0.85, self.offset.z * 0.95)
    }
}

/// Everything about one airframe.
pub struct ModelSpec {
    /// Path relative to `assets/`.
    pub path: &'static str,
    /// Scale from model units to meters (the real aircraft's size).
    pub scale: f32,
    /// Height in the model of the axis the plane rolls about.
    pub roll_y: f32,
    pub camera: CameraSpec,
    /// Nose guns: the center of the spinner (or nose), the spinner gun's
    /// muzzle, the ring guns' muzzles and the ring radius.
    pub nose: Vec3,
    pub spinner_muzzle_z: f32,
    pub ring_muzzle_z: f32,
    pub ring_radius: f32,
    /// Belly guns: height and spacing.
    pub belly_y: f32,
    pub belly_spacing: f32,
    /// Wing guns: where the wing meets the fuselage, the outermost spot a
    /// gun fits and the spacing between guns. Wing gun muzzles sit just
    /// ahead of the leading edge, at `wing_le.0 + (x - root) * wing_le.1`
    /// along Z, and the wing's middle is at `wing_y.0 + (x - root) *
    /// wing_y.1` up.
    pub wing_root_x: f32,
    pub wing_max_x: f32,
    pub wing_spacing: f32,
    pub wing_le: (f32, f32),
    pub wing_y: (f32, f32),
    /// Hit boxes of the airframe (not the cockpit).
    pub boxes: &'static [Aabb],
    /// The cockpit.
    pub cockpit: Aabb,
    /// The top and bottom of the fuselage along its length, (z, y) from
    /// tail to nose.
    pub top: &'static [(f32, f32)],
    pub bottom: &'static [(f32, f32)],
    /// Where top and bottom turrets can slide, tail end to nose end (z).
    pub turret_z: (f32, f32),
    /// Nose and tail turret centers.
    pub front: Vec3,
    pub rear: Vec3,
    /// Size of the turret models on this airframe, meters per model unit.
    pub turret_scale: f32,
}

impl ModelSpec {
    /// The transform from model space to the plane's own frame (meters,
    /// forward -Z, origin on the roll axis).
    pub fn correction(&self) -> Transform {
        Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::PI))
            .with_scale(Vec3::splat(self.scale))
            .with_translation(Vec3::new(0.0, -self.roll_y * self.scale, 0.0))
    }

    pub fn top_at(&self, z: f32) -> f32 {
        profile(self.top, z)
    }

    pub fn bottom_at(&self, z: f32) -> f32 {
        profile(self.bottom, z)
    }

    /// Wing leading edge (z) and middle (y) at `x` out from the center line.
    pub fn wing_at(&self, x: f32) -> (f32, f32) {
        let d = x.abs() - self.wing_root_x;
        (
            self.wing_le.0 + d * self.wing_le.1,
            self.wing_y.0 + d * self.wing_y.1,
        )
    }

    /// Longest root-to-first-gun distance that fits, full size meters.
    pub fn wing_out_max(&self) -> f32 {
        ((self.wing_max_x - self.wing_root_x) * self.scale).max(0.2)
    }

    /// No part of the airframe is farther than this from its center, m.
    pub fn reach(&self) -> f32 {
        self.boxes
            .iter()
            .flat_map(|b| [b.0, b.1])
            .map(|p| (p - Vec3::Y * self.roll_y).length())
            .fold(0.0, f32::max)
            * self.scale
            + 2.0
    }
}

/// Linear interpolation along a (z, y) profile.
fn profile(points: &[(f32, f32)], z: f32) -> f32 {
    let Some(first) = points.first() else {
        return 0.0;
    };
    if z <= first.0 {
        return first.1;
    }
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        if z <= b.0 {
            let t = (z - a.0) / (b.0 - a.0).max(1e-6);
            return a.1 + (b.1 - a.1) * t;
        }
    }
    points.last().map(|p| p.1).unwrap_or(0.0)
}

const fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

/// The Yak-3. The model is about 0.75x the real aircraft (8.5 m long,
/// 9.2 m span).
pub static FIGHTER: ModelSpec = ModelSpec {
    path: "models/fighter.glb",
    scale: 1.33,
    roll_y: 0.6,
    // Measured from War Thunder's chase view: the plane sits about 15 m
    // ahead and 3.6 m below the line of sight.
    camera: CameraSpec {
        offset: v(0.0, 3.6, 15.0),
        zoom_offset: v(0.0, 3.4, 11.0),
        tilt_deg: 3.3,
    },
    nose: v(0.0, 0.50, 2.46),
    spinner_muzzle_z: 2.53,
    ring_muzzle_z: 2.30,
    ring_radius: 0.17,
    belly_y: -0.04,
    belly_spacing: 0.17,
    wing_root_x: 0.48,
    wing_max_x: 3.0,
    wing_spacing: 0.3,
    wing_le: (1.08, -0.1775),
    wing_y: (0.19, 0.09),
    boxes: &[
        // Fuselage
        (v(-0.34, 0.0, -2.7), v(0.34, 0.96, 2.5)),
        // Fin
        (v(-0.1, 0.38, -3.92), v(0.1, 1.64, -2.7)),
        // Wings
        (v(-3.5, 0.0, -1.3), v(3.5, 0.5, 1.0)),
        // Tailplane
        (v(-1.26, 0.70, -3.7), v(1.26, 0.83, -2.74)),
    ],
    cockpit: (v(-0.31, 0.76, -1.53), v(0.31, 1.12, 0.52)),
    top: &[
        (-3.4, 0.9),
        (-2.6, 0.96),
        (-0.5, 0.94),
        (0.74, 0.88),
        (1.58, 0.83),
        (1.99, 0.76),
        (2.41, 0.58),
    ],
    bottom: &[
        (-3.4, 0.4),
        (-2.18, 0.18),
        (-0.93, 0.06),
        (0.74, 0.01),
        (1.58, 0.07),
        (1.99, 0.2),
        (2.41, 0.42),
    ],
    turret_z: (-2.6, 1.9),
    front: v(0.0, 0.5, 2.3),
    rear: v(0.0, 0.75, -3.75),
    turret_scale: 0.5,
};

/// The F7F-3 Tigercat: 13.8 m long, 15.7 m span.
pub static HEAVY_FIGHTER: ModelSpec = ModelSpec {
    path: "models/heavyfighter.glb",
    scale: 1.51,
    roll_y: 0.1,
    camera: CameraSpec {
        offset: v(0.0, 3.7, 14.7),
        zoom_offset: v(0.0, 3.5, 10.8),
        tilt_deg: 5.3,
    },
    nose: v(0.0, -0.08, 3.62),
    spinner_muzzle_z: 3.74,
    ring_muzzle_z: 3.3,
    ring_radius: 0.15,
    belly_y: -0.6,
    belly_spacing: 0.17,
    wing_root_x: 0.4,
    wing_max_x: 4.6,
    wing_spacing: 0.3,
    wing_le: (1.1, 0.0),
    wing_y: (0.22, 0.104),
    boxes: &[
        // Fuselage
        (v(-0.37, -0.58, -4.4), v(0.37, 0.85, 3.71)),
        // Fin
        (v(-0.12, 0.6, -5.42), v(0.12, 2.49, -3.9)),
        // Wings
        (v(-5.19, 0.03, -1.4), v(5.16, 0.78, 1.0)),
        // Engines
        (v(1.46, -0.47, -1.71), v(2.49, 0.51, 1.82)),
        (v(-2.49, -0.47, -1.71), v(-1.45, 0.51, 1.82)),
        // Tailplane
        (v(-2.2, 0.5, -4.73), v(2.22, 0.62, -3.45)),
    ],
    cockpit: (v(-0.32, 0.24, 0.87), v(0.32, 0.89, 1.68)),
    top: &[
        (-4.2, 0.6),
        (-2.9, 0.62),
        (-1.15, 0.65),
        (0.05, 0.77),
        (0.65, 0.83),
        (1.25, 0.56),
        (1.85, 0.54),
        (3.06, 0.33),
        (3.66, 0.03),
    ],
    bottom: &[
        (-4.16, -0.23),
        (-2.36, -0.4),
        (-0.55, -0.51),
        (1.25, -0.58),
        (2.45, -0.5),
        (3.66, -0.21),
    ],
    turret_z: (-4.0, 2.9),
    front: v(0.0, -0.09, 3.55),
    rear: v(0.0, 0.4, -5.3),
    turret_scale: 0.55,
};

/// The F11F-1 Tiger: 14.3 m long, 9.6 m span.
pub static JET_FIGHTER: ModelSpec = ModelSpec {
    path: "models/jetfighter.glb",
    scale: 1.57,
    roll_y: 0.3,
    camera: CameraSpec {
        offset: v(0.0, 4.1, 12.4),
        zoom_offset: v(0.0, 3.9, 9.1),
        tilt_deg: 5.3,
    },
    nose: v(0.0, 0.01, 3.58),
    spinner_muzzle_z: 3.66,
    ring_muzzle_z: 3.0,
    ring_radius: 0.15,
    belly_y: -0.3,
    belly_spacing: 0.2,
    wing_root_x: 0.5,
    wing_max_x: 2.7,
    wing_spacing: 0.25,
    wing_le: (-0.2, -0.82),
    wing_y: (0.36, -0.057),
    boxes: &[
        // Fuselage
        (v(-0.55, -0.26, -4.9), v(0.55, 0.85, 3.63)),
        // Fin
        (v(-0.12, 0.8, -5.52), v(0.12, 2.24, -3.9)),
        // Wings, inner and outer
        (v(-1.5, 0.21, -2.9), v(1.5, 0.44, -0.15)),
        (v(-3.04, 0.21, -3.59), v(3.04, 0.44, -1.3)),
        // Tailplane
        (v(-1.48, 0.12, -5.04), v(1.45, 0.19, -3.63)),
    ],
    cockpit: (v(-0.25, 0.34, 0.39), v(0.25, 0.87, 2.04)),
    top: &[
        (-3.6, 0.8),
        (0.6, 0.83),
        (1.16, 0.69),
        (1.77, 0.57),
        (2.37, 0.44),
        (2.97, 0.30),
        (3.58, 0.09),
    ],
    bottom: &[
        (-3.6, -0.2),
        (-2.46, -0.25),
        (2.37, -0.23),
        (2.97, -0.17),
        (3.58, -0.07),
    ],
    turret_z: (-3.6, 2.9),
    front: v(0.0, 0.01, 3.5),
    rear: v(0.0, 0.25, -5.0),
    turret_scale: 0.55,
};

/// The Avro Shackleton MR.2: 26.6 m long, 36.6 m span.
pub static BOMBER: ModelSpec = ModelSpec {
    path: "models/bomber.glb",
    scale: 0.90,
    roll_y: -0.3,
    camera: CameraSpec {
        offset: v(0.0, 11.0, 26.4),
        zoom_offset: v(0.0, 10.4, 19.4),
        tilt_deg: 10.6,
    },
    nose: v(0.0, -0.3, 14.7),
    spinner_muzzle_z: 14.85,
    ring_muzzle_z: 13.8,
    ring_radius: 0.45,
    belly_y: -2.4,
    belly_spacing: 0.4,
    wing_root_x: 1.5,
    wing_max_x: 18.0,
    wing_spacing: 0.6,
    wing_le: (7.0, -0.09),
    wing_y: (-0.16, 0.04),
    boxes: &[
        // Fuselage
        (v(-1.52, -2.36, -14.93), v(1.52, 1.63, 14.75)),
        // Wings, inner and outer
        (v(-9.0, -0.92, 1.0), v(9.0, 0.89, 6.87)),
        (v(-20.15, -0.6, 1.6), v(20.15, 0.89, 6.4)),
        // Engines
        (v(3.21, -1.73, -0.13), v(9.64, 0.32, 9.15)),
        (v(-9.64, -1.73, -0.13), v(-3.21, 0.32, 9.15)),
        // Tailplane and fins
        (v(-5.5, 1.13, -11.74), v(5.5, 1.53, -8.27)),
        (v(5.5, -0.97, -11.73), v(5.96, 4.28, -8.41)),
        (v(-5.96, -0.97, -11.73), v(-5.5, 4.28, -8.41)),
    ],
    cockpit: (v(-1.27, 0.25, 9.31), v(1.27, 1.57, 10.87)),
    top: &[
        (-14.9, 1.14),
        (-12.9, 1.47),
        (-10.9, 1.56),
        (-9.0, 1.6),
        (8.8, 1.63),
        (11.5, 0.8),
        (12.7, 0.73),
        (14.7, 0.23),
    ],
    bottom: &[
        (-14.9, 0.55),
        (-12.9, -0.28),
        (-10.94, -0.65),
        (-8.97, -0.95),
        (-7.0, -1.19),
        (-5.0, -1.43),
        (-3.0, -1.67),
        (-1.08, -1.91),
        (0.89, -2.15),
        (2.87, -2.36),
        (6.81, -2.35),
        (8.78, -2.26),
        (12.73, -1.76),
        (14.7, -0.77),
    ],
    turret_z: (-12.5, 12.5),
    front: v(0.0, -0.27, 14.55),
    rear: v(0.0, 0.85, -14.75),
    turret_scale: 0.9,
};

/// Marks the airplane model under a plane, which gun and turret models
/// attach to, and which model it shows.
#[derive(Component)]
pub struct PlaneModel(pub ModelKind);

/// The model entity for an airplane of this kind.
pub fn plane_model(assets: &AssetServer, kind: ModelKind) -> impl Bundle {
    let spec = kind.spec();
    (
        WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(spec.path))),
        spec.correction(),
        PlaneModel(kind),
    )
}

pub struct ModelsPlugin;

impl Plugin for ModelsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, swap_models);
    }
}

/// Swap a plane's model when its preset picks a different one.
fn swap_models(
    mut commands: Commands,
    assets: Res<AssetServer>,
    planes: Query<(Entity, &crate::flight::Aircraft, Option<&Children>)>,
    models: Query<&PlaneModel>,
) {
    for (plane, aircraft, children) in &planes {
        let want = aircraft.card.model_kind();
        let mut has = false;
        for c in children.into_iter().flat_map(|c| c.iter()) {
            if let Ok(m) = models.get(c) {
                if m.0 == want {
                    has = true;
                } else {
                    commands.entity(c).despawn();
                }
            }
        }
        if !has {
            commands.entity(plane).with_children(|p| {
                p.spawn(plane_model(&assets, want));
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_are_full_size() {
        // Real lengths: Yak-3 8.5 m, F7F 13.8 m, F11F 14.3 m, Shackleton 26.6 m.
        for (kind, length) in [
            (ModelKind::Fighter, 8.5),
            (ModelKind::HeavyFighter, 13.8),
            (ModelKind::JetFighter, 14.3),
            (ModelKind::Bomber, 26.6),
        ] {
            let s = kind.spec();
            let model_len = s.boxes.iter().map(|b| b.1.z).fold(f32::MIN, f32::max)
                - s.boxes.iter().map(|b| b.0.z).fold(f32::MAX, f32::min);
            let real = model_len * s.scale;
            assert!(
                (real - length).abs() < length * 0.06,
                "{kind:?} is {real:.1} m long"
            );
        }
    }

    #[test]
    fn profiles_interpolate() {
        let s = ModelKind::Bomber.spec();
        assert!((s.top_at(0.0) - 1.61).abs() < 0.05);
        assert!(s.bottom_at(4.0) < -2.2);
        assert_eq!(s.top_at(-100.0), 1.14);
    }
}
