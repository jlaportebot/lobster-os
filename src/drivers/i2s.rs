//! I2S Audio Driver for RP1 (Raspberry Pi 5)
//!
//! Implements I2S (Inter-IC Sound) interface on RP1 for high-quality
//! digital audio output/input. Supports standard I2S, left-justified,
//! and right-justified formats.
//!
//! RP1 has 2 I2S instances (I2S0, I2S1) at offsets 0xA0000, 0xA4000.
//! Each supports TX and RX with configurable sample rates, bit depths,
//! and channel counts.
//!
//! Reference: RP1 Peripherals Datasheet Section 3.8

use crate::drivers::rp1::{rp1_periph_read32, rp1_periph_write32, I2sInstance};
use crate::println;
/// I2S register offsets
pub mod regs {
    pub const CTRL: u64 = 0x00;
    pub const STATUS: u64 = 0x04;
    pub const TX_FIFO: u64 = 0x08;
    pub const RX_FIFO: u64 = 0x0C;
    pub const INT_EN: u64 = 0x10;
    pub const INT_ST: u64 = 0x14;
    pub const INT_CLR: u64 = 0x18;
    pub const CLK_DIV: u64 = 0x1C;
    pub const FRAME_CTRL: u64 = 0x20;
    pub const FIFO_CTRL: u64 = 0x24;
    pub const FIFO_STATUS: u64 = 0x28;
    pub const DMA_CTRL: u64 = 0x2C;
    pub const MODE: u64 = 0x30;
}

/// CTRL register bits
pub mod ctrl_bits {
    pub const EN: u32 = 1 << 0;
    pub const TX_EN: u32 = 1 << 1;
    pub const RX_EN: u32 = 1 << 2;
    pub const LOOPBACK: u32 = 1 << 3;
    pub const RESET: u32 = 1 << 4;
}

/// STATUS register bits
pub mod status_bits {
    pub const TX_UNDERRUN: u32 = 1 << 0;
    pub const RX_OVERRUN: u32 = 1 << 1;
    pub const TX_FIFO_EMPTY: u32 = 1 << 2;
    pub const RX_FIFO_FULL: u32 = 1 << 3;
    pub const TX_FIFO_FULL: u32 = 1 << 4;
    pub const RX_FIFO_EMPTY: u32 = 1 << 5;
}

/// FRAME_CTRL register bits
pub mod frame_bits {
    pub const FORMAT_I2S: u32 = 0 << 0;
    pub const FORMAT_LJ: u32 = 1 << 0;
    pub const FORMAT_RJ: u32 = 2 << 0;
    pub const FORMAT_DSP: u32 = 3 << 0;
    pub const WORD_SIZE_SHIFT: u32 = 4;
    pub const CHANNELS_SHIFT: u32 = 8;
    pub const FRAME_POL_SHIFT: u32 = 12;
    pub const CLK_POL_SHIFT: u32 = 13;
    pub const MSB_FIRST: u32 = 1 << 14;
}

/// MODE register bits
pub mod mode_bits {
    pub const MASTER: u32 = 1 << 0;
    pub const SLAVE: u32 = 0 << 0;
}

/// I2S sample format
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum I2sFormat {
    I2S,
    LeftJustified,
    RightJustified,
    Dsp,
}

/// I2S word size (bits per sample)
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum WordSize {
    Bits16 = 16,
    Bits24 = 24,
    Bits32 = 32,
}

/// I2S configuration
#[derive(Clone, Copy, Debug)]
pub struct I2sConfig {
    pub format: I2sFormat,
    pub word_size: WordSize,
    pub channels: u32,          // 1 (mono) or 2 (stereo)
    pub sample_rate: u32,       // e.g., 44100, 48000, 96000, 192000
    pub master_mode: bool,      // true = master (generates BCLK/LRCLK), false = slave
    pub frame_pol_normal: bool, // true = normal LRCLK polarity
    pub clk_pol_normal: bool,   // true = normal BCLK polarity
    pub msb_first: bool,
}

/// I2S instance state
struct I2sState {
    configured: bool,
    config: I2sConfig,
    tx_underrun_count: u32,
    rx_overrun_count: u32,
}

static mut I2S0_STATE: I2sState = I2sState {
    configured: false,
    config: I2sConfig {
        format: I2sFormat::I2S,
        word_size: WordSize::Bits16,
        channels: 2,
        sample_rate: 48000,
        master_mode: true,
        frame_pol_normal: true,
        clk_pol_normal: true,
        msb_first: true,
    },
    tx_underrun_count: 0,
    rx_overrun_count: 0,
};

static mut I2S1_STATE: I2sState = I2sState {
    configured: false,
    config: I2sConfig {
        format: I2sFormat::I2S,
        word_size: WordSize::Bits16,
        channels: 2,
        sample_rate: 48000,
        master_mode: true,
        frame_pol_normal: true,
        clk_pol_normal: true,
        msb_first: true,
    },
    tx_underrun_count: 0,
    rx_overrun_count: 0,
};

