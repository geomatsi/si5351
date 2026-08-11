/*
   Copyright 2018 Ilya Epifanov

   Modified in 2026 by Sergey Matyukevich: ported to embedded-hal 1.0,
   bitflags 2 and Rust edition 2024.

   Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
   http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
   http://opensource.org/licenses/MIT>, at your option. This file may not be
   copied, modified, or distributed except according to those terms.
*/
/*!
A platform agnostic Rust driver for the [Si5351], based on the
[`embedded-hal`] traits.

## The Device

The Silicon Labs [Si5351] is an any-frequency CMOS clock generator.

The device has an I²C interface.

## Usage

The driver takes ownership of any I²C bus implementing the [`embedded_hal::i2c::I2c`]
trait. Obtaining that bus is platform specific, so instantiate the device, initialize
it, and set a frequency on one of the outputs:

```
use embedded_hal::i2c::I2c;
use si5351::{ClockOutput, CrystalLoad, Error, Frequency, Si5351, Si5351Device, PLL};

fn setup<I2C: I2c>(i2c: I2C) -> Result<Si5351Device<I2C>, Error> {
    let mut clock = Si5351Device::new(i2c, false, 25_000_000);
    clock.init(CrystalLoad::_10pF)?;
    clock.set_frequency(PLL::A, ClockOutput::Clk0, Frequency::from_hz(14_175_000))?;
    Ok(clock)
}
```

Targets are [`Frequency`] values rather than a whole number of hertz, because the
device tunes in rational steps far finer than 1 Hz — WSPR, for example, spaces
its four tones exactly 12000/8192 Hz apart. [`Frequency::from_ratio`] expresses
that without rounding.

[`Si5351::set_frequency`] resets the PLL, which restarts the divider chain and
jumps the output phase. That is fine to key a transmitter with, but not to shift
tones inside one. For that, park the PLL once with
[`Si5351::set_pll_frequency`] and afterwards move only the output MultiSynth
with [`Si5351::set_clock_frequency_fixed_pll`], which needs no reset.

If you have an [Adafruit module], you can use shortcut functions to initialize it:

```
use embedded_hal::i2c::I2c;
use si5351::{Error, Si5351, Si5351Device};

fn setup<I2C: I2c>(i2c: I2C) -> Result<Si5351Device<I2C>, Error> {
    let mut clock = Si5351Device::new_adafruit_module(i2c);
    clock.init_adafruit_module()?;
    Ok(clock)
}
```

If your bus is shared with other devices, wrap it in one of the bus sharing
primitives from [`embedded-hal-bus`] and hand the resulting proxy to
[`Si5351Device::new`].

[Si5351]: https://www.skyworksinc.com/-/media/Skyworks/SL/documents/public/data-sheets/Si5351-B.pdf
[`embedded-hal`]: https://github.com/rust-embedded/embedded-hal
[`embedded-hal-bus`]: https://docs.rs/embedded-hal-bus
[Adafruit module]: https://www.adafruit.com/product/2045
*/
//#![deny(missing_docs)]
#![no_std]

use bitflags::bitflags;
use embedded_hal::i2c::I2c;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Error {
    CommunicationError,
    InvalidParameter,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::CommunicationError => f.write_str("I²C communication error"),
            Error::InvalidParameter => f.write_str("invalid parameter"),
        }
    }
}

impl core::error::Error for Error {}

const MICROHZ_PER_HZ: u64 = 1_000_000;

/// A frequency, held with microhertz resolution.
///
/// Both synthesis stages of the Si5351 divide by `a + b/c` with 20-bit `b` and
/// `c`, so the reachable output frequencies form a dense set of rationals
/// rather than a grid with a fixed step. Targets much finer than 1 Hz are
/// therefore meaningful, and this type carries them.
///
/// [`Frequency::from_ratio`] keeps ratios that are not whole numbers of hertz
/// exact to within the microhertz resolution, which matters for modes like
/// WSPR whose tone spacing is 12000/8192 Hz:
///
/// ```
/// use si5351::Frequency;
///
/// let spacing = Frequency::from_ratio(12_000, 8_192);
/// assert_eq!(spacing.as_microhz(), 1_464_844); // 1.46484375 Hz
/// ```
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Frequency(u64);

impl Frequency {
    /// A frequency of `hz` hertz.
    pub const fn from_hz(hz: u32) -> Self {
        Frequency(hz as u64 * MICROHZ_PER_HZ)
    }

    /// A frequency of `millihz` millihertz.
    pub const fn from_millihz(millihz: u64) -> Self {
        Frequency(millihz * 1_000)
    }

    /// A frequency of `microhz` microhertz.
    pub const fn from_microhz(microhz: u64) -> Self {
        Frequency(microhz)
    }

    /// A frequency of `num / den` hertz, rounded to the nearest microhertz.
    ///
    /// # Panics on
    ///   - division by zero
    ///   - mul overflow
    ///   - add overflow
    ///
    pub const fn from_ratio(num: u64, den: u32) -> Self {
        let den = den as u64;
        let int = num / den;
        let rem = num % den;

        let frac = (rem * MICROHZ_PER_HZ + den / 2) / den;

        let scaled = match int.checked_mul(MICROHZ_PER_HZ) {
            Some(scaled) => scaled,
            None => panic!("from_ratio: overflow multiplying integer part by 1_000_000"),
        };

        match scaled.checked_add(frac) {
            Some(result) => Frequency(result),
            None => panic!("from_ratio: overflow adding fractional mHz to scaled integer part"),
        }
    }

    /// This frequency in whole hertz, rounded towards zero.
    pub const fn as_hz(self) -> u32 {
        (self.0 / MICROHZ_PER_HZ) as u32
    }

    /// This frequency in microhertz.
    pub const fn as_microhz(self) -> u64 {
        self.0
    }
}

impl From<u32> for Frequency {
    fn from(hz: u32) -> Self {
        Frequency::from_hz(hz)
    }
}

impl core::ops::Add for Frequency {
    type Output = Frequency;

