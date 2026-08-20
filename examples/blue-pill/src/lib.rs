//! Board setup shared by the examples in `src/bin`.
//!
//! # Wiring
//!
//! An Si5351 breakout on I²C1, remapped to the pins the reference beacon uses:
//!
//! | Blue Pill | Si5351 module |
//! | --- | --- |
//! | PB8 | SCL |
//! | PB9 | SDA |
//! | 3V3 | VIN |
//! | GND | GND |
//!
//! `example7` counts an output as well, and wants one more wire:
//!
//! | Blue Pill | Si5351 module |
//! | --- | --- |
//! | PB3 | CLK1 |
//!
//! The module carries its own 25 MHz crystal and its own pull-ups, so nothing
//! else is needed. Outputs are clk0 and clk1 on the breakout headers; terminate
//! whatever you probe into 50 Ω.
//!
//! PC13 drives the onboard LED. Every example blinks it, so a dark board means
//! the firmware never got past `init` — almost always the I²C write in
//! [`Si5351::init`] failing because SDA and SCL are swapped or the module is
//! unpowered.

#![no_std]

use stm32f1xx_hal::{
    gpio::{Output, PushPull, gpioc::PC13},
    i2c::{BlockingI2c, DutyCycle, Mode},
    pac,
    prelude::*,
    rcc,
    timer::{SysDelay, Timer},
};

use si5351::{Si5351, Si5351Device};

/// The I²C bus the driver takes ownership of.
pub type Bus = BlockingI2c<pac::I2C1>;

/// Ready to use: the driver, a blocking delay, and the onboard LED.
pub struct Board {
    pub clock: Si5351Device<Bus>,
    pub delay: SysDelay,
    pub led: PC13<Output<PushPull>>,
}

/// A 32-bit counter for whatever is wired to PB3, for `example7`.
///
/// Every timer on an F103 is 16 bit, so a second one carries the top half:
/// TIM2 is clocked by the pin (external clock mode 1 on TI2, which is PB3 at
/// TIM2_REMAP = 01) and TIM3 counts TIM2's overflows over ITR1. See RM0008
/// §15.3.3 and Tables 45 and 86.
pub struct Gate {
    lo: pac::TIM2,
    hi: pac::TIM3,
}

impl Gate {
    /// Counts edges on PB3 for one second.
    ///
    /// The gate is the board's own delay, so this measures the Si5351 against
    /// the Blue Pill's 8 MHz crystal — two ±30 ppm parts, not a reference.
    pub fn count(&self, board: &mut Board) -> u32 {
        // The slave has to be running before the master sends it events.
        self.hi.arr().write(|w| w.arr().set(u16::MAX));
        self.hi.smcr().write(|w| w.ts().itr1().sms().ext_clock_mode());
        self.hi.cr1().write(|w| w.cen().set_bit());

        self.lo.arr().write(|w| w.arr().set(u16::MAX));
        self.lo
            .ccmr1_input()
            .write(|w| w.cc2s().ti2().ic2f().no_filter());
        self.lo.ccer().write(|w| w.cc2p().clear_bit());
        self.lo.smcr().write(|w| w.sms().ext_clock_mode().ts().ti2fp2());
        self.lo.cr2().write(|w| w.mms().update());
        // URS so that only a real overflow reaches TRGO.
        self.lo.cr1().write(|w| w.urs().set_bit().cen().set_bit());

        board.wait_ms(1_000);

        // Stopping the master first is what makes the pair safe to read: with
        // it halted, neither half can move between the two reads.
        self.lo.cr1().modify(|_, w| w.cen().clear_bit());
        self.hi.cr1().modify(|_, w| w.cen().clear_bit());

        let ticks = ((self.hi.cnt().read().cnt().bits() as u32) << 16)
            | self.lo.cnt().read().cnt().bits() as u32;

        self.lo.cnt().write(|w| w.cnt().set(0));
        self.hi.cnt().write(|w| w.cnt().set(0));

        ticks
    }
}

