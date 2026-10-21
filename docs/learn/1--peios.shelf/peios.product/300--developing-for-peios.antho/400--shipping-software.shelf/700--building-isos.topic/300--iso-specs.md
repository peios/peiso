---
title: ISO Specs
type: reference
description: Every key of the TOML spec peiso builds an ISO from, with its
  default, and the keys peiso understands but refuses until Peios can do
  what they ask.
related:
  - peios/building-isos/building-an-iso
  - peios/building-isos/isos
---

An ISO spec is a TOML file that says which packages an ISO carries, where
they come from, and where each goes. Paths in it are relative to the
spec's directory. Any key not listed here is an error, so that a misspelt
key can't silently change nothing.

## Top-level keys

| Key | Default | Meaning |
|---|---|---|
| `disable_default_repos` | `false` | Leave out the default repository, `pkgs.peios.org`. It isn't available yet, so a spec must set this to `true` and give a `[[repository]]`. |

## Repositories

Each `[[repository]]` is a place packages come from. Repositories are
searched in the order given. If two hold the same version of a package,
they must agree on what it is.

| Key | Meaning |
|---|---|
| `filepath` | An artifact directory. Required. |

## Kernel

`[kernel]` is the kernel the ISO boots: a package, such as
`dev.peios.kernel`, or a file. Give one or the other.

| Key | Default | Meaning |
|---|---|---|
| `package` | | The package holding the kernel, at `usr/lib/modules/<release>/vmlinuz-<release>`. The ISO carries it in its store, and a `[[package]]` naming it says where else it goes. |
| `version` | the only one | The kernel package's version, as for a `[[package]]`. |
| `file` | | A kernel image on this machine, built with its EFI stub. |
| `cmdline` | empty | The kernel command line. `rd.shell` makes prelude open its recovery shell if boot fails. |

## Packages

Each `[[package]]` is a package the ISO carries. Every package is in the
ISO's store, wherever else it goes, so a package with `liveboot` and
`initramfs` both `false` is carried for fetching from later.

| Key | Default | Meaning |
|---|---|---|
| `name` | | The package's name. Required, and given once. |
| `version` | the only one | The version to carry. Without it, the repositories together must hold one version of the package, since versions have no order to choose the newest by. |
| `liveboot` | `true` | Whether the live system holds the package. |
| `initramfs` | `false` | Whether the initramfs holds the package. |

At least one package must be in the live system, and at least one in the
initramfs.

## Layouts

`[initramfs]` and `[liveboot]` give what the initramfs and the live system
hold outside `/usr`, written as an environment profile's keys are. For the
format, see [Environments](~peios/packaging/environments).

| Key | Meaning |
|---|---|
| `symlinks` | Each link's path and target. A target that starts with `:` is in `usr`. |
| `dirs` | Empty directories. |

A table replaces the default, rather than adding to it. The defaults are:

```toml
[initramfs]
symlinks = { "init" = ":sbin/prelude" }
dirs = ["proc", "sys", "tmp"]

[liveboot]
symlinks = { "bin" = ":bin", "sbin" = ":sbin" }
dirs = ["tmp"]
```

## Refused until supported

A spec can already use these keys, but `peiso` refuses each, saying why,
until Peios can do what it asks:

| Key | Needs |
|---|---|
| `edition`, `edition_package`, `version` | Packages that depend on other packages, since an edition is a package whose dependencies make the system. |
| `url` in a `[[repository]]` | Repositories on the network. |
| `autoinstall`, `offerinstall` in a `[[package]]` | An installer. |