    fn add(self, rhs: Frequency) -> Frequency {
        match self.0.checked_add(rhs.0) {
            Some(result) => Frequency(result),
            None => panic!("add: overflow adding two mHz Frequencies"),
        }
    }
}

impl core::ops::Sub for Frequency {
    type Output = Frequency;

    fn sub(self, rhs: Frequency) -> Frequency {
        match self.0.checked_sub(rhs.0) {
            Some(result) => Frequency(result),
            None => panic!("sub: underflow subtracting two mHz Frequencies"),
        }
    }
}

/// Internal crystal load capacitance: the `XTAL_CL` field of AN619 register
/// 183, named for the capacitance it presents.
///
/// This has to match the crystal the board was designed around — 10 pF on the
/// common breakouts, which is also the field's power-up default — or the
/// reference is off frequency, and every output with it. [`Si5351::init`]
/// writes the field either way.
///
/// AN619 lists a fourth encoding, 00, as reserved; it has no variant here.
#[derive(Debug, Copy, Clone)]
pub enum CrystalLoad {
    /// 6 pF.
    _6pF,
    /// 8 pF.
    _8pF,
    /// 10 pF.
    _10pF,
}

/// Output driver strength: the `CLKx_IDRV` field of AN619 registers 16-23,
/// named for the current it drives into a 50 Ω load.
///
/// Higher drive means more output power and faster edges, at the cost of
/// supply current and crosstalk between outputs. Outputs default to
/// [`DriveStrength::_8mA`], which is what this driver always used before the
/// setting was exposed.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DriveStrength {
    /// 2 mA.
    _2mA,
    /// 4 mA.
    _4mA,
    /// 6 mA.
    _6mA,
    /// 8 mA.
    _8mA,
}

#[derive(Debug, Copy, Clone)]
pub enum PLL {
    A,
    B,
}

#[derive(Debug, Copy, Clone)]
pub enum FeedbackMultisynth {
    MSNA,
    MSNB,
}

#[derive(Debug, Copy, Clone)]
pub enum Multisynth {
    MS0,
    MS1,
    MS2,
    MS3,
    MS4,
    MS5,
}

#[derive(Debug, Copy, Clone)]
pub enum SimpleMultisynth {
    MS6,
    MS7,
}

#[derive(Debug, Copy, Clone)]
pub enum ClockOutput {
    Clk0 = 0,
    Clk1,
    Clk2,
    Clk3,
    Clk4,
    Clk5,
    Clk6,
    Clk7,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum OutputDivider {
    Div1 = 0,
    Div2,
    Div4,
    Div8,
    Div16,
    Div32,
    Div64,
    Div128,
}

const ADDRESS: u8 = 0b0110_0000;

impl PLL {
    pub fn multisynth(&self) -> FeedbackMultisynth {
        match *self {
            PLL::A => FeedbackMultisynth::MSNA,
            PLL::B => FeedbackMultisynth::MSNB,
        }
    }

    fn ix(&self) -> usize {
        match *self {
            PLL::A => 0,
            PLL::B => 1,
        }
    }
}

impl DriveStrength {
    fn bits(&self) -> ClockControlBits {
        match *self {
            DriveStrength::_2mA => ClockControlBits::CLK_DRV_2,
            DriveStrength::_4mA => ClockControlBits::CLK_DRV_4,
            DriveStrength::_6mA => ClockControlBits::CLK_DRV_6,
            DriveStrength::_8mA => ClockControlBits::CLK_DRV_8,
        }
    }
}

trait FractionalMultisynth {
    fn base_addr(&self) -> u8;
    fn ix(&self) -> u8;
}

impl FractionalMultisynth for FeedbackMultisynth {
    fn base_addr(&self) -> u8 {
        match *self {
            FeedbackMultisynth::MSNA => 26,
            FeedbackMultisynth::MSNB => 34,
        }
    }
    fn ix(&self) -> u8 {
        match *self {
            FeedbackMultisynth::MSNA => 6,
            FeedbackMultisynth::MSNB => 7,
        }
    }
}

impl FractionalMultisynth for Multisynth {
    fn base_addr(&self) -> u8 {
        match *self {
            Multisynth::MS0 => 42,
            Multisynth::MS1 => 50,
            Multisynth::MS2 => 58,
            Multisynth::MS3 => 66,
            Multisynth::MS4 => 74,
            Multisynth::MS5 => 82,
        }
    }
    fn ix(&self) -> u8 {
        match *self {
            Multisynth::MS0 => 0,
            Multisynth::MS1 => 1,
            Multisynth::MS2 => 2,
            Multisynth::MS3 => 3,
            Multisynth::MS4 => 4,
            Multisynth::MS5 => 5,
        }
    }
}

impl SimpleMultisynth {
    pub fn base_addr(&self) -> u8 {
        match *self {
            SimpleMultisynth::MS6 => 90,
            SimpleMultisynth::MS7 => 91,
        }
    }
}

#[derive(Debug, Copy, Clone)]
enum Register {
    DeviceStatus = 0,
    OutputEnable = 3,
    Clk0 = 16,
    Clk1 = 17,
    Clk2 = 18,
    Clk3 = 19,
    Clk4 = 20,
    Clk5 = 21,
    Clk6 = 22,
    Clk7 = 23,
    PLLReset = 177,
    CrystalLoad = 183,
}

impl Register {
    pub fn addr(&self) -> u8 {
        *self as u8
    }
}

bitflags! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct DeviceStatusBits: u8 {
        const SYS_INIT = 0b1000_0000;
        const LOL_B = 0b0100_0000;
        const LOL_A = 0b0010_0000;
        const LOS = 0b0001_0000;
    }
}

bitflags! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    struct CrystalLoadBits: u8 {
        const RESERVED = 0b00_010010;
        const CL_MASK = 0b11_000000;
        const CL_6 = 0b01_000000;
        const CL_8 = 0b10_000000;
        const CL_10 = 0b11_000000;
    }
}

