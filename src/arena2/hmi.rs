//! Reader for the HMI songs in `MIDI.BSA`.
//!
//! Daggerfall's music is stored as HMI files ("HMI-MIDISONG"), a variant of
//! MIDI from Human Machine Interfaces' DOS sound system. The layout follows
//! ZMusic's HMI reader (GZDoom, BSD licence): a song header with the timing
//! and a table of tracks; each track holds MIDI events, except that a note-on
//! is followed by the time until its note ends instead of a separate note-off,
//! and a few HMI-only events that are skipped. Each track names the sound
//! devices it is for; a General MIDI synthesizer plays those meant for
//! General MIDI or an MPU-401.
//!
//! [`MidiSong`] holds the song's events merged into one list; it can be
//! turned into timed messages for a synthesizer ([`MidiSong::timeline`]) or
//! into a standard MIDI file ([`MidiSong::to_standard_midi_file`]).

use super::error::{Arena2Error, Arena2Result};

const SONG_MAGIC: &[u8] = b"HMI-MIDISONG";
const TRACK_MAGIC: &[u8] = b"HMI-MIDITRACK";
/// Offsets in the song header.
const DIVISION_OFFSET: usize = 0xD4;
const TRACK_COUNT_OFFSET: usize = 0xE4;
const TRACK_DIRECTORY_OFFSET: usize = 0xE8;
/// Offsets in a track header.
const TRACK_DATA_OFFSET: usize = 0x57;
const TRACK_DESIGNATION_OFFSET: usize = 0x99;
const DESIGNATION_COUNT: usize = 8;
/// Device numbers in track designations that a General MIDI synthesizer plays.
const DEVICE_GENERAL_MIDI: u16 = 0xA000;
const DEVICE_MPU401: u16 = 0xA001;
/// Tempo (microseconds per quarter note) HMI songs start with.
const INITIAL_TEMPO: u32 = 4_000_000;
/// Data bytes after a channel message's status byte, by message type (0x80-0xF0).
const CHANNEL_DATA_LENGTHS: [usize; 8] = [2, 2, 2, 2, 1, 1, 2, 0];
/// Data bytes after a system common message's status byte, by its low nibble.
const COMMON_DATA_LENGTHS: [usize; 16] = [0, 1, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

/// One event of a song.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MidiEvent {
    /// A channel message: its status byte and data bytes.
    Message(Vec<u8>),
    /// A new tempo, in microseconds per quarter note.
    Tempo(u32),
}

/// A song: its events, in order, with the tick each happens at.
#[derive(Debug, Clone, Default)]
pub struct MidiSong {
    /// Ticks per quarter note.
    pub division: u16,
    pub events: Vec<(u64, MidiEvent)>,
}

impl MidiSong {
    /// Each message with the time it is due, in seconds from the start.
    pub fn timeline(&self) -> Vec<(f64, Vec<u8>)> {
        let mut timeline = Vec::with_capacity(self.events.len());
        let mut tempo = f64::from(INITIAL_TEMPO);
        let mut last_tick = 0;
        let mut seconds = 0.0;
        let division = f64::from(self.division.max(1));
        for (tick, event) in &self.events {
            seconds += (tick - last_tick) as f64 * tempo / division / 1_000_000.0;
            last_tick = *tick;
            match event {
                MidiEvent::Tempo(new_tempo) => tempo = f64::from(*new_tempo),
                MidiEvent::Message(message) => timeline.push((seconds, message.clone())),
            }
        }
        timeline
    }

    /// How long the song lasts, in seconds.
    pub fn duration(&self) -> f64 {
        self.timeline().last().map_or(0.0, |(seconds, _)| *seconds)
    }

    /// The song as a standard MIDI file (format 0, one track).
    pub fn to_standard_midi_file(&self) -> Vec<u8> {
        let mut track = Vec::new();
        let write_variable = |track: &mut Vec<u8>, mut value: u64| {
            let mut bytes = vec![(value & 0x7F) as u8];
            value >>= 7;
            while value > 0 {
                bytes.push(((value & 0x7F) as u8) | 0x80);
                value >>= 7;
            }
            bytes.reverse();
            track.extend(bytes);
        };
        let tempo_event = |tempo: u32| {
            let tempo = tempo.min(0xFF_FFFF);
            vec![0xFF, 0x51, 0x03, (tempo >> 16) as u8, (tempo >> 8) as u8, tempo as u8]
        };
        write_variable(&mut track, 0);
        track.extend(tempo_event(INITIAL_TEMPO));
        let mut last_tick = 0;
        for (tick, event) in &self.events {
            write_variable(&mut track, tick - last_tick);
            last_tick = *tick;
            match event {
                MidiEvent::Tempo(tempo) => track.extend(tempo_event(*tempo)),
                MidiEvent::Message(message) => track.extend(message),
            }
        }
        write_variable(&mut track, 0);
        track.extend([0xFF, 0x2F, 0x00]);

        let mut file = Vec::with_capacity(track.len() + 22);
        file.extend(b"MThd");
        file.extend(6u32.to_be_bytes());
        file.extend(0u16.to_be_bytes());
        file.extend(1u16.to_be_bytes());
        file.extend(self.division.min(0x7FFF).to_be_bytes());
        file.extend(b"MTrk");
        file.extend((track.len() as u32).to_be_bytes());
        file.extend(track);
        file
    }
}

