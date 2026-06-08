//! PWM audio driver for BCM2712 (Raspberry Pi 5)
//!
//! Uses the BCM2712 PWM controller to output audio via PWM (pulse-width modulation).
//! The Pi 5 has two PWM channels (PWM0 and PWM1) routed to GPIO 12/13 (or 18/19 alt).
//! This driver implements:
//! - PWM clock configuration (manage_clock)
//! - PWM channel setup (serialiser mode for audio)
//! - FIFO-based sample output with DMA support
//! - Simple beep and tone generation
//!
//! No_std compatible — uses only core.

use crate::println;

// ─── BCM2712 PWM Registers ──────────────────────────────────────
// PWM controller base (mapped via device region)
const PWM_BASE: u64 = 0x10_7C00_C000;

const PWM_CTL: u64 = PWM_BASE + 0x00; // Control
const PWM_STA: u64 = PWM_BASE + 0x04; // Status
const PWM_DMAC: u64 = PWM_BASE + 0x08; // DMA Control
const PWM_RNG1: u64 = PWM_BASE + 0x10; // Range1
const PWM_DAT1: u64 = PWM_BASE + 0x14; // Data1
const PWM_FIF1: u64 = PWM_BASE + 0x18; // FIFO
const PWM_DMAC_CFG: u64 = PWM_BASE + 0x1C; // DMA Config
const PWM_RNG2: u64 = PWM_BASE + 0x20; // Range2
const PWM_DAT2: u64 = PWM_BASE + 0x24; // Data2

// PWM control flags
const CTL_PWEN1: u32 = 1 << 0; // PWM1 enable
const CTL_MODE1: u32 = 1 << 1; // PWM1 serialiser mode
const CTL_RPTL1: u32 = 1 << 2; // PWM1 repeat last data
const CTL_SBIT1: u32 = 1 << 3; // PWM1 silent bit
const CTL_POLA1: u32 = 1 << 4; // PWM1 polarity inversion
const CTL_USEF1: u32 = 1 << 5; // PWM1 use FIFO
const CTL_CLRF1: u32 = 1 << 6; // Clear FIFO
const CTL_MSEN1: u32 = 1 << 7; // PWM1 M/S enable
const CTL_PWEN2: u32 = 1 << 8; // PWM2 enable
const CTL_MODE2: u32 = 1 << 9; // PWM2 serialiser mode
const CTL_RPTL2: u32 = 1 << 10; // PWM2 repeat last data
const CTL_SBIT2: u32 = 1 << 11; // PWM2 silent bit
const CTL_POLA2: u32 = 1 << 12; // PWM2 polarity inversion
const CTL_USEF2: u32 = 1 << 13; // PWM2 use FIFO
const CTL_MSEN2: u32 = 1 << 15; // PWM2 M/S enable

// PWM status flags
const STA_FULL1: u32 = 1 << 0; // FIFO full
const STA_EMP1: u32 = 1 << 1; // FIFO empty
const STA_WERR1: u32 = 1 << 2; // FIFO write error
const STA_RERR1: u32 = 1 << 3; // FIFO read error
const STA_GAPO1: u32 = 1 << 4; // Gap occurred channel 1
const STA_GAPO2: u32 = 1 << 5; // Gap occurred channel 2
const STA_BERR: u32 = 1 << 8; // Bus error

// DMA config flags
const DMAC_ENAB: u32 = 1 << 31; // DMA enable
const DMAC_PANIC_SHIFT: u32 = 8; // Panic threshold shift
const DMAC_DREQ_SHIFT: u32 = 16; // DREQ threshold shift

// ─── Clock Manager ──────────────────────────────────────────────
// The PWM clock is derived from the PLLD (750 MHz on Pi 5)
const CLOCK_BASE: u64 = 0x10_7C10_1000;
const CLK_PWM_CTL: u64 = CLOCK_BASE + 0xA0; // PWM clock control
const CLK_PWM_DIV: u64 = CLOCK_BASE + 0xA4; // PWM clock divisor

const CLK_CTL_SRC_PLLD: u32 = 6; // Source = PLLD (750 MHz)
const CLK_CTL_ENAB: u32 = 1 << 4; // Clock enable
const CLK_CTL_KILL: u32 = 1 << 5; // Clock kill
const CLK_CTL_BUSY: u32 = 1 << 7; // Clock busy
const CLK_CTL_PASSWD: u32 = 0x5A << 24; // Password field

