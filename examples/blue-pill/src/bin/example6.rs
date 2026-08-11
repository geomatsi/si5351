//! `read_device_status`: asking the part whether it is happy.
//!
//! ```text
//! cargo run --bin example6
//! ```
//!
//! Every other example programs the device and trusts the result. This one
//! reads register 0 back after each configuration and compares it against what
//! that configuration should produce, printing `ok` or `UNEXPECTED` per flag.
//! Nothing needs to be on the pins — the whole example is answered over RTT.
//!
//! A good configuration comes first, then two that are out of spec in opposite
//! directions: a VCO below the 600 MHz floor and one above the 900 MHz ceiling,
//! both reached through calls the driver accepts without complaint. The last
//! step puts the good one back, which is the part worth watching: a flag that
//! never clears again says something different from one that follows the
//! configuration.
//!
//! # What the flags mean here
//!
//! - `SYS_INIT` — set while the device is still starting up. [`Si5351::init`]
//!   already waits for it to clear, so seeing it set after `init` returns would
//!   be a surprise.
//! - `LOL_A`, `LOL_B` — that PLL is not locked. AN619 register 0 ties this to
//!   the reference forcing the PLL outside its lock range, which is what steps
//!   2 and 3 below try to do. Only PLL A is programmed here, so `LOL_B` stays
//!   at whatever the power-up state left it and no check looks at it.
//! - `LOS` — **not** the crystal. AN619 register 0 bit 4 is CLKIN loss of
//!   signal, and only the Si5351C has a CLKIN pin, so on this A-part breakout
//!   it should simply stay clear and says nothing about the reference. The
//!   crystal's own flag is LOS_XTAL at bit 3, which the datasheet documents in
//!   §4 and AN619 Rev 0.6 leaves out — its register 0 calls bits 3:0 reserved.
//!   [`DeviceStatusBits`] follows AN619 and has no variant for it, so
//!   `read_device_status` truncates it away and a dead crystal cannot be
//!   detected from here.
//!
//! # These are expectations, not measurements
//!
//! AN619 gives the VCO range as 600 to 900 MHz, but out of spec is not the same
//! as inoperable: whether the PLL actually drops lock at 375 MHz or at 960 MHz
//! is exactly what this example is for. An `UNEXPECTED` line is a result to
//! record in `notes/HW_CHECK.md`, not a bug in the example — the same
//! configurations are checks `over-150mhz` and `limits` there.

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_rtt_target as _;
use rtt_target::{rprintln, rtt_init_print};

use si5351::{ClockOutput, DeviceStatusBits, Frequency, Si5351, PLL};
use si5351_blue_pill::{self as board, Board};

const OUTPUT: ClockOutput = ClockOutput::Clk0;

/// A configuration well inside every limit: a 900 MHz VCO divided by 180.
const GOOD: Frequency = Frequency::from_hz(5_000_000);

/// Long enough for the PLL to have locked, or given up, before register 0 is
/// read. Loss of lock is not reported instantly.
const SETTLE_MS: u32 = 500;

/// How long each step stays up, so the RTT log can be read as it goes.
const DWELL_MS: u32 = 5_000;

#[entry]
fn main() -> ! {
    rtt_init_print!();
    rprintln!("si5351 example6: device status\n");

    let mut board = board::init();

    loop {
        rprintln!("\n1. clk0 = {} Hz, VCO at 900 MHz — in spec", GOOD.as_hz());
        board.clock.set_frequency(PLL::A, OUTPUT, GOOD).unwrap();
        let status = settle(&mut board);
        check(!status.contains(DeviceStatusBits::SYS_INIT), "SYS_INIT clear: initialisation finished");
        check(!status.contains(DeviceStatusBits::LOS), "LOS clear: no CLKIN on this part to lose");
        check(!status.contains(DeviceStatusBits::LOL_A), "LOL_A clear: PLL A is locked");
        board.hold(1, DWELL_MS);

        // 15 x 25 MHz = 375 MHz. `setup_pll_int` checks the multiplier against
        // AN619 3.2's range of 15 to 90 and this passes, but the VCO range is a
        // separate constraint that no call enforces. clk0 keeps the divider of
        // 180 it was given above, so it drops to about 2.08 MHz with it.
        rprintln!("\n2. PLL A at 375 MHz, below the 600 MHz floor");
        board.clock.setup_pll_int(PLL::A, 15).unwrap();
        board.clock.reset_pll(PLL::A).unwrap();
        let status = settle(&mut board);
        check(status.contains(DeviceStatusBits::LOL_A), "LOL_A set: PLL A cannot lock that low");
        board.hold(2, DWELL_MS);

        // 900 / 160 is 5.625, and the driver clamps the output MultiSynth
        // divider at 6 rather than implementing AN619 4.1.3's divide-by-4 mode,
        // so it solves the PLL for 160 MHz x 6 = 960 MHz and asks for it.
        rprintln!("\n3. clk0 = 160 MHz, which asks the VCO for 960 MHz");
        board.clock.set_frequency(PLL::A, OUTPUT, Frequency::from_hz(160_000_000)).unwrap();
        let status = settle(&mut board);
        check(status.contains(DeviceStatusBits::LOL_A), "LOL_A set: PLL A cannot lock that high");
        board.hold(3, DWELL_MS);

        rprintln!("\n4. back to {} Hz", GOOD.as_hz());
        board.clock.set_frequency(PLL::A, OUTPUT, GOOD).unwrap();
        let status = settle(&mut board);
        check(!status.contains(DeviceStatusBits::LOL_A), "LOL_A clear again: the flag follows the configuration");
        board.hold(4, DWELL_MS);

        rprintln!("\n--- round done, starting over ---");
    }
}

/// Waits for the PLL to settle, then reads and prints register 0.
fn settle(board: &mut Board) -> DeviceStatusBits {
    board.wait_ms(SETTLE_MS);

    let status = board.clock.read_device_status().unwrap();
    rprintln!("   status: {:?}", status);

    status
}

/// One line per flag, saying whether the part agreed with the configuration.
fn check(held: bool, what: &str) {
    if held {
        rprintln!("     ok: {}", what);
    } else {
        rprintln!("     UNEXPECTED: {}", what);
    }
}