/// Reads the HMI song in `data`.
pub fn parse_hmi(data: &[u8]) -> Arena2Result<MidiSong> {
    if data.len() < 0x100 || !data.starts_with(SONG_MAGIC) {
        return Err(Arena2Error::format("not an HMI song"));
    }
    let u16_at = |offset: usize| -> Option<u16> {
        Some(u16::from_le_bytes(data.get(offset..offset + 2)?.try_into().ok()?))
    };
    let u32_at = |offset: usize| -> Option<usize> {
        Some(u32::from_le_bytes(data.get(offset..offset + 4)?.try_into().ok()?) as usize)
    };
    let division = u16_at(DIVISION_OFFSET).unwrap_or(0).saturating_mul(4).max(1);
    let track_count = usize::from(u16_at(TRACK_COUNT_OFFSET).unwrap_or(0));
    let directory = u32_at(TRACK_DIRECTORY_OFFSET).unwrap_or(0);

    struct Track<'a> {
        data: &'a [u8],
        designations: Vec<u16>,
    }
    let mut tracks = Vec::new();
    for index in 0..track_count {
        let Some(start) = u32_at(directory + index * 4) else { continue };
        let end = match index + 1 < track_count {
            true => u32_at(directory + index * 4 + 4).unwrap_or(data.len()).min(data.len()),
            false => data.len(),
        };
        if start >= end || !data[start..].starts_with(TRACK_MAGIC) {
            continue;
        }
        let Some(data_start) = u32_at(start + TRACK_DATA_OFFSET).map(|offset| start + offset) else {
            continue;
        };
        if data_start >= end {
            continue;
        }
        let designations = (0..DESIGNATION_COUNT)
            .filter_map(|slot| u16_at(start + TRACK_DESIGNATION_OFFSET + slot * 2))
            .take_while(|&designation| designation != 0)
            .collect();
        tracks.push(Track { data: &data[data_start..end], designations });
    }
    if tracks.is_empty() {
        return Err(Arena2Error::format("HMI song has no tracks"));
    }
    // The tracks a General MIDI synthesizer plays; if none is marked for one, all of them.
    let for_general_midi = |track: &Track<'_>| {
        track.designations.iter().any(|&device| device == DEVICE_GENERAL_MIDI || device == DEVICE_MPU401)
    };
    let any_for_general_midi = tracks.iter().any(for_general_midi);

    // Events from every track: (tick, order, event). Note ends sort before
    // everything else at the same tick, so a note played again is not cut short.
    let mut events: Vec<(u64, u8, usize, MidiEvent)> = Vec::new();
    let mut sequence = 0;
    for track in tracks.iter().filter(|track| !any_for_general_midi || for_general_midi(track)) {
        read_track(track.data, &mut |tick, is_note_end, event| {
            events.push((tick, u8::from(!is_note_end), sequence, event));
            sequence += 1;
        });
    }
    events.sort_by_key(|(tick, order, sequence, _)| (*tick, *order, *sequence));
    Ok(MidiSong { division, events: events.into_iter().map(|(tick, _, _, event)| (tick, event)).collect() })
}

