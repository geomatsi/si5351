# si5351

Platform-agnostic `no_std` driver for the Silicon Labs Si5351 clock generator,
built on `embedded-hal`. Fork of the upstream `ilya-epifanov/si5351` crate,
which was abandoned on `embedded-hal` 0.2 / edition 2018.

The driver is one file, `src/lib.rs`. Alongside it:

- `src/calibrate.rs` — the `calibrate` module: a gated count in, a correction
  for any output out. Pure arithmetic over `Frequency`, no I²C, so it is
  useful without a part on the bus and testable on the host.
- `src/tests.rs` — the unit tests, pulled in as `#[cfg(test)] mod tests;`. A
  child module rather than an integration test under `tests/`, because
  `approx_fraction` and the constants it is checked against are private. It
  needs its own `extern crate std;` since the crate is `no_std`.
- `examples/blue-pill/` — a **separate crate**, not this one's `examples/`
  directory. Firmware for an STM32F103 Blue Pill with an Si5351 breakout on
  PB8/PB9, matching the wiring in `ham-shack/tools/wspr-beacon--blue-pill`.
  Its `src/lib.rs` brings up the clock tree, I²C1 and the driver; the wiring
  table lives in that file's header. It also carries `Gate`, the TIM2/TIM3
  cascade that counts PB3 for `example7` — `init_with_gate` rather than `init`,
  because freeing PB3 costs the JTAG pins. Eight binaries — seven teaching examples,
  built mostly on the same four easily measured frequencies (500 kHz, 5, 15
  and 50 MHz), and one bench tool:
  - `example1` — `set_frequency` on clk0, one call per frequency.
  - `example2` — the fixed-PLL path: park the PLL once, then retune clk0
    through the same frequencies without touching it again.
  - `example3` — the four drive strengths, stepped through with
    `flush_clock_control` while the output keeps running.
  - `example4` — clk0 and clk1 together: both on PLL A, where the second
    `set_frequency` re-parks the VCO and drags the first output; then one PLL
    each, where they are independent.
  - `example5` — sub-hertz steps around 5 MHz on a parked PLL, with clk1 as a
    coherent 5 MHz reference so the offset is readable as phase slip on a
    two-channel scope. Also shows where the fractional divider runs out of
    resolution; see "Two ways to set a frequency" below.
  - `example6` — `read_device_status` as a health check: a good configuration,
    then a VCO below 600 MHz (`setup_pll_int(PLL::A, 15)`) and one above
    900 MHz (160 MHz out of clk0), then the good one again, with an `ok` or
    `UNEXPECTED` line per flag. Needs no instrument — it answers over RTT. The
    two out-of-range cases are expectations from AN619's VCO range, not
    measured behaviour; an `UNEXPECTED` there is a result for `HW_CHECK.md`.
  - `example7` — the calibration loop from [`si5351::calibrate`]: clk1 is
    counted on PB3 over a one-second gate, `error_ppb`/`correct` close the
    loop, and `correction_ppb` reports the total, which is then carried to
    clk0 — an output the counter never saw. Needs one extra wire, clk1 to PB3,
    and no instrument: it answers over RTT. The gate comes from the board's own
    delay, so what it converges is the Si5351 crystal onto the Blue Pill's;
    the number only means something once the gate is a GPS PPS.
  - `hw_checks` — a bench tool rather than a teaching example: the exact
    configurations in `examples/blue-pill/notes/HW_CHECK.md`, announced over
    RTT with what to look for. Its `ONLY` constant is `None` in the tree, which
    runs every check in turn; set it to a single check's name while working
    through that one, and put it back to `None` before committing.

  The examples deliberately have nothing to do with WSPR or any other mode:
  the frequencies are round numbers chosen to be read off a counter. The
  sub-hertz tuning that motivated the fork is shown by `example5` as an offset
  from one of them, and covered further by the unit tests and by `hw_checks`'s
  `accuracy` check.

Anything longer than a few lines belongs in one of those rather than in a doc
comment: the module header keeps only short snippets and points at them.

**Why a separate crate.** The examples are `no_std` firmware for one board and
have to be linked against a linker script; the library is host-testable and
platform-agnostic. Putting them in this crate's `examples/` would make
`cargo test` — which builds examples — fail on the host, and a default target
in `.cargo/config.toml` would break `cargo test` outright. Splitting keeps both
halves buildable with no flags. The blue-pill crate declares an empty
`[workspace]` so it never attaches to anything above it, and commits its own
`Cargo.lock` as a binary crate should.

Both reference documents are in `docs/` (untracked — they are Skyworks PDFs,
not ours to redistribute):

- `docs/si5351abc-b-datasheet.pdf` — electrical spec and architecture. It has
  **no** register bit layouts; §7 and §8 defer all of them to AN619.
