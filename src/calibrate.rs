//! Correcting the crystal error from a gated frequency count.
//!
//! The part's accuracy is its crystal's. Everything downstream of that crystal
//! — both PLLs and every output — shares one *fractional* error, so a count
//! taken on any output measures the error on all of them, and one measurement
//! corrects any target.
//!
//! That is what makes this practical: gate a convenient output against a known
//! reference such as a GPS PPS, and use the result to pre-distort a target the
//! counter never sees.
//!
//! ```
//! use si5351::{calibrate, Frequency};
//!
//! // 10 MHz counted for one second came out 620 cycles fast.
//! let ppb = calibrate::error_ppb(10_000_620, Frequency::from_hz(10_000_000), 1);
//! assert_eq!(calibrate::as_ppm(ppb), 62);
//!
//! // So ask for 62 ppm less on an entirely different output.
//! assert!(calibrate::plausible(ppb));
//! let dial = calibrate::correct(Frequency::from_hz(14_097_100), ppb);
//! assert_eq!(dial.as_hz(), 14_096_225);
//! ```
//!
//! Resolution is one count in `nominal * periods`: 100 ppb for a one-second
//! gate on 10 MHz, ten times finer over ten seconds. A longer gate buys
//! resolution and nothing else — it cannot reach drift that happens after the
//! correction is applied, and it returns a mean over a wider window that is
//! staler by the time it is used.

use crate::Frequency;

/// The largest fractional error a crystal can plausibly show, 200 ppm.
///
/// Ordinary parts are specified at 20 to 30 ppm and age by a few ppm. A gate
/// reading past this has lost or doubled a reference tick.
pub const MAX_PLAUSIBLE_PPB: i64 = 200_000;

/// The fractional error of a gated count, in parts per billion.
///
/// `ticks` is what the counter saw while gated over `periods` whole intervals
/// of the reference, counting an output the part was asked to put at `nominal`.
/// Positive means the part runs fast.
///
/// ```
/// use si5351::{calibrate, Frequency};
///
/// let nominal = Frequency::from_hz(10_000_000);
/// assert_eq!(calibrate::error_ppb(10_000_000, nominal, 1), 0);
///
/// // One count either way is the resolution floor of a one-second gate.
/// assert_eq!(calibrate::error_ppb(10_000_001, nominal, 1), 100);
///
/// // Ten seconds resolve ten times finer.
/// assert_eq!(calibrate::error_ppb(100_000_001, nominal, 10), 10);
/// ```
///
/// # Panics
///
/// If `nominal * periods` rounds to zero counts.
pub fn error_ppb(ticks: u32, nominal: Frequency, periods: u32) -> i64 {
    let expected = (nominal.as_microhz() * periods as u64 / 1_000_000) as i64;

    // A u32 tick count caps the product at 4.3e18, inside i64.
    (ticks as i64 - expected) * 1_000_000_000 / expected
}

/// Whether `ppb` is small enough to have come from a crystal.
///
/// A cheap guard on a gate that may have missed a reference tick: a doubled
/// gate reads +100%, a halved one -50%, and neither is a measurement.
///
/// ```
/// use si5351::{calibrate, Frequency};
///
/// let nominal = Frequency::from_hz(10_000_000);
/// assert!(calibrate::plausible(calibrate::error_ppb(10_000_620, nominal, 1)));
/// assert!(!calibrate::plausible(calibrate::error_ppb(20_000_000, nominal, 1)));
/// ```
pub fn plausible(ppb: i64) -> bool {
    ppb.abs() <= MAX_PLAUSIBLE_PPB
}

/// `ppb` in parts per million, truncated.
pub fn as_ppm(ppb: i64) -> i64 {
    ppb / 1_000
}

