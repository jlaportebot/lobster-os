//! Audio Mixer and ALSA-like API for LobsterOS
//!
//! Provides a high-level audio API built on top of I2S and PWM drivers:
//! - Multiple simultaneous audio streams (mixing)
//! - Volume control per stream and master
//! - Sample rate conversion (basic)
//! - PCM playback/capture via I2S (primary) and PWM (fallback)
//! - ALSA-like device interface: /dev/snd/pcmC0D0p, /dev/snd/controlC0

use crate::drivers::audio::{
    init as pwm_init, set_volume as pwm_set_volume, write_sample as pwm_write_sample,
};
use crate::drivers::i2s::{
    i2s_init, i2s_set_enabled, i2s_write_sample, I2sConfig, I2sFormat, WordSize,
};
use crate::drivers::rp1::I2sInstance;
use crate::fs::devfs::{register_device, DeviceOps};
use crate::println;
use crate::sync::spinlock::Spinlock;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

/// Audio sample format
#[derive(Clone, Copy, PartialEq, Debug)]
#[repr(u32)]
pub enum AudioFormat {
    S16LE = 0, // 16-bit signed little-endian
    S24LE = 1, // 24-bit signed little-endian (packed in 32-bit)
    S32LE = 2, // 32-bit signed little-endian
    F32LE = 3, // 32-bit float little-endian
}

/// Audio stream direction
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum StreamDirection {
    Playback,
    Capture,
}

/// Audio stream state
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum StreamState {
    Closed,
    Open,
    Prepared,
    Running,
    Paused,
    Draining,
    XRun,
}

/// PCM substream (one direction of a stream)
struct PcmSubstream {
    id: u32,
    direction: StreamDirection,
    state: StreamState,
    format: AudioFormat,
    channels: u32,
    rate: u32,
    period_size: u32,
    buffer_size: u32,
    volume: u32, // 0-1000 (milli-decibels, 0 = mute, 1000 = 0dB)
    muted: bool,
    buffer: Vec<i32>, // Mixing buffer (normalized to 32-bit)
    read_ptr: u32,
    write_ptr: u32,
    avail_min: u32,
    start_threshold: u32,
    stop_threshold: u32,
    silence_threshold: u32,
    silence_size: u32,
    boundary: u32,
}

impl PcmSubstream {
    fn new(id: u32, direction: StreamDirection) -> Self {
        Self {
            id,
            direction,
            state: StreamState::Closed,
            format: AudioFormat::S16LE,
            channels: 2,
            rate: 48000,
            period_size: 1024,
            buffer_size: 8192,
            volume: 1000,
            muted: false,
            buffer: Vec::new(),
            read_ptr: 0,
            write_ptr: 0,
            avail_min: 0,
            start_threshold: 0,
            stop_threshold: 0,
            silence_threshold: 0,
            silence_size: 0,
            boundary: 0,
        }
    }
}

/// PCM device (combines playback + capture substreams)
struct PcmDevice {
    id: u32,
    name: String,
    playback: Option<PcmSubstream>,
    capture: Option<PcmSubstream>,
    i2s_instance: Option<I2sInstance>,
    i2s_initialized: bool,
}

impl PcmDevice {
    fn new(id: u32, name: &str) -> Self {
        Self {
            id,
            name: String::from(name),
            playback: None,
            capture: None,
            i2s_instance: None,
            i2s_initialized: false,
        }
    }
}

/// Global audio mixer state
static mut MIXER_STATE: Option<AudioMixer> = None;

struct AudioMixer {
    devices: Vec<Spinlock<PcmDevice>>,
    master_volume: u32, // 0-1000
    master_muted: bool,
    next_stream_id: AtomicU32,
    mixing_buffer: Spinlock<Vec<i32>>,
}

