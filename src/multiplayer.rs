//! Multiplayer: one player hosts and the others join by address. The only
//! mode is free for all: the last plane flying wins the round.
//!
//! Every player flies their own plane on their own computer and sends its
//! position about 30 times a second; the host passes everyone's on to
//! everyone. Each player's game checks its own rounds against the other
//! planes and tells the host about hits; the host keeps score, decides who
//! is eliminated, and starts each round.

use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use renet::{ConnectionConfig, DefaultChannel, RenetClient, RenetServer, ServerEvent};
use renet_netcode::{
    ClientAuthentication, NetcodeClientTransport, NetcodeServerTransport, ServerAuthentication,
    ServerConfig,
};
use serde::{Deserialize, Serialize};

use crate::GameState;
use crate::camera::{ChaseCamera, MouseAim, snap_view};
use crate::flight::{Aircraft, PaintColor, Player};
use crate::flight_model::{AircraftCard, Body};
use crate::settings::{Action, PlaneColor, Settings};
use crate::target::Target;
use crate::weapons::{GunBook, GunCard, HitStats};

pub const DEFAULT_PORT: u16 = 5050;
/// Changes whenever the messages below change, so different versions of
/// the game don't try to talk to each other.
const PROTOCOL_ID: u64 = 0x5754_4154_0001;
/// The host's own player id.
pub const HOST_ID: u64 = 0;
const MAX_PLAYERS: usize = 16;
/// Position updates per second.
const SEND_RATE: f32 = 30.0;
/// How long the host holds the reset key to start a new round, s.
pub const RESET_HOLD: f32 = 3.0;
/// How long a kill feed line stays up, s.
const FEED_TIME: f32 = 6.0;

/// Settings the host picks for everyone. They can be changed while the
/// game runs; spawn settings apply from the next round.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HostOptions {
    /// Players may turn on their lead indicator.
    pub lead_indicators: bool,
    /// Seconds of countdown before guns are free at the start of a round.
    pub countdown: u32,
    /// How far apart players start, m.
    pub spawn_distance: f32,
    /// Average start altitude, m.
    pub altitude: f32,
    /// Players start up to this far above or below the average, m.
    pub altitude_spread: f32,
    /// Everyone starts facing the middle (each other), or a random way.
    pub face_each_other: bool,
}

impl Default for HostOptions {
    fn default() -> Self {
        HostOptions {
            lead_indicators: false,
            countdown: 5,
            spawn_distance: 2_000.0,
            altitude: 2_000.0,
            altitude_spread: 0.0,
            face_each_other: true,
        }
    }
}

/// What a player looks like to the others.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerInfo {
    pub nickname: String,
    pub color: PlaneColor,
    pub card: AircraftCard,
    /// The gun presets its loadout uses.
    pub guns: Vec<GunCard>,
}

/// A plane's motion, sent many times a second.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaneState {
    pub p: [f32; 3],
    pub r: [f32; 4],
    pub v: [f32; 3],
    pub firing: bool,
    pub alive: bool,
}

/// One player on the scoreboard.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: u64,
    pub info: Option<PlayerInfo>,
    pub kills: u32,
    pub wins: u32,
    pub alive: bool,
    /// Share of the plane shot away (1 = down).
    pub damage: f32,
    /// The host locked this player to one of the host's aircraft presets.
    pub locked: Option<String>,
}

impl Entry {
    fn new(id: u64) -> Entry {
        Entry {
            id,
            info: None,
            kills: 0,
            wins: 0,
            alive: false,
            damage: 0.0,
            locked: None,
        }
    }

    pub fn name(&self) -> String {
        self.info
            .as_ref()
            .map(|i| i.nickname.clone())
            .unwrap_or_else(|| "Joining...".into())
    }
}

/// Where and how a player starts a round.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Spawn {
    pub p: [f32; 3],
    /// Heading, rad (0 = -Z).
    pub heading: f32,
    /// Share of top speed.
    pub speed: f32,
}

#[derive(Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
enum ToHost {
    Info(PlayerInfo),
    State(PlaneState),
    Hit { victim: u64, damage: f32 },
    Crashed,
}

#[derive(Serialize, Deserialize)]
enum ToPlayer {
    Welcome {
        id: u64,
    },
    Presets {
        aircraft: Vec<AircraftCard>,
        guns: Vec<GunCard>,
    },
    Options(HostOptions),
    Roster {
        entries: Vec<Entry>,
        phase: Phase,
    },
    States(Vec<(u64, PlaneState)>),
    RoundStart {
        spawns: Vec<(u64, Spawn)>,
    },
    Eliminated {
        victim: u64,
        by: Option<u64>,
    },
    RoundOver {
        winner: Option<u64>,
    },
    ResetHold(f32),
}

/// Where the round is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Phase {
    /// Before the first round: fly around, guns safe.
    #[default]
    Lobby,
    /// Counting down to the start; you can fly but not shoot.
    Countdown(f32),
    /// Guns free. `.0` counts how long the "Fight!" message has shown.
    Fight(f32),
    /// The round is over.
    Over,
}

enum Net {
    Off,
    Host {
        server: Box<RenetServer>,
        transport: Box<NetcodeServerTransport>,
    },
    Client {
        client: Box<RenetClient>,
        transport: Box<NetcodeClientTransport>,
        connected: bool,
    },
}

/// The multiplayer session (or none).
#[derive(Resource)]
pub struct Mp {
    net: Net,
    pub my_id: u64,
    pub roster: Vec<Entry>,
    pub options: HostOptions,
    pub phase: Phase,
    /// The host's presets, which joined players may fly (not change).
    pub host_aircraft: Vec<AircraftCard>,
    pub host_guns: Vec<GunCard>,
    /// A joined player flying one of the host's presets.
    pub use_host_preset: Option<String>,
    /// Remote planes' latest motion.
    states: HashMap<u64, (PlaneState, f32)>,
    /// The player being watched while you're out.
    pub spectate: Option<u64>,
    /// Kill feed lines and how long each has left.
    pub feed: Vec<(String, f32)>,
    /// A big message in the middle of the screen.
    pub banner: Option<(String, f32)>,
    /// How long the host has held reset, s.
    pub reset_hold: f32,
    /// Connection status for the menu.
    pub status: String,
    last_info: Option<PlayerInfo>,
    send_timer: f32,
    /// Host: the round needs a winner check.
    check_round: bool,
    /// Host: players in the current round.
    round_size: usize,
    /// Where your plane starts the new round, until it's put there.
    pending: Option<Spawn>,
}

impl Default for Mp {
    fn default() -> Self {
        Mp {
            net: Net::Off,
            my_id: HOST_ID,
            roster: vec![],
            options: HostOptions::default(),
            phase: Phase::Lobby,
            host_aircraft: vec![],
            host_guns: vec![],
            use_host_preset: None,
            states: HashMap::new(),
            spectate: None,
            feed: vec![],
            banner: None,
            reset_hold: 0.0,
            status: String::new(),
            last_info: None,
            send_timer: 0.0,
            check_round: false,
            round_size: 0,
            pending: None,
        }
    }
}

