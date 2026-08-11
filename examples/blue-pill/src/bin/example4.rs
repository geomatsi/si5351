//! Two outputs at once, and what sharing a PLL between them costs.
//!
//! ```text
//! cargo run --bin example4
//! ```
//!
//! Scope clk0 and clk1 together. Three phases, each held for [`DWELL_MS`]:
//!
//! 1. clk0 and clk1 both tuned with [`Si5351::set_frequency`] on PLL A. Every
//!    call re-plans the PLL for its own target, so the second call moves the
//!    VCO out from under the first output: clk0 keeps the divider it was given
//!    and lands somewhere else. The log prints where.
//! 2. the same two calls with targets that happen to want the same VCO, where
//!    nothing moves. Handy to know, but it is luck, not a guarantee.
//! 3. clk1 moved to PLL B. Each output now owns a PLL, both are exact, and
//!    retuning one leaves the other alone.
//!
//! Three or more outputs run into this too — there are only two PLLs. The way
//! out is [`Si5351::set_pll_frequency`] once, then
//! [`Si5351::set_clock_frequency_fixed_pll`] per output, which never touches
//! the PLL again; `example2` does that for one output.

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_rtt_target as _;
use rtt_target::{rprintln, rtt_init_print};

use si5351::{ClockOutput, Frequency, Si5351, PLL};
use si5351_blue_pill::{self as board, Board};

/// How long each phase stays on the pins.
const DWELL_MS: u32 = 8_000;

#[entry]
fn main() -> ! {
    rtt_init_print!();
    rprintln!("si5351 example4: two outputs\n");

    let mut board = board::init();

    loop {
        // -- 1. both on PLL A, and the second call drags the first ------------
        //
        // 5 MHz asks for a 900 MHz VCO and a divider of 180. 12 MHz cannot use
        // 900 MHz — 75 is odd, and the driver only hands out even integer
        // dividers here — so it settles on 74 and re-parks PLL A at 888 MHz.
        // clk0 still divides by 180.
        let first = Frequency::from_hz(5_000_000);
        let second = Frequency::from_hz(12_000_000);

        rprintln!("\n1. clk0 = {} Hz on PLL A", first.as_hz());
        board.clock.set_frequency(PLL::A, ClockOutput::Clk0, first).unwrap();
        report(&board, first);
        board.hold(1, DWELL_MS);

        rprintln!("   clk1 = {} Hz on PLL A as well", second.as_hz());
        board.clock.set_frequency(PLL::A, ClockOutput::Clk1, second).unwrap();
        report(&board, first);
        rprintln!("   clk1 is exact: it is the target the PLL was re-planned for");
        board.hold(2, DWELL_MS);

        all_off(&mut board);

        // -- 2. both on PLL A, targets that agree on the VCO ------------------
        //
        // 15 MHz wants 900 MHz with a divider of 60, and 50 MHz wants 900 MHz
        // with a divider of 18. The second call re-parks the PLL at the
        // frequency it was already on, so clk0 comes out unharmed.
        let first = Frequency::from_hz(15_000_000);
        let second = Frequency::from_hz(50_000_000);

        rprintln!("\n2. clk0 = {} Hz and clk1 = {} Hz, both on PLL A", first.as_hz(), second.as_hz());
        board.clock.set_frequency(PLL::A, ClockOutput::Clk0, first).unwrap();
        board.clock.set_frequency(PLL::A, ClockOutput::Clk1, second).unwrap();
        report(&board, first);
        rprintln!("   both exact — the two plans wanted the same VCO");
        board.hold(3, DWELL_MS);

        all_off(&mut board);

        // -- 3. one PLL each --------------------------------------------------
        //
        // The pair from phase 1, with clk1 fed from PLL B. Register 177 has a
        // reset bit per PLL and the driver writes only the one it tuned, so
        // neither call disturbs the other output.
        let first = Frequency::from_hz(5_000_000);
        let second = Frequency::from_hz(12_000_000);

        rprintln!("\n3. clk0 = {} Hz on PLL A, clk1 = {} Hz on PLL B", first.as_hz(), second.as_hz());
        board.clock.set_frequency(PLL::A, ClockOutput::Clk0, first).unwrap();
        board.clock.set_frequency(PLL::B, ClockOutput::Clk1, second).unwrap();
        report(&board, first);
        board.hold(4, DWELL_MS);

        rprintln!("   retune clk0 to 500 kHz; clk1 should not move at all");
        board.clock.set_frequency(PLL::A, ClockOutput::Clk0, Frequency::from_hz(500_000)).unwrap();
        board.hold(5, DWELL_MS);

        all_off(&mut board);
        rprintln!("\n--- round done, starting over ---");
    }
}

/// Prints what clk0 is actually putting out: the divider `set_frequency` chose
/// for `planned`, applied to whatever PLL A is parked at now.
///
/// [`Si5351::find_int_dividers_for_max_pll_freq`] is the same planning step
/// `set_frequency` runs, and it depends only on the target, so asking it again
/// gives back the divider clk0 still holds.
fn report(board: &Board, planned: Frequency) {
    let (ms_div, r_div) = board
        .clock
        .find_int_dividers_for_max_pll_freq(900_000_000, planned)
        .unwrap();
    let total = ms_div as u64 * r_div.denominator_u8() as u64;
    let vco = board.clock.pll_frequency(PLL::A).unwrap();
    let actual = Frequency::from_microhz(vco.as_microhz() / total);

    rprintln!(
        "   PLL A at {} Hz / {} => clk0 is {} Hz",
        vco.as_hz(),
        total,
        actual.as_hz()
    );
}

/// Turns both outputs off for a moment, so the phase boundary is visible on the
/// scope.
fn all_off(board: &mut Board) {
    for clk in [ClockOutput::Clk0, ClockOutput::Clk1] {
        board.clock.set_clock_enabled(clk, false);
    }
    board.clock.flush_output_enabled().unwrap();
    board.wait_ms(1_000);
}