- `docs/an619.pdf` — "Manually Generating an Si5351 Register Map". This is the
  authority for everything `write_ms_config`, `flush_clock_control` and the
  `Register` enum do. Consult it before touching any of them.

The numbers this driver is built on, with AN619 sections:

- §2 VCO range 600 to 900 MHz; MultiSynth output 500 kHz to 200 MHz, down to
  2.5 kHz with the R dividers.
- §3.2 feedback MultiSynth `a + b/c`, `a` from 15 to 90, `c` up to 1,048,575.
- §4.1.2 output MultiSynth `a + b/c` "between 6 and 1800", with the same
  `P1`/`P2`/`P3` packing. Note §2.1.1 states the range differently — "4, 6, 8,
  and any fractional value between 8 + 1/1,048,575 and 900" — so AN619
  contradicts itself on the upper bound and on whether a fraction below 8 is
  legal. The driver follows §4.1.2's 6 to 1800;
  `set_clock_frequency_fixed_pll` additionally refuses a fractional divider
  below 8, per §2.1.1.
- §4.1.3 an output above 150 MHz needs the divide-by-4 mode (`P1`=0, `P2`=0,
  `P3`=1, `MSx_INT`=1, `MSx_DIVBY4`=11b) and `fVCO = fOUT * 4`. Not
  implemented — see the VCO range item below.
- §9 register 0, the device status: `SYS_INIT` at bit 7, `LOL_B` and `LOL_A` at
  6:5, `LOS` at bit 4 — which is **CLKIN** loss of signal, Si5351C only, and so
  says nothing about the crystal on an A part — and bits 3:0 "Reserved". Two
  documents disagree with that last part. The datasheet §4 puts `LOS_XTAL`, the
  crystal's own loss-of-signal flag, at reg0[3]; AN619's own register map
  summary puts `REVID[1:0]` at bits 1:0. `DeviceStatusBits` models only the four
  names AN619 §9 gives, and `read_device_status` uses `from_bits_truncate`, so
  both of those are dropped — see the open item below.

## Toolchain and dependencies

- Edition 2024, `rust-version = "1.85"` (MSRV — keep both in sync).
- `embedded-hal = "1.0.0"` — the unified blocking `i2c::I2c` trait.
- `bitflags = "2.13.1"`.
- `Cargo.lock` **is** committed. Current Cargo guidance is to check it in for
  every project, libraries included: it does not constrain downstream consumers
  (only `Cargo.toml` does), and it makes CI and `git bisect` deterministic.
  CI builds with `--locked`, so a `Cargo.toml` change must come with the
  regenerated lockfile in the same commit.

## Build and verify

`.github/workflows/ci.yml` runs all of the below on push and PR to `master`,
with `RUSTFLAGS`/`RUSTDOCFLAGS` set to `-D warnings`. Run the same thing
locally before committing:

```sh
export RUSTFLAGS="-D warnings" RUSTDOCFLAGS="-D warnings"

cargo build
cargo test              # unit tests plus doctests; the doctests must compile
cargo clippy --all-targets
cargo doc --no-deps     # catches broken intra-doc links

for t in thumbv6m-none-eabi thumbv7m-none-eabi thumbv7em-none-eabihf riscv32imac-unknown-none-elf; do
    cargo build --target "$t"
done
```

Those four bare-metal targets are installed locally and are the de-facto
support matrix. `thumbv6m` matters most: it has no hardware division and only
32-bit atomics, so it catches accidental use of anything too heavy. CI also
builds once on the pinned MSRV toolchain.

The examples are built separately, from their own directory:

```sh
cd examples/blue-pill
cargo build --release           # thumbv7m by default, from .cargo/config.toml
cargo clippy --all-targets
cargo run --bin example1        # needs probe-rs and an SWD probe
```

Two things there are easy to get wrong:

- The linker script is applied by `build.rs`
  (`cargo:rustc-link-arg-bins=-Tlink.x`), **not** by `rustflags` in
  `.cargo/config.toml`. A `RUSTFLAGS` in the environment replaces config
  rustflags wholesale, and losing `-Tlink.x` does not fail the link — it
  produces a binary with no vector table that the linker garbage-collects to
  nothing, which `size` reports as zero `.text`. CI exports `RUSTFLAGS`, so
  this is not hypothetical.
- `[lib]` and every `[[bin]]` set `test = false` and `bench = false`. Without
  that, `--all-targets` tries to build a test harness for the target and fails
  on the missing `test` crate.

The board has 64K of flash. An unoptimised build of anything that prints does
not fit, so `[profile.dev]` uses `opt-level = "z"` and LTO.

Leave the result in the working tree. Do not `git commit`, stage, or push
anything unless explicitly asked to.

