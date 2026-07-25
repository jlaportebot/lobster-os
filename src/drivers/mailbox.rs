//! BCM2835 property mailbox channel driver

/// Mailbox base address
#[cfg(all(not(feature = "qemu"), not(feature = "qemu_raspi5b")))]
const MAILBOX_BASE: u64 = 0x10_7C00_B880; // BCM2712 (real Pi 5)
#[cfg(all(feature = "qemu", not(feature = "qemu_raspi5b")))]
const MAILBOX_BASE: u64 = 0xFE_00_B880; // BCM2711 (QEMU raspi4b)
#[cfg(feature = "qemu_raspi5b")]
const MAILBOX_BASE: u64 = 0x7C01_3880; // BCM2711-style (QEMU raspi5b)

/// Mailbox registers
const MAILBOX_READ: u64 = MAILBOX_BASE + 0x00;
const MAILBOX_PEEK: u64 = MAILBOX_BASE + 0x10;
const MAILBOX_SENDER: u64 = MAILBOX_BASE + 0x14;
const MAILBOX_STATUS: u64 = MAILBOX_BASE + 0x18;
const MAILBOX_CONFIG: u64 = MAILBOX_BASE + 0x1C;
const MAILBOX_WRITE: u64 = MAILBOX_BASE + 0x20;

/// Mailbox channels
const MAILBOX_CHANNEL_PROPERTY: u32 = 8;

/// Mailbox status flags
const MAILBOX_STATUS_EMPTY: u32 = 0x40000000;
const MAILBOX_STATUS_FULL: u32 = 0x80000000;

/// Mailbox property tags
const TAG_GET_FIRMWARE_REVISION: u32 = 0x00000001;
const TAG_GET_BOARD_MODEL: u32 = 0x00010001;
const TAG_GET_BOARD_REVISION: u32 = 0x00010002;
const TAG_GET_BOARD_MAC_ADDRESS: u32 = 0x00010003;
const TAG_GET_ARM_MEMORY: u32 = 0x00010005;
const TAG_GET_POWER_STATE: u32 = 0x00020001;
const TAG_SET_POWER_STATE: u32 = 0x00028001;
const TAG_GET_CLOCK_RATE: u32 = 0x00030002;
const TAG_SET_CLOCK_RATE: u32 = 0x00038002;
const TAG_ALLOCATE_BUFFER: u32 = 0x00040001;
const TAG_RELEASE_BUFFER: u32 = 0x00048001;
const TAG_BLANK_SCREEN: u32 = 0x00040002;
const TAG_GET_PHYSICAL_SIZE: u32 = 0x00040003;
const TAG_SET_PHYSICAL_SIZE: u32 = 0x00048003;
const TAG_GET_VIRTUAL_SIZE: u32 = 0x00040004;
const TAG_SET_VIRTUAL_SIZE: u32 = 0x00048004;
const TAG_GET_DEPTH: u32 = 0x00040005;
const TAG_SET_DEPTH: u32 = 0x00048005;
const TAG_GET_PIXEL_ORDER: u32 = 0x00040006;
const TAG_SET_PIXEL_ORDER: u32 = 0x00048006;
const TAG_GET_PITCH: u32 = 0x00040008;
const TAG_GET_VIRTUAL_OFFSET: u32 = 0x00040009;
const TAG_SET_VIRTUAL_OFFSET: u32 = 0x00048009;
const TAG_GET_ALPHA_MODE: u32 = 0x00040007;
const TAG_SET_ALPHA_MODE: u32 = 0x00048007;
const TAG_GET_OVERSCAN: u32 = 0x0004000A;
const TAG_SET_OVERSCAN: u32 = 0x0004800A;
const TAG_GET_PALETTE: u32 = 0x0004000B;
const TAG_SET_PALETTE: u32 = 0x0004800B;
const TAG_SET_CURSOR_INFO: u32 = 0x00008010;
const TAG_SET_CURSOR_STATE: u32 = 0x00008011;

/// Mailbox message header
#[repr(C)]
struct MailboxMessageHeader {
    size: u32,
    code: u32,
}

/// Mailbox message footer
#[repr(C)]
struct MailboxMessageFooter {
    end_tag: u32,
}

