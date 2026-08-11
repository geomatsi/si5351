//! The configurations listed in `notes/HW_CHECK.md`, driven against a real
//! part.
//!
//! ```text
//! cargo run --bin hw_checks
//! ```
//!
//! Runs every check in turn, holding each configuration on the pin for
//! [`DWELL_MS`] so it can be measured, and saying over RTT what to look for.
//! Set [`ONLY`] to one check's name to stay on it instead — that is the usual
//! way to work through a single item.
//!
//! Some of it the board checks for itself: anything phrased as a device status
//! read is done here and printed. The rest needs a scope, a counter, or a
//! second output to compare against.

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_rtt_target as _;
use rtt_target::{rprintln, rtt_init_print};

use si5351::{ClockOutput, DriveStrength, Error, Frequency, Multisynth, OutputDivider, Si5351, PLL};
use si5351_blue_pill::{self as board, Board};

/// Run one check instead of all of them, by name, e.g. `Some("r-divider")`.
const ONLY: Option<&str> = None;

/// How long each configuration stays on the pin.
const DWELL_MS: u32 = 10_000;

struct Check {
    name: &'static str,
    what: &'static str,
    run: fn(&mut Board) -> Result<(), Error>,
}

const CHECKS: &[Check] = &[
    Check { name: "r-divider", what: "sub-1 MHz outputs, whose R divider bits have never been scoped", run: r_divider },
    Check { name: "fixed-pll", what: "retuning with no PLL reset, which no document states is allowed", run: fixed_pll },
    Check { name: "shared-pll", what: "what set_frequency does to an output already on the same PLL", run: shared_pll },
    Check { name: "pll-isolation", what: "resetting PLL A while an output runs off PLL B", run: pll_isolation },
    Check { name: "accuracy", what: "the four WSPR tones, 1.46484375 Hz apart", run: accuracy },
    Check { name: "drive", what: "the four output drive strengths", run: drive },
    Check { name: "ms-int", what: "jitter on an even integer divider with and without MS_INT", run: ms_int },
    Check { name: "over-150mhz", what: "a target above 150 MHz, which asks the VCO to exceed 900 MHz", run: over_150mhz },
    Check { name: "fractional-low", what: "a fractional MultiSynth divider below 8", run: fractional_low },
    Check { name: "ms-over-900", what: "a MultiSynth divider above 900", run: ms_over_900 },
    Check { name: "limits", what: "the lowest and highest frequency the driver accepts", run: limits },
];

// -- checks ------------------------------------------------------------------

/// Below 1 MHz the R divider does the coarse division. Those bits used to be
/// written into `MSx_P1[17:16]` instead of `Rx_DIV`, so every frequency here is
/// one this driver has never produced correctly.
fn r_divider(board: &mut Board) -> Result<(), Error> {
    for (hz, r) in [(500_000u32, 2u32), (100_000, 16), (32_768, 32), (20_000, 64)] {
        step(board, "clk0");
        rprintln!("  set_frequency({} Hz), R = {}", hz, r);
        board.clock.set_frequency(PLL::A, ClockOutput::Clk0, Frequency::from_hz(hz))?;
        expect("measures the target, not the R-times-faster value");
        rprintln!("    wrong would be {} Hz", hz * r);
        hold(board);
    }

    Ok(())
}

/// The whole reason `set_clock_frequency_fixed_pll` exists.
fn fixed_pll(board: &mut Board) -> Result<(), Error> {
    let dial = Frequency::from_hz(14_097_100);

    step(board, "park the PLL once, then four 1.46 Hz steps with no reset");
    board.clock.set_pll_frequency(PLL::A, Frequency::from_hz(62 * 14_097_100))?;
    board.clock.reset_pll(PLL::A)?;

    for tone in 0..4u64 {
        let freq = dial + Frequency::from_ratio(12_000 * tone, 8_192);
        rprintln!("  tone {} at {} uHz", tone, freq.as_microhz());
        board.clock.set_clock_frequency_fixed_pll(ClockOutput::Clk0, freq)?;
        expect("the output moved, and did not drop out at the transition");
        hold(board);
    }

    step(board, "a large jump on the same parked PLL, to 12.5 MHz");
    board.clock.set_clock_frequency_fixed_pll(ClockOutput::Clk0, Frequency::from_hz(12_500_000))?;
    expect("settles at 12.5 MHz without a reset, same as the small steps");
    expect("if large changes need a reset but small ones do not, this is where it shows");
    hold(board);

    Ok(())
}

