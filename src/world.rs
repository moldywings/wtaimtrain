//! Reference scenery: lighting, ground, scattered buildings and clouds so
//! speed, height and range are readable.

use bevy::prelude::*;

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_world);
    }
}

const GROUND_SIZE: f32 = 60_000.0;
const SCATTER_RADIUS: f32 = 15_000.0;

fn spawn_world(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(GlobalAmbientLight {
        brightness: 600.0,
        ..default()
    });
    commands.spawn((
        DirectionalLight {
            illuminance: 12_000.0,
            ..default()
        },
        Transform::from_xyz(0.0, 1.0, 0.0).looking_at(Vec3::new(0.4, -1.0, -0.6), Vec3::Y),
    ));

    // Ground.
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(GROUND_SIZE, GROUND_SIZE))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.33, 0.45, 0.25),
            perceptual_roughness: 1.0,
            ..default()
        })),
    ));

    let mut rng = Lcg(0x5eed_1234);

    // Fields: flat patches in a few colors break up the ground.
    let field_mesh = meshes.add(Plane3d::default().mesh().size(1.0, 1.0));
    let field_colors = [
        Color::srgb(0.45, 0.50, 0.28),
        Color::srgb(0.55, 0.48, 0.30),
        Color::srgb(0.28, 0.40, 0.22),
    ];
    let field_mats: Vec<_> = field_colors
        .iter()
        .map(|c| {
            materials.add(StandardMaterial {
                base_color: *c,
                perceptual_roughness: 1.0,
                ..default()
            })
        })
        .collect();
    for _ in 0..500 {
        let pos = rng.disc(SCATTER_RADIUS * 1.5);
        commands.spawn((
            Mesh3d(field_mesh.clone()),
            MeshMaterial3d(field_mats[rng.index(field_mats.len())].clone()),
            Transform::from_xyz(pos.x, rng.range(0.5, 3.0), pos.y)
                .with_rotation(Quat::from_rotation_y(rng.range(0.0, std::f32::consts::PI)))
                .with_scale(Vec3::new(
                    rng.range(300.0, 900.0),
                    1.0,
                    rng.range(300.0, 900.0),
                )),
        ));
    }

    // Buildings: small boxes give a sense of height and speed down low.
    let building_mesh = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let building_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.70, 0.66, 0.60),
        ..default()
    });
    for _ in 0..600 {
        let pos = rng.disc(SCATTER_RADIUS);
        let h = rng.range(8.0, 40.0);
        commands.spawn((
            Mesh3d(building_mesh.clone()),
            MeshMaterial3d(building_mat.clone()),
            Transform::from_xyz(pos.x, h / 2.0, pos.y).with_scale(Vec3::new(
                rng.range(15.0, 40.0),
                h,
                rng.range(15.0, 40.0),
            )),
        ));
    }

    // Clouds: clusters of flattened white spheres between 1 and 3 km.
    let puff_mesh = meshes.add(Sphere::new(1.0).mesh().uv(16, 10));
    let cloud_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.97, 0.97, 1.0),
        perceptual_roughness: 1.0,
        ..default()
    });
    for _ in 0..70 {
        let center = rng.disc(SCATTER_RADIUS);
        let alt = rng.range(1_000.0, 3_000.0);
        for _ in 0..6 {
            let offset = Vec3::new(
                rng.range(-250.0, 250.0),
                rng.range(-40.0, 40.0),
                rng.range(-250.0, 250.0),
            );
            let r = rng.range(90.0, 200.0);
            commands.spawn((
                Mesh3d(puff_mesh.clone()),
                MeshMaterial3d(cloud_mat.clone()),
                Transform::from_translation(Vec3::new(center.x, alt, center.y) + offset)
                    .with_scale(Vec3::new(r, r * 0.45, r)),
            ));
        }
    }
}

/// Tiny deterministic random generator so the scenery is the same each run.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32) / ((1u64 << 24) as f32)
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }

    fn index(&mut self, len: usize) -> usize {
        ((self.next() * len as f32) as usize).min(len - 1)
    }

    fn disc(&mut self, radius: f32) -> Vec2 {
        let r = radius * self.next().sqrt();
        let a = self.next() * std::f32::consts::TAU;
        Vec2::new(r * a.cos(), r * a.sin())
    }
}