// ─── Audio Constants ────────────────────────────────────────────
/// Target sample rate: 48 kHz
pub const SAMPLE_RATE: u32 = 48000;

/// Default PWM range for 8-bit audio
const PWM_RANGE: u32 = 256;

/// Audio buffer size (in samples)
pub const AUDIO_BUFFER_SIZE: usize = 1024;

/// Maximum volume (0-255)
pub const MAX_VOLUME: u8 = 255;

/// Minimum volume
pub const MIN_VOLUME: u8 = 0;

// ─── GPIO Function Select ───────────────────────────────────────
const GPIO_BASE: u64 = 0x10_7C20_0000;
const GPFSEL1: u64 = GPIO_BASE + 0x04; // GPIO 10-19 function select

/// GPIO 12 alt function 0 = PWM0
const GPIO_FSEL_ALT0: u32 = 0b100;

// ─── Audio State ────────────────────────────────────────────────
/// Current volume level (0-255)
static mut VOLUME: u8 = 128;

/// Audio playback state
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AudioState {
    Stopped,
    Playing,
    Paused,
}

/// Current audio state
static mut AUDIO_STATE: AudioState = AudioState::Stopped;

/// Audio sample buffer (ring buffer)
static mut AUDIO_BUFFER: [u8; AUDIO_BUFFER_SIZE] = [0; AUDIO_BUFFER_SIZE];

/// Write position in ring buffer
static mut BUFFER_WRITE_POS: usize = 0;

/// Read position in ring buffer
static mut BUFFER_READ_POS: usize = 0;

/// Sample count in buffer
static mut BUFFER_COUNT: usize = 0;

// ─── Low-level register access ──────────────────────────────────

unsafe fn read_reg(addr: u64) -> u32 {
    core::ptr::read_volatile(addr as *const u32)
}

unsafe fn write_reg(addr: u64, val: u32) {
    core::ptr::write_volatile(addr as *mut u32, val);
}

// ─── Clock Configuration ────────────────────────────────────────

/// Configure the PWM clock source and divisor
/// PLLD runs at 750 MHz. For 48 kHz audio with 256-range PWM:
///   750,000,000 / (divisor * 256) = 48,000
///   divisor = 750,000,000 / (48,000 * 256) ≈ 61.035
///   Use divisor = 61 (integer part) with fractional part for precision
unsafe fn configure_clock() {
    // Stop the clock first
    let ctl = read_reg(CLK_PWM_CTL);
    write_reg(
        CLK_PWM_CTL,
        CLK_CTL_PASSWD | (ctl & !CLK_CTL_ENAB) | CLK_CTL_KILL,
    );

    // Wait for clock to stop
    for _ in 0..100 {
        if read_reg(CLK_PWM_CTL) & CLK_CTL_BUSY == 0 {
            break;
        }
        crate::arch::cpu::nop();
    }

    // Set clock divisor: integer part in bits 12..23, fractional in 0..11
    // divisor = 61.035 → integer=61, fractional≈0.035*4096≈144
    let div_int: u32 = 61;
    let div_frac: u32 = 144;
    let divisor = (div_int << 12) | div_frac;
    write_reg(CLK_PWM_DIV, CLK_CTL_PASSWD | divisor);

    // Set source to PLLD and enable
    write_reg(CLK_PWM_CTL, CLK_CTL_PASSWD | CLK_CTL_SRC_PLLD);

    // Small delay for clock to settle
    for _ in 0..100 {
        crate::arch::cpu::nop();
    }

    // Enable the clock
    let ctl = read_reg(CLK_PWM_CTL);
    write_reg(CLK_PWM_CTL, CLK_CTL_PASSWD | ctl | CLK_CTL_ENAB);

    // Wait for clock to become busy (running)
    for _ in 0..1000 {
        if read_reg(CLK_PWM_CTL) & CLK_CTL_BUSY != 0 {
            break;
        }
        crate::arch::cpu::nop();
    }
}

// ─── GPIO Configuration ─────────────────────────────────────────

/// Configure GPIO 12 for PWM0 output (alt function 0)
unsafe fn configure_gpio() {
    let fsel = read_reg(GPFSEL1);
    // GPIO 12 is bits 6-8 in GPFSEL1 (3 bits per pin, pin 12 = (12-10)*3 = 6)
    let mask: u32 = 0b111 << 6;
    let new_fsel = (fsel & !mask) | (GPIO_FSEL_ALT0 << 6);
    write_reg(GPFSEL1, new_fsel);
}