/// Mailbox property message
#[repr(C)]
struct MailboxPropertyMessage {
    tag: u32,
    buffer_size: u32,
    request_code: u32,
    data: [u8; 0], // Variable length data
}

/// Mailbox property buffer
#[repr(C)]
struct MailboxPropertyBuffer {
    header: MailboxMessageHeader,
    tag: u32,
    buffer_size: u32,
    request_code: u32,
    data: [u32; 32], // Variable length data
    footer: MailboxMessageFooter,
}

/// Initialize the mailbox
pub fn init() {
    // Nothing to do for now
}

/// Send a message to the mailbox
pub fn send_message(channel: u32, data: u32) -> Result<u32, MailboxError> {
    // Wait for the mailbox to be ready (bounded — return Timeout if no mailbox)
    let mut spin = 0u32;
    const SPIN_LIMIT: u32 = 1_000_000;
    while unsafe { core::ptr::read_volatile(MAILBOX_STATUS as *const u32) } & MAILBOX_STATUS_FULL
        != 0
    {
        crate::arch::cpu::nop();
        spin += 1;
        if spin >= SPIN_LIMIT {
            return Err(MailboxError::Timeout);
        }
    }

    // Send the message
    let msg = (data & !0xF) | (channel & 0xF);
    unsafe { core::ptr::write_volatile(MAILBOX_WRITE as *mut u32, msg) };

    // Wait for a response (bounded — return Timeout if no mailbox replies)
    spin = 0;
    loop {
        // Wait for the mailbox to have data
        while unsafe { core::ptr::read_volatile(MAILBOX_STATUS as *const u32) }
            & MAILBOX_STATUS_EMPTY
            != 0
        {
            crate::arch::cpu::nop();
            spin += 1;
            if spin >= SPIN_LIMIT {
                return Err(MailboxError::Timeout);
            }
        }

        // Read the message
        let msg = unsafe { core::ptr::read_volatile(MAILBOX_READ as *const u32) };

        // Check if it's for our channel
        if (msg & 0xF) == channel {
            let data = msg & !0xF;
            return Ok(data);
        }
    }
}

/// Mailbox error types
#[derive(Debug)]
pub enum MailboxError {
    Timeout,
    InvalidChannel,
    InvalidData,
}

/// Send a property message to the mailbox
/// Returns true if successful, false otherwise
pub fn property_call(buf: &mut [u32]) -> bool {
    // Build the property message
    // buf[0] = size
    // buf[1] = code (0 = request)
    // buf[2..] = tags...
    // Last tag must be 0

    // Calculate size
    let mut size = 0u32;
    let mut i = 0;
    while i < buf.len() && buf[i] != 0 {
        size += 4;
        i += 1;
        if i >= buf.len() {
            break;
        }
        // Skip value buffer
        let val_size = buf[i] as usize;
        size += 4 + (val_size as u32);
        i += 1 + val_size;
    }
    // Add end tag
    size += 4;

    buf[0] = size;
    buf[1] = 0; // request

    // Send to property channel
    let msg_ptr = buf.as_ptr() as u32;
    match send_message(MAILBOX_CHANNEL_PROPERTY, msg_ptr) {
        Ok(response) => {
            // Check response code
            if response & 0xFFFF == 0 {
                return false;
            }
            // Copy response back to buffer
            // In a real implementation, we'd read the response
            true
        }
        Err(_) => false,
    }
}

/// Get the firmware revision
pub fn get_firmware_revision() -> Result<u32, MailboxError> {
    // This is a simplified implementation
    // In a real implementation, we would send a property message to get the firmware revision
    Ok(0)
}

/// Get the board model
pub fn get_board_model() -> Result<u32, MailboxError> {
    // This is a simplified implementation
    // In a real implementation, we would send a property message to get the board model
    Ok(0)
}

/// Get the board revision
pub fn get_board_revision() -> Result<u32, MailboxError> {
    // This is a simplified implementation
    // In a real implementation, we would send a property message to get the board revision
    Ok(0)
}

/// Get the ARM memory
pub fn get_arm_memory() -> Result<(u32, u32), MailboxError> {
    // This is a simplified implementation
    // In a real implementation, we would send a property message to get the ARM memory
    Ok((0, 0))
}
