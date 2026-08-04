# si5351

Platform-agnostic `no_std` driver for the Silicon Labs Si5351 clock generator,
built on `embedded-hal`. Fork of the upstream `ilya-epifanov/si5351` crate,
which was abandoned on `embedded-hal` 0.2 / edition 2018.

Single source file: `src/lib.rs`. No examples, no test suite (yet).

Datasheet: <https://www.skyworksinc.com/-/media/Skyworks/SL/documents/public/data-sheets/Si5351-B.pdf>
Register programming is driven by AN619 (Si5351 register map / MultiSynth
divider equations) — consult it before touching `write_ms_config`.

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
cargo test              # doctests only; they must compile
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
- Doc examples in the module header are generic over `I2C: I2c` and return
  `Result<_, Error>` so that they compile as real doctests without a HAL or a
  mock. Keep them compiling — do not degrade them to ```ignore.
- Register addresses and bit layouts live in the `Register` enum and the
  `bitflags!` blocks. Add new registers there rather than inlining magic
  numbers at call sites.
- `bitflags` 2 does not auto-derive: every `bitflags!` struct needs an explicit
  `#[derive(Debug, Copy, Clone, PartialEq, Eq)]`.
- `Error` is deliberately non-generic (no `E` type parameter), so the I2C error
  is discarded by `i2c_error()`.

## Known bug (pre-existing, deliberately unfixed)

`write_ms_config()` in `src/lib.rs` updates the integer-mode mask with

```rust
self.ms_int_mode_mask |= ms.ix();
```

but `ix()` is a bit *index*, not a mask, and every reader of that field tests
`mask & (1 << clk.ix())` (see `flush_clock_control`). Consequences:

- `MS0` has `ix() == 0`, so `|= 0` is a no-op — clk0 never gets `MS_INT`.
- `MS1`..`MS5` set the bit of the previous clock.
- The PLL path passes `MSNA`/`MSNB` (`ix()` 6 and 7), writing into the bits
  that belong to clk6/clk7.

The likely fix is `1 << ms.ix()` plus not touching the mask for the feedback
multisynths at all. Left alone because it changes what is actually written to
the `MS_INT` bit on real hardware, which needs verification against a scope /
spectrum analyzer. Ask before "fixing" it.

## Open items

- **Error type**: `Error::CommunicationError` throws away the underlying I2C
  error. `embedded-hal` 1.0 offers a non-generic `i2c::ErrorKind` reachable via
  `Error::kind()` on any I2C error, so the cause could be preserved without
  making `Error` generic. This is a breaking API change.
- **Publishing**: `version` is still upstream's `0.2.0`, and the crate name
  `si5351` is taken on crates.io by the upstream crate. Publishing this fork
  requires a rename and a version decision; `authors`/`repository` in
  `Cargo.toml` already point at this fork.
- **Test suite**: there are no unit tests. The divider math
  (`OutputDivider::min_divider`, `find_int_dividers_for_max_pll_freq`,
  `find_pll_coeffs_for_dividers`) is pure and testable without hardware;
  register writes could be covered with `embedded-hal-mock` as a
  dev-dependency.