/// `set_frequency` re-parks the PLL on every call, so a second output on that
/// PLL keeps its old divider against a new VCO.
fn shared_pll(board: &mut Board) -> Result<(), Error> {
    step(board, "clk0 = 10 MHz on PLL A, then clk1 = 12 MHz on the same PLL");
    board.clock.set_frequency(PLL::A, ClockOutput::Clk0, Frequency::from_hz(10_000_000))?;
    board.clock.set_frequency(PLL::A, ClockOutput::Clk1, Frequency::from_hz(12_000_000))?;
    expect("clk1 at 12 MHz, but clk0 dragged to 888/90 = 9.8667 MHz");
    expect("if clk0 still reads 10 MHz, this driver's model of the part is wrong");
    hold(board);

    step(board, "the intended flow: one PLL setup, then per-output multisynths");
    board.clock.set_pll_frequency(PLL::A, Frequency::from_hz(900_000_000))?;
    board.clock.reset_pll(PLL::A)?;
    board.clock.set_clock_frequency_fixed_pll(ClockOutput::Clk0, Frequency::from_hz(10_000_000))?;
    board.clock.set_clock_frequency_fixed_pll(ClockOutput::Clk1, Frequency::from_hz(12_000_000))?;
    expect("clk0 at 10 MHz and clk1 at 12 MHz together, off one 900 MHz VCO");
    hold(board);

    Ok(())
}

/// Register 177 has one bit per PLL and the driver writes only one.
fn pll_isolation(board: &mut Board) -> Result<(), Error> {
    step(board, "clk0 = 10 MHz on PLL A, clk1 = 12 MHz on PLL B");
    board.clock.set_frequency(PLL::A, ClockOutput::Clk0, Frequency::from_hz(10_000_000))?;
    board.clock.set_frequency(PLL::B, ClockOutput::Clk1, Frequency::from_hz(12_000_000))?;
    expect("both correct at once — scope clk1 and trigger on it");
    hold(board);

    step(board, "retune clk0, resetting PLL A only");
    board.clock.set_frequency(PLL::A, ClockOutput::Clk0, Frequency::from_hz(14_000_000))?;
    expect("clk1 unchanged in frequency and phase");
    hold(board);

    Ok(())
}

/// The rational approximation is the reason this fork exists. Needs a
/// disciplined reference to mean anything.
fn accuracy(board: &mut Board) -> Result<(), Error> {
    let dial = Frequency::from_hz(14_097_100);

    step(board, "14_097_100 Hz on clk0");
    board.clock.set_frequency(PLL::A, ClockOutput::Clk0, dial)?;
    expect("absolute error is the crystal's, tens of ppm — record it for calibration");
    hold(board);

    step(board, "the four tones through the fixed-PLL path");
    board.clock.set_pll_frequency(PLL::A, Frequency::from_hz(62 * 14_097_100))?;
    board.clock.reset_pll(PLL::A)?;
    for tone in 0..4u64 {
        let freq = dial + Frequency::from_ratio(12_000 * tone, 8_192);
        rprintln!("  tone {} at {} uHz", tone, freq.as_microhz());
        board.clock.set_clock_frequency_fixed_pll(ClockOutput::Clk0, freq)?;
        expect("spacing from the previous tone is 1.46484375 Hz to within ~10 mHz");
        hold(board);
    }

    Ok(())
}

/// New API, never measured. AN619 gives 2, 4, 6 and 8 mA for `CLKx_IDRV`.
fn drive(board: &mut Board) -> Result<(), Error> {
    for (strength, ma) in [
        (DriveStrength::_2mA, 2),
        (DriveStrength::_4mA, 4),
        (DriveStrength::_6mA, 6),
        (DriveStrength::_8mA, 8),
    ] {
        step(board, "clk0 at 10 MHz");
        rprintln!("  drive {} mA", ma);
        board.clock.set_clock_drive(ClockOutput::Clk0, strength);
        board.clock.set_frequency(PLL::A, ClockOutput::Clk0, Frequency::from_hz(10_000_000))?;
        expect("amplitude into 50 ohm rises with each step; rise and fall times change too");
        hold(board);
    }

    step(board, "clk0 at 2 mA and clk1 at 8 mA together");
    board.clock.set_clock_drive(ClockOutput::Clk0, DriveStrength::_2mA);
    board.clock.set_clock_drive(ClockOutput::Clk1, DriveStrength::_8mA);
    board.clock.set_frequency(PLL::A, ClockOutput::Clk0, Frequency::from_hz(10_000_000))?;
    board.clock.set_frequency(PLL::A, ClockOutput::Clk1, Frequency::from_hz(10_000_000))?;
    expect("the two differ in amplitude — the setting is per output");
    hold(board);

    Ok(())
}

