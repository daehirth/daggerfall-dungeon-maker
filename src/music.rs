//! Dungeon music: which songs Daggerfall Unity plays in dungeons, how it
//! chooses one, and playing a song to preview it.
//!
//! Inside a dungeon Daggerfall Unity (`SongManager`) seeds its random number
//! generator with a value from the dungeon's record mixed with the region,
//! draws one number and plays the dungeon song it selects from
//! [`DUNGEON_SONGS`]. The export writes that value, so a dungeon can be given
//! any of the songs ([`seed_for_song`]). With FM music the same choice picks
//! the FM version of the song.
//!
//! [`MusicPlayer`] plays one song at a time from `MIDI.BSA`: on Windows and
//! macOS through the system's MIDI synthesizer, on Linux with TiMidity or
//! FluidSynth, if one is installed.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::arena2::{parse_hmi, MidiSong};

/// One of the songs played inside dungeons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DungeonSong {
    /// Daggerfall Unity's name for it (without `song_`), saved in project files.
    pub name: &'static str,
    /// Its file in `MIDI.BSA`.
    pub file: &'static str,
}

/// The songs Daggerfall Unity plays inside dungeons, in its order.
pub const DUNGEON_SONGS: [DungeonSong; 15] = [
    DungeonSong { name: "dungeon", file: "DUNGEON.HMI" },
    DungeonSong { name: "dungeon5", file: "DUNGEON5.HMI" },
    DungeonSong { name: "dungeon6", file: "DUNGEON6.HMI" },
    DungeonSong { name: "dungeon7", file: "DUNGEON7.HMI" },
    DungeonSong { name: "dungeon8", file: "DUNGEON8.HMI" },
    DungeonSong { name: "dungeon9", file: "DUNGEON9.HMI" },
    DungeonSong { name: "gdngn10", file: "GDNGN10.HMI" },
    DungeonSong { name: "gdngn11", file: "GDNGN11.HMI" },
    DungeonSong { name: "gdungn4", file: "GDUNGN4.HMI" },
    DungeonSong { name: "gdungn9", file: "GDUNGN9.HMI" },
    DungeonSong { name: "04", file: "04.HMI" },
    DungeonSong { name: "05", file: "05.HMI" },
    DungeonSong { name: "07", file: "07.HMI" },
    DungeonSong { name: "15", file: "15.HMI" },
    DungeonSong { name: "28", file: "28.HMI" },
];

impl DungeonSong {
    /// Name shown to the user, for example "Dungeon 2 (DUNGEON5)".
    pub fn label(self) -> String {
        let number = DUNGEON_SONGS.iter().position(|song| *song == self).map_or(0, |index| index + 1);
        format!("Dungeon {number} ({})", self.file.trim_end_matches(".HMI"))
    }

    /// The song called `name` (as saved), if there is one.
    pub fn from_name(name: &str) -> Option<Self> {
        DUNGEON_SONGS.into_iter().find(|song| song.name == name)
    }

    /// Its place in [`DUNGEON_SONGS`].
    pub fn index(self) -> usize {
        DUNGEON_SONGS.iter().position(|song| *song == self).unwrap_or(0)
    }
}

/// The song Daggerfall Unity plays inside a dungeon of region `region_index`
/// whose record holds `seed`.
pub fn song_for_seed(seed: u32, region_index: usize) -> DungeonSong {
    // As SongManager: srand((ushort)seed ^ ((byte)region << 8)), then one rand().
    let mixed = u64::from((seed as u16) ^ (u16::from(region_index as u8) << 8));
    let next = mixed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
    let random = (next >> 16) & 0x7FFF;
    DUNGEON_SONGS[(random % DUNGEON_SONGS.len() as u64) as usize]
}

/// A value to write in a dungeon of region `region_index` so Daggerfall Unity
/// plays `song` there: the first that does, counting up from `start`.
pub fn seed_for_song(song: DungeonSong, region_index: usize, start: u16) -> u32 {
    (0..=u16::MAX)
        .map(|step| u32::from(start.wrapping_add(step)))
        .find(|&seed| song_for_seed(seed, region_index) == song)
        .unwrap_or(u32::from(start))
}

/// Plays one song at a time; dropping it stops the music.
#[derive(Default)]
pub struct MusicPlayer {
    playing: Option<Playing>,
}

/// The song being played.
struct Playing {
    file: String,
    stop: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
    /// The player program on Linux.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    child: Option<std::process::Child>,
}

impl MusicPlayer {
    /// Starts playing the HMI song `data` (the file `file` of `MIDI.BSA`),
    /// stopping any song already playing.
    pub fn play(&mut self, file: &str, data: &[u8]) -> Result<(), String> {
        self.stop();
        let song = parse_hmi(data).map_err(|error| format!("Could not read {file}: {error}"))?;
        let stop = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let child = start(&song, file, stop.clone(), finished.clone())?;
        self.playing = Some(Playing { file: file.to_string(), stop, finished, child });
        Ok(())
    }

