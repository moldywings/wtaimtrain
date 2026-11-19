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
| Left click (hold) | Fire all guns |
| Right click | Toggle zoom |
| L Shift / L Ctrl | Throttle up / down |
| S / W | Pitch up / down (overrides mouse aim while held) |
| A / D | Roll left / right |
| Q / E | Yaw left / right |
| L Alt (hold) | Free look |
| R | Restart the scenario |
| Esc | Pause menu |

The pause menu has four pages. **Settings** covers units (metric, imperial or nautical), aim and free look sensitivity (aim feels the same zoomed or not), crosshair style and color, the lead indicator, and every key binding. **Aircraft** has the flight model presets: you pick which preset you fly and which one the target flies from drop-down lists. **Weapons** has gun presets and each aircraft preset's loadout. **Scenario** picks what you practice. Settings and all your presets are saved to `WT Aim Trainer/settings.json` in your user config folder (`%APPDATA%` on Windows, `~/.config` on Linux), so they're there every time you open the game. Set `WTAT_CONFIG_DIR` to use another folder.

The white circle is where you're aiming. The crosshair is where the nose points. The target shows its preset name, a bracket marker and its range. When it's off screen, a red arrow at the screen edge points to it. The readout shows indicated (IAS) and true (TAS) airspeed. Hits and accuracy show in the top right, and "HIT" flashes when a round connects. With the lead indicator on, a green circle shows where to put your gun crosshair to hit the target. It's worked out for the biggest gun on your plane, counting your speed, the target's speed, bullet drop and drag.

## Scenarios

- **Head on:** you fly straight at each other. The target holds a straight line at a random speed, from a slightly different height and toward a different point near you each time.
- **Chase:** you start behind the target, a random distance between the two you set (300 to 800 m by default). It keeps its heading but weaves and dodges: some runs hard, some gently, some not at all.
- **Stall:** the target starts just above and ahead of you and pulls straight up, rolling and dodging.
- **Dogfight:** the target starts close by, usually ahead of you, at a random spot, height and speed, and tries to ram you.
- **All in one:** a random one of the others each time.

When the target takes enough hits it's destroyed, with a sound and a "TARGET DESTROYED" notice, and the scenario restarts 2 seconds later. It also restarts when a plane crashes, the planes collide, the target gets more than 4 km away, a head-on pass is over, or you press R.

## Flight model

The default flight model is the Yak-3. Planes are flown with physics: lift, drag, thrust, gravity, and stalls once the angle of attack is too high. Mouse aim works through an instructor that keeps you out of a stall. Near the nose it flies coordinated turns: bank, pitch and rudder are worked out together from one wanted turn, so small aim moves don't make the crosshair jump. Holding a pitch key takes over every axis from it, but like the game's instructor it still stops you just short of a stall. Roll and yaw keys only take over their own axis. The aim circle can go straight up and over the top, and the view keeps the horizon level.

A preset is a stat card. It has power per engine (hp, or kgf of thrust for jets), the number of engines, climb rate, top speed, and the max sustained and max instantaneous turn rates, each in degrees per second at a speed (the Yak-3: 23.59 deg/s at 175 kt sustained, 41.1 deg/s at 302 kt instantaneous). The sustained turn sets the induced drag. The instantaneous turn sets the G limit and, unless you enter one, the max lift coefficient. Top speed altitude, mass, wing area, max lift coefficient and best roll rate (in deg/s at a speed) are optional: leave them blank and the game fills them in from typical values for the engine type and power, and the editor lists what it picked. The game solves for drag, induced drag and propeller efficiency so the plane reproduces those numbers. The editor shows a "model check" with the figures the solved model actually reaches.

Save as many presets as you like. Saving under a new name makes a new preset and keeps the old one. The built-in Yak-3 can't be changed or deleted: edits to it are saved as a copy.

Roll rate builds up with speed to the preset's best roll rate at its best roll speed, then falls off gently above that (to about three quarters at 1.6 times that speed). Left blank, the best roll speed is 60% of top speed, so fast planes keep their roll.

## Weapons

A gun preset has a name, muzzle velocity (m/s), projectile mass (kg), fire rate (rounds per minute) and tracer color. Caliber, spread and hits to kill are optional. A blank caliber is estimated from the projectile mass, spread defaults to 2 mils, and hits to kill comes from the caliber (about 6 for 20 mm, 16 for 12.7 mm, 50 for 7.62 mm). Hits from different guns add up: each takes its share of the target's health. Rounds fly with gravity and air drag worked out from the caliber and mass. The editor shows each gun's flight time, remaining speed and drop at 1 km. Tracers are true to caliber (thin round streaks with a soft glow just wider than the round, kept about a pixel wide far away), and disappear after 2.5 miles (about 4 km).

Gunshots and hits have sounds made in code, with no sound files. Small, fast rounds crack higher and big, slow ones boom lower. The built-in gun is the 20 mm ShVAK (800 m/s, 0.1 kg, 800 rpm), and it can't be changed or deleted. The 12.7 mm UBS comes as a second preset.

Each aircraft preset has a loadout with four mounting spots, and each spot can carry a different gun preset:

- Nose, in the spinner: 1 gun through the propeller hub.
- Nose, around it: up to 6, on a hexagon around the spinner gun.
- Belly: up to 4 under the fuselage.
- Wings: up to 4 per wing (8 in total).

The gun model (`assets/models/gun.glb`) appears in each spot, sized by caliber. Nose guns fire straight ahead. Wing and belly guns are angled to meet at the loadout's convergence range, 400 m by default, which you set with - and + on the Weapons page (100 to 1500 m, in 50 m steps), or turn off so they fire parallel. The Yak-3 carries a ShVAK in the spinner and two UBS above it. Changing its loadout saves a copy.

## Layout

- `src/camera.rs` handles mouse aim and the chase camera.
- `src/flight_model.rs` holds the stat card, the solver that fills in the blanks, the physics, and the mouse aim instructor.
- `src/flight.rs` is the player plane: keyboard and mouse input, and applying presets.
- `src/target.rs` runs the scenarios: it places both planes, flies the target, counts damage and restarts runs.
- `src/hud.rs` draws the aim circle, nose marker, target box and flight readout.
- `src/world.rs` sets up the lighting, ground, buildings and clouds.
- `src/settings.rs` holds units, sensitivity and key bindings, and saves them.
- `src/menu.rs` is the pause menu.
- `src/sound.rs` makes the gunshot and hit sounds.
- `src/weapons.rs` holds gun presets, loadouts and mount positions, firing, ballistics and hit detection.
- `assets/models/airplane.glb` is the aircraft model. It's scaled 1.33x in code to roughly match a real Yak-3.

## Roadmap

1. Flight and mouse aim prototype (done)
2. Guns with set velocity, spread and rate of fire, bullet drop, hit detection, and saved gun presets (done)
3. Target scenarios: head on, chase, stall, dogfight (done)
4. Scoring and a settings menu
