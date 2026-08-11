//! `set_clock_frequency_fixed_pll`: retuning an output without touching the
//! PLL.
//!
//! ```text
//! cargo run --bin example2
//! ```
//!
//! The PLL is parked once at [`PARKED`] and reset once, before the loop. Each
//! frequency after that rewrites only clk0's output MultiSynth — eight register
//! bytes, no PLL write and no reset — so the output runs through the change
//! instead of dropping out and restarting. Same four frequencies as `example1`;
//! scope clk0 across a transition to see the difference.
//!
//! # What a parked PLL can reach
//!
//! The output is `PARKED / (MultiSynth * R)`, and the driver will only build
//! that ratio out of a MultiSynth divider of 6 to 1800 — fractional only at 8
//! and above, per AN619 2.1.1 — an R divider of 1 to 128, and a total divider
//! that fits a `u16`. From 900 MHz that is about 13.7 kHz to 150 MHz, so all
//! four frequencies here are covered; the R divider does the work below 1 MHz.
//! Outside that window the call returns [`Error::InvalidParameter`] rather than
//! programming something wrong — a target above 112.5 MHz, for instance, is
//! only reachable when 900 MHz divides into it exactly.
//!
//! [`Error::InvalidParameter`]: si5351::Error::InvalidParameter

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_rtt_target as _;
use rtt_target::{rprintln, rtt_init_print};

use si5351::{ClockOutput, Frequency, Si5351, PLL};
use si5351_blue_pill as board;

const OUTPUT: ClockOutput = ClockOutput::Clk0;

/// Where the VCO sits for the whole run: the top of its 600 to 900 MHz range,
/// which is also the choice that reaches the highest output.
const PARKED: Frequency = Frequency::from_hz(900_000_000);

const FREQUENCIES: [u32; 4] = [500_000, 5_000_000, 15_000_000, 50_000_000];

/// How long each frequency stays on the pin.
const DWELL_MS: u32 = 5_000;

#[entry]
fn main() -> ! {
    rtt_init_print!();
    rprintln!("si5351 example2: set_clock_frequency_fixed_pll\n");

    let mut board = board::init();

    // Which PLL feeds the output has to be settled before the first call: the
    // driver looks up the parked frequency of whichever PLL this output is on.
    // PLL A is the power-up default, so this line only makes it explicit.
    board.clock.select_clock_pll(OUTPUT, PLL::A);

    board.clock.set_pll_frequency(PLL::A, PARKED).unwrap();
    board.clock.reset_pll(PLL::A).unwrap();

    let vco = board.clock.pll_frequency(PLL::A).unwrap();
    rprintln!("PLL A parked at {} Hz, and not written again", vco.as_hz());

    loop {
        for (step, &hz) in FREQUENCIES.iter().enumerate() {
            rprintln!("\nclk0 = {} Hz", hz);
            board
                .clock
                .set_clock_frequency_fixed_pll(OUTPUT, Frequency::from_hz(hz))
                .unwrap();

            board.hold(step as u8 + 1, DWELL_MS);
        }

        rprintln!("\n--- round done, starting over ---");
    }
}