bitflags! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    struct ClockControlBits: u8 {
        const CLK_PDN = 0b1000_0000;
        const MS_INT = 0b0100_0000;
        const MS_SRC = 0b0010_0000;
        const CLK_INV = 0b0001_0000;
        const CLK_SRC_MASK = 0b0000_1100;
        const CLK_SRC_XTAL = 0b0000_0000;
        const CLK_SRC_CLKIN = 0b0000_0100;
        /// AN619 registers 16-23, CLKx_SRC: "Reserved. Do not select this
        /// option." Kept so the mask is complete; never write it.
        const CLK_SRC_RESERVED = 0b0000_1000;
        const CLK_SRC_MS = 0b0000_1100;
        const CLK_DRV_MASK = 0b0000_0011;
        const CLK_DRV_2 = 0b0000_0000;
        const CLK_DRV_4 = 0b0000_0001;
        const CLK_DRV_6 = 0b0000_0010;
        const CLK_DRV_8 = 0b0000_0011;
    }
}

bitflags! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    struct PLLResetBits: u8 {
        const PLLB_RST = 0b1000_0000;
        const PLLA_RST = 0b0010_0000;
    }
}

impl ClockOutput {
    fn register(self) -> Register {
        match self {
            ClockOutput::Clk0 => Register::Clk0,
            ClockOutput::Clk1 => Register::Clk1,
            ClockOutput::Clk2 => Register::Clk2,
            ClockOutput::Clk3 => Register::Clk3,
            ClockOutput::Clk4 => Register::Clk4,
            ClockOutput::Clk5 => Register::Clk5,
            ClockOutput::Clk6 => Register::Clk6,
            ClockOutput::Clk7 => Register::Clk7,
        }
    }

    fn ix(&self) -> u8 {
        *self as u8
    }

    /// The fractional MultiSynth that feeds this output.
    ///
    /// Clk6 and clk7 are fed by the integer-only [`SimpleMultisynth`] dividers,
    /// which this driver does not program, so they have none.
    pub fn multisynth(self) -> Result<Multisynth, Error> {
        match self {
            ClockOutput::Clk0 => Ok(Multisynth::MS0),
            ClockOutput::Clk1 => Ok(Multisynth::MS1),
            ClockOutput::Clk2 => Ok(Multisynth::MS2),
            ClockOutput::Clk3 => Ok(Multisynth::MS3),
            ClockOutput::Clk4 => Ok(Multisynth::MS4),
            ClockOutput::Clk5 => Ok(Multisynth::MS5),
            ClockOutput::Clk6 | ClockOutput::Clk7 => Err(Error::InvalidParameter),
        }
    }
}

impl OutputDivider {
    fn bits(&self) -> u8 {
        *self as u8
    }

    fn min_divider(desired_divider: u16) -> Result<OutputDivider, Error> {
        match 16 - (desired_divider.max(1) - 1).leading_zeros() {
            0 => Ok(OutputDivider::Div1),
            1 => Ok(OutputDivider::Div2),
            2 => Ok(OutputDivider::Div4),
            3 => Ok(OutputDivider::Div8),
            4 => Ok(OutputDivider::Div16),
            5 => Ok(OutputDivider::Div32),
            6 => Ok(OutputDivider::Div64),
            7 => Ok(OutputDivider::Div128),
            _ => Err(Error::InvalidParameter),
        }
    }

    /// What this divider divides by.
    ///
    /// Public because [`Si5351::find_int_dividers_for_max_pll_freq`] hands out
    /// an `OutputDivider` and [`Si5351::find_pll_coeffs_for_dividers`] expects
    /// the total divider back.
    pub fn denominator_u8(&self) -> u8 {
        match *self {
            OutputDivider::Div1 => 1,
            OutputDivider::Div2 => 2,
            OutputDivider::Div4 => 4,
            OutputDivider::Div8 => 8,
            OutputDivider::Div16 => 16,
            OutputDivider::Div32 => 32,
            OutputDivider::Div64 => 64,
            OutputDivider::Div128 => 128,
        }
    }
}

/// Largest fractional denominator a MultiSynth divider can hold (20 bits).
const MAX_FRACTION_DENOM: u32 = 0xf_ffff;

/// Best rational approximation of `num / den` with a denominator no greater
/// than `max_den`.
///
/// The MultiSynth dividers take an arbitrary 20-bit denominator, so pinning it
/// to a constant and truncating the numerator would quantize the output to a
/// fixed grid — of `fXTAL / max_den / total_divider`, around 0.38 Hz on 20 m —
/// for no reason. Approximating with a free denominator instead normally lands
/// within microhertz of the target. The exception is a target that falls in a
/// gap of the Farey sequence of order `max_den`, that is, one sitting just
/// beside a ratio with a small denominator; those are rare but can still be a
/// few millihertz out.
///
/// This walks the continued fraction expansion of `num / den`, keeping the last
/// convergent whose denominator still fits, and then checks whether the largest
/// semiconvergent built from it is closer to the true value.
///
/// `num` must be smaller than `den`, and `max_den` must be non-zero.
fn approx_fraction(num: u64, den: u64, max_den: u32) -> (u32, u32) {
    debug_assert!(num < den);
    debug_assert!(max_den > 0);

    let max_den = max_den as u64;

    // Remaining tail of the expansion, and the last two convergents.
    let (mut n, mut d) = (num, den);
    let (mut p0, mut q0) = (0u64, 1u64);
    let (mut p1, mut q1) = (1u64, 0u64);

    loop {
        let a = n / d;

        // Stop before the denominator runs past what the register can hold.
        let q2 = match a.checked_mul(q1).and_then(|q| q.checked_add(q0)) {
            Some(q2) if q2 <= max_den => q2,
            _ => break,
        };
        let p2 = a * p1 + p0;

        (p0, q0) = (p1, q1);
        (p1, q1) = (p2, q2);

        (n, d) = (d, n - a * d);
        if d == 0 {
            // num / den is representable exactly.
            return (p1 as u32, q1 as u32);
        }
    }

    // The next convergent would not fit. Of the semiconvergents that do, the
    // one with the largest denominator is the best; compare it against the
    // convergent we stopped at. With x the exact value and t = n / d the tail,
    // the two errors are 1 / (q1 * (q1 * t + q0)) and (t - k) / (qa * (q1 * t +
    // q0)), which reduces to the cross-multiplied comparison below. Every
    // product there stays at or below `den`.
    let k = (max_den - q0) / q1;
    let (pa, qa) = (p0 + k * p1, q0 + k * q1);

    if (n - k * d) * q1 < d * qa {
        (pa as u32, qa as u32)
    } else {
        (p1 as u32, q1 as u32)
    }
}

