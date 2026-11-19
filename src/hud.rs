//! On-screen markers: the mouse aim circle, the gun crosshair (where the nose
//! points), the target marker with name and range, and a flight readout.

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};

use crate::GameState;
use crate::camera::{ChaseCamera, MouseAim};
use crate::flight::{Aircraft, Player};
use crate::settings::{Crosshair, CrosshairColor, Settings};
use crate::target::Target;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_hud).add_systems(
            PostUpdate,
            (rebuild_crosshair, update_hud)
                .chain()
                .after(TransformSystems::Propagate),
        );
    }
}

#[derive(Component)]
struct AimCircle;

#[derive(Component)]
struct GunCrosshair;

#[derive(Component)]
struct TargetMarker;

#[derive(Component)]
struct TargetName;

#[derive(Component)]
struct TargetRange;

#[derive(Component)]
struct FlightReadout;

#[derive(Component)]
struct StallWarning;

#[derive(Component)]
struct HelpText;

const AIM_SIZE: f32 = 26.0;
const CROSSHAIR_SIZE: f32 = 40.0;

const TARGET_RED: Color = Color::srgb(0.95, 0.18, 0.15);
const BRACKET: Color = Color::srgb(0.92, 0.92, 0.92);
const RETICLE_WHITE: Color = Color::srgb(0.95, 0.95, 0.95);
const RETICLE_GOLD: Color = Color::srgb(0.85, 0.72, 0.45);

// Target marker layout: name, then the bracket diamond, then the range.
const MARKER_WIDTH: f32 = 180.0;
const NAME_HEIGHT: f32 = 22.0;
const DIAMOND_BOX: f32 = 44.0;

fn shadow() -> TextShadow {
    TextShadow {
        offset: Vec2::splat(1.0),
        color: Color::srgba(0.0, 0.0, 0.0, 0.7),
    }
}

fn absolute(width: f32, height: f32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        width: px(width),
        height: px(height),
        ..default()
    }
}

/// A filled rectangle at a position inside its parent.
fn rect(x: f32, y: f32, w: f32, h: f32, color: Color, round: bool) -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            left: px(x),
            top: px(y),
            width: px(w),
            height: px(h),
            border_radius: if round {
                BorderRadius::MAX
            } else {
                BorderRadius::ZERO
            },
            ..default()
        },
        BackgroundColor(color),
    )
}

/// A ring of diameter `d` centered in a `size` box.
fn ring(size: f32, d: f32, width: f32, color: Color) -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            left: px((size - d) / 2.0),
            top: px((size - d) / 2.0),
            width: px(d),
            height: px(d),
            border: UiRect::all(px(width)),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        BorderColor::all(color),
    )
}

fn dot(size: f32, d: f32, color: Color) -> impl Bundle {
    let o = (size - d) / 2.0;
    rect(o, o, d, d, color, true)
}

/// Four ticks around the center with a gap, like a gunsight cross.
fn ticks(parent: &mut ChildSpawnerCommands, size: f32, gap: f32, len: f32, w: f32, color: Color) {
    let c = size / 2.0;
    parent.spawn(rect(c - w / 2.0, c - gap - len, w, len, color, false));
    parent.spawn(rect(c - w / 2.0, c + gap, w, len, color, false));
    parent.spawn(rect(c - gap - len, c - w / 2.0, len, w, color, false));
    parent.spawn(rect(c + gap, c - w / 2.0, len, w, color, false));
}