    /// Stops the music, if any is playing.
    pub fn stop(&mut self) {
        if let Some(mut playing) = self.playing.take() {
            playing.stop.store(true, Ordering::Relaxed);
            if let Some(child) = playing.child.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    /// The `MIDI.BSA` file of the song playing, or `None` once it has ended.
    pub fn playing_file(&mut self) -> Option<&str> {
        let playing = self.playing.as_mut()?;
        let ended = playing.finished.load(Ordering::Relaxed)
            || playing.child.as_mut().is_some_and(|child| !matches!(child.try_wait(), Ok(None)));
        if ended {
            self.playing = None;
        }
        self.playing.as_ref().map(|playing| playing.file.as_str())
    }
}

impl Drop for MusicPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Plays `song` through the system's MIDI synthesizer on a thread of its
/// own, until it ends or `stop` is set.
#[cfg(not(target_os = "linux"))]
fn start(
    song: &MidiSong,
    _file: &str,
    stop: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
) -> Result<Option<std::process::Child>, String> {
    use std::time::{Duration, Instant};

    let output = midir::MidiOutput::new("Daggerfall Dungeon Maker")
        .map_err(|error| format!("Could not use the MIDI output: {error}"))?;
    let ports = output.ports();
    // The built-in synthesizer when there is a choice (Windows: Microsoft GS Wavetable Synth).
    let port = ports
        .iter()
        .find(|port| {
            output.port_name(port).is_ok_and(|name| name.contains("Wavetable") || name.contains("DLS"))
        })
        .or_else(|| ports.first())
        .cloned()
        .ok_or("No MIDI synthesizer was found to play the music on")?;
    let mut connection = output
        .connect(&port, "Daggerfall Dungeon Maker preview")
        .map_err(|error| format!("Could not open the MIDI synthesizer: {error}"))?;
    let timeline = song.timeline();
    std::thread::spawn(move || {
        // Start from a clean state: every controller reset, every note off.
        let silence = |connection: &mut midir::MidiOutputConnection| {
            for channel in 0..16u8 {
                let _ = connection.send(&[0xB0 | channel, 123, 0]);
                let _ = connection.send(&[0xB0 | channel, 120, 0]);
            }
        };
        for channel in 0..16u8 {
            let _ = connection.send(&[0xB0 | channel, 121, 0]);
        }
        let started = Instant::now();
        'song: for (seconds, message) in &timeline {
            loop {
                if stop.load(Ordering::Relaxed) {
                    break 'song;
                }
                let due = Duration::from_secs_f64(*seconds);
                let elapsed = started.elapsed();
                if elapsed >= due {
                    break;
                }
                std::thread::sleep((due - elapsed).min(Duration::from_millis(20)));
            }
            let _ = connection.send(message);
        }
        silence(&mut connection);
        connection.close();
        finished.store(true, Ordering::Relaxed);
    });
    Ok(None)
}

/// Plays `song` with TiMidity or FluidSynth, from a standard MIDI file
/// written to the temporary folder.
#[cfg(target_os = "linux")]
fn start(
    song: &MidiSong,
    file: &str,
    _stop: Arc<AtomicBool>,
    _finished: Arc<AtomicBool>,
) -> Result<Option<std::process::Child>, String> {
    use std::process::{Command, Stdio};

    let path =
        std::env::temp_dir().join(format!("daggerfall-dungeon-maker-{}.mid", file.trim_end_matches(".HMI")));
    std::fs::write(&path, song.to_standard_midi_file())
        .map_err(|error| format!("Could not write {}: {error}", path.display()))?;
    let path = path.display().to_string();
    let sound_fonts = [
        "/usr/share/sounds/sf2/FluidR3_GM.sf2",
        "/usr/share/sounds/sf2/default-GM.sf2",
        "/usr/share/soundfonts/FluidR3_GM.sf2",
        "/usr/share/soundfonts/default.sf2",
    ];
    let mut players: Vec<Vec<String>> = vec![vec!["timidity".into(), "-idqq".into(), path.clone()]];
    for sound_font in sound_fonts.iter().filter(|font| std::path::Path::new(font).is_file()) {
        players.push(vec!["fluidsynth".into(), "-ni".into(), (*sound_font).into(), path.clone()]);
    }
    for player in players {
        let spawned = Command::new(&player[0])
            .args(&player[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(child) = spawned {
            return Ok(Some(child));
        }
    }
    Err("Previewing music on Linux needs TiMidity (timidity) or FluidSynth with a General MIDI sound font \
         installed"
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_choose_the_wanted_song() {
        for region in [0, 17, 61] {
            for song in DUNGEON_SONGS {
                let seed = seed_for_song(song, region, 1234);
                assert_eq!(song_for_seed(seed, region), song, "{} in region {region}", song.name);
            }
        }
    }

    #[test]
    fn matches_daggerfall_unitys_random_numbers() {
        // DFRandom.srand(0); rand() is (12345 >> 16) & 0x7FFF = 0, so song 0.
        assert_eq!(song_for_seed(0, 0), DUNGEON_SONGS[0]);
        // srand(1): (1103515245 + 12345) >> 16 = 16838, and 16838 % 15 = 8.
        assert_eq!(song_for_seed(1, 0), DUNGEON_SONGS[8]);
        // The region's low byte goes into the high byte of the seed.
        assert_eq!(song_for_seed(1 ^ (17 << 8), 0), song_for_seed(1, 17));
    }

    #[test]
    fn names_round_trip() {
        for song in DUNGEON_SONGS {
            assert_eq!(DungeonSong::from_name(song.name), Some(song));
        }
        assert_eq!(DUNGEON_SONGS[1].label(), "Dungeon 2 (DUNGEON5)");
    }
}
