//! Unit tests for the divider math and the register bytes it turns into.
//!
//! A child module of the crate root rather than an integration test under
//! `tests/`, because `approx_fraction` and the register-level constants it is
//! checked against are private.

// The crate is `no_std`, but the test harness is not.
extern crate std;

use std::vec::Vec;

use embedded_hal::i2c::Operation;

use super::*;

/// The divider math is pure, but it hangs off the trait, which needs a bus.
/// This one keeps the last byte written to each register, so tests that care
/// about what actually reaches the chip can look.
#[derive(Default)]
struct FakeI2c {
    writes: Vec<(u8, u8)>,
}

impl embedded_hal::i2c::ErrorType for FakeI2c {
    type Error = core::convert::Infallible;
}

impl I2c for FakeI2c {
    fn transaction(
        &mut self,
        _address: u8,
        operations: &mut [Operation<'_>],
    ) -> Result<(), Self::Error> {
        for operation in operations {
            match operation {
                Operation::Write([register, payload @ ..]) => {
                    self.writes.extend(
                        payload
                            .iter()
                            .enumerate()
                            .map(|(offset, byte)| (register + offset as u8, *byte)),
                    );
                }
                Operation::Write([]) => {}
                Operation::Read(buffer) => buffer.fill(0),
            }
        }

        Ok(())
    }
}

impl FakeI2c {
    /// The last byte written to `register`.
    fn last(&self, register: u8) -> Option<u8> {
        self.writes
            .iter()
            .rev()
            .find(|(reg, _)| *reg == register)
            .map(|(_, byte)| *byte)
    }
}

const XTAL: u32 = 25_000_000;

fn device() -> Si5351Device<FakeI2c> {
    Si5351Device::new(FakeI2c::default(), false, XTAL)
}

/// Exact output frequency of `fXTAL * (a + b/c) / total_div`, in microhertz.
fn output_microhz(mult: u8, num: u32, denom: u32, total_div: u32) -> u128 {
    let xtal = Frequency::from_hz(XTAL).as_microhz() as u128;
    let pll = xtal * (mult as u128 * denom as u128 + num as u128);

    pll / (denom as u128 * total_div as u128)
}

/// How far `p / q` sits from `num / den`, as a numerator over the common
/// denominator `q * den`. Only meaningful when compared against another
/// distance scaled back by the same `q`, as [`closer`] does.
fn distance(num: u64, den: u64, p: u32, q: u32) -> u128 {
    (p as u128 * den as u128).abs_diff(num as u128 * q as u128)
}

/// Is `a` a strictly better approximation of `num / den` than `b`?
fn closer(num: u64, den: u64, a: (u32, u32), b: (u32, u32)) -> bool {
    distance(num, den, a.0, a.1) * (b.1 as u128) < distance(num, den, b.0, b.1) * (a.1 as u128)
}

/// Slow but obviously correct: the closest fraction found by trying every
/// denominator up to `max_den`.
fn brute_force(num: u64, den: u64, max_den: u32) -> (u32, u32) {
    let mut best = (0u32, 1u32);

    for q in 1..=max_den {
        // Nearest numerator for this denominator.
        let p = ((num as u128 * q as u128 * 2 + den as u128) / (den as u128 * 2)) as u32;
        if closer(num, den, (p, q), best) {
            best = (p, q);
        }
    }

    best
}

/// What the fixed denominator this driver used to hardcode would give.
fn truncated(num: u64, den: u64, max_den: u32) -> (u32, u32) {
    (
        (num as u128 * max_den as u128 / den as u128) as u32,
        max_den,
    )
}

#[test]
fn frequency_constructors() {
    assert_eq!(
        Frequency::from_hz(14_097_100).as_microhz(),
        14_097_100_000_000
    );
    assert_eq!(Frequency::from_millihz(1_464).as_microhz(), 1_464_000);
    assert_eq!(Frequency::from_microhz(1_464_844).as_hz(), 1);

    // 1.46484375 Hz, rounded up to the microhertz.
    assert_eq!(Frequency::from_ratio(12_000, 8_192).as_microhz(), 1_464_844);
    // Whole numbers of hertz stay exact whichever way they are spelled.
    assert_eq!(Frequency::from_ratio(1_000, 8), Frequency::from_hz(125));
    // The integer part must not be lost for ratios far above 1 Hz.
    assert_eq!(
        Frequency::from_ratio(28_194_200, 2),
        Frequency::from_hz(14_097_100)
    );
}

#[test]
fn approx_fraction_exact_when_representable() {
    assert_eq!(approx_fraction(0, 1, MAX_FRACTION_DENOM), (0, 1));
    assert_eq!(approx_fraction(0, 25_000_000, MAX_FRACTION_DENOM), (0, 1));
    assert_eq!(approx_fraction(1, 2, MAX_FRACTION_DENOM), (1, 2));
    assert_eq!(approx_fraction(3, 4, 4), (3, 4));
    assert_eq!(
        approx_fraction(500_000, 1_000_000, MAX_FRACTION_DENOM),
        (1, 2)
    );
    // Denominator already at the limit, nothing to approximate.
    assert_eq!(
        approx_fraction(7, MAX_FRACTION_DENOM as u64, MAX_FRACTION_DENOM),
        (7, MAX_FRACTION_DENOM)
    );
}

#[test]
fn approx_fraction_respects_max_den() {
    for max_den in [1u32, 2, 3, 10, 999, 1024, MAX_FRACTION_DENOM] {
        for num in [1u64, 7, 12_345, 999_983, 24_999_999] {
            let (p, q) = approx_fraction(num, 25_000_000, max_den);
            assert!(q >= 1 && q <= max_den, "{num}/{max_den} -> {p}/{q}");
            assert!(p <= q, "{num}/{max_den} -> {p}/{q}");
        }
    }
}

/// The fractions the registers can hold are not spread evenly. With
/// denominators capped at `max_den`, the nearest neighbours of `0/1` and `1/1`
/// are `1/max_den` and `(max_den - 1)/max_den`, so the gaps beside those two
/// are `1/max_den` wide — the widest anywhere, and everything else is finer.
/// A value that falls inside one of them can only be answered with the endpoint
/// itself.
///
/// That is not academic: `1/1` is improper, so `write_ms_config` refuses it and
/// the caller gets an error, while `0/1` is legal and goes through as an
/// integer divider. `fixed_pll_step_resolution_depends_on_the_park` is the same
/// thing seen from the API.
#[test]
fn approx_fraction_snaps_to_the_endpoints() {
    // Quarter-steps of the gap width, so a numerator of 1 or 3 is inside it or
    // outside it respectively.
    let den = 4 * MAX_FRACTION_DENOM as u64;
    let max = MAX_FRACTION_DENOM;

    // Inside the gaps, where the endpoints are the closest there is.
    assert_eq!(approx_fraction(den - 1, den, max), (1, 1));
    assert_eq!(approx_fraction(1, den, max), (0, 1));

    // Just outside them, where the neighbours win.
    assert_eq!(approx_fraction(den - 3, den, max), (max - 1, max));
    assert_eq!(approx_fraction(3, den, max), (1, max));
}

#[test]
fn approx_fraction_matches_brute_force() {
    for max_den in [1u32, 2, 5, 17, 64, 1000] {
        for num in 0..400u64 {
            let got = approx_fraction(num, 400, max_den);
            let want = brute_force(num, 400, max_den);
            // Ties are allowed: only being strictly worse is a failure.
            assert!(
                !closer(num, 400, want, got),
                "{num}/400 with max_den {max_den}: got {got:?}, want {want:?}"
            );
        }
    }
}

#[test]
fn approx_fraction_beats_a_fixed_denominator() {
    // The property that motivates the whole change: a free denominator is
    // never worse than pinning it to 1048575 and truncating, and is
    // normally orders of magnitude better.
    let den = Frequency::from_hz(XTAL).as_microhz();

    for num in (1..den).step_by(97_777_777) {
        let got = approx_fraction(num, den, MAX_FRACTION_DENOM);
        let fixed = truncated(num, den, MAX_FRACTION_DENOM);
        assert!(
            !closer(num, den, fixed, got),
            "{num}/{den}: got {got:?}, fixed {fixed:?}"
        );
    }
}

#[test]
fn wspr_tones_land_within_a_millihertz() {
    let dev = device();
    let base = Frequency::from_hz(14_097_100);

    let mut previous = None;
    for tone in 0..4u64 {
        let target = base + Frequency::from_ratio(12_000 * tone, 8_192);

        let (ms_div, r_div) = dev
            .find_int_dividers_for_max_pll_freq(900_000_000, target)
            .unwrap();
        let total_div = ms_div as u32 * r_div.denominator_u8() as u32;
        let (mult, num, denom) = dev.find_pll_coeffs_for_dividers(total_div, target).unwrap();

        // The PLL has to stay inside its 600-900 MHz operating range.
        let pll = output_microhz(mult, num, denom, 1);
        assert!(
            (600_000_000_000_000..=900_000_000_000_000).contains(&pll),
            "PLL at {pll} uHz"
        );

        let out = output_microhz(mult, num, denom, total_div);
        let err = out.abs_diff(target.as_microhz() as u128);
        assert!(err < 1_000, "tone {tone}: off by {err} uHz");

        // Spacing matters more than absolute placement: 1.46484375 Hz.
        if let Some(previous) = previous {
            assert!(
                out.abs_diff(previous).abs_diff(1_464_844) < 2_000,
                "tone {tone} spacing"
            );
        }
        previous = Some(out);
    }
}

#[test]
fn plain_frequencies_stay_exact() {
    let dev = device();

    for hz in [
        3_500_000u32,
        7_000_000,
        10_000_000,
        14_175_000,
        27_000_000,
        100_000_000,
    ] {
        let target = Frequency::from_hz(hz);

        let (ms_div, r_div) = dev
            .find_int_dividers_for_max_pll_freq(900_000_000, target)
            .unwrap();
        let total_div = ms_div as u32 * r_div.denominator_u8() as u32;
        let (mult, num, denom) = dev.find_pll_coeffs_for_dividers(total_div, target).unwrap();

        let out = output_microhz(mult, num, denom, total_div);
        assert_eq!(out, target.as_microhz() as u128, "{hz} Hz");
    }
}

#[test]
fn out_of_range_frequencies_are_rejected() {
    let dev = device();

    assert_eq!(
        dev.find_int_dividers_for_max_pll_freq(900_000_000, Frequency::from_hz(0)),
        Err(Error::InvalidParameter)
    );
    // Below 900 MHz / u16::MAX the total divider no longer fits.
    assert_eq!(
        dev.find_int_dividers_for_max_pll_freq(900_000_000, Frequency::from_hz(13_000)),
        Err(Error::InvalidParameter)
    );
}

/// The three `P` parameters a MultiSynth register block holds, read back out
/// of the bytes the driver sent.
fn synth_parameters(bus: &FakeI2c, base: u8) -> (u32, u32, u32) {
    let byte = |offset: u8| {
        bus.last(base + offset)
            .expect("multisynth block never written") as u32
    };

    let p1 = ((byte(2) & 0x03) << 16) | (byte(3) << 8) | byte(4);
    let p2 = ((byte(5) & 0x0f) << 16) | (byte(6) << 8) | byte(7);
    let p3 = ((byte(5) & 0xf0) << 12) | (byte(0) << 8) | byte(1);

    (p1, p2, p3)
}

/// The `a + b/c` that block encodes, as an exact fraction.
fn synth_ratio(bus: &FakeI2c, base: u8) -> (u128, u128) {
    let (p1, p2, p3) = synth_parameters(bus, base);

    (
        (p1 as u128 + 512) * p3 as u128 + p2 as u128,
        128 * p3 as u128,
    )
}

/// The R divider register 44 asks for. AN619 puts `R0_DIV` in bits 6:4, coded
/// as the log2 of the division.
fn clk0_r_divider(bus: &FakeI2c) -> u128 {
    1 << ((bus.last(44).expect("multisynth block never written") >> 4) & 0b111)
}

/// The frequency clk0 is actually left running at, in microhertz, worked out
/// from the register bytes alone: fXTAL * MSNA / (MS0 * R).
fn clk0_microhz(bus: &FakeI2c) -> u128 {
    let (pll_num, pll_den) = synth_ratio(bus, 26);
    let (ms_num, ms_den) = synth_ratio(bus, 42);

    Frequency::from_hz(XTAL).as_microhz() as u128 * pll_num * ms_den
        / (pll_den * ms_num * clk0_r_divider(bus))
}

#[test]
fn fixed_pll_retunes_without_resetting_the_pll() {
    let mut dev = device();
    let dial = Frequency::from_hz(14_097_100);

    // 62 x the dial frequency, inside the VCO range.
    dev.set_pll_frequency(PLL::A, Frequency::from_hz(874_020_200))
        .unwrap();
    dev.reset_pll(PLL::A).unwrap();

    let pll_block = dev.i2c.writes.len();
    let resets = dev.i2c.writes.iter().filter(|(reg, _)| *reg == 177).count();

    for tone in 0..4u64 {
        let target = dial + Frequency::from_ratio(12_000 * tone, 8_192);
        dev.set_clock_frequency_fixed_pll(ClockOutput::Clk0, target)
            .unwrap();

        let out = clk0_microhz(&dev.i2c);
        assert!(
            out.abs_diff(target.as_microhz() as u128) < 1_000,
            "tone {tone}: {out} uHz"
        );
    }

    // Neither the feedback multisynth nor the reset register was touched again.
    let after = &dev.i2c.writes[pll_block..];
    assert!(
        !after.iter().any(|(reg, _)| (26..34).contains(reg)),
        "PLL registers rewritten"
    );
    assert_eq!(
        dev.i2c.writes.iter().filter(|(reg, _)| *reg == 177).count(),
        resets
    );
}

#[test]
fn fixed_pll_needs_a_configured_pll() {
    let mut dev = device();

    assert_eq!(
        dev.set_clock_frequency_fixed_pll(ClockOutput::Clk0, Frequency::from_hz(14_097_100)),
        Err(Error::InvalidParameter)
    );

    // clk6 and clk7 have no fractional multisynth at all.
    dev.set_pll_frequency(PLL::A, Frequency::from_hz(874_020_200))
        .unwrap();
    assert_eq!(
        dev.set_clock_frequency_fixed_pll(ClockOutput::Clk6, Frequency::from_hz(14_097_100)),
        Err(Error::InvalidParameter)
    );
}

#[test]
fn pll_frequency_is_remembered() {
    let mut dev = device();
    assert_eq!(dev.pll_frequency(PLL::A), None);
    assert_eq!(dev.pll_frequency(PLL::B), None);

    for hz in [600_000_000u32, 700_000_000, 874_020_200, 900_000_000] {
        dev.set_pll_frequency(PLL::B, Frequency::from_hz(hz))
            .unwrap();

        let parked = dev.pll_frequency(PLL::B).unwrap();
        assert!(
            parked
                .as_microhz()
                .abs_diff(Frequency::from_hz(hz).as_microhz())
                < 1_000,
            "{hz} Hz -> {parked:?}"
        );
        // Only the PLL that was asked for moves.
        assert_eq!(dev.pll_frequency(PLL::A), None);
    }

    // Outside the VCO range the request is refused and nothing is recorded.
    assert_eq!(
        dev.set_pll_frequency(PLL::A, Frequency::from_hz(599_999_999)),
        Err(Error::InvalidParameter)
    );
    assert_eq!(
        dev.set_pll_frequency(PLL::A, Frequency::from_hz(900_000_001)),
        Err(Error::InvalidParameter)
    );
    assert_eq!(dev.pll_frequency(PLL::A), None);
}

#[test]
fn drive_strength_reaches_the_control_register() {
    // Control register bits below the drive field: multisynth as the source,
    // output powered up.
    const SRC_MS: u8 = 0b0000_1100;

    let mut dev = device();
    dev.set_frequency(PLL::A, ClockOutput::Clk0, Frequency::from_hz(14_097_100))
        .unwrap();
    // 8 mA unless asked otherwise, which is what the driver always did.
    assert_eq!(dev.i2c.last(16), Some(SRC_MS | 0b11));

    for (drive, bits) in [
        (DriveStrength::_2mA, 0b00),
        (DriveStrength::_4mA, 0b01),
        (DriveStrength::_6mA, 0b10),
        (DriveStrength::_8mA, 0b11),
    ] {
        dev.set_clock_drive(ClockOutput::Clk0, drive);
        dev.flush_clock_control(ClockOutput::Clk0).unwrap();
        assert_eq!(dev.i2c.last(16), Some(SRC_MS | bits), "{drive:?}");
    }

    // Outputs carry their own setting.
    dev.set_clock_drive(ClockOutput::Clk5, DriveStrength::_2mA);
    dev.set_clock_drive(ClockOutput::Clk0, DriveStrength::_6mA);
    dev.flush_clock_control(ClockOutput::Clk5).unwrap();
    dev.flush_clock_control(ClockOutput::Clk0).unwrap();
    // clk5 is not enabled, so it is also powered down. Its drive bits are 0b00.
    assert_eq!(dev.i2c.last(21), Some(0b1000_0000 | SRC_MS));
    assert_eq!(dev.i2c.last(16), Some(SRC_MS | 0b10));
}

#[test]
fn improper_fractions_are_rejected() {
    let mut dev = device();

    assert_eq!(
        dev.setup_pll(PLL::A, 30, 3, 2),
        Err(Error::InvalidParameter)
    );
    assert_eq!(
        dev.setup_pll(PLL::A, 30, 2, 2),
        Err(Error::InvalidParameter)
    );
    assert_eq!(dev.setup_pll(PLL::A, 30, 1, 2), Ok(()));
}

#[test]
fn fractional_dividers_below_eight_are_rejected() {
    let mut dev = device();
    dev.set_pll_frequency(PLL::A, Frequency::from_hz(900_000_000))
        .unwrap();

    // 900 / 123 is 7.3, and AN619 2.1.1 allows no fraction below 8.
    assert_eq!(
        dev.set_clock_frequency_fixed_pll(ClockOutput::Clk0, Frequency::from_hz(123_000_000)),
        Err(Error::InvalidParameter)
    );

    // The integers 6 and 8 are fine, and so is anything at or above 8.
    for hz in [150_000_000u32, 112_500_000, 100_000_000] {
        assert_eq!(
            dev.set_clock_frequency_fixed_pll(ClockOutput::Clk0, Frequency::from_hz(hz)),
            Ok(()),
            "{hz} Hz"
        );
    }
}

/// How fine a step the fixed-PLL path can take is decided by where the PLL was
/// parked, because the step has to be expressible as a change in `a + b/c` with
/// `c` at most [`MAX_FRACTION_DENOM`]. A park that divides evenly into the
/// output puts the fraction against the widest gap there is — see
/// `approx_fraction_snaps_to_the_endpoints` — and both ways out of that gap are
/// bad, in different ways.
#[test]
fn fixed_pll_step_resolution_depends_on_the_park() {
    let base = Frequency::from_hz(5_000_000);
    let up = base + Frequency::from_microhz(10_000);
    let down = base - Frequency::from_microhz(10_000);

    // 900 MHz is exactly 180 times the output, so the fraction is 0/1 and the
    // gap either side of it spans about 26 mHz at this frequency.
    let mut dev = device();
    dev.set_pll_frequency(PLL::A, Frequency::from_hz(900_000_000))
        .unwrap();

    // Upwards the closest fraction the registers can hold is 1/1, improper, so
    // the request is refused rather than rounded.
    assert_eq!(
        dev.set_clock_frequency_fixed_pll(ClockOutput::Clk0, up),
        Err(Error::InvalidParameter)
    );

    // Downwards it is 0/1, which is a legal integer divider — so this one
    // succeeds and quietly leaves the output on 5 MHz. An `Ok` here is not a
    // promise that the step happened.
    assert_eq!(
        dev.set_clock_frequency_fixed_pll(ClockOutput::Clk0, down),
        Ok(())
    );
    assert_eq!(clk0_microhz(&dev.i2c), base.as_microhz() as u128);

    // It is the gap that is the problem, not the step being small: a step wide
    // enough to clear it is fine from the same park.
    let wider = base + Frequency::from_microhz(100_000);
    assert_eq!(
        dev.set_clock_frequency_fixed_pll(ClockOutput::Clk0, wider),
        Ok(())
    );
    assert!(clk0_microhz(&dev.i2c).abs_diff(wider.as_microhz() as u128) <= 1);

    // 888 MHz is 177.6 times the output. The fraction lands at 3/5 with room on
    // both sides, and the same two steps now land within a microhertz.
    let mut dev = device();
    dev.set_pll_frequency(PLL::A, Frequency::from_hz(888_000_000))
        .unwrap();

    for target in [up, down] {
        dev.set_clock_frequency_fixed_pll(ClockOutput::Clk0, target)
            .unwrap();

        let out = clk0_microhz(&dev.i2c);
        assert!(
            out.abs_diff(target.as_microhz() as u128) <= 1,
            "{} uHz: {out} uHz",
            target.as_microhz()
        );
    }
}

#[test]
fn low_frequencies_use_the_r_divider() {
    let mut dev = device();

    // Above 1 MHz the R divider stays at 1 and the whole field is zero.
    dev.set_frequency(PLL::A, ClockOutput::Clk0, Frequency::from_hz(14_097_100))
        .unwrap();
    assert_eq!(clk0_r_divider(&dev.i2c), 1);

    // Below it, R does the coarse division and the multisynth the rest. These
    // were silently coming out wrong before the Rx_DIV field was shifted into
    // bits 6:4: the divider read back as 1 and the output as R times too fast.
    for (hz, r) in [
        (500_000u32, 2u128),
        (100_000, 16),
        (32_768, 32),
        (20_000, 64),
    ] {
        let target = Frequency::from_hz(hz);
        dev.set_frequency(PLL::A, ClockOutput::Clk0, target)
            .unwrap();

        assert_eq!(clk0_r_divider(&dev.i2c), r, "{hz} Hz");

        let out = clk0_microhz(&dev.i2c);
        assert!(
            out.abs_diff(target.as_microhz() as u128) < 1_000,
            "{hz} Hz: {out} uHz"
        );

        // The PLL still has to sit in its 600 to 900 MHz range.
        let (pll_num, pll_den) = synth_ratio(&dev.i2c, 26);
        let vco = Frequency::from_hz(XTAL).as_microhz() as u128 * pll_num / pll_den;
        assert!(
            (600_000_000_000_000..=900_000_000_000_000).contains(&vco),
            "{hz} Hz: VCO at {vco} uHz"
        );
    }
}

/// `scale` is `value * num / den` truncated, and the split it does is the
/// whole point: the naive spelling overflows. A 27 MHz crystal in microhertz
/// is 2.7e13, and times a 20-bit numerator that is 2.8e19, past `u64::MAX` —
/// silently, since release builds wrap. So this checks the value against an
/// exact `u128` rather than looking for a panic.
#[test]
fn scale_is_exact() {
    let exact = |value: u64, num: u32, den: u32| (value as u128 * num as u128 / den as u128) as u64;

    let values = [
        0u64,
        1,
        999_999,
        MAX_FRACTION_DENOM as u64,
        Frequency::from_hz(25_000_000).as_microhz(),
        Frequency::from_hz(27_000_000).as_microhz(),
        Frequency::from_hz(u32::MAX).as_microhz(),
        u64::MAX,
    ];

    // `num < den` is the precondition; the widest denominators are the ones
    // that overflow the naive form, and `den - 1` the worst numerator.
    let fractions = [
        (0u32, 1u32),
        (1, 2),
        (1, MAX_FRACTION_DENOM),
        (MAX_FRACTION_DENOM - 1, MAX_FRACTION_DENOM),
        (MAX_FRACTION_DENOM / 2, MAX_FRACTION_DENOM),
        // Nothing calls it with more than a 20-bit denominator, but the split
        // holds for anything a u32 can hold, and that is what makes it safe.
        (u32::MAX - 1, u32::MAX),
    ];

    for value in values {
        for (num, den) in fractions {
            assert_eq!(
                scale(value, num, den),
                exact(value, num, den),
                "{value} * {num} / {den}"
            );
        }
    }
}

/// The one caller: `setup_pll` records fXTAL * (a + b/c) so that
/// `set_clock_frequency_fixed_pll` knows what it is dividing. With the widest
/// fraction the registers can hold, that has to still be the right number.
#[test]
fn recorded_pll_frequency_is_exact() {
    let xtal = Frequency::from_hz(XTAL).as_microhz() as u128;

    for (mult, num, denom) in [
        (15u8, 0, 1),
        (35, 1, 5),
        (35, MAX_FRACTION_DENOM - 1, MAX_FRACTION_DENOM),
        (90, MAX_FRACTION_DENOM / 3, MAX_FRACTION_DENOM),
    ] {
        let mut dev = device();
        dev.setup_pll(PLL::A, mult, num, denom).unwrap();

        // Truncated the same way `scale` truncates, so this is exact equality.
        let want = xtal * mult as u128 + xtal * num as u128 / denom as u128;
        assert_eq!(
            dev.pll_frequency(PLL::A).unwrap().as_microhz() as u128,
            want,
            "{mult} + {num}/{denom}"
        );
    }
}