fn spawn_hud(mut commands: Commands) {
    commands.spawn((
        Node {
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::MAX,
            ..absolute(AIM_SIZE, AIM_SIZE)
        },
        BorderColor::all(Color::WHITE),
        Visibility::Hidden,
        AimCircle,
    ));

    commands.spawn((
        absolute(CROSSHAIR_SIZE, CROSSHAIR_SIZE),
        Visibility::Hidden,
        GunCrosshair,
    ));

    // Target marker: red name, bracket diamond with a red dot, red range.
    commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..absolute(MARKER_WIDTH, NAME_HEIGHT + DIAMOND_BOX + 24.0)
            },
            Visibility::Hidden,
            TargetMarker,
        ))
        .with_children(|m| {
            m.spawn((
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(17.0),
                    ..default()
                },
                TextColor(TARGET_RED),
                shadow(),
                Node {
                    height: px(NAME_HEIGHT),
                    ..default()
                },
                TargetName,
            ));
            m.spawn(Node {
                width: px(DIAMOND_BOX),
                height: px(DIAMOND_BOX),
                ..default()
            })
            .with_children(|d| {
                // A square of corner brackets turned 45° into a diamond.
                let side = 30.0;
                let off = (DIAMOND_BOX - side) / 2.0;
                d.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(off),
                        top: px(off),
                        width: px(side),
                        height: px(side),
                        ..default()
                    },
                    UiTransform::from_rotation(Rot2::degrees(45.0)),
                ))
                .with_children(|sq| {
                    let arm = 10.0;
                    let w = 2.0;
                    for (x, y, border) in [
                        (0.0, 0.0, UiRect::new(px(w), px(0), px(w), px(0))),
                        (side - arm, 0.0, UiRect::new(px(0), px(w), px(w), px(0))),
                        (0.0, side - arm, UiRect::new(px(w), px(0), px(0), px(w))),
                        (
                            side - arm,
                            side - arm,
                            UiRect::new(px(0), px(w), px(0), px(w)),
                        ),
                    ] {
                        sq.spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                left: px(x),
                                top: px(y),
                                width: px(arm),
                                height: px(arm),
                                border,
                                ..default()
                            },
                            BorderColor::all(BRACKET),
                        ));
                    }
                });
                // Red dot sits at the top tip of the diamond, like the game.
                let tip = (DIAMOND_BOX - side * std::f32::consts::SQRT_2) / 2.0;
                d.spawn(rect(
                    DIAMOND_BOX / 2.0 - 3.5,
                    tip - 1.0,
                    7.0,
                    7.0,
                    TARGET_RED,
                    true,
                ));
            });
            m.spawn((
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(17.0),
                    ..default()
                },
                TextColor(TARGET_RED),
                shadow(),
                TargetRange,
            ));
        });

    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(16.0),
            ..default()
        },
        shadow(),
        Node {
            position_type: PositionType::Absolute,
            top: px(10),
            left: px(12),
            ..default()
        },
        FlightReadout,
    ));
    commands.spawn((
        Text::new("STALL"),
        TextFont {
            font_size: FontSize::Px(26.0),
            ..default()
        },
        TextColor(TARGET_RED),
        shadow(),
        Node {
            position_type: PositionType::Absolute,
            top: percent(30),
            left: percent(50),
            margin: UiRect::left(px(-36)),
            ..default()
        },
        Visibility::Hidden,
        StallWarning,
    ));
    commands.spawn((
        Text::new("Click to capture the mouse    Esc: pause menu"),
        TextFont {
            font_size: FontSize::Px(16.0),
            ..default()
        },
        shadow(),
        Node {
            position_type: PositionType::Absolute,
            bottom: px(12),
            left: px(12),
            ..default()
        },
        HelpText,
    ));
}

