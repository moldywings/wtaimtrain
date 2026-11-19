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
| Left click | Capture the mouse |
| Mouse | Move the aim circle; the plane turns to follow it |
| W / S (or Shift / Ctrl) | Throttle up / down |
| R | Reset position |
| Esc | Release the mouse |

The white circle is where you're aiming. The green square is where the nose (and later, the guns) point. The red box marks the target and shows its range.

## Layout

- `src/camera.rs` handles mouse aim and the chase camera.
- `src/flight.rs` is the player plane and the controller that turns it toward the aim point.
- `src/target.rs` flies the target aircraft.
- `src/hud.rs` draws the aim circle, nose marker, target box and flight readout.
- `src/world.rs` sets up the lighting, ground, buildings and clouds.
- `assets/models/airplane.glb` is the aircraft model. It's scaled 1.33x in code to roughly match a real Yak-3.

## Roadmap

1. Flight and mouse aim prototype (this step)
2. Guns with set velocity, spread and rate of fire, bullet drop, hit detection, and saved gun presets
3. Target scenarios: crossing, head-on, turning tail chase, long range
4. Scoring and a settings menu
