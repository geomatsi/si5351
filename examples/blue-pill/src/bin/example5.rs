//! Fine steps around 5 MHz: what the fractional divider is actually for.
//!
//! ```text
//! cargo run --bin example5
//! ```
//!
//! clk0 is stepped by one hertz and less, through
//! [`Si5351::set_clock_frequency_fixed_pll`] so the PLL is never rewritten and
//! the output runs continuously across every step. clk1 sits at exactly 5 MHz
//! as a reference.
//!
//! # Measuring it
//!
//! Two channels, trigger on clk1, watch clk0 slide. The offset *is* the slip
//! rate: at +1 Hz clk0 gains a whole cycle every second, at +100 mHz one every
//! ten seconds, and at −1 Hz it slides the other way. That reads offsets a
//! counter would need an enormous gate time to see, and it needs no reference
//! better than the board itself: both outputs come off the same 25 MHz crystal,
//! so the crystal's error — tens of ppm, thousands of times bigger than these
//! steps — is common to both and cancels in the comparison. What is left is
//! exactly the programmed difference.
//!
//! # Why the PLL is parked at 888 MHz
//!
//! The output divider is `a + b/c` with `c` at most 2^20-1, so the reachable
//! frequencies near the target are not evenly spread: they crowd together
//! around fractions with a large `c` and thin out sharply around simple ones,
//! and the gap beside `b/c = 1` is the widest of all.
//!
//! Park at 900 MHz and 5 MHz needs a divider of exactly 180 — sitting right
//! against that widest gap, which is about 26 mHz wide here. A step up of less
//! than that has `1/1` as its closest fraction, an improper one that
//! [`Si5351::set_clock_frequency_fixed_pll`] refuses outright, so the call
//! comes back [`Error::InvalidParameter`]; the last step below does exactly
//! that and prints it. A step *down* of less than that is worse: `0/1` is a
//! perfectly legal fraction, so the driver takes it, writes the same eight
//! bytes it would for 5 MHz, and returns `Ok`.
//!
//! Park at 888 MHz instead and the divider is 177 + 3/5, which leaves room on
//! both sides; every step here lands within a microhertz of its target. It is
//! not perfect either — 3/5 is itself a simple fraction, so a 1 mHz step is
//! absorbed into it the same silent way. The rule of thumb is to park the PLL
//! somewhere that does *not* divide evenly into the output.
//!
//! [`Error::InvalidParameter`]: si5351::Error::InvalidParameter

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_rtt_target as _;
use rtt_target::{rprintln, rtt_init_print};

use si5351::{ClockOutput, Frequency, Si5351, PLL};
use si5351_blue_pill::{self as board, Board};

/// The output being stepped, on PLL A.
const STEPPED: ClockOutput = ClockOutput::Clk0;

/// The reference it is compared against, on PLL B.
const REFERENCE: ClockOutput = ClockOutput::Clk1;

const BASE: Frequency = Frequency::from_hz(5_000_000);

/// 888 MHz is 177.6 times the target, so the output divider carries a real
/// fraction. See the note above on what happens at 900 MHz.
const PARKED: Frequency = Frequency::from_hz(888_000_000);

/// Offsets from [`BASE`] in microhertz, with how long to hold each. The dwell
/// is a few slips' worth: below about 10 mHz a full cycle takes so long that
/// only the drift is visible.
const STEPS: [(i64, u32); 6] = [
    (0, 10_000),
    (1_000_000, 10_000),
    (-1_000_000, 10_000),
    // A third of a hertz: exact as a ratio, and not expressible as any whole
    // number of hertz or millihertz.
    (Frequency::from_ratio(1, 3).as_microhz() as i64, 15_000),
    (100_000, 30_000),
    (10_000, 60_000),
];

#[entry]
fn main() -> ! {
    rtt_init_print!();
    rprintln!("si5351 example5: fine steps\n");

    let mut board = board::init();

    // The reference: exactly 5 MHz, off PLL B, tuned once and left alone. PLL B
    // lands on 900 MHz with a divider of 180, both integers, so clk1 is as
    // exact as the crystal it comes from.
    board
        .clock
        .set_frequency(PLL::B, REFERENCE, BASE)
        .unwrap();
    rprintln!("clk1 = {} Hz on PLL B, the reference", BASE.as_hz());

    loop {
        board.clock.select_clock_pll(STEPPED, PLL::A);
        board.clock.set_pll_frequency(PLL::A, PARKED).unwrap();
        board.clock.reset_pll(PLL::A).unwrap();
        rprintln!("\nPLL A parked at {} Hz", PARKED.as_hz());

        for (index, &(offset, dwell)) in STEPS.iter().enumerate() {
            let target = offset_from_base(offset);

            rprintln!("\nclk0 = {} uHz", target.as_microhz());
            announce(offset);

            // No PLL write and no reset in here: only clk0's output MultiSynth
            // changes, so the carrier keeps its phase across the step and the
            // slip against clk1 stays meaningful.
            board
                .clock
                .set_clock_frequency_fixed_pll(STEPPED, target)
                .unwrap();

            board.hold(index as u8 + 1, dwell);
        }

        too_fine(&mut board);

        rprintln!("\n--- round done, starting over ---");
    }
}

/// [`BASE`] shifted by `offset` microhertz, either way.
fn offset_from_base(offset: i64) -> Frequency {
    if offset < 0 {
        BASE - Frequency::from_microhz(offset.unsigned_abs())
    } else {
        BASE + Frequency::from_microhz(offset as u64)
    }
}

/// Says what the offset should look like against clk1.
fn announce(offset: i64) {
    if offset == 0 {
        rprintln!("  no offset: clk0 and clk1 hold their relative position");
        return;
    }

    // One cycle of slip per 1/offset seconds, and 1 Hz is 1_000_000 uHz.
    let seconds = 1_000_000 / offset.unsigned_abs();
    let direction = if offset > 0 { "forwards" } else { "backwards" };

    rprintln!("  one cycle of slip every {} s, {}", seconds, direction);
}

/// The same 10 mHz step from a 900 MHz park, where the driver cannot express
/// it. Nothing to measure — this one is answered by the return value.
fn too_fine(board: &mut Board) {
    let target = offset_from_base(10_000);

    rprintln!("\n10 mHz from a 900 MHz park, where the divider would be 180:");
    board
        .clock
        .set_pll_frequency(PLL::A, Frequency::from_hz(900_000_000))
        .unwrap();
    board.clock.reset_pll(PLL::A).unwrap();

    match board.clock.set_clock_frequency_fixed_pll(STEPPED, target) {
        Ok(()) => rprintln!("  accepted — the fraction fitted after all"),
        Err(error) => rprintln!("  {:?}, as expected: no fraction below 1/1 is closer", error),
    }

    // clk0 keeps the divider the 888 MHz park gave it, against a VCO that has
    // just moved to 900 MHz, so it is off target until the loop re-parks —
    // `example4`'s lesson, arrived at from the other direction.
    board.wait_ms(2_000);
}