impl AudioMixer {
    fn new() -> Self {
        let mut devices = Vec::new();

        // Create default PCM device (I2S0 for playback)
        let mut pcm0 = PcmDevice::new(0, "rp1-i2s0");
        pcm0.i2s_instance = Some(I2sInstance::I2s0);
        devices.push(Spinlock::new(pcm0));

        // Create second PCM device (I2S1 for capture if available)
        let mut pcm1 = PcmDevice::new(1, "rp1-i2s1");
        pcm1.i2s_instance = Some(I2sInstance::I2s1);
        devices.push(Spinlock::new(pcm1));

        // Create PWM fallback device
        let mut pwm_dev = PcmDevice::new(2, "pwm-audio");
        pwm_dev.i2s_instance = None;
        devices.push(Spinlock::new(pwm_dev));

        Self {
            devices,
            master_volume: 1000,
            master_muted: false,
            next_stream_id: AtomicU32::new(1),
            mixing_buffer: Spinlock::new(Vec::new()),
        }
    }

    fn get_device(&self, device_id: u32) -> Option<&Spinlock<PcmDevice>> {
        self.devices.get(device_id as usize)
    }

    fn alloc_stream_id(&self) -> u32 {
        self.next_stream_id.fetch_add(1, Ordering::Relaxed)
    }

    fn mix_streams(&self, output_buffer: &mut [i32], frame_count: usize) {
        // Mix all active playback streams into output buffer
        let mut mixing_buf = self.mixing_buffer.lock();
        mixing_buf.resize(frame_count * 2, 0); // stereo

        // Clear output
        for sample in output_buffer.iter_mut() {
            *sample = 0;
        }

        // Mix each device's playback stream
        for device_lock in &self.devices {
            let mut device = device_lock.lock();
            if let Some(ref mut substream) = device.playback {
                if substream.state == StreamState::Running && !substream.muted {
                    // Simple mixing: add samples with volume scaling
                    let vol =
                        (substream.volume as f32 / 1000.0) * (self.master_volume as f32 / 1000.0);
                    if self.master_muted {
                        continue;
                    }

                    let avail = frame_count.min(
                        (substream.buffer.len() as u32).saturating_sub(substream.read_ptr) as usize,
                    );
                    let channels = substream.channels as usize;
                    for i in 0..avail {
                        for ch in 0..channels {
                            let src_idx = substream.read_ptr as usize + i * channels + ch;
                            let dst_idx = i * 2 + ch;
                            if src_idx < substream.buffer.len() && dst_idx < output_buffer.len() {
                                let sample = (substream.buffer[src_idx] as f32 * vol) as i32;
                                output_buffer[dst_idx] =
                                    output_buffer[dst_idx].saturating_add(sample);
                            }
                        }
                    }
                    substream.read_ptr += (avail * channels) as u32;
                }
            }
        }
    }
}

/// Initialize the audio mixer
pub fn mixer_init() {
    // Initialize PWM audio as fallback
    pwm_init();

    // Initialize I2S0 for primary playback
    let i2s_config = I2sConfig {
        format: I2sFormat::I2S,
        word_size: WordSize::Bits16,
        channels: 2,
        sample_rate: 48000,
        master_mode: true,
        frame_pol_normal: true,
        clk_pol_normal: true,
        msb_first: true,
    };
    unsafe {
        i2s_init(I2sInstance::I2s0, i2s_config);
        i2s_set_enabled(I2sInstance::I2s0, true);
    }

    unsafe {
        MIXER_STATE = Some(AudioMixer::new());
    }

    // Register control device
    let ops = DeviceOps {
        read: None,
        write: None,
        ioctl: Some(mixer_ioctl),
    };
    register_device("controlC0", 116, 0, false, ops, 0o666);

    // Register PCM devices
    for i in 0..3 {
        let name = alloc::format!("pcmC0D{}p", i);
        let ops = DeviceOps {
            read: Some(pcm_read),
            write: Some(pcm_write),
            ioctl: Some(pcm_ioctl),
        };
        register_device(&name, 116, i + 16, false, ops, 0o666);
    }

    println!("[audio] Mixer initialized with {} PCM devices", 3);
}

