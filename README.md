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

The pause menu has two pages. **Settings** covers units (metric, imperial or nautical), mouse sensitivity, crosshair style and color (drop-down lists) and every key binding. **Aircraft** has the flight model presets: you pick which preset you fly and which one the target flies. Settings and all your presets are saved to `WT Aim Trainer/settings.json` in your user config folder (`%APPDATA%` on Windows, `~/.config` on Linux), so they're there every time you open the game. Set `WTAT_CONFIG_DIR` to use another folder.

The white circle is where you're aiming. The crosshair is where the nose (and later, the guns) point. The target shows its preset name, a bracket marker and its range.

## Flight model

The default flight model is the Yak-3. Planes are flown with physics: lift, drag, thrust, gravity, and stalls once the angle of attack is too high. Mouse aim works through an instructor that keeps you out of a stall. Holding a pitch, roll or yaw key overrides it, so you can stall the plane yourself.

A preset is a stat card. It has power per engine (hp, or kgf of thrust for jets), the number of engines, climb rate, sustained turn time and top speed. Top speed altitude, mass, wing area, max lift coefficient and roll rate are optional: leave them blank and the game fills them in from typical values for the engine type and power, and the editor lists what it picked. The game solves for drag, induced drag and propeller efficiency so the plane reproduces those numbers. The editor shows a "model check" with the figures the solved model actually reaches.

Save as many presets as you like. Saving under a new name makes a new preset and keeps the old one. The built-in Yak-3 can't be changed or deleted: edits to it are saved as a copy.

## Layout

- `src/camera.rs` handles mouse aim and the chase camera.
- `src/flight_model.rs` holds the stat card, the solver that fills in the blanks, the physics, and the mouse aim instructor.
- `src/flight.rs` is the player plane: keyboard and mouse input, and applying presets.
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