/// The VCO range the datasheet guarantees.
const MIN_PLL_FREQ: Frequency = Frequency::from_hz(600_000_000);
const MAX_PLL_FREQ: Frequency = Frequency::from_hz(900_000_000);

/// `value * num / den`, for `num < den`, without the intermediate product
/// overflowing.
///
/// `value * num` alone runs past 64 bits for a crystal frequency in microhertz
/// times a 20-bit numerator, so split `value` at `den` first.
fn scale(value: u64, num: u32, den: u32) -> u64 {
    debug_assert!(num < den);

    let (num, den) = (num as u64, den as u64);

    (value / den) * num + (value % den) * num / den
}

fn i2c_error<E>(_: E) -> Error {
    Error::CommunicationError
}

/// Si5351 driver
pub struct Si5351Device<I2C> {
    i2c: I2C,
    address: u8,
    xtal_freq: u32,
    clk_enabled_mask: u8,
    ms_int_mode_mask: u8,
    ms_src_mask: u8,
    /// Two bits per output, laid out like the `CLK_DRV` field they end up in.
    clk_drive_mask: u16,
    /// Where each PLL was last parked, indexed by [`PLL::ix`]. Kept so that
    /// [`Si5351::set_clock_frequency_fixed_pll`] knows what it is dividing.
    pll_freq: [Option<Frequency>; 2],
}

pub trait Si5351 {
    /// [`Si5351::init`] with the 10 pF load an Adafruit module's crystal wants.
    fn init_adafruit_module(&mut self) -> Result<(), Error>;

    /// Waits for the device to finish its power-up initialization, then powers
    /// every output down and selects the crystal load capacitance.
    ///
    /// `xtal_load` must match the crystal on the board — 10 pF on the common
    /// breakouts — or the reference, and every output with it, is off frequency.
    ///
    /// ```
    /// # use embedded_hal::i2c::I2c;
    /// # use si5351::{CrystalLoad, Error, Si5351, Si5351Device};
    /// # fn example<I2C: I2c>(clock: &mut Si5351Device<I2C>) -> Result<(), Error> {
    /// clock.init(CrystalLoad::_10pF)?;
    /// # Ok(())
    /// # }
    /// ```
    fn init(&mut self, xtal_load: CrystalLoad) -> Result<(), Error>;

    /// Reads register 0: the initialization, loss-of-lock and loss-of-signal
    /// flags.
    ///
    /// ```
    /// # use embedded_hal::i2c::I2c;
    /// # use si5351::{DeviceStatusBits, Error, Si5351, Si5351Device};
    /// # fn example<I2C: I2c>(clock: &mut Si5351Device<I2C>) -> Result<bool, Error> {
    /// let status = clock.read_device_status()?;
    /// Ok(status.contains(DeviceStatusBits::LOL_A)) // PLL A has lost lock
    /// # }
    /// ```
    fn read_device_status(&mut self) -> Result<DeviceStatusBits, Error>;

    fn find_int_dividers_for_max_pll_freq(
        &self,
        max_pll_freq: u32,
        freq: Frequency,
    ) -> Result<(u16, OutputDivider), Error>;
    fn find_pll_coeffs_for_dividers(
        &self,
        total_div: u32,
        freq: Frequency,
    ) -> Result<(u8, u32, u32), Error>;

    /// Sets `clk` to `freq`, retuning the PLL that feeds it and resetting it.
    ///
    /// The reset is what makes the output settle at the new frequency, but it
    /// restarts the divider chain of **every output on that PLL**, not just
    /// `clk`, so all of them jump in phase. Two cases want
    /// [`Si5351::set_pll_frequency`] once and then
    /// [`Si5351::set_clock_frequency_fixed_pll`] per step instead:
    ///
    /// - shifting frequency mid-transmission, as WSPR and FSK keying do, where
    ///   the phase jump lands in the middle of the signal;
    /// - retuning one output of several, where this call would disturb the
    ///   others. Putting them on different PLLs also works, there being two.
    ///
    /// ```
    /// # use embedded_hal::i2c::I2c;
    /// # use si5351::{ClockOutput, Error, Frequency, Si5351, Si5351Device, PLL};
    /// # fn example<I2C: I2c>(clock: &mut Si5351Device<I2C>) -> Result<(), Error> {
    /// clock.set_frequency(PLL::A, ClockOutput::Clk0, Frequency::from_hz(14_175_000))?;
    /// # Ok(())
    /// # }
    /// ```
    fn set_frequency(&mut self, pll: PLL, clk: ClockOutput, freq: Frequency) -> Result<(), Error>;

    /// Parks `pll` at `freq`, which must be within the 600 to 900 MHz VCO
    /// range. Does not reset the PLL; call [`Si5351::reset_pll`] for that.
    ///
    /// This is the first step of the phase-continuous path: park the PLL and
    /// reset it once, then move the output with
    /// [`Si5351::set_clock_frequency_fixed_pll`].
    ///
    /// ```
    /// # use embedded_hal::i2c::I2c;
    /// # use si5351::{ClockOutput, Error, Frequency, Si5351, Si5351Device, PLL};
    /// # fn example<I2C: I2c>(clock: &mut Si5351Device<I2C>) -> Result<(), Error> {
    /// clock.set_pll_frequency(PLL::A, Frequency::from_hz(700_000_000))?;
    /// clock.reset_pll(PLL::A)?;
    /// clock.set_clock_frequency_fixed_pll(ClockOutput::Clk0, Frequency::from_hz(14_097_100))?;
    /// # Ok(())
    /// # }
    /// ```
    fn set_pll_frequency(&mut self, pll: PLL, freq: Frequency) -> Result<(), Error>;