/// Get mutable state for I2S instance
fn get_state(instance: I2sInstance) -> &'static mut I2sState {
    unsafe {
        match instance {
            I2sInstance::I2s0 => &mut I2S0_STATE,
            I2sInstance::I2s1 => &mut I2S1_STATE,
        }
    }
}

/// Calculate clock divider for target sample rate
/// RP1 I2S clock source is 48 MHz (clk_i2s)
/// BCLK = sample_rate * word_size * channels
/// CLK_DIV = (48_000_000 / BCLK) - 1
fn calc_clk_div(sample_rate: u32, word_size: WordSize, channels: u32) -> u32 {
    const I2S_CLK: u32 = 48_000_000;
    let bclk = sample_rate * (word_size as u32) * channels;
    if bclk == 0 {
        return 0;
    }
    (I2S_CLK / bclk).saturating_sub(1)
}

/// Initialize I2S instance with configuration
pub unsafe fn i2s_init(instance: I2sInstance, config: I2sConfig) {
    let base = instance.base_offset();
    let state = get_state(instance);

    // Reset the I2S block
    rp1_periph_write32(base, regs::CTRL, ctrl_bits::RESET);
    for _ in 0..100 {
        crate::arch::cpu::nop();
    }

    // Disable first
    rp1_periph_write32(base, regs::CTRL, 0);

    // Set clock divider
    let clk_div = calc_clk_div(config.sample_rate, config.word_size, config.channels);
    rp1_periph_write32(base, regs::CLK_DIV, clk_div);

    // Configure frame format
    let mut frame_ctrl = 0u32;
    match config.format {
        I2sFormat::I2S => frame_ctrl |= frame_bits::FORMAT_I2S,
        I2sFormat::LeftJustified => frame_ctrl |= frame_bits::FORMAT_LJ,
        I2sFormat::RightJustified => frame_ctrl |= frame_bits::FORMAT_RJ,
        I2sFormat::Dsp => frame_ctrl |= frame_bits::FORMAT_DSP,
    }
    frame_ctrl |= ((config.word_size as u32 - 1) / 8) << frame_bits::WORD_SIZE_SHIFT;
    frame_ctrl |= (config.channels - 1) << frame_bits::CHANNELS_SHIFT;
    if !config.frame_pol_normal {
        frame_ctrl |= 1 << frame_bits::FRAME_POL_SHIFT;
    }
    if !config.clk_pol_normal {
        frame_ctrl |= 1 << frame_bits::CLK_POL_SHIFT;
    }
    if config.msb_first {
        frame_ctrl |= frame_bits::MSB_FIRST;
    }
    rp1_periph_write32(base, regs::FRAME_CTRL, frame_ctrl);

    // Configure mode (master/slave)
    let mut mode = 0u32;
    if config.master_mode {
        mode |= mode_bits::MASTER;
    }
    rp1_periph_write32(base, regs::MODE, mode);

    // FIFO configuration: enable TX/RX, set thresholds
    let fifo_ctrl = (1 << 0) | (1 << 1) | (4 << 4) | (4 << 8); // TX_EN, RX_EN, TX_THRESH=4, RX_THRESH=4
    rp1_periph_write32(base, regs::FIFO_CTRL, fifo_ctrl);

    // Clear interrupts
    rp1_periph_write32(base, regs::INT_CLR, 0xFFFFFFFF);

    // Enable interrupts (underrun/overrun)
    rp1_periph_write32(
        base,
        regs::INT_EN,
        status_bits::TX_UNDERRUN | status_bits::RX_OVERRUN,
    );

    // Enable I2S
    let mut ctrl = ctrl_bits::EN;
    ctrl |= ctrl_bits::TX_EN | ctrl_bits::RX_EN;
    rp1_periph_write32(base, regs::CTRL, ctrl);

    state.configured = true;
    state.config = config;

    println!(
        "[i2s] {:?} initialized: {}Hz, {}ch, {}bits, {:?}, {}",
        instance,
        config.sample_rate,
        config.channels,
        config.word_size as u32,
        config.format,
        if config.master_mode {
            "master"
        } else {
            "slave"
        }
    );
}

/// Write a sample to TX FIFO (blocking if full)
pub unsafe fn i2s_write_sample(instance: I2sInstance, sample: i32) {
    let base = instance.base_offset();
    // Wait for TX FIFO not full
    loop {
        let status = rp1_periph_read32(base, regs::FIFO_STATUS);
        if (status & status_bits::TX_FIFO_FULL) == 0 {
            break;
        }
        crate::arch::cpu::nop();
    }
    rp1_periph_write32(base, regs::TX_FIFO, sample as u32);
}

