# Hardware verification

The unit tests decode the register bytes back into a frequency, which catches
arithmetic and packing errors but says nothing about how the chip behaves.
Nothing here has been tested on a part. This is what a bench session should
cover, in order of how much rests on the answer.

`examples/blue-pill`'s `hw_checks` binary produces every configuration below
except the two marked *(no check yet)*. Wiring is in that crate's `src/lib.rs`;
flashing needs `probe-rs` and an SWD probe.

```sh
cd examples/blue-pill
cargo run --bin hw_checks     # all of them; set ONLY in src/bin/hw_checks.rs for one
```

Each configuration is held on the pin for ten seconds and announced over RTT
with what to look for. A dark LED means the firmware hung — nearly always
`init`'s first I²C write, on swapped SDA/SCL or an unpowered module.

A scope and a frequency counter cover everything but check 3, which wants a
disciplined reference, and check 7, which wants a jitter measurement.

---

## Blocking — these decide whether documented behaviour is real

### 1. Sub-1 MHz outputs and the R divider — `r-divider`

R was being written into `MSx_P1[17:16]` instead of `Rx_DIV`. Fixed and unit
tested against AN619 register 44, but **no released version of this driver ever
produced a correct sub-1 MHz output**.

| target | R | if R fails |
| --- | --- | --- |
| 500 kHz | ÷2 | 1 MHz |
| 100 kHz | ÷16 | 1.6 MHz |
| 32.768 kHz | ÷32 | 1.048576 MHz |
| 20 kHz | ÷64 | 1.28 MHz |

- [ ] each measures its target, not the R-times-faster value
- [ ] 20 kHz waveform is clean — R is the last stage, so check duty cycle
- [ ] the same targets through `set_clock_frequency_fixed_pll`, which derives R
      by its own arithmetic and shares none of it with `set_frequency`
      *(no check yet)*

### 2. Retuning with no PLL reset — `fixed-pll`

Neither AN619 nor the datasheet says whether writing an output MultiSynth takes
effect without a PLL reset. Everything beacon-shaped depends on it not needing
one.

- [ ] each of four 1.46 Hz steps moves the output
- [ ] no dropout or squelch at the transition — scope the carrier, or listen on
      a receiver in CW
- [ ] carrier phase is continuous across a step, which is the property WSPR
      needs and the reason not to reset
- [ ] a large jump on the same parked PLL (14.0971 → 12.5 MHz) settles too; if
      only small changes work, the API needs a documented limit

### 3. Frequency accuracy — `accuracy`, *needs a disciplined reference*

- [ ] 14_097_100 Hz: record the error. It will be the crystal's, tens of ppm,
      and it is the input to any future calibration API
- [ ] the four WSPR tones are spaced 1.46484375 Hz to within ~10 mHz. Spacing is
      what WSPR decodes on and is independent of crystal error, so it is the
      real test of the arithmetic
- [ ] a fractional target through `set_frequency`, where the free-denominator
      approximation lands on the *PLL* side — the reason this fork exists, and
      the check only measures a whole number of hertz there *(no check yet)*
- [ ] optional: decode a full frame with `wsprd` or WSJT-X

---

## Correctness of the multi-output model

### 4. One PLL feeding two outputs — `shared-pll`

`set_frequency` re-parks the PLL on every call, so a second call leaves the
first output dividing against a VCO that has moved. Nothing in the API prevents
it.

- [ ] `set_frequency(clk0, 10 MHz)` then `(clk1, 12 MHz)`, both on PLL A: clk1
      is 12 MHz and **clk0 has drifted to 9.8667 MHz**. If clk0 survives, this
      driver's model of the part is wrong and the docs need revising
- [ ] that second call also jumps clk0's phase, per the reset warning on
      `set_frequency` — scope both, trigger on clk0
- [ ] the intended flow (`set_pll_frequency` once, then
      `set_clock_frequency_fixed_pll` per output) holds both at once, and
      retuning clk0 that way leaves clk1 undisturbed in phase

### 5. Per-PLL reset isolation — `pll-isolation`

Register 177 has one bit per PLL and the driver writes only one.

- [ ] clk0 on PLL A and clk2 on PLL B run correctly together
- [ ] retuning clk0 leaves clk2 unchanged in frequency **and** phase — scope
      both, trigger on clk2. Confirms the advice that outputs needing
      independent retuning belong on different PLLs

### 6. Drive strength — `drive`

