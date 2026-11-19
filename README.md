# WT Aim Trainer

A desktop aim trainer for War Thunder Air Realistic Battles, built with Rust and [Bevy](https://bevy.org). You fly with mouse aim, the way the game does, and practice shooting at a target aircraft.

## Running

You need the Rust toolchain (installed with [rustup](https://rustup.rs)). From the repository folder:

```
cargo run --release
```

Run it with `cargo run` from the repository folder so the game can find `assets/`. The first build takes several minutes while Bevy compiles. After that it's quick.

## Controls

| Input | Action |
| --- | --- |
| Mouse | Move the aim circle. The camera swings after it and the plane turns to follow |
| Right click | Toggle zoom |
| L Shift / L Ctrl | Throttle up / down |
| S / W | Pitch up / down (overrides mouse aim while held) |
| A / D | Roll left / right |
| Q / E | Yaw left / right |
| L Alt (hold) | Free look |
| R | Reset position |
| Esc | Pause menu |

The pause menu lets you change units (metric, imperial or nautical), mouse sensitivity and every key binding. Settings are saved to `settings.json` in the folder you run the game from.

The white circle is where you're aiming. The green square is where the nose (and later, the guns) point. The red box marks the target and shows its range.

## Layout

- `src/camera.rs` handles mouse aim and the chase camera.
- `src/flight.rs` is the player plane and the controller that turns it toward the aim point.
- `src/target.rs` flies the target aircraft.
- `src/hud.rs` draws the aim circle, nose marker, target box and flight readout.
- `src/world.rs` sets up the lighting, ground and buildings.
- `src/settings.rs` holds units, sensitivity and key bindings, and saves them.
- `src/menu.rs` is the pause menu.
- `assets/models/airplane.glb` is the aircraft model. It's scaled 1.33x in code to roughly match a real Yak-3.

## Roadmap

1. Flight and mouse aim prototype (this step)
2. Guns with set velocity, spread and rate of fire, bullet drop, hit detection, and saved gun presets
3. Target scenarios: crossing, head-on, turning tail chase, long range
4. Scoring and a settings menu