/// Pre-distorts `freq` so the part emits it despite `ppb` of crystal error.
///
/// Ask for the result instead of `freq` and the output lands on `freq`. The
/// correction is linearised, leaving a residual of `freq * (ppb/1e9)^2` — 54 mHz
/// at 62 ppm on 20 m, four orders below the error being removed, and nil in a
/// loop that re-measures after applying it.
///
/// ```
/// use si5351::{calibrate, Frequency};
///
/// // A part running 62 ppm fast is asked for 62 ppm less.
/// let dial = calibrate::correct(Frequency::from_hz(14_097_100), 62_000);
/// assert_eq!(dial.as_microhz(), 14_096_225_979_800);
///
/// // A slow part is asked for more.
/// let dial = calibrate::correct(Frequency::from_hz(14_097_100), -62_000);
/// assert!(dial.as_hz() > 14_097_100);
/// ```
pub fn correct(freq: Frequency, ppb: i64) -> Frequency {
    let uhz = freq.as_microhz() as i64;

    // Splitting at 1e9 keeps this in i64: widening to i128 would pull in the
    // 128-bit division helpers, around 900 bytes of flash on a Cortex-M. Exact,
    // since q * 1e9 leaves no remainder — it truncates where `uhz * ppb / 1e9`
    // would, and stays sound for any |ppb| below 9e9.
    let (q, r) = (uhz / 1_000_000_000, uhz % 1_000_000_000);

    Frequency::from_microhz((uhz - (q * ppb + r * ppb / 1_000_000_000)) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOMINAL: Frequency = Frequency::from_hz(10_000_000);

    #[test]
    fn error_ppb_is_signed_and_scales_with_the_gate() {
        assert_eq!(error_ppb(9_999_999, NOMINAL, 1), -100);
        assert_eq!(error_ppb(10_000_620, NOMINAL, 1), 62_000);

        // Nine periods resolve nine times finer than one.
        assert_eq!(error_ppb(90_000_001, NOMINAL, 9), 11);
        assert_eq!(error_ppb(89_999_999, NOMINAL, 9), -11);
    }

    #[test]
    fn a_fractional_nominal_is_honoured() {
        // 3.5 MHz + 0.5 Hz over two seconds is 7_000_001 counts.
        let nominal = Frequency::from_microhz(3_500_000_500_000);
        assert_eq!(error_ppb(7_000_001, nominal, 2), 0);
    }

    #[test]
    fn correct_cancels_the_measured_error() {
        let target = Frequency::from_hz(14_097_100);
        let dial = correct(target, 62_000);

        // Re-measuring what the part now emits leaves the linearisation
        // residual alone: target * (62e-6)^2, 54 mHz.
        let emitted = dial.as_microhz() as i128 * (1_000_000_000 + 62_000) / 1_000_000_000;
        assert!((emitted - target.as_microhz() as i128).abs() < 60_000);
    }

    #[test]
    fn correct_matches_a_128_bit_reference() {
        for hz in [1_000_000u32, 10_000_000, 14_097_100, 200_000_000] {
            for ppb in [
                -9_000_000_000i64,
                -200_000,
                -62_000,
                -1,
                0,
                1,
                62_000,
                200_000,
                1_000_000_000,
                9_000_000_000,
            ] {
                let freq = Frequency::from_hz(hz);
                let uhz = freq.as_microhz() as i128;
                let want = (uhz - uhz * ppb as i128 / 1_000_000_000) as u64;

                assert_eq!(correct(freq, ppb).as_microhz(), want, "{hz} Hz, {ppb} ppb");
            }
        }
    }

    #[test]
    fn round_trip_through_a_deliberately_wrong_crystal() {
        // Pre-distort, emit through a part 30 ppm slow, and land back on target.
        let target = Frequency::from_hz(14_097_100);
        let dial = correct(target, -30_000);
        let emitted = dial.as_microhz() as i128 * (1_000_000_000 - 30_000) / 1_000_000_000;

        assert!((emitted - target.as_microhz() as i128).abs() < 15_000);
    }
}
