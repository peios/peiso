---
title: ISO Specs
type: reference
description: Every key of the TOML spec peiso builds an ISO from, with its
  default, how a spec's packages bring what they need, and the keys peiso
  understands but refuses until Peios can do what they ask.
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
| `edition` | | The Peios edition the live system is, such as `"Experimental"`. Its package is `dev.peios.peios-` followed by the edition in lowercase, such as `dev.peios.peios-experimental`, and the live system holds it and everything it needs. |
| `edition_package` | | The package of an edition that isn't one of Peios's own, such as `com.example.peios-inhouse`, in place of `edition`. Give one or the other. |
| `version` | the newest | The edition package's version, as for a `[[package]]`. |
| `disable_default_repos` | `false` | Leave out the default repository, `pkgs.peios.org`. It isn't available yet, so a spec must set this to `true` and give a `[[repository]]`. |

## Repositories

Each `[[repository]]` is a place packages come from. A package is taken
from the first repository, in the order given, that holds the version
chosen. If two hold the same version of a package, they must agree on what
it is.

| Key | Meaning |
|---|---|
| `filepath` | An artifact directory. Required. |

## Kernel

`[kernel]` is the kernel the ISO boots: a package, such as
`dev.peios.kernel`, or a file. Give one or the other.

| Key | Default | Meaning |
|---|---|---|
| `package` | | The package holding the kernel, at `usr/lib/modules/vmlinuz`. The ISO carries it in its store, with what it needs, and a `[[package]]` naming it says where else it goes. |
| `version` | the newest | The kernel package's version, as for a `[[package]]`. |
| `file` | | A kernel image on this machine, built with its EFI stub. |
| `cmdline` | empty | The kernel command line. `rd.shell` makes prelude open its recovery shell if boot fails. |

## Packages

Each `[[package]]` is a package the ISO carries. Every package is in the
ISO's store, wherever else it goes, so a package with `liveboot` and
`initramfs` both `false` is carried for fetching from later. An edition's
package can't also be given as a `[[package]]`.

| Key | Default | Meaning |
|---|---|---|
| `name` | | The package's name. Required, and given once. |
| `version` | the newest | The versions that will do, as a version constraint, such as `">= 1.2"`. A version on its own, such as `"1.2.0-1"`, is that version exactly. The newest version that will do is carried. |
| `liveboot` | `true` | Whether the live system holds the package and everything it needs. |
| `initramfs` | `false` | Whether the initramfs holds the package and everything it needs. |

At least one package must be in the live system, and at least one in the
initramfs.

## What packages bring

A package brings every package it depends on, and a package providing
each capability it requires, such as `sh` or `libc.so.6`. `peiso` chooses
them with `ppk`'s resolver, from the edition's package, the kernel
package, and every `[[package]]` together:

- Each package name is chosen once, at the newest version that satisfies
  what asks for it first. Anything that asks for another version is an
  error, which names both.
- A capability is satisfied by a package already chosen if one provides
  it. Otherwise, the package providing it is chosen. If several packages
  provide it, name the one to carry with a `[[package]]`.

The ISO's store holds every package chosen. The live system holds the
edition, each package with `liveboot = true`, and everything they need.
The initramfs holds each package with `initramfs = true` and everything
they need. A place takes the versions and providers the store holds, so
the live system and the initramfs never hold two versions of a package.

A dependency goes where the package that needs it goes, and nowhere else.
A package of the live system can depend on one meant for the initramfs,
such as `dev.peios.live-boot` on its boot hook, `dev.peios.live-boot-irf`.
The live system then holds the hook as well, and the initramfs holds it
only if the spec places it there.

To see what a spec brings before building it, run `peiso resolve`. See
[Building an ISO](~peios/building-isos/building-an-iso).

## Registry seeds

Packages ship seed masters under `/usr/share/regim`, but the spec chooses
which the image applies. No release metadata is read for this policy.

```toml
[seeds]
apply = ["base-service"]
live = ["seeds/console-shell.reg"]
install = ["first-boot"]
```

All three lists default to empty. A bare seed name selects
`usr/share/regim/<name>.reg` from the composed live closure, including
dependencies. A package carried only in the store or initramfs cannot
supply it. A relative path ending in `.reg` selects a file beside the
spec, staged using its filename. Names use ASCII letters, digits, dots,
underscores, and hyphens. Absolute file paths, unknown keys, and duplicate
destination names, within or across queues, are errors.

| List | Medium directory | Applied on |
|---|---|---|
| `apply` | `/lcl/policy/autoapply.d` | The live medium and an installed system. |
| `live` | `/lcl/policy/autoapply.live.d` | The live medium only. |
| `install` | `/lcl/policy/autoapply.install.d` | An installed system, after an installer promotes it. |

When any seed is named, peiso also stages the executable
`/lcl/policy/autorun.d/10-apply-seeds.sh`. The live-root hook copies the
medium's `/lcl` into the composed root, preserving modes. After registryd
starts, peinit runs the script, applying the common queue and then the
live queue with `reg apply --once-delete`. The install queue remains
untouched. An installer must remove the live queue and promote the
install queue into the target's common queue.

Seed files have mode `0644` and the script has mode `0755`. Their paths,
contents, and modes contribute to the medium identifier. With no seeds,
peiso stages no `/lcl` tree.

## Layouts

`[initramfs]` and `[liveboot]` give what the initramfs and the live system
hold outside `/usr`, written as an environment profile's keys are. For the
format, see [Environments](~peios/packaging/environments).

| Key | Meaning |
|---|---|
| `symlinks` | Each link's path and target. A target that starts with `:` is in `usr`. |
| `dirs` | Empty directories. |
| `include` | Profile fragments whose links and directories the layout holds as well, each relative to the spec's directory. A fragment that names packages is refused, since the spec chooses those. |

A table replaces the default, rather than adding to it. The defaults are:

```toml
[initramfs]
dirs = ["proc", "sys", "tmp"]

[liveboot]
symlinks = { "bin" = ":bin", "sbin" = ":sbin" }
```

The initramfs's `/init` isn't in its layout: prelude's package claims it,
and composing the initramfs creates the link. A layout can't also give a
path that a package claims.

A layout needs no directories or links that fsbase's skeleton gives, such
as `/var`, `/lcl`, `/tmp` or `/lib64`. Each root gets those from the root
skeleton file its own closure holds, at `/usr/share/skeleton`. Carry
`dev.peios.fsbase` in the live system and `dev.peios.fsbase-irf` in the
initramfs. `peiso` applies the initramfs's file after composing it, and
the live-boot hook applies the live system's at boot, with each
directory's security descriptor. An initramfs can't hold descriptors, so
its directories get none. `peiso` warns when the initramfs has no skeleton
file.

Don't list a skeleton directory in `[liveboot]`. The profile's directories
are made first, and the skeleton leaves an existing directory as it is, so
it would lose the skeleton's descriptor.

## Refused until supported

A spec can already use these keys, but `peiso` refuses each, saying why,
until Peios can do what it asks:

| Key | Needs |
|---|---|
| `url` in a `[[repository]]` | Repositories on the network. |
| `autoinstall`, `offerinstall` in a `[[package]]` | An installer. |