    /// Where `pll` was last parked, or `None` if it has not been programmed
    /// through this driver.
    ///
    /// ```
    /// # use embedded_hal::i2c::I2c;
    /// # use si5351::{Frequency, Si5351, Si5351Device, PLL};
    /// # fn example<I2C: I2c>(clock: &Si5351Device<I2C>) -> Option<Frequency> {
    /// clock.pll_frequency(PLL::A)
    /// # }
    /// ```
    fn pll_frequency(&self, pll: PLL) -> Option<Frequency>;

    /// Sets `clk` to `freq` by moving only its output MultiSynth, leaving the
    /// PLL alone.
    ///
    /// No PLL reset, so the output keeps running through the change. The PLL
    /// feeding `clk` must already be programmed, by
    /// [`Si5351::set_pll_frequency`] or [`Si5351::set_frequency`], and `freq`
    /// has to be reachable from it with a divider of 6 to 1800 and an R
    /// divider of up to 128.
    ///
    /// Enables the output, as [`Si5351::set_frequency`] does.
    ///
    /// Shifting up one WSPR tone, the PLL already parked and reset:
    ///
    /// ```
    /// # use embedded_hal::i2c::I2c;
    /// # use si5351::{ClockOutput, Error, Frequency, Si5351, Si5351Device};
    /// # fn example<I2C: I2c>(clock: &mut Si5351Device<I2C>) -> Result<(), Error> {
    /// let freq = Frequency::from_hz(14_097_100) + Frequency::from_ratio(12_000, 8_192);
    /// clock.set_clock_frequency_fixed_pll(ClockOutput::Clk0, freq)?;
    /// # Ok(())
    /// # }
    /// ```
    fn set_clock_frequency_fixed_pll(
        &mut self,
        clk: ClockOutput,
        freq: Frequency,
    ) -> Result<(), Error>;

    /// Resets `pll`, relocking it.
    ///
    /// This restarts the divider chain of every output fed by `pll`, so all of
    /// them jump in phase — the outputs are not addressed individually here,
    /// only the PLL is. The other PLL is untouched: register 177 has a bit per
    /// PLL and only one is written.
    ///
    /// Needed after the feedback MultiSynth moves, which is the point in the
    /// datasheet's programming procedure where it appears. Moving an output
    /// MultiSynth on its own does not need it — that is what
    /// [`Si5351::set_clock_frequency_fixed_pll`] relies on, though note that
    /// neither the datasheet nor AN619 states the rule either way.
    ///
    /// ```
    /// # use embedded_hal::i2c::I2c;
    /// # use si5351::{Error, Si5351, Si5351Device, PLL};
    /// # fn example<I2C: I2c>(clock: &mut Si5351Device<I2C>) -> Result<(), Error> {
    /// clock.reset_pll(PLL::A)?;
    /// # Ok(())
    /// # }
    /// ```
    fn reset_pll(&mut self, pll: PLL) -> Result<(), Error>;

    /// Marks `clk` as enabled or disabled.
    ///
    /// Driver state only: the output enable register is written by
    /// [`Si5351::flush_output_enabled`], and the power-down bit by
    /// [`Si5351::flush_clock_control`]. Keying an output on and off:
    ///
    /// ```
    /// # use embedded_hal::i2c::I2c;
    /// # use si5351::{ClockOutput, Error, Si5351, Si5351Device};
    /// # fn example<I2C: I2c>(clock: &mut Si5351Device<I2C>) -> Result<(), Error> {
    /// clock.set_clock_enabled(ClockOutput::Clk0, false);
    /// clock.flush_output_enabled()?;
    /// # Ok(())
    /// # }
    /// ```
    fn set_clock_enabled(&mut self, clk: ClockOutput, enabled: bool);

    /// Selects the output driver strength for `clk`.
    ///
    /// Like [`Si5351::set_clock_enabled`], this only updates driver state; the
    /// chip sees it at the next [`Si5351::flush_clock_control`], which
    /// [`Si5351::set_frequency`] and
    /// [`Si5351::set_clock_frequency_fixed_pll`] both do.
    ///
    /// ```
    /// # use embedded_hal::i2c::I2c;
    /// # use si5351::{ClockOutput, DriveStrength, Error, Frequency, Si5351, Si5351Device, PLL};
    /// # fn example<I2C: I2c>(clock: &mut Si5351Device<I2C>) -> Result<(), Error> {
    /// clock.set_clock_drive(ClockOutput::Clk0, DriveStrength::_2mA);
    /// clock.set_frequency(PLL::A, ClockOutput::Clk0, Frequency::from_hz(10_000_000))?;
    /// # Ok(())
    /// # }
    /// ```
    fn set_clock_drive(&mut self, clk: ClockOutput, drive: DriveStrength);

    /// Writes the output enable register from the state
    /// [`Si5351::set_clock_enabled`] keeps.
    fn flush_output_enabled(&mut self) -> Result<(), Error>;

    /// Writes the control register of `clk`: power-down, integer mode, PLL
    /// source and drive strength, from the state the setters keep.
    fn flush_clock_control(&mut self, clk: ClockOutput) -> Result<(), Error>;

    /// [`Si5351::setup_pll`] with a whole-number multiplier.
    fn setup_pll_int(&mut self, pll: PLL, mult: u8) -> Result<(), Error>;

