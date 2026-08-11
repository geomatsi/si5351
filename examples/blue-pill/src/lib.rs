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
    timer::SysDelay,
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
    let cp = cortex_m::Peripherals::take().unwrap();
    let dp = pac::Peripherals::take().unwrap();

    let mut flash = dp.FLASH.constrain();
    let mut rcc = dp.RCC.freeze(
        rcc::Config::hse(8.MHz()).sysclk(32.MHz()).pclk1(16.MHz()),
        &mut flash.acr,
    );

    let mut afio = dp.AFIO.constrain(&mut rcc);
    let gpiob = dp.GPIOB.split(&mut rcc);
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

    Board {
        clock,
        delay: cp.SYST.delay(&rcc.clocks),
        led: gpioc.pc13.into_push_pull_output(&mut gpioc.crh),
    }
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
