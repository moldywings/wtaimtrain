//! Procedural sound effects: gunshots shaped by caliber and muzzle velocity,
//! and a hit sound. Everything is synthesized at startup or on first use
//! and played from memory, so there are no sound files.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::audio::Volume;
use bevy::prelude::*;

const RATE: u32 = 44_100;

pub struct SoundPlugin;

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_sounds);
    }
}

/// Cached sounds.
#[derive(Resource)]
pub struct Sounds {
    guns: HashMap<(u32, u32), Handle<AudioSource>>,
    hit: Handle<AudioSource>,
    kill: Handle<AudioSource>,
}

fn setup_sounds(mut commands: Commands, mut sources: ResMut<Assets<AudioSource>>) {
    commands.insert_resource(Sounds {
        guns: HashMap::new(),
        hit: sources.add(source(&hit_samples())),
        kill: sources.add(source(&kill_samples())),
    });
}

impl Sounds {
    /// The gunshot for a gun, made the first time it's asked for.
    pub fn gunshot(
        &mut self,
        sources: &mut Assets<AudioSource>,
        caliber_mm: f32,
        muzzle_velocity: f32,
    ) -> Handle<AudioSource> {
        let key = ((caliber_mm * 10.0) as u32, muzzle_velocity as u32);
        self.guns
            .entry(key)
            .or_insert_with(|| sources.add(source(&gunshot_samples(caliber_mm, muzzle_velocity))))
            .clone()
    }

    pub fn hit(&self) -> Handle<AudioSource> {
        self.hit.clone()
    }

    pub fn kill(&self) -> Handle<AudioSource> {
        self.kill.clone()
    }
}

/// Play a sound once at `volume`, sped up or slowed down by `speed`.
pub fn play(commands: &mut Commands, sound: Handle<AudioSource>, volume: f32, speed: f32) {
    commands.spawn((
        AudioPlayer(sound),
        PlaybackSettings {
            volume: Volume::Linear(volume),
            speed,
            ..PlaybackSettings::DESPAWN
        },
    ));
}

/// How much higher (above 1) or lower a gun sounds: small, fast rounds
/// crack higher, big slow ones boom lower. 1.0 is the 20 mm ShVAK.
pub fn gun_pitch(caliber_mm: f32, muzzle_velocity: f32) -> f32 {
    ((20.0 / caliber_mm.max(3.0)).powf(0.6) * (muzzle_velocity.max(100.0) / 800.0).powf(0.4))
        .clamp(0.35, 3.0)
}

/// Small deterministic noise source.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// A cannon shot: a sharp muzzle crack, a short burst of filtered noise for
/// the blast, and a falling low thump for the body. Pitch scales the
/// frequencies and how quickly each part dies away.
pub fn gunshot_samples(caliber_mm: f32, muzzle_velocity: f32) -> Vec<f32> {
    let p = gun_pitch(caliber_mm, muzzle_velocity);
    // Bigger guns are a bit heavier and longer.
    let weight = (caliber_mm / 20.0).clamp(0.4, 2.5);
    let len = (0.32 / p.sqrt()).clamp(0.15, 0.6);
    let n = (len * RATE as f32) as usize;
    let mut out = Vec::with_capacity(n);
    let mut noise = Noise(0x1234_5678);
    let (mut lp, mut lp2, mut prev) = (0.0f32, 0.0f32, 0.0f32);
    let mut phase = 0.0f32;
    let dt = 1.0 / RATE as f32;
    for i in 0..n {
        let t = i as f32 * dt;
        let w = noise.next();
        // Crack: high-passed noise, gone in a few milliseconds.
        let hp = w - prev;
        prev = w;
        let crack = hp * (-t / (0.004 / p)).exp() * 0.9;
        // Blast: noise low-passed toward a cutoff that follows the pitch.
        let a = (1.0 - (-std::f32::consts::TAU * 1800.0 * p * dt).exp()).min(1.0);
        lp += (w - lp) * a;
        lp2 += (lp - lp2) * a;
        let blast = lp2 * (-t / (0.045 * weight.sqrt() / p)).exp() * 2.2;
        // Thump: a sine that drops in pitch, like the pressure wave.
        let f = 60.0 * p + 110.0 * p * (-t / 0.03).exp();
        phase += std::f32::consts::TAU * f * dt;
        let thump = phase.sin() * (-t / (0.07 * weight / p)).exp() * 0.8 * weight.min(1.5);
        // Soft attack so it doesn't click.
        let attack = (t / 0.0006).min(1.0);
        out.push((crack + blast + thump) * attack);
    }
    normalize(&mut out, 0.9);
    out
}