    /// Multiplies the crystal by `mult + num/denom` into `pll`, `mult` from 15
    /// to 90 and the fraction proper with `denom` no more than 2^20 - 1.
    ///
    /// The manual path, for when the exact AN619 coefficients matter more than
    /// letting [`Si5351::set_frequency`] pick them. Here 25 MHz × (35 + 1/5)
    /// puts the VCO at 880 MHz, and dividing by 88 gives 10 MHz on clk0:
    ///
    /// ```
    /// # use embedded_hal::i2c::I2c;
    /// # use si5351::{ClockOutput, Error, Multisynth, OutputDivider, Si5351, Si5351Device, PLL};
    /// # fn example<I2C: I2c>(clock: &mut Si5351Device<I2C>) -> Result<(), Error> {
    /// clock.setup_pll(PLL::A, 35, 1, 5)?;
    /// clock.setup_multisynth_int(Multisynth::MS0, 88, OutputDivider::Div1)?;
    /// clock.select_clock_pll(ClockOutput::Clk0, PLL::A);
    /// clock.set_clock_enabled(ClockOutput::Clk0, true);
    /// clock.flush_clock_control(ClockOutput::Clk0)?;
    /// clock.reset_pll(PLL::A)?;
    /// clock.flush_output_enabled()?;
    /// # Ok(())
    /// # }
    /// ```
    fn setup_pll(&mut self, pll: PLL, mult: u8, num: u32, denom: u32) -> Result<(), Error>;

    /// [`Si5351::setup_multisynth`] with a whole-number divider.
    fn setup_multisynth_int(
        &mut self,
        ms: Multisynth,
        mult: u16,
        r_div: OutputDivider,
    ) -> Result<(), Error>;

    /// Divides the PLL feeding `ms` by `div + num/denom`, then by `r_div`.
    ///
    /// `div` must be 6 to 1800 and the fraction proper. Does not reset the PLL,
    /// so an output already running keeps running; see
    /// [`Si5351::set_clock_frequency_fixed_pll`], which is this with the
    /// coefficients worked out for you.
    fn setup_multisynth(
        &mut self,
        ms: Multisynth,
        div: u16,
        num: u32,
        denom: u32,
        r_div: OutputDivider,
    ) -> Result<(), Error>;

    /// Points `clk` at `pll`. Driver state only, written by the next
    /// [`Si5351::flush_clock_control`].
    fn select_clock_pll(&mut self, clk: ClockOutput, pll: PLL);
}

impl<I2C> Si5351Device<I2C>
where
    I2C: I2c,
{
    /// Creates a new driver from a I2C peripheral
    ///
    /// `address_bit` is the state of the A0 pin, and `xtal_freq` the reference
    /// crystal in hertz — 25 or 27 MHz on every part in the wild.
    ///
    /// ```
    /// # use embedded_hal::i2c::I2c;
    /// # use si5351::{CrystalLoad, Error, Si5351, Si5351Device};
    /// # fn example<I2C: I2c>(i2c: I2C) -> Result<(), Error> {
    /// let mut clock = Si5351Device::new(i2c, false, 25_000_000);
    /// clock.init(CrystalLoad::_10pF)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(i2c: I2C, address_bit: bool, xtal_freq: u32) -> Self {
        Si5351Device {
            i2c,
            address: ADDRESS | u8::from(address_bit),
            xtal_freq,
            clk_enabled_mask: 0,
            ms_int_mode_mask: 0,
            ms_src_mask: 0,
            // Every output at 8 mA, which is what was hardcoded before
            // `set_clock_drive` existed.
            clk_drive_mask: u16::MAX,
            pll_freq: [None; 2],
        }
    }

    /// Creates a driver for an [Adafruit module]: A0 low, 25 MHz crystal.
    ///
    /// ```
    /// # use embedded_hal::i2c::I2c;
    /// # use si5351::{Error, Si5351, Si5351Device};
    /// # fn example<I2C: I2c>(i2c: I2C) -> Result<(), Error> {
    /// let mut clock = Si5351Device::new_adafruit_module(i2c);
    /// clock.init_adafruit_module()?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [Adafruit module]: https://www.adafruit.com/product/2045
    pub fn new_adafruit_module(i2c: I2C) -> Self {
        Si5351Device::new(i2c, false, 25_000_000)
    }

    /// Releases the I2C peripheral back to the caller
    ///
    /// ```
    /// # use embedded_hal::i2c::I2c;
    /// # use si5351::Si5351Device;
    /// # fn example<I2C: I2c>(clock: Si5351Device<I2C>) -> I2C {
    /// clock.release()
    /// # }
    /// ```
    pub fn release(self) -> I2C {
        self.i2c
    }

    fn write_ms_config<MS: FractionalMultisynth + Copy>(
        &mut self,
        ms: MS,
        int: u16,
        frac_num: u32,
        frac_denom: u32,
        r_div: OutputDivider,
    ) -> Result<(), Error> {
        if frac_denom == 0 {
            return Err(Error::InvalidParameter);
        }
        if frac_num > 0xfffff {
            return Err(Error::InvalidParameter);
        }
        if frac_denom > 0xfffff {
            return Err(Error::InvalidParameter);
        }
        // AN619 writes every divider as `a + b/c` with the fraction proper, and
        // the range checks in `setup_pll` and `setup_multisynth` only look at
        // the integer part, so an improper fraction would slip past them.
        // `setup_pll` also relies on it to work out where the PLL ended up.
        if frac_num >= frac_denom {
            return Err(Error::InvalidParameter);
        }

        let p1: u32;
        let p2: u32;
        let p3: u32;

        if frac_num == 0 {
            p1 = 128 * int as u32 - 512;
            p2 = 0;
            p3 = 1;
        } else {
            let ratio = (128u64 * (frac_num as u64) / (frac_denom as u64)) as u32;

            p1 = 128 * int as u32 + ratio - 512;
            p2 = 128 * frac_num - frac_denom * ratio;
            p3 = frac_denom;
        }

        self.write_synth_registers(
            ms,
            [
                ((p3 & 0x0000FF00) >> 8) as u8,
                p3 as u8,
                // AN619 register 44 and its counterparts: Rx_DIV in bits 6:4,
                // MSx_DIVBY4 in 3:2, MSx_P1[17:16] in 1:0. The feedback multisynth
                // has no R field here and is always passed `Div1`.
                ((p1 & 0x00030000) >> 16) as u8 | (r_div.bits() << 4),
                ((p1 & 0x0000FF00) >> 8) as u8,
                p1 as u8,
                (((p3 & 0x000F0000) >> 12) | ((p2 & 0x000F0000) >> 16)) as u8,
                ((p2 & 0x0000FF00) >> 8) as u8,
                p2 as u8,
            ],
        )?;

        if frac_num == 0 {
            self.ms_int_mode_mask |= ms.ix();
        } else {
            self.ms_int_mode_mask &= !ms.ix();
        }

        Ok(())
    }

    /// Which PLL currently feeds `clk`, per [`Si5351::select_clock_pll`].
    fn clock_pll(&self, clk: ClockOutput) -> PLL {
        if self.ms_src_mask & (1u8 << clk.ix()) == 0 {
            PLL::A
        } else {
            PLL::B
        }
    }

    fn read_register(&mut self, reg: Register) -> Result<u8, Error> {
        let mut buffer = [0u8; 1];
        self.i2c
            .write_read(self.address, &[reg.addr()], &mut buffer)
            .map_err(i2c_error)?;
        Ok(buffer[0])
    }

    fn write_register(&mut self, reg: Register, byte: u8) -> Result<(), Error> {
        self.i2c
            .write(self.address, &[reg.addr(), byte])
            .map_err(i2c_error)
    }

    fn write_synth_registers<MS: FractionalMultisynth>(
        &mut self,
        ms: MS,
        params: [u8; 8],
    ) -> Result<(), Error> {
        self.i2c
            .write(
                self.address,
                &[
                    ms.base_addr(),
                    params[0],
                    params[1],
                    params[2],
                    params[3],
                    params[4],
                    params[5],
                    params[6],
                    params[7],
                ],
            )
            .map_err(i2c_error)
    }
}

