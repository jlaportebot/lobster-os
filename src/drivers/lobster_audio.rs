//! LobsterOS audio driver for the QEMU `lobster-audio` device (0x7d400000)
//!
//! Guest pushes 16-bit mono PCM chunks into the MMIO wave window and
//! writes LEN to play them on the HOST's speakers through QEMU's audio
//! backend. RATE sets the sample rate; VOL scales 0..100.

const AUDIO_BASE: u64 = 0x7D_4000_00;

const REG_MAGIC: u64 = 0x00; // "LAUD"
const REG_RATE: u64 = 0x08;
const REG_VOL: u64 = 0x0C;
const REG_LEN: u64 = 0x10;
const REG_STATUS: u64 = 0x14;
const REG_WAVE: u64 = 0x1000; // 4096-byte PCM window

const MAGIC: u32 = 0x4C41_5544; // "LAUD"

const WAVE_SIZE: usize = 4096;

#[inline]
unsafe fn mmio_read32(offset: u64) -> u32 {
    core::ptr::read_volatile((AUDIO_BASE + offset) as *const u32)
}

#[inline]
unsafe fn mmio_write32(offset: u64, val: u32) {
    core::ptr::write_volatile((AUDIO_BASE + offset) as *mut u32, val)
}

#[inline]
unsafe fn mmio_write_bytes(offset: u64, buf: &[u8]) {
    for (i, &b) in buf.iter().enumerate() {
        core::ptr::write_volatile((AUDIO_BASE + offset + i as u64) as *mut u8, b);
    }
}

/// Probe for the device.
pub fn detect() -> bool {
    unsafe { mmio_read32(REG_MAGIC) == MAGIC }
}

/// Set the sample rate (4000..=192000).
pub fn set_rate(rate: u32) {
    unsafe { mmio_write32(REG_RATE, rate) };
}

/// Set the volume (0..=100).
pub fn set_volume(vol: u32) {
    unsafe { mmio_write32(REG_VOL, vol.min(100)) };
}

/// Push a PCM chunk (16-bit mono, little-endian). Chunks larger than
/// 4096 bytes are split. Returns the number of bytes played.
pub fn play_pcm(pcm: &[u8]) -> usize {
    if !detect() {
        return 0;
    }
    let mut played = 0usize;
    let mut off = 0usize;
    while off < pcm.len() {
        let chunk = (pcm.len() - off).min(WAVE_SIZE) & !1; // even bytes
        unsafe {
            mmio_write_bytes(REG_WAVE, &pcm[off..off + chunk]);
            mmio_write32(REG_LEN, chunk as u32);
        }
        off += chunk;
        played += chunk;
    }
    played
}

/// Synthesize and play a sine beep. `freq_milihz` is the frequency in
/// millihertz (e.g. 440_000 = A4). Duration in ms.
pub fn beep(freq_mhz: u64, duration_ms: u64) -> usize {
    if !detect() {
        return 0;
    }
    const RATE: u32 = 8000;
    set_rate(RATE);
    let samples = (duration_ms as u64 * RATE as u64 / 1000) as usize;
    let mut pcm = alloc::vec::Vec::with_capacity(samples * 2);
    for i in 0..samples {
        // 32-bit phase accumulator: step = freq_mHz * 2^32 / RATE / 1000
        let step = freq_mhz * 0x1_0000_0000u64 / (RATE as u64) / 1000;
        let phase = (i as u64).wrapping_mul(step) >> 16 & 0xFFFF; // 0..65535
                                                                  // Integer sine via the classic parabolic approximation:
                                                                  //   sin(pi * x) ≈ 4x(1-x) for x in [0,1] — good enough for beeps.
        let t = phase as u32; // 0..65535 = full cycle
        let quadrant = t >> 14; // 0..3
        let frac = (t & 0x3FFF) as u32; // position within quadrant
        let half = 4 * (frac * (16384 - frac)) >> 14; // 0..~16384 triangle-ish parabola
        let value: i32 = match quadrant {
            0 => half as i32,
            1 => 16384 - half as i32,
            2 => -(half as i32),
            _ => -((16384 - half) as i32),
        };
        let sample = (value * 12000 / 16384) as i16;
        pcm.extend_from_slice(&sample.to_le_bytes());
    }
    play_pcm(&pcm)
}
