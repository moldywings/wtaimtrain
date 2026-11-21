//! Reference scenery: lighting, ground and scattered buildings so
//! speed, height and range are readable.

use bevy::prelude::*;

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (spawn_world, spawn_clouds))
            .add_systems(Update, wrap_clouds.after(crate::flight::fly_player));
    }
}

/// The ground is a disc this wide, m.
const GROUND_RADIUS: f32 = 60_000.0;
const SCATTER_RADIUS: f32 = 15_000.0;
/// Fields and roads cover the ground out to here, m.
const FARMLAND_RADIUS: f32 = 30_000.0;
/// Farmland comes in square districts this wide, each turned its own way.
const DISTRICT: f32 = 2_400.0;

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

    // Ground: a big round plain.
    commands.spawn((
        Mesh3d(meshes.add(Circle::new(GROUND_RADIUS).mesh().resolution(160))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.33, 0.45, 0.25),
            perceptual_roughness: 1.0,
            ..default()
        })),
        Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
    ));

    let mut rng = Lcg(0x5eed_1234);

    // Fields and roads, all in one flat mesh (one draw call) painted with
    // vertex colors. They make it easy to judge height down low.
    let mut farm = FlatMesh::default();
    let field_colors = [
        Vec3::new(0.45, 0.50, 0.28),
        Vec3::new(0.55, 0.48, 0.30),
        Vec3::new(0.28, 0.40, 0.22),
        Vec3::new(0.62, 0.56, 0.34),
        Vec3::new(0.38, 0.47, 0.24),
    ];
    let road = Vec3::new(0.50, 0.47, 0.42);
    let n = (FARMLAND_RADIUS / DISTRICT).ceil() as i32;
    for dx in -n..=n {
        for dz in -n..=n {
            let center = Vec2::new(dx as f32, dz as f32) * DISTRICT;
            if center.length() > FARMLAND_RADIUS {
                continue;
            }
            let angle = rng.range(-0.5, 0.5);
            let turn = Vec2::from_angle(angle);
            let size = rng.range(220.0, 480.0);
            // Turned, the district (and its roads) must still fit in its own
            // square, or its fields overlap the neighbors' and flicker.
            let fits = (DISTRICT / 2.0 - 20.0) / (angle.cos().abs() + angle.sin().abs());
            let cells = (2.0 * fits / size).floor().max(1.0) as i32;
            let half = cells as f32 * size / 2.0;
            for i in 0..cells {
                for j in 0..cells {
                    if rng.next() > 0.7 {
                        continue;
                    }
                    let local = Vec2::new(
                        i as f32 * size - half + size / 2.0,
                        j as f32 * size - half + size / 2.0,
                    );
                    let at = center + turn.rotate(local);
                    let color = field_colors[rng.index(field_colors.len())] * rng.range(0.92, 1.08);
                    farm.quad(at, turn, Vec2::splat(size * 0.92), 0.8, color);
                }
            }
            // A road along one or two sides of the district.
            let edge = half + 6.0;
            if rng.next() < 0.6 {
                let at = center + turn.rotate(Vec2::new(0.0, -edge));
                farm.quad(at, turn, Vec2::new(half * 2.0 + 12.0, 9.0), 1.4, road);
            }
            if rng.next() < 0.4 {
                let at = center + turn.rotate(Vec2::new(-edge, 0.0));
                farm.quad(at, turn, Vec2::new(9.0, half * 2.0 + 12.0), 1.4, road);
            }
        }
    }
    // Long straight highways across the whole map.
    for _ in 0..24 {
        let a = rng.disc(FARMLAND_RADIUS * 0.8);
        let dir = Vec2::from_angle(rng.range(0.0, std::f32::consts::PI));
        let len = rng.range(15_000.0, 40_000.0);
        farm.quad(a, dir, Vec2::new(len, 14.0), 1.8, road * 0.9);
    }
    commands.spawn((
        Mesh3d(meshes.add(farm.build())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 1.0,
            // Draw over the ground even far away, without flicker.
            depth_bias: 50.0,
            ..default()
        })),
    ));

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
}

/// A puffy cloud: a cluster of flattened spheres.
#[derive(Component)]
struct Cloud;

/// Clouds fill a square this far each side of the player, and those that
/// fall behind reappear ahead, so there are always some around.
const CLOUD_RANGE: f32 = 9_000.0;
const CLOUD_COUNT: usize = 70;
const CLOUD_ALPHA: f32 = 0.78;