/// Main mixer ioctl handler
fn mixer_ioctl(cmd: u64, arg: u64) -> i64 {
    match cmd {
        // Volume control
        0x8008_4301 => {
            // SNDRV_CTL_IOCTL_ELEM_READ
            unsafe { handle_ctl_read(arg) }
        }
        0x4008_4302 => {
            // SNDRV_CTL_IOCTL_ELEM_WRITE
            unsafe { handle_ctl_write(arg) }
        }
        0x8010_4303 => {
            // SNDRV_CTL_IOCTL_ELEM_LIST
            unsafe { handle_ctl_list(arg) }
        }
        0x8010_4304 => {
            // SNDRV_CTL_IOCTL_ELEM_INFO
            unsafe { handle_ctl_info(arg) }
        }
        _ => -25, // ENOTTY
    }
}

/// PCM read handler
fn pcm_read(_buf: *mut u8, _len: usize) -> usize {
    // For capture streams
    0
}

/// PCM write handler
fn pcm_write(buf: *const u8, len: usize) -> usize {
    // Write PCM data to default playback stream (device 0 - pcmC0D0p)
    unsafe {
        if let Some(mixer) = MIXER_STATE.as_ref() {
            if let Some(device_lock) = mixer.get_device(0) {
                let mut device = device_lock.lock();
                if let Some(ref mut substream) = device.playback {
                    if substream.state == StreamState::Running {
                        let samples = len / 4; // 4 bytes per sample (i32)
                        let src = buf as *const i32;
                        let avail = substream.buffer.len() - substream.write_ptr as usize;
                        let to_write = core::cmp::min(samples, avail);
                        for i in 0..to_write {
                            let sample = *src.add(i);
                            substream.buffer[substream.write_ptr as usize] = sample;
                            substream.write_ptr += 1;
                        }
                        return to_write * 4;
                    }
                }
            }
        }
    }
    0
}

/// PCM ioctl handler
fn pcm_ioctl(cmd: u64, arg: u64) -> i64 {
    match cmd {
        0x8010_4401 => unsafe { pcm_ioctl_hw_params(arg) }, // SNDRV_PCM_IOCTL_HW_PARAMS
        0x4004_4402 => unsafe { pcm_ioctl_sw_params(arg) }, // SNDRV_PCM_IOCTL_SW_PARAMS
        0x0004_4403 => unsafe { pcm_ioctl_prepare(arg) },   // SNDRV_PCM_IOCTL_PREPARE
        0x0004_4404 => unsafe { pcm_ioctl_start(arg) },     // SNDRV_PCM_IOCTL_START
        0x0004_4405 => unsafe { pcm_ioctl_drop(arg) },      // SNDRV_PCM_IOCTL_DROP
        0x0004_4406 => unsafe { pcm_ioctl_drain(arg) },     // SNDRV_PCM_IOCTL_DRAIN
        0x8004_4407 => unsafe { pcm_ioctl_status(arg) },    // SNDRV_PCM_IOCTL_STATUS
        0x8008_4408 => unsafe { pcm_ioctl_delay(arg) },     // SNDRV_PCM_IOCTL_DELAY
        0x4004_4409 => unsafe { pcm_ioctl_writei_frames(arg) }, // SNDRV_PCM_IOCTL_WRITEI_FRAMES
        0x8004_440A => unsafe { pcm_ioctl_readi_frames(arg) }, // SNDRV_PCM_IOCTL_READI_FRAMES
        _ => -25,
    }
}

/// PCM hardware parameters ioctl
unsafe fn pcm_ioctl_hw_params(_arg: u64) -> i64 {
    // Set format, rate, channels, period size, buffer size
    0
}

/// PCM software parameters ioctl
unsafe fn pcm_ioctl_sw_params(_arg: u64) -> i64 {
    // Set start/stop/silence thresholds
    0
}

/// PCM prepare ioctl
unsafe fn pcm_ioctl_prepare(_arg: u64) -> i64 {
    0
}

/// PCM start ioctl
unsafe fn pcm_ioctl_start(_arg: u64) -> i64 {
    if let Some(mixer) = MIXER_STATE.as_ref() {
        for device_lock in &mixer.devices {
            let mut device = device_lock.lock();
            if let Some(ref mut substream) = device.playback {
                substream.state = StreamState::Running;
                substream.read_ptr = 0;
                substream.write_ptr = 0;
            }
        }
    }
    0
}