impl Mp {
    pub fn online(&self) -> bool {
        !matches!(self.net, Net::Off)
    }

    pub fn is_host(&self) -> bool {
        matches!(self.net, Net::Host { .. })
    }

    pub fn is_client(&self) -> bool {
        matches!(self.net, Net::Client { .. })
    }

    pub fn me(&self) -> Option<&Entry> {
        self.roster.iter().find(|e| e.id == self.my_id)
    }

    fn entry_mut(&mut self, id: u64) -> Option<&mut Entry> {
        self.roster.iter_mut().find(|e| e.id == id)
    }

    pub fn name_of(&self, id: u64) -> String {
        self.roster
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.name())
            .unwrap_or_else(|| "Someone".into())
    }

    /// Your plane is in the round (or flying in the lobby).
    pub fn alive(&self) -> bool {
        !self.online() || matches!(self.phase, Phase::Lobby) || self.me().is_some_and(|e| e.alive)
    }

    /// You may fire.
    pub fn guns_free(&self) -> bool {
        !self.online() || (matches!(self.phase, Phase::Fight(_)) && self.alive())
    }

    /// Your lead indicator may show.
    pub fn lead_allowed(&self) -> bool {
        !self.online() || self.options.lead_indicators
    }

    /// The aircraft preset you fly, and the gun presets it uses: a locked
    /// or chosen host preset, or your own.
    pub fn local_loadout(&self, settings: &Settings) -> (AircraftCard, Vec<GunCard>) {
        let host_card = |name: &str| {
            if self.is_host() {
                settings.aircraft.iter().find(|c| c.name == name).cloned()
            } else {
                self.host_aircraft.iter().find(|c| c.name == name).cloned()
            }
        };
        let from_host = self
            .me()
            .and_then(|e| e.locked.clone())
            .or_else(|| self.use_host_preset.clone().filter(|_| self.is_client()))
            .and_then(|n| host_card(&n));
        let (card, book): (AircraftCard, &[GunCard]) = match from_host {
            Some(c) if self.is_client() => (c, &self.host_guns),
            Some(c) => (c, &settings.guns),
            None => (settings.player_card().clone(), &settings.guns),
        };
        let guns = card
            .weapons
            .guns()
            .iter()
            .map(|(_, g, _)| {
                book.iter()
                    .chain(settings.guns.iter())
                    .find(|c| c.name == *g)
                    .cloned()
                    .unwrap_or_else(|| settings.gun(g).clone())
            })
            .fold(Vec::<GunCard>::new(), |mut v, g| {
                if !v.iter().any(|x| x.name == g.name) {
                    v.push(g);
                }
                v
            });
        (card, guns)
    }

    /// Your aircraft is locked by the host.
    pub fn locked(&self) -> Option<&str> {
        self.me().and_then(|e| e.locked.as_deref())
    }

    fn feed(&mut self, line: String) {
        self.feed.push((line, FEED_TIME));
        if self.feed.len() > 6 {
            self.feed.remove(0);
        }
    }
}

/// Run conditions.
pub fn offline(mp: Res<Mp>) -> bool {
    !mp.online()
}

/// The world keeps going: playing, or in the menu during multiplayer (a
/// pause there doesn't stop the game for everyone).
pub fn world_runs(state: Res<State<GameState>>, mp: Res<Mp>) -> bool {
    *state.get() == GameState::Playing || mp.online()
}

/// Your plane flies (not while you're out of the round).
pub fn local_flies(state: Res<State<GameState>>, mp: Res<Mp>) -> bool {
    (*state.get() == GameState::Playing || mp.online()) && mp.alive()
}

/// Another player's plane.
#[derive(Component)]
pub struct RemotePlane {
    pub id: u64,
    pub firing: bool,
    pub alive: bool,
}

/// Your round hit another player's plane.
#[derive(Message)]
pub struct RemoteHit {
    pub id: u64,
    pub damage: f32,
}

/// Menu requests.
#[derive(Message, Clone)]
pub enum MpCommand {
    Host,
    Join,
    Leave,
    /// Host: lock a player to a host preset (or unlock with `None`).
    Lock(u64, Option<String>),
}

pub struct MultiplayerPlugin;

impl Plugin for MultiplayerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Mp>()
            .add_message::<RemoteHit>()
            .add_message::<MpCommand>()
            .add_systems(Startup, spawn_mp_ui)
            .add_systems(PreUpdate, (handle_commands, receive).chain())
            .add_systems(
                Update,
                (
                    hold_reset,
                    run_round,
                    sync_remote_planes,
                    move_remote_planes,
                    local_status,
                    spectate_controls,
                    send,
                )
                    .chain()
                    .after(crate::flight::fly_player)
                    .before(crate::weapons::Firing),
            )
            .add_systems(Update, (hide_target_online, test_hooks))
            .add_systems(PostUpdate, update_mp_ui.after(TransformSystems::Propagate));
    }
}

fn now_since_epoch() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}

fn encode<T: Serialize>(msg: &T) -> Vec<u8> {
    serde_json::to_vec(msg).unwrap_or_default()
}

/// Start hosting on `port`.
fn start_host(mp: &mut Mp, port: u16) -> Result<(), String> {
    let socket = UdpSocket::bind(("0.0.0.0", port)).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AddrInUse {
            format!(
                "Port {port} is already in use on this computer. If another copy \
                 of the game is hosting, join it with 127.0.0.1 instead. Otherwise \
                 pick another port (and forward that one on your router)."
            )
        } else {
            format!("Couldn't open port {port}: {e}")
        }
    })?;
    let addr = socket
        .local_addr()
        .map_err(|e| format!("Couldn't read the port: {e}"))?;
    let config = ServerConfig {
        current_time: now_since_epoch(),
        max_clients: MAX_PLAYERS - 1,
        protocol_id: PROTOCOL_ID,
        public_addresses: vec![addr],
        authentication: ServerAuthentication::Unsecure,
    };
    let transport = NetcodeServerTransport::new(config, socket)
        .map_err(|e| format!("Couldn't start hosting: {e}"))?;
    *mp = Mp {
        net: Net::Host {
            server: Box::new(RenetServer::new(ConnectionConfig::default())),
            transport: Box::new(transport),
        },
        my_id: HOST_ID,
        roster: vec![Entry {
            alive: true,
            ..Entry::new(HOST_ID)
        }],
        status: format!("Hosting on port {port}. Friends join with your IP address and :{port}."),
        ..default()
    };
    Ok(())
}