/// Read a sample from RX FIFO (blocking if empty)
pub unsafe fn i2s_read_sample(instance: I2sInstance) -> i32 {
    let base = instance.base_offset();
    // Wait for RX FIFO not empty
    loop {
        let status = rp1_periph_read32(base, regs::FIFO_STATUS);
        if (status & status_bits::RX_FIFO_EMPTY) == 0 {
            break;
        }
        crate::arch::cpu::nop();
    }
    rp1_periph_read32(base, regs::RX_FIFO) as i32
}

/// Check if TX FIFO has space
pub unsafe fn i2s_tx_ready(instance: I2sInstance) -> bool {
    let base = instance.base_offset();
    let status = rp1_periph_read32(base, regs::FIFO_STATUS);
    (status & status_bits::TX_FIFO_FULL) == 0
}

/// Check if RX FIFO has data
pub unsafe fn i2s_rx_ready(instance: I2sInstance) -> bool {
    let base = instance.base_offset();
    let status = rp1_periph_read32(base, regs::FIFO_STATUS);
    (status & status_bits::RX_FIFO_EMPTY) == 0
}

/// Write multiple samples (stereo interleaved: L, R, L, R...)
pub unsafe fn i2s_write_buffer(instance: I2sInstance, samples: &[i32]) {
    for &sample in samples {
        i2s_write_sample(instance, sample);
    }
}

/// Read multiple samples into buffer
pub unsafe fn i2s_read_buffer(instance: I2sInstance, buffer: &mut [i32]) {
    for i in 0..buffer.len() {
        buffer[i] = i2s_read_sample(instance);
    }
}

/// Get status flags
pub unsafe fn i2s_get_status(instance: I2sInstance) -> u32 {
    let base = instance.base_offset();
    rp1_periph_read32(base, regs::STATUS)
}

/// Clear status flags
pub unsafe fn i2s_clear_status(instance: I2sInstance, flags: u32) {
    let base = instance.base_offset();
    rp1_periph_write32(base, regs::INT_CLR, flags);
}

/// Enable/disable I2S
pub unsafe fn i2s_set_enabled(instance: I2sInstance, enabled: bool) {
    let base = instance.base_offset();
    let mut ctrl = rp1_periph_read32(base, regs::CTRL);
    if enabled {
        ctrl |= ctrl_bits::EN | ctrl_bits::TX_EN | ctrl_bits::RX_EN;
    } else {
        ctrl &= !(ctrl_bits::EN | ctrl_bits::TX_EN | ctrl_bits::RX_EN);
    }
    rp1_periph_write32(base, regs::CTRL, ctrl);
}

/// Get current configuration
pub unsafe fn i2s_get_config(instance: I2sInstance) -> Option<I2sConfig> {
    let state = get_state(instance);
    if state.configured {
        Some(state.config)
    } else {
        None
    }
}

/// Print I2S status for debugging
pub unsafe fn i2s_print_status(instance: I2sInstance) {
    let base = instance.base_offset();
    let state = get_state(instance);
    let ctrl = rp1_periph_read32(base, regs::CTRL);
    let status = rp1_periph_read32(base, regs::STATUS);
    let fifo_status = rp1_periph_read32(base, regs::FIFO_STATUS);
    let clk_div = rp1_periph_read32(base, regs::CLK_DIV);

    println!("[i2s] {:?} Status:", instance);
    println!("  Configured: {}", state.configured);
    println!("  CTRL: {:#010x}", ctrl);
    println!("  STATUS: {:#010x}", status);
    println!("  FIFO_STATUS: {:#010x}", fifo_status);
    println!("  CLK_DIV: {}", clk_div);
    if state.configured {
        let cfg = &state.config;
        println!(
            "  Config: {}Hz {}ch {}bits {:?} {}",
            cfg.sample_rate,
            cfg.channels,
            cfg.word_size as u32,
            cfg.format,
            if cfg.master_mode { "master" } else { "slave" }
        );
    }
    println!(
        "  TX Underruns: {}, RX Overruns: {}",
        state.tx_underrun_count, state.rx_overrun_count
    );
}

/// Handle I2S interrupt (call from IRQ handler)
pub unsafe fn i2s_handle_irq(instance: I2sInstance) {
    unsafe {
        let base = instance.base_offset();
        let state = get_state(instance);
        let int_status = rp1_periph_read32(base, regs::INT_ST);

        if int_status & status_bits::TX_UNDERRUN != 0 {
            state.tx_underrun_count += 1;
            rp1_periph_write32(base, regs::INT_CLR, status_bits::TX_UNDERRUN);
        }
        if int_status & status_bits::RX_OVERRUN != 0 {
            state.rx_overrun_count += 1;
            rp1_periph_write32(base, regs::INT_CLR, status_bits::RX_OVERRUN);
        }
    }
}