impl<I2C> Si5351 for Si5351Device<I2C>
where
    I2C: I2c,
{
    fn init_adafruit_module(&mut self) -> Result<(), Error> {
        self.init(CrystalLoad::_10pF)
    }

    fn init(&mut self, xtal_load: CrystalLoad) -> Result<(), Error> {
        loop {
            let device_status = self.read_device_status()?;
            if !device_status.contains(DeviceStatusBits::SYS_INIT) {
                break;
            }
        }

        self.flush_output_enabled()?;
        const CLK_REGS: [Register; 8] = [
            Register::Clk0,
            Register::Clk1,
            Register::Clk2,
            Register::Clk3,
            Register::Clk4,
            Register::Clk5,
            Register::Clk6,
            Register::Clk7,
        ];
        for &reg in CLK_REGS.iter() {
            self.write_register(reg, ClockControlBits::CLK_PDN.bits())?;
        }

        self.write_register(
            Register::CrystalLoad,
            (CrystalLoadBits::RESERVED
                | match xtal_load {
                    CrystalLoad::_6pF => CrystalLoadBits::CL_6,
                    CrystalLoad::_8pF => CrystalLoadBits::CL_8,
                    CrystalLoad::_10pF => CrystalLoadBits::CL_10,
                })
            .bits(),
        )?;

        Ok(())
    }

    fn read_device_status(&mut self) -> Result<DeviceStatusBits, Error> {
        Ok(DeviceStatusBits::from_bits_truncate(
            self.read_register(Register::DeviceStatus)?,
        ))
    }

    fn find_int_dividers_for_max_pll_freq(
        &self,
        max_pll_freq: u32,
        freq: Frequency,
    ) -> Result<(u16, OutputDivider), Error> {
        if freq.as_microhz() == 0 {
            return Err(Error::InvalidParameter);
        }

        let total_divider = Frequency::from_hz(max_pll_freq).as_microhz() / freq.as_microhz();
        if total_divider > u16::MAX as u64 {
            return Err(Error::InvalidParameter);
        }
        let total_divider = total_divider as u16;

        let r_div = OutputDivider::min_divider(total_divider / 900)?;

        let ms_div = (total_divider / (2 * r_div.denominator_u8() as u16) * 2).max(6);
        if ms_div > 1800 {
            return Err(Error::InvalidParameter);
        }

        Ok((ms_div, r_div))
    }

    /// Feedback multisynth coefficients `a`, `b` and `c` placing the PLL at
    /// `freq * total_div`.
    fn find_pll_coeffs_for_dividers(
        &self,
        total_div: u32,
        freq: Frequency,
    ) -> Result<(u8, u32, u32), Error> {
        let xtal_freq = Frequency::from_hz(self.xtal_freq).as_microhz();
        if xtal_freq == 0 {
            return Err(Error::InvalidParameter);
        }

        let pll_freq = freq
            .as_microhz()
            .checked_mul(total_div as u64)
            .ok_or(Error::InvalidParameter)?;

        let mult = u8::try_from(pll_freq / xtal_freq).map_err(|_| Error::InvalidParameter)?;
        let (num, denom) = approx_fraction(pll_freq % xtal_freq, xtal_freq, MAX_FRACTION_DENOM);

        Ok((mult, num, denom))
    }

    fn set_frequency(&mut self, pll: PLL, clk: ClockOutput, freq: Frequency) -> Result<(), Error> {
        let (ms_divider, r_div) = self.find_int_dividers_for_max_pll_freq(900_000_000, freq)?;
        let total_div = ms_divider as u32 * r_div.denominator_u8() as u32;
        let (mult, num, denom) = self.find_pll_coeffs_for_dividers(total_div, freq)?;

        let ms = clk.multisynth()?;

        self.setup_pll(pll, mult, num, denom)?;
        self.setup_multisynth_int(ms, ms_divider, r_div)?;
        self.select_clock_pll(clk, pll);
        self.set_clock_enabled(clk, true);
        self.flush_clock_control(clk)?;
        self.reset_pll(pll)?;
        self.flush_output_enabled()?;

        Ok(())
    }

    fn set_pll_frequency(&mut self, pll: PLL, freq: Frequency) -> Result<(), Error> {
        if !(MIN_PLL_FREQ..=MAX_PLL_FREQ).contains(&freq) {
            return Err(Error::InvalidParameter);
        }

        // A total divider of one: the coefficients that put the feedback
        // multisynth itself on `freq`.
        let (mult, num, denom) = self.find_pll_coeffs_for_dividers(1, freq)?;

        self.setup_pll(pll, mult, num, denom)
    }

    fn pll_frequency(&self, pll: PLL) -> Option<Frequency> {
        self.pll_freq[pll.ix()]
    }

    fn set_clock_frequency_fixed_pll(
        &mut self,
        clk: ClockOutput,
        freq: Frequency,
    ) -> Result<(), Error> {
        let ms = clk.multisynth()?;
        let pll_freq = self
            .pll_frequency(self.clock_pll(clk))
            .ok_or(Error::InvalidParameter)?
            .as_microhz();

        if freq.as_microhz() == 0 {
            return Err(Error::InvalidParameter);
        }

        let total_divider = pll_freq / freq.as_microhz();
        if total_divider > u16::MAX as u64 {
            return Err(Error::InvalidParameter);
        }
        let r_div = OutputDivider::min_divider(total_divider as u16 / 900)?;

        // Unlike set_frequency, the multisynth carries the fraction here, so
        // it is solved against the PLL rather than the other way round.
        let denom = freq
            .as_microhz()
            .checked_mul(r_div.denominator_u8() as u64)
            .ok_or(Error::InvalidParameter)?;
        let div = u16::try_from(pll_freq / denom).map_err(|_| Error::InvalidParameter)?;
        let (num, denom) = approx_fraction(pll_freq % denom, denom, MAX_FRACTION_DENOM);

        // AN619 2.1.1: below 8 the only legal MultiSynth ratios are the
        // integers 4 and 6, so a fraction there is out of spec. Reachable from
        // a parked PLL for outputs above 112.5 MHz.
        if num != 0 && div < 8 {
            return Err(Error::InvalidParameter);
        }

        self.setup_multisynth(ms, div, num, denom, r_div)?;
        self.set_clock_enabled(clk, true);
        self.flush_clock_control(clk)?;
        self.flush_output_enabled()
    }

    fn reset_pll(&mut self, pll: PLL) -> Result<(), Error> {
        self.write_register(
            Register::PLLReset,
            match pll {
                PLL::A => PLLResetBits::PLLA_RST.bits(),
                PLL::B => PLLResetBits::PLLB_RST.bits(),
            },
        )
    }

    fn set_clock_enabled(&mut self, clk: ClockOutput, enabled: bool) {
        let bit = 1u8 << clk.ix();
        if enabled {
            self.clk_enabled_mask |= bit;
        } else {
            self.clk_enabled_mask &= !bit;
        }
    }

    fn set_clock_drive(&mut self, clk: ClockOutput, drive: DriveStrength) {
        let shift = 2 * clk.ix();
        self.clk_drive_mask &= !(0b11u16 << shift);
        self.clk_drive_mask |= (drive.bits().bits() as u16) << shift;
    }

    fn flush_output_enabled(&mut self) -> Result<(), Error> {
        let mask = self.clk_enabled_mask;
        self.write_register(Register::OutputEnable, !mask)
    }

    fn flush_clock_control(&mut self, clk: ClockOutput) -> Result<(), Error> {
        let bit = 1u8 << clk.ix();
        let clk_control_pdn = if self.clk_enabled_mask & bit != 0 {
            ClockControlBits::empty()
        } else {
            ClockControlBits::CLK_PDN
        };

        let ms_int_mode = if self.ms_int_mode_mask & bit == 0 {
            ClockControlBits::empty()
        } else {
            ClockControlBits::MS_INT
        };

        let ms_src = if self.ms_src_mask & bit == 0 {
            ClockControlBits::empty()
        } else {
            ClockControlBits::MS_SRC
        };

        let drive = ClockControlBits::from_bits_truncate(
            ((self.clk_drive_mask >> (2 * clk.ix())) & 0b11) as u8,
        );

        let base = ClockControlBits::CLK_SRC_MS | drive;

        self.write_register(
            clk.register(),
            (clk_control_pdn | ms_int_mode | ms_src | base).bits(),
        )
    }

    fn setup_pll_int(&mut self, pll: PLL, mult: u8) -> Result<(), Error> {
        self.setup_pll(pll, mult, 0, 1)
    }

    fn setup_pll(&mut self, pll: PLL, mult: u8, num: u32, denom: u32) -> Result<(), Error> {
        if !(15..=90).contains(&mult) {
            return Err(Error::InvalidParameter);
        }

        self.write_ms_config(
            pll.multisynth(),
            mult.into(),
            num,
            denom,
            OutputDivider::Div1,
        )?;

        // Remember fXTAL * (mult + num/denom) so that
        // `set_clock_frequency_fixed_pll` knows what it is dividing down. The
        // microhertz it is rounded to is worth about 1e-15 of the VCO.
        let xtal_freq = Frequency::from_hz(self.xtal_freq).as_microhz();
        let pll_freq = xtal_freq * mult as u64 + scale(xtal_freq, num, denom);
        self.pll_freq[pll.ix()] = Some(Frequency::from_microhz(pll_freq));

        Ok(())
    }

    fn setup_multisynth_int(
        &mut self,
        ms: Multisynth,
        mult: u16,
        r_div: OutputDivider,
    ) -> Result<(), Error> {
        self.setup_multisynth(ms, mult, 0, 1, r_div)
    }

    fn setup_multisynth(
        &mut self,
        ms: Multisynth,
        div: u16,
        num: u32,
        denom: u32,
        r_div: OutputDivider,
    ) -> Result<(), Error> {
        if !(6..=1800).contains(&div) {
            return Err(Error::InvalidParameter);
        }

        self.write_ms_config(ms, div, num, denom, r_div)?;

        Ok(())
    }

    fn select_clock_pll(&mut self, clock: ClockOutput, pll: PLL) {
        let bit = 1u8 << clock.ix();
        match pll {
            PLL::A => self.ms_src_mask &= !bit,
            PLL::B => self.ms_src_mask |= bit,
        }
    }
}

#[cfg(test)]
mod tests;