/// Join a game at `address` (host:port, port optional).
fn start_join(mp: &mut Mp, address: &str) -> Result<(), String> {
    let address = address.trim();
    if address.is_empty() {
        return Err("Type the host's address first, like 203.0.113.5:5050.".into());
    }
    let with_port = if address.contains(':') {
        address.to_string()
    } else {
        format!("{address}:{DEFAULT_PORT}")
    };
    let server_addr: SocketAddr = with_port
        .to_socket_addrs()
        .map_err(|e| format!("Couldn't find {with_port}: {e}"))?
        .find(|a| a.is_ipv4())
        .ok_or_else(|| format!("Couldn't find {with_port}."))?;
    let socket =
        UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("Couldn't open a network port: {e}"))?;
    let now = now_since_epoch();
    let client_id = (now.as_nanos() as u64) | 1;
    let auth = ClientAuthentication::Unsecure {
        protocol_id: PROTOCOL_ID,
        client_id,
        server_addr,
        user_data: None,
    };
    let transport = NetcodeClientTransport::new(now, auth, socket)
        .map_err(|e| format!("Couldn't connect: {e}"))?;
    *mp = Mp {
        net: Net::Client {
            client: Box::new(RenetClient::new(ConnectionConfig::default())),
            transport: Box::new(transport),
            connected: false,
        },
        my_id: client_id,
        status: format!("Connecting to {with_port}..."),
        ..default()
    };
    Ok(())
}

fn leave(mp: &mut Mp) {
    match &mut mp.net {
        Net::Host { server, transport } => {
            transport.disconnect_all(server);
        }
        Net::Client {
            client, transport, ..
        } => {
            client.disconnect();
            let _ = transport.send_packets(client);
        }
        Net::Off => {}
    }
    *mp = Mp {
        status: "Left the game.".into(),
        ..default()
    };
}

#[allow(clippy::too_many_arguments)]
fn handle_commands(
    mut commands: MessageReader<MpCommand>,
    mut mp: ResMut<Mp>,
    settings: Res<Settings>,
    mut reset: MessageWriter<crate::target::ResetScenario>,
) {
    for cmd in commands.read() {
        match cmd.clone() {
            MpCommand::Host => {
                if mp.online() {
                    leave(&mut mp);
                }
                if let Err(e) = start_host(&mut mp, settings.host_port) {
                    mp.status = e;
                } else {
                    mp.options = settings.host.clone();
                }
            }
            MpCommand::Join => {
                if mp.online() {
                    leave(&mut mp);
                }
                if let Err(e) = start_join(&mut mp, &settings.join_address) {
                    mp.status = e;
                }
            }
            MpCommand::Leave => {
                leave(&mut mp);
                reset.write(crate::target::ResetScenario);
            }
            MpCommand::Lock(id, preset) => {
                if let Some(e) = mp.entry_mut(id) {
                    e.locked = preset;
                }
                broadcast_roster(&mut mp);
            }
        }
    }
}

/// Host: send the scoreboard to everyone.
fn broadcast_roster(mp: &mut Mp) {
    let msg = encode(&ToPlayer::Roster {
        entries: mp.roster.clone(),
        phase: mp.phase,
    });
    if let Net::Host { server, .. } = &mut mp.net {
        server.broadcast_message(DefaultChannel::ReliableOrdered, msg);
    }
}

fn host_send_all(mp: &mut Mp, msg: &ToPlayer) {
    let bytes = encode(msg);
    if let Net::Host { server, .. } = &mut mp.net {
        server.broadcast_message(DefaultChannel::ReliableOrdered, bytes);
    }
}

/// Host: a player is out of the round.
fn eliminate(mp: &mut Mp, victim: u64, by: Option<u64>) {
    let Some(e) = mp.entry_mut(victim) else {
        return;
    };
    if !e.alive {
        return;
    }
    e.alive = false;
    e.damage = 1.0;
    if let Some(by) = by.filter(|b| *b != victim)
        && let Some(k) = mp.entry_mut(by)
    {
        k.kills += 1;
    }
    host_send_all(mp, &ToPlayer::Eliminated { victim, by });
    apply_elimination(mp, victim, by);
    mp.check_round = true;
    broadcast_roster(mp);
}

/// Everyone: show an elimination and start watching someone if it was you.
fn apply_elimination(mp: &mut Mp, victim: u64, by: Option<u64>) {
    let line = match by.filter(|b| *b != victim) {
        Some(by) => format!(
            "{} was eliminated by {}",
            mp.name_of(victim),
            mp.name_of(by)
        ),
        None => format!("{} crashed", mp.name_of(victim)),
    };
    info!("{line}");
    mp.feed(line);
    if let Some(e) = mp.entry_mut(victim) {
        e.alive = false;
    }
    if victim == mp.my_id {
        mp.spectate = by.filter(|b| *b != victim);
        mp.banner = Some((
            match by.filter(|b| *b != victim) {
                Some(b) => format!("Eliminated by {}", mp.name_of(b)),
                None => "You crashed".into(),
            },
            3.0,
        ));
    }
}

/// Host: hits from a player on another.
fn apply_hit(mp: &mut Mp, attacker: u64, victim: u64, damage: f32) {
    if !matches!(mp.phase, Phase::Fight(_)) {
        return;
    }
    let attacker_alive = mp.roster.iter().any(|e| e.id == attacker && e.alive);
    let Some(e) = mp.entry_mut(victim) else {
        return;
    };
    if !e.alive || !attacker_alive {
        return;
    }
    e.damage += damage.clamp(0.0, 1.0);
    if e.damage >= 1.0 {
        eliminate(mp, victim, Some(attacker));
    } else {
        broadcast_roster(mp);
    }
}

/// Host: place everyone and count down.
fn start_round(mp: &mut Mp) {
    let opts = mp.options.clone();
    let mut seed = now_since_epoch().as_nanos() as u32 | 1;
    let mut rand = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32
    };
    let ids: Vec<u64> = mp
        .roster
        .iter()
        .filter(|e| e.info.is_some() || e.id == HOST_ID)
        .map(|e| e.id)
        .collect();
    let n = ids.len().max(1);
    // Around a circle, so players across it are `spawn_distance` apart.
    let radius = opts.spawn_distance * 0.5;
    let turn = rand() * std::f32::consts::TAU;
    let spawns: Vec<(u64, Spawn)> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let a = turn + i as f32 / n as f32 * std::f32::consts::TAU;
            let x = a.sin() * radius;
            let z = -a.cos() * radius;
            let y = (opts.altitude + (rand() * 2.0 - 1.0) * opts.altitude_spread).max(200.0);
            let heading = if opts.face_each_other && n > 1 {
                // Toward the middle. Heading h flies toward
                // (-sin h, -cos h), so point that at (-x, -z).
                x.atan2(z)
            } else {
                rand() * std::f32::consts::TAU
            };
            (
                *id,
                Spawn {
                    p: [x, y, z],
                    heading,
                    speed: crate::flight::START_SPEED_MIN
                        + rand()
                            * (crate::flight::START_SPEED_MAX - crate::flight::START_SPEED_MIN),
                },
            )
        })
        .collect();
    for e in &mut mp.roster {
        e.alive = ids.contains(&e.id);
        e.damage = 0.0;
    }
    mp.round_size = ids.len();
    mp.phase = Phase::Countdown(opts.countdown as f32);
    host_send_all(
        mp,
        &ToPlayer::RoundStart {
            spawns: spawns.clone(),
        },
    );
    broadcast_roster(mp);
    pending_spawn(mp, &spawns);
}

