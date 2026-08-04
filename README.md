# si5351

A platform agnostic driver for the Silicon Labs Si5351 clock generator, built on
[`embedded-hal`](https://github.com/rust-embedded/embedded-hal) 1.0.

This is a modified fork of [ilya-epifanov/si5351](https://github.com/ilya-epifanov/si5351).
The original crate targeted `embedded-hal` 0.2, `bitflags` 1 and Rust edition 2018;
this fork has been ported to `embedded-hal` 1.0, `bitflags` 2 and Rust edition 2024.
See the git history for the full list of changes.

The Si5351 datasheet and application note AN619 (which documents the MultiSynth
divider equations this driver implements) are not redistributed here — download
them from
[Skyworks](https://www.skyworksinc.com/-/media/Skyworks/SL/documents/public/data-sheets/Si5351-B.pdf).

## License

Licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE))
 * MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Modifications made in this fork are released under the same dual license. The
original copyright notice (Copyright 2018 Ilya Epifanov) is retained.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.
