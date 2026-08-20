//! The calibration loop: measure an output, correct for what the crystal is
//! doing, and carry the answer to an output that was never measured.
//!
//! ```text
//! cargo run --bin example7
//! ```
//!
//! clk1 is counted on PB3 over a one-second gate, and each reading feeds
//! [`calibrate::error_ppb`] and [`calibrate::correct`] until it settles. The
//! gate is the Blue Pill's own crystal, so what converges is one ±30 ppm part
//! onto another — the loop is the point here, not the number. Gate against a
//! GPS PPS to make the number mean something.
//!
//! Nothing to measure: it answers over RTT.

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_rtt_target as _;
use rtt_target::{rprintln, rtt_init_print};

use si5351::{ClockOutput, Frequency, PLL, Si5351, calibrate};

/// The counted output, wired to PB3.
const COUNTED: ClockOutput = ClockOutput::Clk1;
const NOMINAL: Frequency = Frequency::from_hz(10_000_000);

/// The output that is never counted, corrected from clk1's reading alone.
const OTHER: ClockOutput = ClockOutput::Clk0;
const OTHER_NOMINAL: Frequency = Frequency::from_hz(15_000_000);

const PARKED: Frequency = Frequency::from_hz(700_000_000);

/// Stop once a reading is this close; a one-second gate on 10 MHz resolves
/// 100 ppb, so there is no point chasing much below it.
const SETTLED_PPB: i64 = 300;

const STEPS: u8 = 5;

#[entry]
fn main() -> ! {
    rtt_init_print!();
    rprintln!("si5351 example7: calibration loop\n");

    let (mut board, gate) = si5351_blue_pill::init_with_gate();

    board.clock.select_clock_pll(COUNTED, PLL::A);
    board.clock.set_pll_frequency(PLL::A, PARKED).unwrap();
    board.clock.reset_pll(PLL::A).unwrap();

    loop {
        // Back to the uncorrected dial, so every round starts from scratch.
        let mut dial = NOMINAL;
        board
            .clock
            .set_clock_frequency_fixed_pll(COUNTED, dial)
            .unwrap();

        for step in 1..=STEPS {
            let ticks = gate.count(&mut board);
            let ppb = calibrate::error_ppb(ticks, NOMINAL, 1);

            rprintln!("step {}: {} ticks, {} ppb", step, ticks, ppb);

            if !calibrate::plausible(ppb) {
                rprintln!("  implausible, ignored — a gate boundary was missed");
                continue;
            }

            if ppb.abs() <= SETTLED_PPB {
                rprintln!("  within {} ppb, settled", SETTLED_PPB);
                break;
            }

            dial = calibrate::correct(dial, ppb);
            board
                .clock
                .set_clock_frequency_fixed_pll(COUNTED, dial)
                .unwrap();

            rprintln!("  dial now {} uHz", dial.as_microhz());
        }

        // What the dial ended up carrying, which is the whole error: each
        // reading above saw only what was left of it.
        let total = calibrate::correction_ppb(dial, NOMINAL);
        rprintln!("total {} ppb ({} ppm)", total, calibrate::as_ppm(total));

        // The payoff: the same correction on an output the counter never saw.
        let corrected = calibrate::correct(OTHER_NOMINAL, total);
        board
            .clock
            .set_frequency(PLL::B, OTHER, corrected)
            .unwrap();
        rprintln!(
            "clk0 asked for {} uHz to land on {} Hz\n",
            corrected.as_microhz(),
            OTHER_NOMINAL.as_hz()
        );

        board.hold(1, 5_000);
    }
}