/// `ms_int_mode_mask` is indexed with a bit number instead of a bit, so clk0
/// never gets `MS_INT` by the intended route. Whether that matters is a
/// measurement, and both states are reachable.
fn ms_int(board: &mut Board) -> Result<(), Error> {
    step(board, "10 MHz on clk0: MS0 = 90, an even integer — MS_INT clear");
    board.clock.set_frequency(PLL::A, ClockOutput::Clk0, Frequency::from_hz(10_000_000))?;
    expect("reg 16 holds 0x0f. Measure close-in phase noise and period jitter");
    hold(board);

    step(board, "the same 10 MHz, now with MS_INT set");
    // There is no API for this bit — it is driven by the buggy mask. Writing an
    // integer divider to MS1 sets mask bit 0, which is clk0's, so flushing
    // clk0's control register now sends MS_INT with it. Exploiting the bug is
    // the only way to reach the state through this driver.
    board.clock.setup_multisynth_int(Multisynth::MS1, 90, OutputDivider::Div1)?;
    board.clock.flush_clock_control(ClockOutput::Clk0)?;
    expect("reg 16 holds 0x4f, MS0 still 90. Measure again and compare");
    expect("AN619 3.2.1 and 4.1.2.1 claim integer mode improves jitter here");
    hold(board);

    step(board, "MS_INT on an odd divider, which AN619 never sanctions");
    board.clock.set_pll_frequency(PLL::A, Frequency::from_hz(900_000_000))?;
    board.clock.reset_pll(PLL::A)?;
    board.clock.set_clock_frequency_fixed_pll(ClockOutput::Clk0, Frequency::from_hz(10_000_000))?;
    board.clock.set_clock_frequency_fixed_pll(ClockOutput::Clk1, Frequency::from_hz(12_000_000))?;
    expect("clk1 runs MS1 = 75, odd, with MS_INT set — reachable today");
    expect("check clk1 is still 12 MHz and not visibly degraded");
    hold(board);

    Ok(())
}

/// AN619 4.1.3 wants divide-by-4 mode above 150 MHz. This driver does not
/// implement it and clamps the MultiSynth divider at 6 instead.
fn over_150mhz(board: &mut Board) -> Result<(), Error> {
    step(board, "150 MHz on clk0 — MS0 = 6, VCO at 900 MHz, the top of what is in spec");
    board.clock.set_frequency(PLL::A, ClockOutput::Clk0, Frequency::from_hz(150_000_000))?;
    status(board)?;
    expect("clean 150 MHz, no loss of lock");
    hold(board);

    step(board, "160 MHz on clk0 — the VCO is asked for 960 MHz");
    board.clock.set_frequency(PLL::A, ClockOutput::Clk0, Frequency::from_hz(160_000_000))?;
    status(board)?;
    expect("LOL_A above is expected to be set; record what the output does");
    hold(board);

    Ok(())
}

/// AN619 2.1.1 allows only 4, 6, 8 and fractions above 8; 4.1.2 says "between
/// 6 and 1800" with no such carve-out. `set_clock_frequency_fixed_pll` refuses
/// this, so it is built by hand.
fn fractional_low(board: &mut Board) -> Result<(), Error> {
    step(board, "PLL A at 900 MHz, MS0 = 7 + 1/3 — about 122.727 MHz");
    board.clock.set_pll_frequency(PLL::A, Frequency::from_hz(900_000_000))?;
    hand_built(board, 7, 1, 3)?;
    status(board)?;
    expect("clean 122.727 MHz means 2.1.1's floor of 8 is advisory");
    expect("wrong or absent means the guard in set_clock_frequency_fixed_pll earns its keep");
    hold(board);

    step(board, "for contrast, MS0 = 8 + 1/3 — about 108 MHz, legal either way");
    hand_built(board, 8, 1, 3)?;
    expect("108 MHz, clean");
    hold(board);

    Ok(())
}