fn pending_spawn(mp: &mut Mp, spawns: &[(u64, Spawn)]) {
    mp.spectate = None;
    mp.banner = None;
    mp.pending = spawns.iter().find(|(id, _)| *id == mp.my_id).map(|s| s.1);
    info!(
        "Round starts: {} players, my spawn {:?}",
        spawns.len(),
        mp.pending
    );
}

/// Read the network.
#[allow(clippy::too_many_arguments)]
fn receive(time: Res<Time<Real>>, mut mp: ResMut<Mp>, settings: Res<Settings>) {
    let dt = Duration::from_secs_f32(time.delta_secs().max(0.0));
    let now = time.elapsed_secs();
    let mp = &mut *mp;
    match &mut mp.net {
        Net::Off => {}
        Net::Host { server, transport } => {
            server.update(dt);
            if let Err(e) = transport.update(dt, server) {
                warn!("Network error: {e}");
            }
            let mut joined = vec![];
            let mut left = vec![];
            while let Some(event) = server.get_event() {
                match event {
                    ServerEvent::ClientConnected { client_id } => joined.push(client_id),
                    ServerEvent::ClientDisconnected { client_id, .. } => left.push(client_id),
                }
            }
            let mut inbox = vec![];
            for id in server.clients_id() {
                while let Some(m) = server.receive_message(id, DefaultChannel::ReliableOrdered) {
                    inbox.push((id, m));
                }
                while let Some(m) = server.receive_message(id, DefaultChannel::Unreliable) {
                    inbox.push((id, m));
                }
            }
            for id in joined {
                let mut entry = Entry::new(id);
                // Joining mid-round: watch until the next one.
                entry.alive = matches!(mp.phase, Phase::Lobby);
                mp.roster.push(entry);
                let welcome = [
                    encode(&ToPlayer::Welcome { id }),
                    encode(&ToPlayer::Presets {
                        aircraft: settings.aircraft.clone(),
                        guns: settings.guns.clone(),
                    }),
                    encode(&ToPlayer::Options(mp.options.clone())),
                ];
                if let Net::Host { server, .. } = &mut mp.net {
                    for m in welcome {
                        server.send_message(id, DefaultChannel::ReliableOrdered, m);
                    }
                }
                broadcast_roster(mp);
            }
            for id in left {
                let name = mp.name_of(id);
                mp.roster.retain(|e| e.id != id);
                mp.states.remove(&id);
                mp.feed(format!("{name} left"));
                mp.check_round = true;
                broadcast_roster(mp);
            }
            for (id, bytes) in inbox {
                let msg = match serde_json::from_slice::<ToHost>(&bytes) {
                    Ok(m) => m,
                    Err(e) => {
                        warn!("Bad message from {id}: {e}");
                        continue;
                    }
                };
                match msg {
                    ToHost::Info(info) => {
                        if let Some(e) = mp.entry_mut(id)
                            && e.info.as_ref() != Some(&info)
                        {
                            let first = e.info.is_none();
                            e.info = Some(info);
                            if first {
                                let name = mp.name_of(id);
                                info!("{name} joined");
                                mp.feed(format!("{name} joined"));
                            }
                            broadcast_roster(mp);
                        }
                    }
                    ToHost::State(s) => {
                        mp.states.insert(id, (s, now));
                    }
                    ToHost::Hit { victim, damage } => {
                        if victim == HOST_ID || mp.roster.iter().any(|e| e.id == victim) {
                            apply_hit(mp, id, victim, damage);
                        }
                    }
                    ToHost::Crashed => {
                        if matches!(mp.phase, Phase::Fight(_) | Phase::Countdown(_)) {
                            eliminate(mp, id, None);
                        }
                    }
                }
            }
        }
        Net::Client {
            client,
            transport,
            connected,
        } => {
            client.update(dt);
            if let Err(e) = transport.update(dt, client) {
                mp.status = format!("Disconnected: {e}");
                *mp = Mp {
                    status: mp.status.clone(),
                    ..default()
                };
                return;
            }
            if client.is_disconnected() {
                let reason = client
                    .disconnect_reason()
                    .map(|r| format!("{r:?}"))
                    .unwrap_or_default();
                let status = if *connected {
                    format!("The host closed the game or the connection dropped. {reason}")
                } else {
                    format!(
                        "Couldn't reach the host. Check the address, and that the host's \
                         port is open (see the Multiplayer page). {reason}"
                    )
                };
                *mp = Mp {
                    status,
                    ..default()
                };
                return;
            }
            if client.is_connected() && !*connected {
                info!("Connected to the host");
                *connected = true;
                mp.status = "Connected.".into();
            }
            let mut inbox = vec![];
            while let Some(m) = client.receive_message(DefaultChannel::ReliableOrdered) {
                inbox.push(m);
            }
            while let Some(m) = client.receive_message(DefaultChannel::Unreliable) {
                inbox.push(m);
            }
            for bytes in inbox {
                let Ok(msg) = serde_json::from_slice::<ToPlayer>(&bytes) else {
                    continue;
                };
                match msg {
                    ToPlayer::Welcome { id } => {
                        debug!("Welcome as {id}");
                        mp.my_id = id;
                        mp.last_info = None;
                    }
                    ToPlayer::Presets { aircraft, guns } => {
                        mp.host_aircraft = aircraft;
                        mp.host_guns = guns;
                    }
                    ToPlayer::Options(o) => mp.options = o,
                    ToPlayer::Roster { entries, phase } => {
                        mp.roster = entries;
                        // The countdown runs locally; only take a new phase.
                        if std::mem::discriminant(&phase) != std::mem::discriminant(&mp.phase) {
                            mp.phase = phase;
                        }
                    }
                    ToPlayer::States(list) => {
                        for (id, s) in list {
                            if id != mp.my_id {
                                mp.states.insert(id, (s, now));
                            }
                        }
                    }
                    ToPlayer::RoundStart { spawns } => {
                        mp.phase = Phase::Countdown(mp.options.countdown as f32);
                        for e in &mut mp.roster {
                            e.alive = spawns.iter().any(|(id, _)| *id == e.id);
                            e.damage = 0.0;
                        }
                        pending_spawn(mp, &spawns);
                    }
                    ToPlayer::Eliminated { victim, by } => apply_elimination(mp, victim, by),
                    ToPlayer::RoundOver { winner } => {
                        mp.phase = Phase::Over;
                        round_over_banner(mp, winner);
                    }
                    ToPlayer::ResetHold(t) => mp.reset_hold = t,
                }
            }
        }
    }
}

fn round_over_banner(mp: &mut Mp, winner: Option<u64>) {
    let text = match winner {
        Some(w) if w == mp.my_id => "You win the round!".to_string(),
        Some(w) => format!("{} wins the round!", mp.name_of(w)),
        None => "Nobody survived the round".into(),
    };
    mp.banner = Some((text, 6.0));
}