/// A hit: a bright metallic "tink" with a little ring, over a dull thud and
/// a click, so you can feel the round land.
pub fn hit_samples() -> Vec<f32> {
    let len = 0.22;
    let n = (len * RATE as f32) as usize;
    let mut out = Vec::with_capacity(n);
    let mut noise = Noise(0x9E37_79B9);
    let dt = 1.0 / RATE as f32;
    let mut prev = 0.0f32;
    for i in 0..n {
        let t = i as f32 * dt;
        let tau = std::f32::consts::TAU;
        // Inharmonic partials ring like struck sheet metal.
        let ring = (tau * 1320.0 * t).sin() * 0.55 * (-t / 0.06).exp()
            + (tau * 2290.0 * t).sin() * 0.35 * (-t / 0.035).exp()
            + (tau * 3710.0 * t).sin() * 0.2 * (-t / 0.02).exp();
        let thud = (tau * (95.0 + 60.0 * (-t / 0.02).exp()) * t).sin() * 0.9 * (-t / 0.05).exp();
        let w = noise.next();
        let click = (w - prev) * 0.8 * (-t / 0.002).exp();
        prev = w;
        let attack = (t / 0.0004).min(1.0);
        out.push((ring + thud + click) * attack);
    }
    normalize(&mut out, 0.85);
    out
}

/// Target destroyed: a deep boom under a bright rising three-note chime
/// (a major arpeggio of bell-like tones), so a kill sounds like a reward.
pub fn kill_samples() -> Vec<f32> {
    let len = 1.3;
    let n = (len * RATE as f32) as usize;
    let mut out = Vec::with_capacity(n);
    let mut noise = Noise(0x2545_F491);
    let dt = 1.0 / RATE as f32;
    let tau = std::f32::consts::TAU;
    let (mut lp, mut lp2) = (0.0f32, 0.0f32);
    // A5, C#6, E6, each struck a little after the last.
    let notes = [(880.0f32, 0.0f32), (1108.7, 0.09), (1318.5, 0.18)];
    for i in 0..n {
        let t = i as f32 * dt;
        // Boom: low-passed noise and a falling sine.
        let w = noise.next();
        lp += (w - lp) * 0.04;
        lp2 += (lp - lp2) * 0.04;
        let boom = lp2 * 6.0 * (-t / 0.35).exp()
            + (tau * (45.0 + 70.0 * (-t / 0.08).exp()) * t).sin() * 0.7 * (-t / 0.4).exp();
        // Chime: each note a bell (fundamental plus a soft octave and a
        // slightly sharp upper partial), with a quick attack.
        let mut chime = 0.0;
        for (f, start) in notes {
            let u = t - start;
            if u < 0.0 {
                continue;
            }
            let env = (u / 0.004).min(1.0) * (-u / 0.45).exp();
            chime += env
                * ((tau * f * u).sin()
                    + 0.35 * (tau * 2.0 * f * u).sin() * (-u / 0.15).exp()
                    + 0.15 * (tau * 2.76 * f * u).sin() * (-u / 0.08).exp());
        }
        let attack = (t / 0.001).min(1.0);
        out.push((boom * 0.8 + chime * 0.45) * attack);
    }
    normalize(&mut out, 0.9);
    out
}

fn normalize(samples: &mut [f32], peak: f32) {
    let max = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if max > 0.0 {
        for s in samples {
            *s *= peak / max;
        }
    }
}

fn source(samples: &[f32]) -> AudioSource {
    AudioSource {
        bytes: Arc::from(wav_bytes(samples).into_boxed_slice()),
    }
}

/// 16-bit mono PCM WAV file in memory.
pub fn wav_bytes(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&RATE.to_le_bytes());
    b.extend_from_slice(&(RATE * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rough dominant frequency from zero crossings of the low end.
    fn crossings_per_second(s: &[f32]) -> f32 {
        let mut lp = 0.0f32;
        let mut last = 0.0f32;
        let mut count = 0;
        for &x in s {
            lp += (x - lp) * 0.02;
            if (lp > 0.0) != (last > 0.0) {
                count += 1;
            }
            last = lp;
        }
        count as f32 / (s.len() as f32 / RATE as f32) / 2.0
    }

    #[test]
    fn bigger_slower_guns_sound_lower() {
        assert!((gun_pitch(20.0, 800.0) - 1.0).abs() < 1e-4);
        assert!(gun_pitch(12.7, 860.0) > 1.2);
        assert!(gun_pitch(37.0, 690.0) < 0.75);
        let mg = crossings_per_second(&gunshot_samples(7.62, 850.0));
        let cannon = crossings_per_second(&gunshot_samples(37.0, 690.0));
        println!("7.62 mm ~{mg:.0} Hz, 37 mm ~{cannon:.0} Hz");
        assert!(mg > cannon);
    }

    #[test]
    fn sounds_are_valid_wavs() {
        let s = gunshot_samples(20.0, 800.0);
        assert!(s.iter().all(|x| x.is_finite() && x.abs() <= 1.0));
        let w = wav_bytes(&s);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(w.len(), 44 + s.len() * 2);
        assert!(hit_samples().iter().all(|x| x.is_finite()));
        assert!(
            kill_samples()
                .iter()
                .all(|x| x.is_finite() && x.abs() <= 1.0)
        );
    }
}