/// The other half of the same contradiction: 4.1.2 allows up to 1800, 2.1.1
/// stops at 900.
fn ms_over_900(board: &mut Board) -> Result<(), Error> {
    step(board, "PLL A at 900 MHz, MS0 = 1200 — 750 kHz, R = 1");
    board.clock.set_pll_frequency(PLL::A, Frequency::from_hz(900_000_000))?;
    hand_built(board, 1200, 0, 1)?;
    expect("clean 750 kHz means 4.1.2's ceiling of 1800 is the real one");
    expect("wrong means setup_multisynth's range check should come down to 900");
    hold(board);

    Ok(())
}

/// Where the driver stops accepting frequencies. The floor is not a chip limit
/// — it is `find_int_dividers_for_max_pll_freq` keeping the total divider
/// inside a `u16`.
fn limits(board: &mut Board) -> Result<(), Error> {
    rprintln!("\n  what the driver accepts:");
    for hz in [13_732u32, 13_733, 20_000, 150_000_000, 160_000_000, 200_000_000] {
        let accepted = board
            .clock
            .set_frequency(PLL::A, ClockOutput::Clk0, Frequency::from_hz(hz))
            .is_ok();
        rprintln!("    {:>11} Hz  {}", hz, if accepted { "accepted" } else { "rejected" });
    }

    step(board, "the floor, 13_733 Hz, left on the pin");
    board.clock.set_frequency(PLL::A, ClockOutput::Clk0, Frequency::from_hz(13_733))?;
    expect("13_733 Hz, not something aliased. The chip itself goes to 8 kHz");
    hold(board);

    Ok(())
}

// -- plumbing ----------------------------------------------------------------

/// Programs clk0 from a hand-built MultiSynth ratio, bypassing the frequency
/// planning in `set_frequency`. For the ratios AN619 disagrees with itself
/// about, which the safe path will not produce.
fn hand_built(board: &mut Board, div: u16, num: u32, denom: u32) -> Result<(), Error> {
    board.clock.setup_multisynth(Multisynth::MS0, div, num, denom, OutputDivider::Div1)?;
    board.clock.select_clock_pll(ClockOutput::Clk0, PLL::A);
    board.clock.set_clock_enabled(ClockOutput::Clk0, true);
    board.clock.flush_clock_control(ClockOutput::Clk0)?;
    board.clock.reset_pll(PLL::A)?;
    board.clock.flush_output_enabled()
}

/// Reads and prints the device status, which is the part of a check the board
/// can settle by itself.
fn status(board: &mut Board) -> Result<(), Error> {
    let status = board.clock.read_device_status()?;
    rprintln!("    device status: {:?}", status);

    Ok(())
}

fn step(board: &mut Board, text: &str) {
    // A short dark gap marks the boundary between steps on the LED.
    board.led.set_high();
    board.wait_ms(400);
    rprintln!("\n  {}", text);
}

fn expect(text: &str) {
    rprintln!("    expect: {}", text);
}

fn hold(board: &mut Board) {
    board.hold(2, DWELL_MS);
}

#[entry]
fn main() -> ! {
    rtt_init_print!();
    rprintln!("si5351 hardware checks\n");

    let mut board = board::init();

    loop {
        for check in CHECKS {
            if let Some(only) = ONLY {
                if only != check.name {
                    continue;
                }
            }

            rprintln!("\n=== {} ===\n{}", check.name, check.what);

            if let Err(error) = (check.run)(&mut board) {
                rprintln!("  FAILED: {:?}", error);
            }

            // Leave the part in a known state before the next check.
            reset_state(&mut board);
        }

        rprintln!("\n--- all checks done, starting over ---");
        board.wait_ms(5_000);
    }
}

/// Turns every output off between checks, so nothing from one is still on a
/// pin during the next.
fn reset_state(board: &mut Board) {
    for clk in [ClockOutput::Clk0, ClockOutput::Clk1, ClockOutput::Clk2] {
        board.clock.set_clock_enabled(clk, false);
        let _ = board.clock.flush_clock_control(clk);
    }
    let _ = board.clock.flush_output_enabled();
}
