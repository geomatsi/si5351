//! Output drive strength.
//!
//! ```text
//! cargo run --bin example3
//! ```
//!
//! For each of the four frequencies, clk0 is programmed once and then stepped
//! through all four drive strengths. Terminate clk0 into 50 Ω and watch the
//! amplitude: it rises with each step, and the edges get faster with it.
//!
//! [`Si5351::set_clock_drive`] writes nothing by itself — it only records the
//! setting, like [`Si5351::set_clock_enabled`] does. The `CLKx_IDRV` bits share
//! a register with the clock's power-down, MultiSynth source and integer-mode
//! bits, so the driver keeps the whole byte in memory and sends it on
//! [`Si5351::flush_clock_control`]. Here that flush is the only write between
//! steps, which is why the output keeps running across a change in drive.
//! [`Si5351::set_frequency`] ends with the same flush, so setting the drive
//! before tuning works too, with one write fewer.

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_rtt_target as _;
use rtt_target::{rprintln, rtt_init_print};

use si5351::{ClockOutput, DriveStrength, Frequency, Si5351, PLL};
use si5351_blue_pill as board;

const OUTPUT: ClockOutput = ClockOutput::Clk0;

const FREQUENCIES: [u32; 4] = [500_000, 5_000_000, 15_000_000, 50_000_000];

/// The `CLKx_IDRV` settings of AN619 register 16, with the current each one
/// names.
const DRIVES: [(DriveStrength, u8); 4] = [
    (DriveStrength::_2mA, 2),
    (DriveStrength::_4mA, 4),
    (DriveStrength::_6mA, 6),
    (DriveStrength::_8mA, 8),
];

/// How long each combination stays on the pin.
const DWELL_MS: u32 = 5_000;

#[entry]
fn main() -> ! {
    rtt_init_print!();
    rprintln!("si5351 example3: drive strength\n");

    let mut board = board::init();

    loop {
        for &hz in FREQUENCIES.iter() {
            rprintln!("\nclk0 = {} Hz", hz);
            board
                .clock
                .set_frequency(PLL::A, OUTPUT, Frequency::from_hz(hz))
                .unwrap();

            for (step, &(drive, ma)) in DRIVES.iter().enumerate() {
                rprintln!("  drive {} mA", ma);
                board.clock.set_clock_drive(OUTPUT, drive);
                board.clock.flush_clock_control(OUTPUT).unwrap();

                board.hold(step as u8 + 1, DWELL_MS);
            }
        }

        rprintln!("\n--- round done, starting over ---");
    }
}
