# Building and testing peiso

## Building

[pekit](https://learn.peios.org/pekit) builds peiso as a static,
position-independent musl program with full RELRO, which runs on any build
host:

```sh
pekit build peiso
```

The rust library's pinned Rust and the gcc library's Bootlin toolchain
build it, so no Rust or C toolchain is needed on the host. `pekit.lock`
pins the libraries.

peiso takes ppk's crates by git, from `peios/ppk`. `Cargo.toml` and
`pekit.lock` pin ppk at the same commit: `pekit.lock` imports ppk's GCSL
library, whose `crate_settings()` gives ppk's crates what they need to
build. To move to another ppk commit, change both pins together:

1. Change the `rev` of ppk in `Cargo.toml`, and run `cargo update -p ppk`
   to update `Cargo.lock`.
2. Change ppk's source `rev` in `pekit.lock`, remove ppk's entry from its
   `libraries`, and run `pekit lock`, which pins it again at that commit.
   `pekit lock --update` would pin every other library again too.

## Tests

```sh
tools/check
```

This checks style with the pinned PSC tools, then runs `pekit test`. It
builds peiso, runs its unit tests and its integration test, and checks it
with Clippy, with every warning denied. Prepare the style tools once,
outside the repository:

```sh
python3 tools/prepare-style-tools.py ~/.cache/peios/peiso-style
export PEIOS_STYLE_BIN=~/.cache/peios/peiso-style/bin
```

The integration test builds ISOs end to end. It runs the same tools as
peiso: dosfstools's `mkfs.vfat`, mtools's `mmd` and `mcopy`, and GNU
xorriso. `iso-tools.gcsl` builds each statically from its pinned release,
whose signature was checked when it was pinned. The test needs none of
them on the host.

GitHub CI checks style only. Builds and tests run with pekit on the build
server and with `tools/check` locally.