/// Host: hold the reset key for a few seconds to start a round.
fn hold_reset(
    time: Res<Time<Real>>,
    keys: Res<ButtonInput<KeyCode>>,
    settings: Res<Settings>,
    state: Res<State<GameState>>,
    mut mp: ResMut<Mp>,
) {
    if !mp.is_host() {
        return;
    }
    let held = *state.get() == GameState::Playing && settings.pressed(&keys, Action::Reset);
    let before = mp.reset_hold;
    if held {
        mp.reset_hold += time.delta_secs();
    } else {
        mp.reset_hold = 0.0;
    }
    if mp.reset_hold >= RESET_HOLD {
        mp.reset_hold = 0.0;
        start_round(&mut mp);
    }
    // Tell everyone when it starts, stops, and each whole second.
    if (before > 0.0) != (mp.reset_hold > 0.0) || before.floor() != mp.reset_hold.floor() {
        let t = mp.reset_hold;
        host_send_all(&mut mp, &ToPlayer::ResetHold(t));
    }
}

/// Countdowns, the round end, and timers.
#[allow(clippy::too_many_arguments)]
fn run_round(
    time: Res<Time<Real>>,
    mut mp: ResMut<Mp>,
    settings: Res<Settings>,
    mut aim: ResMut<MouseAim>,
    mut camera: Single<&mut ChaseCamera>,
    player: Single<(&mut Aircraft, &mut Transform), With<Player>>,
    mut stats: ResMut<HitStats>,
) {
    if !mp.online() {
        return;
    }
    let dt = time.delta_secs();
    // A new round: put your plane at its start.
    if let Some(spawn) = mp.pending.take() {
        let (mut plane, mut tf) = player.into_inner();
        let (card, _) = mp.local_loadout(&settings);
        plane.set_card(&card);
        let rot = Quat::from_rotation_y(spawn.heading);
        let speed = spawn.speed * plane.model.top_speed(spawn.p[1]);
        plane.body = Body::new(Vec3::from(spawn.p), rot, speed);
        plane.controls.throttle = 1.0;
        *tf = Transform::from_translation(plane.body.position).with_rotation(rot);
        aim.rot = rot;
        snap_view(&mut camera, &aim);
        stats.reset();
    }
    match mp.phase {
        Phase::Countdown(left) => {
            let left = left - dt;
            mp.phase = if left <= 0.0 {
                mp.banner = Some(("FIGHT!".into(), 1.5));
                Phase::Fight(0.0)
            } else {
                Phase::Countdown(left)
            };
        }
        Phase::Fight(t) => mp.phase = Phase::Fight(t + dt),
        _ => {}
    }
    for line in &mut mp.feed {
        line.1 -= dt;
    }
    mp.feed.retain(|l| l.1 > 0.0);
    if let Some((_, t)) = &mut mp.banner {
        *t -= dt;
        if *t <= 0.0 {
            mp.banner = None;
        }
    }
    // Host: last plane flying wins.
    if mp.is_host() && mp.check_round {
        mp.check_round = false;
        if matches!(mp.phase, Phase::Fight(_) | Phase::Countdown(_)) {
            let alive: Vec<u64> = mp.roster.iter().filter(|e| e.alive).map(|e| e.id).collect();
            let over = if mp.round_size >= 2 {
                alive.len() <= 1
            } else {
                alive.is_empty()
            };
            if over {
                let winner = alive.first().copied().filter(|_| mp.round_size >= 2);
                if let Some(w) = winner
                    && let Some(e) = mp.entry_mut(w)
                {
                    e.wins += 1;
                }
                mp.phase = Phase::Over;
                host_send_all(&mut mp, &ToPlayer::RoundOver { winner });
                round_over_banner(&mut mp, winner);
                broadcast_roster(&mut mp);
            }
        }
    }
}

/// Spawn and remove other players' planes.
#[allow(clippy::type_complexity)]
fn sync_remote_planes(
    mut commands: Commands,
    mp: Res<Mp>,
    assets: Res<AssetServer>,
    mut remotes: Query<(
        Entity,
        &mut RemotePlane,
        &mut Aircraft,
        &mut PaintColor,
        &mut GunBook,
    )>,
) {
    let mut seen = vec![];
    for (entity, mut remote, mut aircraft, mut paint, mut book) in &mut remotes {
        let entry = mp
            .roster
            .iter()
            .find(|e| e.id == remote.id && e.id != mp.my_id);
        let Some(entry) = entry.filter(|_| mp.online()) else {
            commands.entity(entity).despawn();
            continue;
        };
        seen.push(remote.id);
        remote.alive = entry.alive;
        if let Some(info) = &entry.info {
            aircraft.set_card(&info.card);
            if paint.0 != info.color {
                paint.0 = info.color;
            }
            if book.0 != info.guns {
                book.0 = info.guns.clone();
            }
        }
    }
    for e in &mp.roster {
        if e.id == mp.my_id || seen.contains(&e.id) {
            continue;
        }
        let Some(info) = &e.info else {
            continue;
        };
        let aircraft = Aircraft::new(
            &info.card,
            Body::new(Vec3::new(0.0, -10_000.0, 0.0), Quat::IDENTITY, 0.0),
        );
        commands
            .spawn((
                Transform::from_xyz(0.0, -10_000.0, 0.0),
                Visibility::Hidden,
                RemotePlane {
                    id: e.id,
                    firing: false,
                    alive: e.alive,
                },
                PaintColor(info.color),
                GunBook(info.guns.clone()),
                aircraft,
            ))
            .with_children(|plane| {
                plane.spawn((
                    WorldAssetRoot(
                        assets.load(GltfAssetLabel::Scene(0).from_asset(crate::AIRPLANE_MODEL)),
                    ),
                    crate::model_correction(),
                    crate::weapons::PlaneModel,
                ));
            });
    }
}

/// Glide other players' planes toward where they were last seen, carried
/// forward by their speed.
fn move_remote_planes(
    time: Res<Time<Real>>,
    mp: Res<Mp>,
    mut remotes: Query<(
        &mut RemotePlane,
        &mut Aircraft,
        &mut Transform,
        &mut Visibility,
    )>,
) {
    let now = time.elapsed_secs();
    let k = 1.0 - (-15.0 * time.delta_secs()).exp();
    for (mut remote, mut aircraft, mut tf, mut vis) in &mut remotes {
        let Some((s, at)) = mp.states.get(&remote.id) else {
            continue;
        };
        let shown = remote.alive && s.alive;
        let want = if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
        remote.firing = s.firing && shown;
        let v = Vec3::from(s.v);
        let age = (now - at).clamp(0.0, 0.5);
        let p = Vec3::from(s.p) + v * age;
        let r = Quat::from_array(s.r).normalize();
        // Far off (a respawn), jump; otherwise ease toward it.
        if tf.translation.distance(p) > 200.0 {
            tf.translation = p;
            tf.rotation = r;
        } else {
            tf.translation = tf.translation.lerp(p, k);
            tf.rotation = tf.rotation.slerp(r, k);
        }
        aircraft.body.position = tf.translation;
        aircraft.body.rotation = tf.rotation;
        aircraft.body.velocity = v;
    }
}