The tree is **not** `rustfmt`-clean — the upstream formatting was left as-is
during the port, so `cargo fmt --check` fails and is deliberately not in CI.
Reformatting is a separate, whole-file decision; do not let `cargo fmt` sneak
into an unrelated change.

## Conventions

- `#![no_std]` at the top of `src/lib.rs`. There is deliberately no
  `#![deny(warnings)]` — warnings are denied through CI env vars instead, so a
  future rustc lint cannot break downstream builds. Do not add it back.
  `#![deny(missing_docs)]` is commented out — most public items are still
  undocumented, so it cannot be enabled without a docs pass first.
- The Apache-2.0 §4(b) modification notice in the `src/lib.rs` header comment
  is a licensing requirement, not a stale comment. Leave it in place.
- Doc examples are generic over `I2C: I2c` and return `Result<_, Error>` so
  that they compile as real doctests without a HAL or a mock. Every major API
  function carries one; the imports and the `fn` wrapper are hidden behind `#`
  so only the calls render. Keep them compiling — do not degrade them to
  ```ignore. Keep them short, too: a worked-through use case goes in
  `examples/blue-pill`, and the docs link to it. `RUSTDOCFLAGS` reaches the
  doctest `rustdoc` invocation as well as `cargo doc`, so an unused import or
  an unused `let` in an example fails CI; write value-returning examples as the
  wrapper's tail expression rather than binding them.
- An enum variant standing for a physical quantity carries its unit:
  `DriveStrength::_2mA` … `_8mA`, `CrystalLoad::_6pF` … `_10pF`. The leading
  underscore is there because an identifier cannot begin with a digit, and the
  unit suffix must stay mixed-case (`mA`, not `MA` or `_ma`) —
  `non_camel_case_types` rejects consecutive capitals and interior underscores,
  and CI denies warnings, so datasheet-style names like `IDRV_8ma` would need
  an `#[allow]`. Both types name their AN619 register field (`CLKx_IDRV`,
  `XTAL_CL`) in the type-level doc instead, so the datasheet spelling stays
  greppable.
- Register addresses and bit layouts live in the `Register` enum and the
  `bitflags!` blocks. Add new registers there rather than inlining magic
  numbers at call sites.
- `bitflags` 2 does not auto-derive: every `bitflags!` struct needs an explicit
  `#[derive(Debug, Copy, Clone, PartialEq, Eq)]`.
- `Error` is deliberately non-generic (no `E` type parameter), so the I2C error
  is discarded by `i2c_error()`.

## Nothing here has been tested on a part

The unit tests decode the register bytes back into a frequency and check them
against AN619, which catches arithmetic and packing errors but says nothing
about how the chip behaves. `examples/blue-pill/notes/HW_CHECK.md` is the bench
checklist — what to measure, what result would confirm or refute each
assumption, and which `hw_checks` check produces the configuration. Keep it
current: when a behavioural claim goes into a doc comment or into this file, it
belongs there too until someone has measured it.

## Sub-1 MHz outputs are unverified on hardware

`write_ms_config()` used to OR the R divider into the third synth byte at bits
1:0, where AN619 register 44 has `MSx_P1[17:16]`; `Rx_DIV` is at bits 6:4. Any
divider but `Div1` therefore failed to set R *and* corrupted the top of P1.
Fixed by shifting, and `low_frequencies_use_the_r_divider` guards it.

Nothing at or above about 1 MHz ever changed — `min_divider(total / 900)`
returns `Div1` there, and the feedback multisynth has no R field in that byte —
but **outputs below 1 MHz now emit different bytes than any released version of
this driver ever did, and no one has put a scope on one.** Worth checking
before relying on that range.

## Known bug (pre-existing, deliberately unfixed)

`write_ms_config()` in `src/lib.rs` updates the integer-mode mask with

```rust
self.ms_int_mode_mask |= ms.ix();
```

but `ix()` is a bit *index*, not a mask, and every reader of that field tests
`mask & (1 << clk.ix())` (see `flush_clock_control`). Consequences:

- `MS0` has `ix() == 0`, so `|= 0` is a no-op — clk0 never gets `MS_INT`.
- `MS1`..`MS5` set the bit of the previous clock.
- The clear path is worse than the set path: `&= !ms.ix()` clears every bit of
  `ix()`, so `MS2` (`ix() == 2`) clears clk1's bit, `MS3` clears clk0's and
  clk1's, and so on.

The fix is `1 << ms.ix()` on both branches, and nothing more. The mapping of
`MSNA`/`MSNB` to indices 6 and 7 is *right*, contrary to what this file said
before AN619 was to hand: PLL integer mode lives in `FBA_INT` and `FBB_INT`,
which are bit 6 of registers 22 and 23 — the clk6 and clk7 control registers.
Index 6 and 7 land there exactly. In practice nothing writes them anyway, since
the driver only ever flushes the control register of the clock being tuned.