// ─── PWM Configuration ──────────────────────────────────────────

/// Configure PWM channel 1 for audio output in serialiser mode
unsafe fn configure_pwm() {
    // Disable both PWM channels first
    write_reg(PWM_CTL, 0);

    // Wait for channels to stop
    for _ in 0..100 {
        crate::arch::cpu::nop();
    }

    // Clear any pending status flags
    write_reg(PWM_STA, 0x1F4);

    // Set range for channel 1 (8-bit resolution)
    write_reg(PWM_RNG1, PWM_RANGE);

    // Set range for channel 2 (same)
    write_reg(PWM_RNG2, PWM_RANGE);

    // Configure DMA: enable with threshold
    let dmac = DMAC_ENAB
        | (7 << DMAC_PANIC_SHIFT)   // Panic threshold = 7
        | (7 << DMAC_DREQ_SHIFT); // DREQ threshold = 7
    write_reg(PWM_DMAC, dmac);

    // Configure channel 1: serialiser mode, use FIFO, MSB first
    let ctl = CTL_MODE1      // Serialiser mode
            | CTL_USEF1      // Use FIFO
            | CTL_RPTL1      // Repeat last sample
            | CTL_SBIT1; // Silent bit = 0
    write_reg(PWM_CTL, ctl);

    // Small delay
    for _ in 0..100 {
        crate::arch::cpu::nop();
    }

    // Enable channel 1
    write_reg(PWM_CTL, ctl | CTL_PWEN1);
}

// ─── Public API ─────────────────────────────────────────────────

/// Initialize the PWM audio subsystem
pub fn init() {
    unsafe {
        configure_gpio();
        configure_clock();
        configure_pwm();

        VOLUME = 128;
        AUDIO_STATE = AudioState::Stopped;
        BUFFER_WRITE_POS = 0;
        BUFFER_READ_POS = 0;
        BUFFER_COUNT = 0;
    }
    println!("[audio] PWM audio initialized (48 kHz, 8-bit mono)");
}

/// Set the audio volume (0-255)
pub fn set_volume(vol: u8) {
    unsafe {
        VOLUME = vol;
    }
}

/// Get the current volume level
pub fn volume() -> u8 {
    unsafe { VOLUME }
}

/// Get the current audio state
pub fn state() -> AudioState {
    unsafe { AUDIO_STATE }
}

/// Write a sample to the audio ring buffer
/// Returns true if the sample was written, false if the buffer is full
pub fn write_sample(sample: u8) -> bool {
    unsafe {
        if BUFFER_COUNT >= AUDIO_BUFFER_SIZE {
            return false;
        }

        // Apply volume
        let scaled = ((sample as u16 * VOLUME as u16) / 255) as u8;

        AUDIO_BUFFER[BUFFER_WRITE_POS] = scaled;
        BUFFER_WRITE_POS = (BUFFER_WRITE_POS + 1) % AUDIO_BUFFER_SIZE;
        BUFFER_COUNT += 1;
        true
    }
}

/// Read a sample from the ring buffer and push it to the PWM FIFO
/// Returns true if a sample was available, false if buffer is empty
pub fn pump_sample() -> bool {
    unsafe {
        if BUFFER_COUNT == 0 {
            return false;
        }

        let sample = AUDIO_BUFFER[BUFFER_READ_POS];
        BUFFER_READ_POS = (BUFFER_READ_POS + 1) % AUDIO_BUFFER_SIZE;
        BUFFER_COUNT -= 1;

        // Write to PWM FIFO if not full
        if read_reg(PWM_STA) & STA_FULL1 == 0 {
            write_reg(PWM_FIF1, sample as u32);
        }

        true
    }
}