/// Your own plane: the host's loadout lock, being out, and crashing.
#[allow(clippy::type_complexity)]
fn local_status(
    mut commands: Commands,
    mut mp: ResMut<Mp>,
    settings: Res<Settings>,
    player: Single<
        (
            Entity,
            &mut Aircraft,
            &mut Visibility,
            &mut PaintColor,
            Option<&mut GunBook>,
        ),
        With<Player>,
    >,
    mut hits: MessageReader<RemoteHit>,
) {
    let (entity, mut plane, mut vis, mut paint, book) = player.into_inner();
    if paint.0 != settings.plane_color {
        paint.0 = settings.plane_color;
    }
    if !mp.online() {
        if book.is_some() {
            commands.entity(entity).remove::<GunBook>();
        }
        if *vis != Visibility::Inherited {
            *vis = Visibility::Inherited;
        }
        hits.clear();
        return;
    }
    let (card, guns) = mp.local_loadout(&settings);
    plane.set_card(&card);
    match book {
        Some(mut b) if b.0 != guns => b.0 = guns,
        Some(_) => {}
        None => {
            commands.entity(entity).insert(GunBook(guns));
        }
    }
    let alive = mp.alive();
    let want = if alive {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if *vis != want {
        *vis = want;
    }
    // Hitting the ground: out of the round, or back up in the lobby.
    if alive && plane.body.position.y < 3.0 {
        match mp.phase {
            Phase::Fight(_) | Phase::Countdown(_) => {
                let me = mp.my_id;
                if mp.is_host() {
                    eliminate(&mut mp, me, None);
                } else {
                    if let Some(e) = mp.entry_mut(me) {
                        e.alive = false;
                    }
                    client_send(&mut mp, &ToHost::Crashed, true);
                }
            }
            _ => {
                let speed = plane.model.top_speed(1_500.0) * 0.55;
                let rot = plane.body.rotation;
                plane.body = Body::new(
                    Vec3::new(plane.body.position.x, 1_500.0, plane.body.position.z),
                    rot,
                    speed,
                );
            }
        }
    }
    // Your hits on others go to the host.
    let list: Vec<_> = hits.read().map(|h| (h.id, h.damage)).collect();
    for (victim, damage) in list {
        if mp.is_host() {
            apply_hit(&mut mp, HOST_ID, victim, damage);
        } else {
            client_send(&mut mp, &ToHost::Hit { victim, damage }, true);
        }
    }
}

/// Send to the host. False if not connected yet (nothing was sent).
fn client_send(mp: &mut Mp, msg: &ToHost, reliable: bool) -> bool {
    if let Net::Client { client, .. } = &mut mp.net
        && client.is_connected()
    {
        let channel = if reliable {
            DefaultChannel::ReliableOrdered
        } else {
            DefaultChannel::Unreliable
        };
        client.send_message(channel, encode(msg));
        return true;
    }
    false
}

/// Out of the round: click to watch the next player (right click for the
/// previous one).
fn spectate_controls(
    mouse: Res<ButtonInput<MouseButton>>,
    state: Res<State<GameState>>,
    mut mp: ResMut<Mp>,
) {
    if !mp.online() || mp.alive() {
        mp.spectate = None;
        return;
    }
    let alive: Vec<u64> = mp
        .roster
        .iter()
        .filter(|e| e.alive && e.id != mp.my_id)
        .map(|e| e.id)
        .collect();
    if alive.is_empty() {
        mp.spectate = None;
        return;
    }
    let at = mp.spectate.and_then(|s| alive.iter().position(|a| *a == s));
    let playing = *state.get() == GameState::Playing;
    let step = if playing && mouse.just_pressed(MouseButton::Left) {
        1
    } else if playing && mouse.just_pressed(MouseButton::Right) {
        -1
    } else {
        0
    };
    let i = match at {
        Some(i) => (i as i32 + step).rem_euclid(alive.len() as i32) as usize,
        None => 0,
    };
    mp.spectate = Some(alive[i]);
}

/// Send your plane's motion and details, and (host) everyone's.
#[allow(clippy::type_complexity)]
fn send(
    time: Res<Time<Real>>,
    mut mp: ResMut<Mp>,
    settings: Res<Settings>,
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: Single<&bevy::window::CursorOptions>,
    player: Single<&Aircraft, With<Player>>,
) {
    if !mp.online() {
        return;
    }
    // Your details, when they change.
    let (card, guns) = mp.local_loadout(&settings);
    let info = PlayerInfo {
        nickname: {
            let n = settings.nickname.trim();
            if n.is_empty() {
                "Pilot".into()
            } else {
                n.chars().take(20).collect()
            }
        },
        color: settings.plane_color,
        card,
        guns,
    };
    if mp.last_info.as_ref() != Some(&info) {
        if mp.is_host() {
            mp.last_info = Some(info.clone());
            if let Some(e) = mp.entry_mut(HOST_ID) {
                e.info = Some(info);
            }
            broadcast_roster(&mut mp);
        } else if client_send(&mut mp, &ToHost::Info(info.clone()), true) {
            mp.last_info = Some(info);
        }
    }
    if mp.is_host() && mp.options != settings.host {
        mp.options = settings.host.clone();
        let o = mp.options.clone();
        host_send_all(&mut mp, &ToPlayer::Options(o));
    }

    mp.send_timer -= time.delta_secs();
    let tick = mp.send_timer <= 0.0;
    if tick {
        mp.send_timer += 1.0 / SEND_RATE;
        if mp.send_timer < 0.0 {
            mp.send_timer = 0.0;
        }
        let firing =
            mp.guns_free() && crate::weapons::trigger_now(&mouse, &cursor, settings.single_fire);
        let b = &player.body;
        let state = PlaneState {
            p: b.position.to_array(),
            r: b.rotation.to_array(),
            v: b.velocity.to_array(),
            firing,
            alive: mp.alive(),
        };
        if mp.is_host() {
            let mut list: Vec<(u64, PlaneState)> =
                mp.states.iter().map(|(id, (s, _))| (*id, *s)).collect();
            list.push((HOST_ID, state));
            let bytes = encode(&ToPlayer::States(list));
            if let Net::Host { server, .. } = &mut mp.net {
                server.broadcast_message(DefaultChannel::Unreliable, bytes);
            }
        } else {
            client_send(&mut mp, &ToHost::State(state), false);
        }
    }
    match &mut mp.net {
        Net::Host { server, transport } => transport.send_packets(server),
        Net::Client {
            client, transport, ..
        } => {
            let _ = transport.send_packets(client);
        }
        Net::Off => {}
    }
}

/// Testing aid: `WTAT_MP=host` or `WTAT_MP=join:ADDRESS` hosts or joins at
/// launch, and `WTAT_MP_ROUND=SECONDS` has the host start a round then.
fn test_hooks(
    time: Res<Time<Real>>,
    mut done: Local<(bool, bool)>,
    mut settings: ResMut<Settings>,
    mut commands: MessageWriter<MpCommand>,
    mut mp: ResMut<Mp>,
) {
    let Ok(how) = std::env::var("WTAT_MP") else {
        return;
    };
    if !done.0 && time.elapsed_secs() > 2.0 {
        done.0 = true;
        if let Some(address) = how.strip_prefix("join:") {
            settings.join_address = address.into();
            commands.write(MpCommand::Join);
        } else {
            commands.write(MpCommand::Host);
        }
    }
    let round_at = std::env::var("WTAT_MP_ROUND")
        .ok()
        .and_then(|v| v.parse::<f32>().ok());
    if let Some(at) = round_at
        && !done.1
        && mp.is_host()
        && time.elapsed_secs() > at
    {
        done.1 = true;
        start_round(&mut mp);
    }
}

/// The practice target sits out multiplayer games.
fn hide_target_online(
    mp: Res<Mp>,
    mut was_online: Local<bool>,
    mut target: Query<(&mut Visibility, &mut Aircraft, &mut Transform), With<Target>>,
    mut reset: MessageWriter<crate::target::ResetScenario>,
) {
    let online = mp.online();
    if online {
        for (mut vis, mut a, mut tf) in &mut target {
            *vis = Visibility::Hidden;
            a.body.position = Vec3::new(0.0, -50_000.0, 0.0);
            a.body.velocity = Vec3::ZERO;
            tf.translation = a.body.position;
        }
    } else if *was_online {
        reset.write(crate::target::ResetScenario);
    }
    *was_online = online;
}

// ---------------------------------------------------------------------------
// On-screen: markers over other planes, countdown, kill feed, scoreboard.
// ---------------------------------------------------------------------------

#[derive(Component)]
struct PlayerMarker(u64);

#[derive(Component)]
struct PlayerMarkerText;

#[derive(Component)]
struct PlayerMarkerRange;

#[derive(Component)]
struct Banner;

#[derive(Component)]
struct Feed;

#[derive(Component)]
struct Scoreboard;

#[derive(Component)]
struct ScoreboardText;

#[derive(Component)]
struct MpStatusLine;

const RED: Color = Color::srgb(0.95, 0.22, 0.18);
const MARKER_W: f32 = 220.0;

fn text(size: f32, color: Color) -> impl Bundle {
    (
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
        TextShadow {
            offset: Vec2::splat(1.5),
            color: Color::srgba(0.0, 0.0, 0.0, 0.75),
        },
    )
}

fn spawn_mp_ui(mut commands: Commands) {
    commands.spawn((
        text(40.0, Color::WHITE),
        TextLayout::justify(Justify::Center),
        Node {
            position_type: PositionType::Absolute,
            top: percent(18),
            width: percent(100),
            justify_content: JustifyContent::Center,
            ..default()
        },
        Visibility::Hidden,
        Banner,
    ));
    commands.spawn((
        text(16.0, Color::WHITE),
        TextLayout::justify(Justify::Right),
        Node {
            position_type: PositionType::Absolute,
            top: px(80),
            right: px(12),
            ..default()
        },
        Feed,
    ));
    commands.spawn((
        text(16.0, Color::srgb(1.0, 0.85, 0.4)),
        TextLayout::justify(Justify::Center),
        Node {
            position_type: PositionType::Absolute,
            bottom: px(40),
            width: percent(100),
            justify_content: JustifyContent::Center,
            ..default()
        },
        MpStatusLine,
    ));
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            Visibility::Hidden,
            GlobalZIndex(5),
            Scoreboard,
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    padding: UiRect::all(px(18)),
                    border_radius: BorderRadius::all(px(10)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.05, 0.06, 0.08, 0.88)),
            ))
            .with_children(|panel| {
                // The default font is monospaced, so the columns line up.
                panel.spawn((text(18.0, Color::WHITE), ScoreboardText));
            });
        });
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn update_mp_ui(
    mut commands: Commands,
    mp: Res<Mp>,
    keys: Res<ButtonInput<KeyCode>>,
    settings: Res<Settings>,
    camera: Single<(&Camera, &GlobalTransform), With<ChaseCamera>>,
    player: Single<&GlobalTransform, With<Player>>,
    remotes: Query<(&RemotePlane, &GlobalTransform, &Aircraft, &Visibility)>,
    mut markers: Query<
        (Entity, &PlayerMarker, &mut Node, &mut Visibility, &Children),
        Without<RemotePlane>,
    >,
    mut texts: ParamSet<(
        Query<&mut Text, With<PlayerMarkerText>>,
        Query<&mut Text, With<PlayerMarkerRange>>,
        Single<
            (&mut Text, &mut Visibility),
            (
                With<Banner>,
                Without<PlayerMarker>,
                Without<RemotePlane>,
                Without<Scoreboard>,
            ),
        >,
        Single<&mut Text, With<Feed>>,
        Single<&mut Text, With<ScoreboardText>>,
        Single<&mut Text, With<MpStatusLine>>,
    )>,
    mut board: Single<
        &mut Visibility,
        (
            With<Scoreboard>,
            Without<Banner>,
            Without<PlayerMarker>,
            Without<RemotePlane>,
        ),
    >,
) {
    let (cam, cam_tf) = *camera;
    let project = |world: Vec3| -> Option<Vec2> {
        if (world - cam_tf.translation()).dot(cam_tf.forward().as_vec3()) <= 0.0 {
            return None;
        }
        cam.world_to_viewport(cam_tf, world).ok()
    };

    // One marker per other plane, like the target's: red, with the
    // player's name above the aircraft name, and the range below.
    let mut have = vec![];
    for (entity, marker, mut node, mut vis, children) in &mut markers {
        let found = remotes.iter().find(|(r, ..)| r.id == marker.0);
        let Some((_, tf, aircraft, rvis)) = found.filter(|_| mp.online()) else {
            commands.entity(entity).despawn();
            continue;
        };
        have.push(marker.0);
        let screen = project(tf.translation()).filter(|_| *rvis != Visibility::Hidden);
        match screen {
            Some(p) => {
                node.left = px(p.x - MARKER_W / 2.0);
                node.top = px(p.y - 58.0);
                *vis = Visibility::Inherited;
            }
            None => *vis = Visibility::Hidden,
        }
        let name = format!("{}\n{}", mp.name_of(marker.0), aircraft.model.name);
        let range = settings
            .units
            .range(player.translation().distance(tf.translation()));
        for c in children.iter() {
            if let Ok(mut t) = texts.p0().get_mut(c)
                && t.0 != name
            {
                t.0 = name.clone();
            }
            if let Ok(mut t) = texts.p1().get_mut(c)
                && t.0 != range
            {
                t.0 = range.clone();
            }
        }
    }
    for (remote, ..) in &remotes {
        if have.contains(&remote.id) || !mp.online() {
            continue;
        }
        commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: px(MARKER_W),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    ..default()
                },
                Visibility::Hidden,
                PlayerMarker(remote.id),
            ))
            .with_children(|m| {
                m.spawn((
                    text(15.0, RED),
                    TextLayout::justify(Justify::Center),
                    PlayerMarkerText,
                ));
                // The same red-dotted diamond as the target's marker.
                m.spawn((
                    Node {
                        width: px(18),
                        height: px(18),
                        margin: UiRect::vertical(px(4)),
                        border: UiRect::all(px(2)),
                        ..default()
                    },
                    BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.85)),
                    UiTransform::from_rotation(Rot2::degrees(45.0)),
                ))
                .with_children(|d| {
                    d.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(-4),
                            top: px(-4),
                            width: px(6),
                            height: px(6),
                            ..default()
                        },
                        BackgroundColor(RED),
                    ));
                });
                m.spawn((text(15.0, RED), PlayerMarkerRange));
            });
    }

    // Countdown, reset hold, round messages.
    {
        let (mut t, mut vis) = texts.p2().into_inner();
        let message = if !mp.online() {
            None
        } else if mp.reset_hold > 0.0 {
            Some(format!(
                "New round in {}",
                (RESET_HOLD - mp.reset_hold).ceil().max(1.0) as i32
            ))
        } else if let Phase::Countdown(left) = mp.phase {
            Some(format!("{}", left.ceil().max(1.0) as i32))
        } else {
            mp.banner.as_ref().map(|b| b.0.clone())
        };
        match message {
            Some(m) => {
                if t.0 != m {
                    t.0 = m;
                }
                *vis = Visibility::Inherited;
            }
            None => *vis = Visibility::Hidden,
        }
    }
    {
        let feed: String = mp
            .feed
            .iter()
            .map(|(l, _)| l.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let mut t = texts.p3();
        if t.0 != feed {
            t.0 = feed;
        }
    }
    {
        let reset_key = crate::settings::key_name(settings.key(Action::Reset));
        let line = if !mp.online() {
            String::new()
        } else if !mp.alive() {
            match mp.spectate {
                Some(id) => format!(
                    "Watching {}. Click for the next player, right click for the previous.",
                    mp.name_of(id)
                ),
                None => "You're out until the next round.".into(),
            }
        } else {
            match mp.phase {
                Phase::Lobby if mp.is_host() => {
                    format!("Hold {reset_key} for 3 seconds to start the round.")
                }
                Phase::Lobby => "Waiting for the host to start the round.".into(),
                Phase::Over if mp.is_host() => {
                    format!("Hold {reset_key} for 3 seconds for the next round.")
                }
                Phase::Over => "Waiting for the host to start the next round.".into(),
                Phase::Countdown(_) => "Guns are safe until the countdown ends.".into(),
                Phase::Fight(_) => {
                    let d = mp.me().map(|e| e.damage).unwrap_or(0.0);
                    if d > 0.0 {
                        format!("Damage {:.0}%", (d * 100.0).min(100.0))
                    } else {
                        String::new()
                    }
                }
            }
        };
        let mut t = texts.p5();
        if t.0 != line {
            t.0 = line;
        }
    }
    // Scoreboard while the key is held.
    let show = mp.online() && settings.pressed(&keys, Action::Scoreboard);
    let want = if show {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if **board != want {
        **board = want;
    }
    if show {
        let mut rows: Vec<&Entry> = mp.roster.iter().collect();
        rows.sort_by(|a, b| b.wins.cmp(&a.wins).then(b.kills.cmp(&a.kills)));
        let mut s = format!("{:<22}{:>8}{:>7}  \n", "Player", "Wins", "Kills");
        for e in rows {
            let mut name = e.name();
            if e.id == HOST_ID {
                name += " (host)";
            }
            if e.id == mp.my_id {
                name = format!("> {name}");
            }
            let name: String = name.chars().take(21).collect();
            s += &format!(
                "{:<22}{:>8}{:>7}{}\n",
                name,
                e.wins,
                e.kills,
                if e.alive { "" } else { "  out" }
            );
        }
        let mut t = texts.p4();
        if t.0 != s {
            t.0 = s;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip() {
        let msg = ToPlayer::RoundStart {
            spawns: vec![(
                7,
                Spawn {
                    p: [1.0, 2.0, 3.0],
                    heading: 0.5,
                    speed: 0.55,
                },
            )],
        };
        let back: ToPlayer = serde_json::from_slice(&encode(&msg)).unwrap();
        assert!(matches!(back, ToPlayer::RoundStart { spawns } if spawns[0].0 == 7));
        let settings = Settings::default();
        let (card, guns) = Mp::default().local_loadout(&settings);
        let info = ToHost::Info(PlayerInfo {
            nickname: "A".into(),
            color: PlaneColor::Blue,
            card,
            guns,
        });
        let bytes = encode(&info);
        let back: Result<ToHost, _> = serde_json::from_slice(&bytes);
        assert!(back.is_ok(), "{back:?}", back = back.err());
    }

    /// A host and a client on this computer find each other and swap
    /// details and positions.
    #[test]
    fn host_and_client_connect() {
        let mut host = Mp::default();
        start_host(&mut host, 0).unwrap();
        let port = match &host.net {
            Net::Host { transport, .. } => transport.addresses()[0].port(),
            _ => unreachable!(),
        };
        let mut client = Mp::default();
        start_join(&mut client, &format!("127.0.0.1:{port}")).unwrap();
        let settings = Settings::default();
        let mut app = App::new();
        app.insert_resource(settings);
        let dt = Duration::from_millis(16);
        let mut connected = false;
        for _ in 0..200 {
            for mp in [&mut host, &mut client] {
                match &mut mp.net {
                    Net::Host { server, transport } => {
                        server.update(dt);
                        transport.update(dt, server).unwrap();
                        while let Some(ServerEvent::ClientConnected { .. }) = server.get_event() {
                            connected = true;
                        }
                        transport.send_packets(server);
                    }
                    Net::Client {
                        client, transport, ..
                    } => {
                        client.update(dt);
                        transport.update(dt, client).unwrap();
                        transport.send_packets(client).unwrap();
                    }
                    Net::Off => {}
                }
            }
            if connected {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(connected, "the client never reached the host");
    }

    #[test]
    fn spawns_face_each_other() {
        let mut mp = Mp::default();
        start_host(&mut mp, 0).unwrap();
        mp.roster.push(Entry {
            info: Some(PlayerInfo {
                nickname: "B".into(),
                color: PlaneColor::Blue,
                card: AircraftCard::yak3(),
                guns: vec![],
            }),
            ..Entry::new(5)
        });
        mp.options.spawn_distance = 2000.0;
        start_round(&mut mp);
        assert!(matches!(mp.phase, Phase::Countdown(_)));
        assert!(mp.roster.iter().all(|e| e.alive));
        // Each one's nose points at the middle.
        let spawn = mp.pending.unwrap();
        let p = Vec3::from(spawn.p);
        let nose = Quat::from_rotation_y(spawn.heading) * Vec3::NEG_Z;
        let to_middle = Vec3::new(-p.x, 0.0, -p.z).normalize();
        assert!(nose.dot(to_middle) > 0.999, "{nose} vs {to_middle}");
        assert!((Vec2::new(p.x, p.z).length() - 1000.0).abs() < 1.0);
    }
}