New API, never measured. `CLKx_IDRV` is 2/4/6/8 mA.

- [ ] amplitude into 50 Ω rises across the four settings
- [ ] two outputs at different settings differ — it is per output, not global
- [ ] rise/fall times change with it, as the register name implies

---

## Open questions the datasheet and AN619 do not settle

### 7. Is `MS_INT` worth fixing? — `ms-int`, *needs a jitter measurement*

`ms_int_mode_mask` is indexed with a bit *number* instead of a bit, so the
integer-mode bit lands on the wrong clocks. Deliberately unfixed, because
whether it matters is a measurement. At 10 MHz on clk0 with MS0 = 90, an even
integer:

- [ ] reg 16 = `0x0f`, bit 6 clear: measure close-in phase noise and period
      jitter
- [ ] the same 10 MHz with reg 16 = `0x4f`, which the check reaches by writing
      an integer divider to MS1 — that sets the mask bit belonging to clk0.
      Exploiting the bug is the only route to that state through this driver.
      AN619 §3.2.1 and §4.1.2.1 claim integer mode improves jitter here
- [ ] the check ends with reg 17 = `0x4f` on MS1 = **75, an odd integer**, which
      AN619 sanctions only for even dividers: confirm clk1 is still 12 MHz and
      not degraded
- [ ] the result decides whether the fix (`1 << ms.ix()`, plus an evenness test)
      is worth the register-behaviour change

### 8. Above 150 MHz — `over-150mhz`

AN619 §4.1.3 requires divide-by-4 mode above 150 MHz. The driver does not
implement it and clamps the MultiSynth divider at 6, so a 160 MHz request asks
the VCO for 960 MHz.

- [ ] 150 MHz (VCO exactly 900 MHz, MS = 6) is clean — the top of what works
- [ ] 160 MHz: confirm `LOL_A` is set in the status the check prints, and record
      what the output does. This is the acceptance test for a future
      divide-by-4 implementation

### 9. AN619 contradicts itself on the MultiSynth range

§2.1.1 says the valid ratios are "4, 6, 8, and any fractional value between
8 + 1/1,048,575 and 900"; §4.1.2 says "between 6 and 1800". The driver takes the
ceiling from one and the floor from the other, so both halves need resolving.

- [ ] `fractional-low`: MS0 = 7 + 1/3 off a 900 MHz PLL — does 122.727 MHz come
      out clean? Then §2.1.1's floor of 8 is advisory and the guard in
      `set_clock_frequency_fixed_pll` can go. MS0 = 8 + 1/3 → 108 MHz is the
      control, legal under both readings
- [ ] `ms-over-900`: MS0 = 1200 off 900 MHz → 750 kHz. Clean means §4.1.2's
      ceiling of 1800 is the real one; wrong means `setup_multisynth`'s range
      check should come down to 900

---

## Range and edges

### 10. Accepted frequency range — `limits`

- [ ] 13_733 Hz is the lowest the driver accepts and comes out unaliased. The
      floor is ours, not the chip's — the total divider has to fit a `u16` —
      while the part goes to 8 kHz
- [ ] the check also prints what 160 and 200 MHz do; 150 MHz is the top, per
      check 8

---

## Calibration

### 11. The calibration loop — `example7`, *needs a disciplined reference*

`si5351::calibrate` turns a gated count into a correction for any output. The
loop is exercised by `example7` rather than by `hw_checks`, since it needs a
counter wired to PB3 — see that crate's `src/lib.rs` for the extra wire.

- [ ] as it stands, the gate is the Blue Pill's own 8 MHz crystal, so the
      readings are the difference between two ±30 ppm parts. Confirm the loop
      converges at all and that the total it reports is stable round to round;
      a total that wanders by more than a few ppm is the gate, not the driver
- [ ] gate against a GPS PPS instead — a rising edge to start and the next to
      stop, in place of the delay in `Gate::count` — and the total becomes the
      Si5351 crystal's real error. That is the number worth keeping
- [ ] with a real reference, check the payoff: clk0 is asked for 15 MHz
      pre-distorted by the correction clk1 measured, and should land within the
      counter's resolution of 15 MHz. This is the whole claim of the module —
      one measurement corrects an output that was never measured
- [ ] `calibrate::correct` linearises, leaving `freq * (ppb/1e9)^2`: 54 mHz at
      62 ppm on 20 m, and below a millihertz once the loop has closed