fn spawn_clouds(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Each cloud is one mesh, so its puffs always draw in the same order
    // (no flicker as the camera moves). Unlit, with the shading baked into
    // vertex colors, so flying through one stays cheap.
    let material = materials.add(StandardMaterial {
        // Above 1 so the vertex colors come out bright after tone mapping.
        base_color: Color::linear_rgba(1.4, 1.4, 1.4, 1.0),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    });
    let sphere = Sphere::new(1.0).mesh().ico(2).unwrap();
    let positions: Vec<Vec3> = match sphere.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) => {
            p.iter().map(|v| Vec3::from(*v)).collect()
        }
        _ => Vec::new(),
    };
    let indices: Vec<u32> = sphere
        .indices()
        .map(|i| i.iter().map(|i| i as u32).collect())
        .unwrap_or_default();
    let sun = Vec3::new(-0.4, 1.0, 0.6).normalize();
    let mut rng = Lcg(0xc10d_5eed);
    for _ in 0..CLOUD_COUNT {
        let x = rng.range(-CLOUD_RANGE, CLOUD_RANGE);
        let z = rng.range(-CLOUD_RANGE, CLOUD_RANGE);
        // Several layers, so climbing and diving go past them too.
        let y = rng.range(700.0, 4_500.0);
        let size = rng.range(60.0, 220.0);
        let mut pos = Vec::new();
        let mut normals = Vec::new();
        let mut colors = Vec::new();
        let mut idx = Vec::new();
        let puffs = 4 + rng.index(6);
        for i in 0..puffs {
            let r = size * rng.range(0.45, 1.0);
            let at = Vec3::new(
                rng.range(-1.6, 1.6) * size,
                rng.range(-0.15, 0.35) * size,
                rng.range(-1.0, 1.0) * size,
            );
            let scale = Vec3::new(r, r * 0.55, r);
            let tint = if i % 3 == 2 { 0.9 } else { 1.0 };
            let base = pos.len() as u32;
            for v in &positions {
                pos.push((at + *v * scale).to_array());
                let n = (*v / scale).normalize_or(Vec3::Y);
                normals.push(n.to_array());
                // Bright on top and toward the sun, grey underneath.
                let light = 0.72 + 0.18 * n.y + 0.1 * n.dot(sun).max(0.0);
                let c = Color::srgb(
                    0.97 * light * tint,
                    0.98 * light * tint,
                    1.0 * light * tint.max(0.94),
                )
                .to_linear();
                colors.push([c.red, c.green, c.blue, CLOUD_ALPHA]);
            }
            idx.extend(indices.iter().map(|i| base + i));
        }
        let mesh = Mesh::new(
            bevy::mesh::PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
        .with_inserted_indices(bevy::mesh::Indices::U32(idx));
        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(material.clone()),
            Transform::from_xyz(x, y, z),
            Cloud,
        ));
    }
}

fn wrap_clouds(
    player: Single<&Transform, (With<crate::flight::Player>, Without<Cloud>)>,
    mut clouds: Query<&mut Transform, With<Cloud>>,
) {
    let p = player.translation;
    let span = CLOUD_RANGE * 2.0;
    for mut tf in &mut clouds {
        for axis in [0, 2] {
            let d = tf.translation[axis] - p[axis];
            if d > CLOUD_RANGE {
                tf.translation[axis] -= span;
            } else if d < -CLOUD_RANGE {
                tf.translation[axis] += span;
            }
        }
    }
}

/// Flat colored quads on the ground, gathered into one mesh.
#[derive(Default)]
struct FlatMesh {
    positions: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
}

impl FlatMesh {
    /// A `size` (length along `dir`, width across) rectangle centered on
    /// `at`, `y` above the ground.
    fn quad(&mut self, at: Vec2, dir: Vec2, size: Vec2, y: f32, color: Vec3) {
        let across = dir.perp();
        let (hx, hz) = (dir * size.x / 2.0, across * size.y / 2.0);
        let base = self.positions.len() as u32;
        for c in [at - hx - hz, at + hx - hz, at + hx + hz, at - hx + hz] {
            self.positions.push([c.x, y, c.y]);
            // Vertex colors are linear; the numbers above are sRGB.
            let c = Color::srgb(color.x, color.y, color.z).to_linear();
            self.colors.push([c.red, c.green, c.blue, 1.0]);
        }
        // Counter-clockwise seen from above, so the face points up.
        let order = if dir.perp_dot(across) > 0.0 {
            [0, 2, 1, 0, 3, 2]
        } else {
            [0, 1, 2, 0, 2, 3]
        };
        self.indices.extend(order.iter().map(|i| base + i));
    }

    fn build(self) -> Mesh {
        let normals = vec![[0.0, 1.0, 0.0]; self.positions.len()];
        Mesh::new(
            bevy::mesh::PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colors)
        .with_inserted_indices(bevy::mesh::Indices::U32(self.indices))
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