One nuance for whoever fixes this: AN619 §3.2.1 and §4.1.2.1 both say integer
mode applies when the ratio is an **even** integer. `write_ms_config` keys off
`frac_num == 0`, which is any integer.

Left alone because it changes what is actually written to the `MS_INT` bit on
real hardware, which needs verification against a scope / spectrum analyzer.
Ask before "fixing" it.

## Two ways to set a frequency

`set_frequency` retunes the PLL and resets it, which restarts the divider chain
and jumps the output phase. For anything that shifts frequency while
transmitting, the sequence is `set_pll_frequency` once, `reset_pll` once, then
`set_clock_frequency_fixed_pll` per step — that writes only the output
MultiSynth block, so the output runs through the change. There the *output*
divider carries the fraction and the PLL is fixed, the reverse of
`set_frequency`.

`setup_pll` records the resulting VCO frequency in `pll_freq` (rounded to the
microhertz, worth about 1e-15 of the VCO), because
`set_clock_frequency_fixed_pll` has to know what it is dividing. Any new path
that programs the feedback multisynth must go through `setup_pll` or keep that
field up to date, or the fixed-PLL path will divide from a stale number.

**Where the PLL is parked decides how fine the steps can be.** On the fixed-PLL
path the output divider is `a + b/c` with `c` at most 2^20-1, so the reachable
frequencies crowd around fractions with a large `c` and thin out around simple
ones — and the gap beside `b/c = 1` is the widest there is. A park that divides
evenly into the target lands on exactly that gap: from 900 MHz, 5 MHz needs a
divider of 180, and a step of less than about 26 mHz cannot be expressed at all.
Upwards the closest fraction is the improper `1/1`, which `write_ms_config`
rejects, so the call returns `InvalidParameter`; downwards it is `0/1`, which is
legal, so the driver writes the same eight bytes as for 5 MHz and returns `Ok`
— a silent no-op. Parking at 888 MHz instead gives a divider of 177 + 3/5 and
steps good to a microhertz. `examples/blue-pill`'s `example5` demonstrates both,
and the numbers above come from the driver's own arithmetic, not from a
measurement.

## Open items

- **Device status is missing `LOS_XTAL`**: on a crystal-only A part, reg0[3] is
  the one flag that says the reference has failed, and `DeviceStatusBits` has no
  variant for it — `from_bits_truncate` drops it, so `read_device_status` cannot
  see a dead crystal. Adding the variant is additive to the API but changes what
  the function returns, and the two documents disagree about whether the bit
  exists at all (see the register 0 item above), so it wants a part to test
  against: shorting or unpowering the crystal on a breakout should set it.
  `examples/blue-pill`'s `example6` documents the gap where it would bite.
- **Error type**: `Error::CommunicationError` throws away the underlying I2C
  error. `embedded-hal` 1.0 offers a non-generic `i2c::ErrorKind` reachable via
  `Error::kind()` on any I2C error, so the cause could be preserved without
  making `Error` generic. This is a breaking API change.
- **Publishing**: `version` is still upstream's `0.2.0`, and the crate name
  `si5351` is taken on crates.io by the upstream crate. Publishing this fork
  requires a rename and a version decision; `authors`/`repository` in
  `Cargo.toml` already point at this fork.
- **Test suite**: the divider math (`approx_fraction`,
  `find_int_dividers_for_max_pll_freq`, `find_pll_coeffs_for_dividers`),
  `Frequency`, the fixed-PLL path and the drive strength bits are covered by
  the unit tests. They drive the trait through a `FakeI2c` that records
  register writes, and decode the multisynth blocks back to a frequency, R
  divider included, so `write_ms_config`'s packing is checked too. Still
  uncovered: `OutputDivider::min_divider` on its own, `init`, and clk6/clk7.
- **Frequency accuracy tail**: `set_frequency` picks one output divider and
  makes the PLL fraction fit it. That is within microhertz of the target
  normally and a few millihertz at worst, but a target sitting just beside a
  ratio with a small denominator (a wide gap in the Farey sequence of order
  1048575) can land a good deal further out, up to the ~0.19 Hz the old fixed
  denominator gave. Trying the handful of other output dividers that keep the
  VCO in range and keeping the closest would remove that tail; measured over
  random HF targets it takes the worst case from ~3 mHz to ~0.1 mHz.
- **VCO range**: `find_int_dividers_for_max_pll_freq` clamps the MultiSynth
  divider to at least 6 (`.max(6)`), so a target above 150 MHz silently asks
  the VCO for more than its 900 MHz ceiling instead of erroring. Pre-existing.
  Doing it properly means implementing AN619 §4.1.3's divide-by-4 mode, which
  needs a `MSx_DIVBY4` field in bits 3:2 of the third synth byte, next to the R
  divider.