/// Brings up the clock tree, I²C1 and the Si5351, and returns the lot.
///
/// Clocks match the reference beacon: 8 MHz HSE, 32 MHz core, 16 MHz APB1.
/// The Si5351 is initialised as an Adafruit-style module — address 0x60, a
/// 25 MHz crystal and a 10 pF internal load.
///
/// # Panics
///
/// If the peripherals have already been taken, or if the Si5351 does not
/// answer on the bus.
pub fn init() -> Board {
    bring_up(false).0
}

/// [`init`], plus the PB3 counter `example7` measures with.
///
/// Freeing PB3 costs the JTAG pins — SWD survives, so the probe stays
/// attached — which is why the plain [`init`] does not do it.
pub fn init_with_gate() -> (Board, Gate) {
    let (board, gate) = bring_up(true);

    (board, gate.unwrap())
}

fn bring_up(with_gate: bool) -> (Board, Option<Gate>) {
    let cp = cortex_m::Peripherals::take().unwrap();
    let dp = pac::Peripherals::take().unwrap();

    let mut flash = dp.FLASH.constrain();
    let mut rcc = dp.RCC.freeze(
        rcc::Config::hse(8.MHz()).sysclk(32.MHz()).pclk1(16.MHz()),
        &mut flash.acr,
    );

    let mut afio = dp.AFIO.constrain(&mut rcc);
    let mut gpiob = dp.GPIOB.split(&mut rcc);
    let mut gpioc = dp.GPIOC.split(&mut rcc);

    // I2C1 remapped from PB6/PB7 to PB8/PB9.
    let scl = gpiob.pb8;
    let sda = gpiob.pb9;

    let i2c = dp.I2C1.remap(&mut afio.mapr).blocking_i2c(
        (scl, sda),
        Mode::Fast {
            frequency: 400.kHz(),
            duty_cycle: DutyCycle::Ratio2to1,
        },
        &mut rcc,
        1000,
        10,
        1000,
        1000,
    );

    let mut clock = Si5351Device::new_adafruit_module(i2c);
    clock.init_adafruit_module().unwrap();

    let gate = if with_gate {
        let gpioa = dp.GPIOA.split(&mut rcc);
        let (_pa15, pb3, _pb4) = afio.mapr.disable_jtag(gpioa.pa15, gpiob.pb3, gpiob.pb4);

        // TIM2_REMAP = 01 puts CH2 on PB3. This goes through `modify_mapr` and
        // not the PAC: MAPR's swj_cfg reads back undefined, so a plain
        // read-modify-write can put JTAG back.
        afio.mapr
            .modify_mapr(|_, w| unsafe { w.tim2_remap().bits(0b01) });

        // The Si5351 drives this pin and does not want a pull.
        let _clk = pb3.into_floating_input(&mut gpiob.crl);

        Some(Gate {
            lo: Timer::new(dp.TIM2, &mut rcc).release(),
            hi: Timer::new(dp.TIM3, &mut rcc).release(),
        })
    } else {
        None
    };

    let board = Board {
        clock,
        delay: cp.SYST.delay(&rcc.clocks),
        led: gpioc.pc13.into_push_pull_output(&mut gpioc.crh),
    };

    (board, gate)
}

impl Board {
    /// Leaves whatever was just programmed on the pin for `ms`, blinking the
    /// LED `blinks` times first so it is obvious the board has not hung.
    ///
    /// The blink pattern also numbers the step, which is handy when the RTT
    /// terminal is not in view.
    pub fn hold(&mut self, blinks: u8, ms: u32) {
        for _ in 0..blinks {
            self.led.set_low();
            self.delay.delay_ms(60u32);
            self.led.set_high();
            self.delay.delay_ms(140u32);
        }

        self.wait_ms(ms);
    }

    /// Blocking delay, saving every example an import of the `DelayNs` trait.
    pub fn wait_ms(&mut self, ms: u32) {
        // SysDelay tops out well below a second per call at 32 MHz.
        for _ in 0..ms / 100 {
            self.delay.delay_ms(100u32);
        }
        self.delay.delay_ms(ms % 100);
    }
}