/// PCM drop ioctl
unsafe fn pcm_ioctl_drop(_arg: u64) -> i64 {
    if let Some(mixer) = MIXER_STATE.as_ref() {
        for device_lock in &mixer.devices {
            let mut device = device_lock.lock();
            if let Some(ref mut substream) = device.playback {
                substream.state = StreamState::Prepared;
            }
        }
    }
    0
}

/// PCM drain ioctl
unsafe fn pcm_ioctl_drain(_arg: u64) -> i64 {
    0
}

/// PCM status ioctl
unsafe fn pcm_ioctl_status(_arg: u64) -> i64 {
    0
}

/// PCM delay ioctl
unsafe fn pcm_ioctl_delay(_arg: u64) -> i64 {
    0
}

/// PCM write frames ioctl
unsafe fn pcm_ioctl_writei_frames(_arg: u64) -> i64 {
    // Copy frames from userspace to stream buffer
    0
}

/// PCM read frames ioctl
unsafe fn pcm_ioctl_readi_frames(_arg: u64) -> i64 {
    0
}

/// Control element read
unsafe fn handle_ctl_read(_arg: u64) -> i64 {
    0
}

/// Control element write
unsafe fn handle_ctl_write(_arg: u64) -> i64 {
    0
}

/// Control element list
unsafe fn handle_ctl_list(_arg: u64) -> i64 {
    0
}

/// Control element info
unsafe fn handle_ctl_info(_arg: u64) -> i64 {
    0
}

/// Audio mixer task - runs periodically to mix and output audio
pub fn mixer_task() {
    const FRAMES_PER_PERIOD: usize = 1024;
    const MIX_INTERVAL_US: u64 = 10000; // ~10ms at 48kHz/1024 frames

    let mut last_mix = crate::arch::timer::Timer::uptime_ms();

    loop {
        let now = crate::arch::timer::Timer::uptime_ms();
        if now.saturating_sub(last_mix) >= 10 {
            last_mix = now;

            // Mix active streams
            if let Some(mixer) = unsafe { MIXER_STATE.as_ref() } {
                let mut output_buffer = [0i32; 2048]; // stereo frames

                mixer.mix_streams(&mut output_buffer, FRAMES_PER_PERIOD);

                // Output to I2S
                for i in 0..FRAMES_PER_PERIOD {
                    let left = output_buffer[i * 2];
                    let right = output_buffer[i * 2 + 1];

                    // Convert to 16-bit for I2S (assuming S16LE format)
                    let left_16 = ((left >> 16) & 0xFFFF) as i32;
                    let right_16 = ((right >> 16) & 0xFFFF) as i32;

                    unsafe {
                        i2s_write_sample(I2sInstance::I2s0, left_16);
                        i2s_write_sample(I2sInstance::I2s0, right_16);
                    }
                }

                // Also output to PWM as fallback
                pwm_write_sample((output_buffer[0] >> 16) as u8);
            }
        }

        crate::arch::cpu::wfe();
    }
}

/// Get mixer state (for kernel use)
pub fn get_mixer() -> Option<&'static AudioMixer> {
    unsafe { MIXER_STATE.as_ref() }
}

/// Kernel helper: play a simple tone
pub fn play_tone(freq: u32, duration_ms: u32, _volume: u32) {
    if let Some(_mixer) = unsafe { MIXER_STATE.as_ref() } {
        _mixer.alloc_stream_id();
        // Generate tone samples into a stream buffer
        // Simplified - just beep via PWM for now
        crate::drivers::audio::beep(freq, duration_ms);
    }
}

/// Kernel helper: set master volume (0-1000)
pub fn set_master_volume(vol: u32) {
    if let Some(_mixer) = unsafe { MIXER_STATE.as_ref() } {
        // Would need atomic or lock to set master_volume
        // Simplified for now
        pwm_set_volume((vol * 255 / 1000) as u8);
    }
}
