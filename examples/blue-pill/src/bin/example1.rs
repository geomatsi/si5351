//! `set_frequency`: the one-call way to put a frequency on a pin.
//!
//! ```text
//! cargo run --bin example1
//! ```
//!
//! Cycles clk0 through four frequencies that any counter or scope reads back at
//! a glance, holding each for [`DWELL_MS`]. The LED blinks the step number
//! before each hold, so the board can be followed without the RTT terminal.
//!
//! [`Si5351::set_frequency`] plans the whole chain itself: it picks an output
//! MultiSynth divider, solves the PLL to suit, programs both, and resets the
//! PLL. The reset restarts the divider chain, so the output stops and comes
//! back with a new phase on every call — fine for setting a frequency and
//! leaving it there. To move frequency while the output keeps running, see
//! `example2`.

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_rtt_target as _;
use rtt_target::{rprintln, rtt_init_print};

use si5351::{ClockOutput, Frequency, Si5351, PLL};
use si5351_blue_pill as board;

const OUTPUT: ClockOutput = ClockOutput::Clk0;

/// 500 kHz needs the R divider; the other three come straight off the output
/// MultiSynth.
const FREQUENCIES: [u32; 4] = [500_000, 5_000_000, 15_000_000, 50_000_000];

/// How long each frequency stays on the pin.
const DWELL_MS: u32 = 5_000;

#[entry]
fn main() -> ! {
    rtt_init_print!();
    rprintln!("si5351 example1: set_frequency\n");

    let mut board = board::init();
    rprintln!("Si5351 answered on I2C1, PB8/PB9");

    loop {
        for (step, &hz) in FREQUENCIES.iter().enumerate() {
            let freq = Frequency::from_hz(hz);

            // Not needed to set a frequency — this is the same plan
            // set_frequency is about to make, printed so the log shows how the
            // target is reached. 900 MHz is the top of the VCO range.
            let (ms_div, r_div) = board
                .clock
                .find_int_dividers_for_max_pll_freq(900_000_000, freq)
                .unwrap();

            board.clock.set_frequency(PLL::A, OUTPUT, freq).unwrap();

            let vco = board.clock.pll_frequency(PLL::A).unwrap();
            rprintln!("\nclk0 = {} Hz", hz);
            rprintln!(
                "  VCO {} Hz / MultiSynth {} / R {}",
                vco.as_hz(),
                ms_div,
                r_div.denominator_u8()
            );

            board.hold(step as u8 + 1, DWELL_MS);
        }

        rprintln!("\n--- round done, starting over ---");
    }
}