/// Reads one track's events, passing each to `emit` with its tick and
/// whether it ends a note.
fn read_track(data: &[u8], emit: &mut dyn FnMut(u64, bool, MidiEvent)) {
    let mut position = 0;
    let mut tick: u64 = 0;
    let mut running_status = 0u8;
    let byte = |position: &mut usize| -> Option<u8> {
        let value = *data.get(*position)?;
        *position += 1;
        Some(value)
    };
    let variable = |position: &mut usize| -> Option<u64> {
        let mut value = 0u64;
        for _ in 0..5 {
            let next = *data.get(*position)?;
            *position += 1;
            value = (value << 7) | u64::from(next & 0x7F);
            if next & 0x80 == 0 {
                return Some(value);
            }
        }
        Some(value)
    };
    loop {
        let Some(delay) = variable(&mut position) else { return };
        tick += delay;
        let Some(status) = byte(&mut position) else { return };
        match status {
            0xF0 | 0xF7 => {
                // System exclusive: skipped.
                let Some(length) = variable(&mut position) else { return };
                position += length as usize;
            }
            0xFF => {
                let (Some(kind), Some(length)) = (byte(&mut position), variable(&mut position)) else {
                    return;
                };
                let length = length as usize;
                let Some(body) = data.get(position..position + length) else { return };
                position += length;
                match kind {
                    0x2F => return,
                    0x51 if length >= 3 => {
                        let tempo =
                            (u32::from(body[0]) << 16) | (u32::from(body[1]) << 8) | u32::from(body[2]);
                        emit(tick, false, MidiEvent::Tempo(tempo));
                    }
                    _ => {}
                }
            }
            0xFE => {
                // HMI-only events: skipped, as ZMusic does.
                let Some(kind) = byte(&mut position) else { return };
                match kind {
                    0x13 | 0x15 => position += 6,
                    0x12 | 0x14 => position += 2,
                    0x10 => {
                        position += 2;
                        let Some(&length) = data.get(position) else { return };
                        position += usize::from(length) + 5;
                    }
                    _ => return,
                }
            }
            _ if status & 0xF0 == 0xF0 => {
                // Other system messages: skipped with their data.
                position += COMMON_DATA_LENGTHS[usize::from(status & 0x0F)];
            }
            _ => {
                let (status, first) = if status & 0x80 == 0 {
                    (running_status, status)
                } else {
                    running_status = status;
                    let Some(first) = byte(&mut position) else { return };
                    (status, first)
                };
                if status & 0x80 == 0 {
                    return;
                }
                let mut message = vec![status, first];
                if CHANNEL_DATA_LENGTHS[usize::from((status & 0x70) >> 4)] == 2 {
                    let Some(second) = byte(&mut position) else { return };
                    message.push(second);
                }
                let is_note_on = status & 0xF0 == 0x90;
                let (key, velocity) = (first, message.get(2).copied().unwrap_or(0));
                emit(tick, is_note_on && velocity == 0, MidiEvent::Message(message));
                if is_note_on {
                    // An HMI note-on is followed by how long the note lasts.
                    let Some(length) = variable(&mut position) else { return };
                    if velocity > 0 {
                        emit(tick + length, true, MidiEvent::Message(vec![status, key, 0]));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-track HMI song holding `events`, for General MIDI.
    fn song(events: &[u8]) -> Vec<u8> {
        let mut data = vec![0u8; 0x100];
        data[..SONG_MAGIC.len()].copy_from_slice(SONG_MAGIC);
        data[DIVISION_OFFSET..DIVISION_OFFSET + 2].copy_from_slice(&15u16.to_le_bytes());
        data[TRACK_COUNT_OFFSET..TRACK_COUNT_OFFSET + 2].copy_from_slice(&1u16.to_le_bytes());
        data[TRACK_DIRECTORY_OFFSET..TRACK_DIRECTORY_OFFSET + 4].copy_from_slice(&0xF0u32.to_le_bytes());
        data[0xF0..0xF4].copy_from_slice(&0x100u32.to_le_bytes());
        let mut track = vec![0u8; 0xC0];
        track[..TRACK_MAGIC.len()].copy_from_slice(TRACK_MAGIC);
        track[TRACK_DATA_OFFSET..TRACK_DATA_OFFSET + 4].copy_from_slice(&0xC0u32.to_le_bytes());
        track[TRACK_DESIGNATION_OFFSET..TRACK_DESIGNATION_OFFSET + 2]
            .copy_from_slice(&DEVICE_GENERAL_MIDI.to_le_bytes());
        data.extend(track);
        data.extend(events);
        data
    }

    #[test]
    fn reads_notes_with_their_lengths() {
        // Programme 5 on channel 0, a note held 60 ticks, then a running-status note, then the end.
        let events =
            [0x00, 0xC0, 0x05, 0x00, 0x90, 0x3C, 0x64, 0x3C, 0x0A, 0x3E, 0x50, 0x1E, 0x00, 0xFF, 0x2F, 0x00];
        let parsed = parse_hmi(&song(&events)).unwrap();
        assert_eq!(parsed.division, 60);
        let messages: Vec<(u64, Vec<u8>)> = parsed
            .events
            .iter()
            .filter_map(|(tick, event)| match event {
                MidiEvent::Message(message) => Some((*tick, message.clone())),
                MidiEvent::Tempo(_) => None,
            })
            .collect();
        assert_eq!(
            messages,
            vec![
                (0, vec![0xC0, 0x05]),
                (0, vec![0x90, 0x3C, 0x64]),
                (10, vec![0x90, 0x3E, 0x50]),
                (40, vec![0x90, 0x3E, 0x00]),
                (60, vec![0x90, 0x3C, 0x00]),
            ]
        );
        // 60 ticks per quarter note at 4 s per quarter: one tick lasts 1/15 s.
        assert!((parsed.duration() - 4.0).abs() < 1e-9);
        let file = parsed.to_standard_midi_file();
        assert!(file.starts_with(b"MThd") && file.windows(4).any(|window| window == b"MTrk"));
    }

    #[test]
    fn rejects_other_data() {
        assert!(parse_hmi(b"MThd").is_err());
    }
}