/// Play a tone at the given frequency for the given duration (in ms)
/// Generates a simple square wave at the specified frequency
pub fn beep(freq: u32, duration_ms: u32) {
    if freq == 0 {
        return;
    }

    unsafe {
        AUDIO_STATE = AudioState::Playing;
    }

    // Calculate the number of samples for the duration
    let total_samples = (SAMPLE_RATE as u64 * duration_ms as u64 / 1000) as u32;
    let samples_per_cycle = SAMPLE_RATE / freq;

    let mut sample_num: u32 = 0;
    while sample_num < total_samples {
        // Generate square wave: high for half cycle, low for half cycle
        let phase = sample_num % samples_per_cycle;
        let sample = if phase < samples_per_cycle / 2 {
            255u8
        } else {
            0u8
        };

        // Write to ring buffer
        if write_sample(sample) {
            sample_num += 1;
        } else {
            // Buffer full — pump samples to PWM
            pump_sample();
        }

        // Periodically pump the FIFO
        if sample_num % 64 == 0 {
            for _ in 0..64 {
                if !pump_sample() {
                    break;
                }
            }
        }
    }

    // Drain remaining samples
    unsafe {
        for _ in 0..AUDIO_BUFFER_SIZE {
            if !pump_sample() {
                break;
            }
        }
        AUDIO_STATE = AudioState::Stopped;
    }
}

/// Play a sine wave tone at the given frequency for the given duration (in ms)
/// Uses a lookup table approach for no_std sine generation
pub fn tone(freq: u32, duration_ms: u32, vol: u8) {
    if freq == 0 {
        return;
    }

    let old_vol = volume();
    set_volume(vol);

    unsafe {
        AUDIO_STATE = AudioState::Playing;
    }

    let total_samples = (SAMPLE_RATE as u64 * duration_ms as u64 / 1000) as u32;
    let phase_increment = (freq as u64 * 65536) / SAMPLE_RATE as u64;

    let mut phase: u64 = 0;
    let mut sample_num: u32 = 0;

    while sample_num < total_samples {
        // Simple sine approximation using quadratic bezier curve
        // Maps phase [0,65536) to sine [0,255]
        let p = (phase & 0xFFFF) as u32;
        let sample = if p < 16384 {
            // 0 to π/2: rising
            let x = p as u64 * 255 / 16384;
            x as u8
        } else if p < 32768 {
            // π/2 to π: falling
            let x = (32768 - p) as u64 * 255 / 16384;
            x as u8
        } else if p < 49152 {
            // π to 3π/2: falling negative (offset to 0)
            let x = (p - 32768) as u64 * 255 / 16384;
            (255 - x as u16) as u8
        } else {
            // 3π/2 to 2π: rising negative (offset to 0)
            let x = (65536 - p) as u64 * 255 / 16384;
            (255 - x as u16) as u8
        };

        if write_sample(sample) {
            sample_num += 1;
            phase = phase.wrapping_add(phase_increment);
        } else {
            pump_sample();
        }

        if sample_num % 64 == 0 {
            for _ in 0..64 {
                if !pump_sample() {
                    break;
                }
            }
        }
    }

    // Drain
    unsafe {
        for _ in 0..AUDIO_BUFFER_SIZE {
            if !pump_sample() {
                break;
            }
        }
        AUDIO_STATE = AudioState::Stopped;
    }

    set_volume(old_vol);
}

/// Stop all audio output
pub fn stop() {
    unsafe {
        // Disable PWM
        write_reg(PWM_CTL, 0);

        // Clear buffer
        BUFFER_WRITE_POS = 0;
        BUFFER_READ_POS = 0;
        BUFFER_COUNT = 0;

        AUDIO_STATE = AudioState::Stopped;
    }
}

/// Resume audio playback (re-enable PWM)
pub fn resume() {
    unsafe {
        let ctl = CTL_MODE1 | CTL_USEF1 | CTL_RPTL1 | CTL_SBIT1 | CTL_PWEN1;
        write_reg(PWM_CTL, ctl);
        AUDIO_STATE = AudioState::Playing;
    }
}

/// Get the number of samples in the ring buffer
pub fn buffer_count() -> usize {
    unsafe { BUFFER_COUNT }
}

/// Print audio status for debugging
pub fn print_status() {
    unsafe {
        let sta = read_reg(PWM_STA);
        let ctl = read_reg(PWM_CTL);
        println!("[audio] PWM Status: {:#010x}", sta);
        println!("[audio] PWM Control: {:#010x}", ctl);
        println!("[audio] State: {:?}", AUDIO_STATE);
        println!("[audio] Volume: {}/255", VOLUME);
        println!(
            "[audio] Buffer: {}/{} samples",
            BUFFER_COUNT, AUDIO_BUFFER_SIZE
        );
        println!("[audio] Sample rate: {} Hz", SAMPLE_RATE);
    }
}
