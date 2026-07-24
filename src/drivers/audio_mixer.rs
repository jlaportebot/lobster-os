//! Audio mixer — multi-stream software mixing for LobsterOS
//!
//! Mixes multiple audio streams (system sounds, music, app audio) into
//! a single output stream with independent volume per stream.
//! Implements a simple linear mixer with clamping.

use crate::drivers::audio;
use crate::println;
use crate::sync::spinlock::Spinlock;

/// Maximum number of simultaneous audio streams
const MAX_STREAMS: usize = 4;

/// Stream types for priority mixing
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StreamType {
    /// System sounds (notifications, beeps) — highest priority
    System,
    /// Music/media playback
    Music,
    /// Application audio
    App,
    /// Voice/communication
    Voice,
}

impl StreamType {
    fn priority(&self) -> u8 {
        match self {
            Self::System => 0,
            Self::Voice => 1,
            Self::Music => 2,
            Self::App => 3,
        }
    }
}

/// Audio stream state
#[derive(Clone, Copy)]
struct AudioStream {
    active: bool,
    stream_type: StreamType,
    volume: u8,
    /// Buffer of pending samples
    samples: [u8; 256],
    sample_count: usize,
    read_pos: usize,
}

impl AudioStream {
    const fn empty() -> Self {
        AudioStream {
            active: false,
            stream_type: StreamType::App,
            volume: 128,
            samples: [0; 256],
            sample_count: 0,
            read_pos: 0,
        }
    }
}

static STREAMS: Spinlock<[AudioStream; MAX_STREAMS]> =
    Spinlock::new([AudioStream::empty(); MAX_STREAMS]);
static MASTER_VOLUME: Spinlock<u8> = Spinlock::new(200);

/// Initialize the audio mixer
pub fn init() {
    let mut streams = STREAMS.lock();
    for s in streams.iter_mut() {
        *s = AudioStream::empty();
    }
    *MASTER_VOLUME.lock() = 200;
    println!("[audio_mixer] Initialized ({} streams)", MAX_STREAMS);
}

/// Allocate a new audio stream and return its ID
pub fn allocate_stream(stream_type: StreamType) -> Option<usize> {
    let mut streams = STREAMS.lock();
    for (i, s) in streams.iter_mut().enumerate() {
        if !s.active {
            s.active = true;
            s.stream_type = stream_type;
            s.volume = 200;
            s.sample_count = 0;
            s.read_pos = 0;
            return Some(i);
        }
    }
    None
}

/// Free an audio stream
pub fn free_stream(id: usize) {
    if id >= MAX_STREAMS {
        return;
    }
    let mut streams = STREAMS.lock();
    streams[id].active = false;
    streams[id].sample_count = 0;
}

/// Set per-stream volume (0-255)
pub fn set_stream_volume(id: usize, vol: u8) {
    if id >= MAX_STREAMS {
        return;
    }
    let mut streams = STREAMS.lock();
    if streams[id].active {
        streams[id].volume = vol;
    }
}

/// Write samples to a stream
pub fn write_stream(id: usize, data: &[u8]) -> usize {
    if id >= MAX_STREAMS {
        return 0;
    }
    let mut streams = STREAMS.lock();
    let s = &mut streams[id];
    if !s.active {
        return 0;
    }

    let written = data.len().min(s.samples.len() - s.sample_count);
    s.samples[s.sample_count..s.sample_count + written].copy_from_slice(&data[..written]);
    s.sample_count += written;
    written
}

/// Set the master output volume (0-255)
pub fn set_master_volume(vol: u8) {
    *MASTER_VOLUME.lock() = vol;
    audio::set_volume(vol);
}

/// Get the master output volume
pub fn master_volume() -> u8 {
    *MASTER_VOLUME.lock()
}

/// Mix all active streams and output one sample to the PWM driver.
/// Called from the audio pump timer/ISR.
pub fn pump_mixed_sample() -> bool {
    let streams = STREAMS.lock();
    let master = *MASTER_VOLUME.lock();

    if streams.iter().all(|s| !s.active || s.sample_count == 0) {
        return false;
    }

    // Mix all active streams — simple weighted average
    let mut mixed: u32 = 0;
    let mut count: u32 = 0;

    // Sort by priority: System > Voice > Music > App
    for priority in 0..4u8 {
        for s in streams.iter() {
            if s.active && s.sample_count > 0 && s.stream_type.priority() == priority {
                // Get next sample from this stream
                let sample = s.samples[s.read_pos];
                // Apply per-stream volume
                let scaled = (sample as u32 * s.volume as u32) / 255;
                mixed = mixed.saturating_add(scaled);
                count += 1;
                break; // Only take one sample per priority level
            }
        }
    }

    if count == 0 {
        return false;
    }

    // Average the mixed samples
    let avg = mixed / count;
    // Apply master volume
    let output = (avg * master as u32 / 255) as u8;

    // Write to PWM
    audio::write_sample(output);

    // Advance read positions and update stream state
    drop(streams);
    let mut streams = STREAMS.lock();
    for s in streams.iter_mut() {
        if s.active && s.sample_count > 0 {
            s.read_pos = (s.read_pos + 1) % s.samples.len();
            s.sample_count = s.sample_count.saturating_sub(1);
        }
    }

    true
}

/// Pump all available mixed samples to the audio driver
pub fn pump_all() {
    for _ in 0..256 {
        if !pump_mixed_sample() {
            break;
        }
    }
    // Also pump the PWM driver
    for _ in 0..64 {
        if !audio::pump_sample() {
            break;
        }
    }
}

/// Get the number of active streams
pub fn active_stream_count() -> usize {
    let streams = STREAMS.lock();
    streams.iter().filter(|s| s.active).count()
}

/// Print mixer status
pub fn print_status() {
    let streams = STREAMS.lock();
    let master = *MASTER_VOLUME.lock();
    println!(
        "[audio_mixer] Master volume: {}/255, Active streams: {}/{}",
        master,
        streams.iter().filter(|s| s.active).count(),
        MAX_STREAMS
    );
    for (i, s) in streams.iter().enumerate() {
        if s.active {
            println!(
                "  Stream {}: type={:?} vol={} samples={}",
                i, s.stream_type, s.volume, s.sample_count
            );
        }
    }
}