/// Redraw the gun crosshair when its style changes.
fn rebuild_crosshair(
    mut commands: Commands,
    settings: Res<Settings>,
    mut current: Local<Option<(Crosshair, CrosshairColor)>>,
    crosshair: Single<Entity, With<GunCrosshair>>,
) {
    let style = settings.crosshair;
    let choice = settings.crosshair_color;
    if *current == Some((style, choice)) {
        return;
    }
    *current = Some((style, choice));
    let white = choice.color().unwrap_or(RETICLE_WHITE);
    let gold = choice.color().unwrap_or(RETICLE_GOLD);
    let s = CROSSHAIR_SIZE;
    let mut root = commands.entity(*crosshair);
    root.despawn_children();
    root.with_children(|c| match style {
        Crosshair::Cross => {
            ticks(c, s, 6.0, 10.0, 2.0, white);
            c.spawn(dot(s, 3.0, white));
        }
        Crosshair::Circle => {
            c.spawn(ring(s, 22.0, 2.0, gold));
        }
        Crosshair::CircleDot => {
            c.spawn(ring(s, 22.0, 2.0, gold));
            c.spawn(dot(s, 3.0, gold));
        }
        Crosshair::Dot => {
            c.spawn(dot(s, 4.0, gold));
        }
        Crosshair::SmallCross => {
            ticks(c, s, 2.0, 5.0, 2.0, gold);
        }
    });
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn update_hud(
    aim: Res<MouseAim>,
    settings: Res<Settings>,
    state: Res<State<GameState>>,
    cursor: Single<&CursorOptions>,
    camera: Single<(&Camera, &GlobalTransform), With<ChaseCamera>>,
    player: Single<(&GlobalTransform, &Aircraft), With<Player>>,
    target: Single<(&GlobalTransform, &Aircraft), With<Target>>,
    mut nodes: ParamSet<(
        Single<(&mut Node, &mut Visibility), With<AimCircle>>,
        Single<(&mut Node, &mut Visibility), With<GunCrosshair>>,
        Single<(&mut Node, &mut Visibility), With<TargetMarker>>,
        Single<&mut Visibility, With<StallWarning>>,
        Single<&mut Visibility, With<HelpText>>,
    )>,
    mut texts: ParamSet<(
        Single<&mut Text, With<TargetName>>,
        Single<&mut Text, With<TargetRange>>,
        Single<&mut Text, With<FlightReadout>>,
    )>,
) {
    let (cam, cam_tf) = *camera;
    let (player_tf, plane) = *player;
    let (target_tf, target_plane) = *target;
    let units = settings.units;
    let project = |world: Vec3| -> Option<Vec2> {
        // Behind the camera means not on screen.
        if (world - cam_tf.translation()).dot(cam_tf.forward().as_vec3()) <= 0.0 {
            return None;
        }
        cam.world_to_viewport(cam_tf, world).ok()
    };

    // Mouse aim circle: a point far along the aim direction.
    let aim_point = cam_tf.translation() + aim.direction() * 20_000.0;
    place(
        nodes.p0().into_inner(),
        project(aim_point),
        Vec2::splat(AIM_SIZE / 2.0),
    );

    // Gun crosshair: where the nose is actually pointing.
    let nose_point = player_tf.translation() + player_tf.forward().as_vec3() * 20_000.0;
    place(
        nodes.p1().into_inner(),
        project(nose_point),
        Vec2::splat(CROSSHAIR_SIZE / 2.0),
    );

    // Target marker, anchored so the diamond's center sits on the target.
    let target_pos = target_tf.translation();
    let range = player_tf.translation().distance(target_pos);
    place(
        nodes.p2().into_inner(),
        project(target_pos),
        Vec2::new(MARKER_WIDTH / 2.0, NAME_HEIGHT + DIAMOND_BOX / 2.0),
    );
    texts.p0().0 = target_plane.model.name.clone();
    texts.p1().0 = units.range(range);

    let body = &plane.body;
    texts.p2().0 = format!(
        "{}\nIAS {}\nALT {}\nTHR {:.0} %\nG {:.1}",
        plane.model.name,
        units.speed(body.speed()),
        units.distance(body.position.y),
        plane.controls.throttle * 100.0,
        body.g_load,
    );
    **nodes.p3() = if plane.is_stalled() {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };

    **nodes.p4() = if cursor.grab_mode == CursorGrabMode::None && *state.get() == GameState::Playing
    {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
}

/// Move a marker so `anchor` (offset inside the marker) lands on `screen`.
fn place((mut node, mut vis): (Mut<Node>, Mut<Visibility>), screen: Option<Vec2>, anchor: Vec2) {
    match screen {
        Some(p) => {
            node.left = px(p.x - anchor.x);
            node.top = px(p.y - anchor.y);
            *vis = Visibility::Visible;
        }
        None => *vis = Visibility::Hidden,
    }
}
