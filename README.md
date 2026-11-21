# WT Aim Trainer

A desktop aim trainer for War Thunder Air Realistic Battles, built with Rust and [Bevy](https://bevy.org). You fly with mouse aim, the way the game does, and practice shooting at a target aircraft.

## Download (Windows)

Get `WT-Aim-Trainer-Windows.zip` from the [latest build](https://github.com/moldywings/wtaimtrain/releases/tag/latest), unzip it anywhere, and double-click `WT Aim Trainer.exe`. Keep the `assets` folder next to the .exe. If Windows says it protected your PC, click **More info**, then **Run anyway** (the game isn't code-signed). A new build is posted automatically every time `main` changes, and your settings and presets carry over between versions.

## Running from source

You need the Rust toolchain (installed with [rustup](https://rustup.rs)). From the repository folder:

```
cargo run --release
```

Run it with `cargo run` from the repository folder so the game can find `assets/`. The first build takes several minutes while Bevy compiles. After that it's quick.

## Controls

| Input | Action |
| --- | --- |
| Mouse | Move the aim circle. The camera swings after it and the plane turns to follow |
| Left click | Fire all guns (hold, or one shot per click with Single fire on) |
| Right click | Toggle zoom |
| L Shift / L Ctrl | Throttle up / down (you start at 100%; jets with an afterburner go to 110% to light it) |
| S / W | Pitch up / down (overrides mouse aim while held) |
| A / D | Roll left / right |
| Q / E | Yaw left / right |
| L Alt (hold) | Free look. The view holds still and level while you look all the way round, and the keyboard turns the plane under it (the aim circle rides along with the crosshair, so the plane flies straight) |
| F6 | Gunner view, on planes with at least one turret. The mouse aims every turret, left click fires them, and you fly with the keys |
| F3 | Back to the pilot view |
| R | Restart the scenario (in multiplayer the host holds it 3 s to start a round) |
| Tab (hold) | Multiplayer scoreboard |
| Esc | Pause menu |

The pause menu has six pages. **Settings** covers aim, zoomed aim and free look sensitivity, crosshair style and color, the lead indicator, single or automatic fire, and every key binding. **Units** has a drop-down for each kind of number: airspeed (km/h, mph, kn, m/s), altitude (m, ft), distance and range (km, mi, nmi, m, ft, yd), climb rate (m/s, ft/min, ft/s), engine power (hp, kW), thrust (kgf, lbf, kN), mass (kg, lb) and wing area (sq m, sq ft). Presets are stored in fixed units, so switching a unit converts everything shown, presets included. **Aircraft** has the flight model presets: you pick which preset you fly and which one the target flies from drop-down lists. **Weapons** has gun presets and each aircraft preset's loadout. **Scenario** picks what you practice and sets that scenario's start distances. **Multiplayer** hosts or joins a game (see below). Settings and all your presets are saved to `WT Aim Trainer/settings.json` in your user config folder (`%APPDATA%` on Windows, `~/.config` on Linux), so they're there every time you open the game. Set `WTAT_CONFIG_DIR` to use another folder.

The white circle is where you're aiming. The crosshair is where the nose points. The target shows its preset name, a bracket marker and its range. When it's off screen, a red arrow at the screen edge points to it. The readout shows indicated (IAS) and true (TAS) airspeed. Hits and accuracy show in the top right, and "HIT" flashes when a round connects. They start over when you change aircraft or guns. Underneath, once enough rounds have gone close by, it says whether your misses mostly pass ahead of the plane (over-leading) or behind it (under-leading). With the lead indicator on, a green circle shows where to put your gun crosshair to hit the target. It's worked out for the biggest gun on your plane, counting your speed, the target's speed, bullet drop and drag.

## Scenarios

- **Head on:** you fly straight at each other. The target holds a straight line at a random speed, from a slightly different height and toward a different point near you each time.
- **Chase:** you start behind the target (300 to 800 m by default). It keeps its heading but weaves and dodges: some runs hard, some gently, some not at all.
- **Stall:** the target starts just above and ahead of you and pulls straight up, rolling and dodging.
- **Dogfight:** the target starts close by, usually ahead of you, at a random spot, height, heading and speed, and dogfights you with its own guns: it leads its shots for its biggest gun, manages its speed and throttle, breaks off if it gets too close and keeps clear of the ground. Its hits show as "Your damage". At 100% you're shot down and the scenario resets 2 seconds later. The target shoots back in the other scenarios too, and every target AI pulls up before the ground.
- **Banking:** the target starts ahead (500 to 1300 m by default), off to one side and above, below or level with you, then pulls a hard banked turn, usually across your nose and sometimes away. The path changes every run (low to high, high to low, or straight across, from either side). After turning 150 to 300 degrees it rolls out, and the run ends after about 22 seconds.
- **Diving:** you start above the target (300 to 1200 m higher and 800 to 2000 m ahead by default) and dive on it. Half the time it hasn't seen you and flies straight on until you open fire. The other half it's watching and reacts as soon as you dive at it. Once alerted it goes to full throttle and dives away from you, leveling off near the ground, while jinking, rolling or corkscrewing (some hard, some gently, some not at all).
- **All in one:** a random one of the others each time, using each scenario's own start distances.

Each scenario's start distances (and the Diving height) are set on the Scenario page: click the scenario and its ranges show on the right. Each run picks a random value in each range.

When the target takes enough hits it's destroyed, with a sound and a "TARGET DESTROYED" notice, and the scenario restarts 2 seconds later. It also restarts when a plane crashes, the planes collide, the target gets more than 4 km away, a head-on pass is over, or you press R.

## Flight model

The default flight model is the Yak-3. Planes are flown with physics: lift, drag, thrust, gravity, and stalls once the angle of attack is too high. Mouse aim works through an instructor that keeps you out of a stall. Like War Thunder's, it takes whichever path to the aim is faster: small moves are mostly pitch and rudder with little roll, while bigger moves (or a diagonal one downward) roll the aim over the canopy, rolling onto the back if needed, and pull, with the rudder only for fine tuning at the end. Holding a pitch key takes over every axis from it, but like the game's instructor it still stops you just short of a stall. Roll and yaw keys only take over their own axis. The aim circle can go straight up and over the top, and the view keeps the horizon level.

A preset is a stat card. It has power per engine (or thrust for jets, with an optional afterburner and its thrust; leave the afterburner thrust blank for 1.5 times dry thrust), the number of engines, climb rate, top speed, and the max sustained and max instantaneous turn rates, each in degrees per second at a speed (the Yak-3: 23.59 deg/s at 175 kt sustained, 41.1 deg/s at 302 kt instantaneous). The sustained turn sets the induced drag. The instantaneous turn sets the G limit and, unless you enter one, the max lift coefficient. Top speed altitude, mass, wing area, max lift coefficient and best roll rate (in deg/s at a speed) are optional: leave them blank and the game fills them in from typical values for the engine type and power, and the editor lists what it picked. The game solves for drag, induced drag and propeller efficiency so the plane reproduces those numbers. The editor shows a "model check" with the figures the solved model actually reaches.

Save as many presets as you like. Saving under a new name makes a new preset and keeps the old one. The built-in Yak-3 can't be changed or deleted: edits to it are saved as a copy.

Roll rate builds up with speed to the preset's best roll rate at its best roll speed, then falls off gently above that (to about three quarters at 1.6 times that speed). Left blank, the best roll speed is 60% of top speed, so fast planes keep their roll.

## Weapons

A gun preset has a name, muzzle velocity (m/s), projectile mass (kg), fire rate (rounds per minute), tracer color, and how many rounds there are for each tracer. Caliber, spread, hits to kill and the tracer ratio are optional. Rounds between tracers are invisible but still fly and hit. A blank tracer ratio is 1 in 4 for 12.7 mm and smaller, and 1 in 3 for bigger guns (the ShVAK is 1 in 3, the UBS 1 in 4). A blank caliber is estimated from the projectile mass, spread is the width of the circle the rounds land in at 500 m, in meters (1 m by default), and hits to kill comes from the caliber (about 6 for 20 mm, 16 for 12.7 mm, 50 for 7.62 mm). Hits from different guns add up: each takes its share of the target's health. Rounds fly with gravity and air drag worked out from the caliber and mass. The editor shows each gun's flight time, remaining speed and drop at 1 km. Tracers are true to caliber: a solid white core with a bright glow around it in the tracer color, kept about a pixel wide far away. They disappear after 2.5 miles (about 4 km).

Gunshots and hits have sounds made in code, with no sound files. Small, fast rounds crack higher and big, slow ones boom lower. The built-in gun is the 20 mm ShVAK (800 m/s, 0.1 kg, 800 rpm), and it can't be changed or deleted. The 12.7 mm UBS comes as a second preset.

Each aircraft preset has a loadout with four mounting spots, and each spot can carry a different gun preset:

- Nose, in the spinner: 1 gun through the propeller hub.
- Nose, around it: up to 6, on a hexagon around the spinner gun.
- Belly: up to 4 under the fuselage. A slider sets how far behind the spinner they sit.
- Wings: up to 4 per wing (8 in total). A slider sets how far out from the wing root the first one sits; the rest follow outward.

Belly and wing guns can mix calibers: with two or more, each position gets its own gun drop-down (wing positions match on both wings).

The gun model (`assets/models/gun.glb`) appears in each spot, sized by caliber. Nose guns fire straight ahead. Wing and belly guns angle in sideways only (like War Thunder's horizontal convergence) to meet at the loadout's gun targeting distance, 400 m by default. Pick it from the same list the game has (No, 50 to 300 m in 50 m steps, then 400 to 800 m); No fires them parallel. The Yak-3 carries a ShVAK in the spinner and two UBS above it. Changing its loadout saves a copy.

## Multiplayer

One player hosts from the Multiplayer page, and the others type the host's address and join.

- **Free for all:** the last plane flying wins the round.
- **Team deathmatch:** each player picks Team A or Team B. A team wins the round when everyone on the other team is eliminated. Teammates' markers are blue and can't be hurt.
- **AI targets:** the host can add AI planes (in TDM, to each team). They fly the target preset, get a random name with "(AI)", lead their shots for their biggest gun, manage their energy and throttle, and chase the nearest enemy (in free for all that includes other AI targets).
- A new round starts by itself 3 seconds after a win. Eliminated planes explode.

- **Joining over the internet:** the host forwards UDP port 5050 (or the port they picked) on their router to their computer, and friends join with the host's public IP address, like `203.0.113.5:5050`. Without port forwarding, everyone can join the same free VPN (Radmin VPN, ZeroTier or Tailscale) and use the host's VPN address. On the same home network, use the host's local address.
- **Rounds:** the host holds the reset key (R) for 3 seconds to start a round, with a countdown everyone sees. Everyone is placed around a circle, and can fly during the countdown but not fire. "FIGHT!" shows when guns are free.
- **Host settings** (change them any time, no need to host again): whether lead indicators are allowed (if so, each player's own setting applies), the countdown, spawn distance, average altitude, altitude range, and facing each other or random headings.
- **Players** pick a name and a paint color (the cockpit keeps its color). Other planes get the same marker as the target, red for enemies and blue for teammates, with the player's name above the aircraft name.
- **Presets:** joined players can fly the host's aircraft presets (but not change them), or their own. The host can lock any player to one of the host's presets.
- Hits, eliminations ("X was eliminated by Y") and wins are worked out by the host. Eliminated players watch the others: left click for the next player, right click for the previous one. Hold Tab for the scoreboard: wins, kills and deaths in free for all; each team's round wins and every player's kills and deaths in team deathmatch.
- The pause menu doesn't pause the game for everyone: your plane keeps flying while it's open.

## Models

Each aircraft preset picks one of four models on the Aircraft page: Fighter (Yak-3), Heavy Fighter (F7F), Jet Fighter (F11F-1) or Bomber (Avro Shackleton). Each one is scaled to the real plane's length and span, and the chase camera sits where the game puts it for that plane, measured from the reference videos. Presets that haven't picked a model get the jet if they have a jet engine, the heavy fighter with two engines, and the bomber with three or more.

## Damage models

Each preset also picks a damage model. **Fighter**: hits add up until the plane goes down, and a hit on the cockpit brings it down at once. **Bomber**: only a cockpit hit brings it down. A bomber with no fixed guns, only turrets, also goes down once every turret is destroyed.

## Turrets

The Weapons page has a turret column for the loadout's aircraft preset: up to 2 on top, 2 underneath, 1 in the nose and 1 in the tail. Top and bottom turrets are cubes and slide along the fuselage on a slider, and they can face forward or back. Nose and tail turrets are balls. Each turret carries 1 to 4 guns of one gun preset (1 in the middle, 2 or 3 in a row, 4 in a rectangle), has left, right, down and up traverse limits, and a traverse speed (60 deg/s by default). All turrets aim at the same point 400 m out from the plane's center; this can't be changed.

A turret turns toward the aim point at its traverse speed. When the point is outside its limits, it swings as close as it can. It won't fire while it's held at a limit with the aim beyond it, but it does fire while it's still turning toward it.

Turrets take their own damage and are knocked out one by one, with a small explosion. A destroyed turret disappears and can't be used. AI gunners lead the nearest enemy and fire in bursts, with some aim error. Against a plane with the Bomber damage model, AI pilots aim at the cockpit or a live turret, whichever faces them best. Some dodge when you shoot at them and some never do.

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
- `src/multiplayer.rs` hosts and joins games (renet over UDP), runs rounds and shows the scoreboard.
- `assets/presets.json` holds extra gun and aircraft presets that are added to everyone's game once.
- `src/models.rs` holds the four plane models: their size, camera spot, gun spots, hit boxes and turret placement.
- `src/turrets.rs` places, aims and fires turrets, and draws them.
- `src/damage.rs` holds the Fighter and Bomber damage models.
- `src/ai.rs` flies AI planes.
- `assets/models/` holds the plane models (`fighter.glb`, `heavyfighter.glb`, `jetfighter.glb`, `bomber.glb`), the turret models and the gun model.

## Roadmap

1. Flight and mouse aim prototype (done)
2. Guns with set velocity, spread and rate of fire, bullet drop, hit detection, and saved gun presets (done)
3. Target scenarios: head on, chase, stall, dogfight (done)
4. Scoring and a settings menu
