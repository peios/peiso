---
title: Peios ISOs
type: concept
description: What a Peios ISO holds, covering the unified kernel image and
  its initramfs, the medium's package store and live system image, and how
  the live system is mounted from the medium at boot.
related:
  - peios/building-isos/building-an-iso
  - peios/building-isos/iso-specs
  - peios/packaging/environments
---

A Peios ISO is a disc image that boots a live Peios system under UEFI, on
a virtual machine or on a computer it is written to as a USB stick.
`peiso` builds one from Peios packages, as a spec describes. For what a
package is, see [Packages](~peios/packaging/packages).

## What an ISO holds

An ISO holds six things:

- **A unified kernel image (UKI).** One EFI program holding the kernel,
  its command line, and the initramfs, the small filesystem the kernel
  starts from. It sits in an EFI system partition image, where UEFI
  firmware looks for the loader of a removable medium, so the firmware
  starts it directly.
- **A package store,** at `/artifacts`. It holds every package the spec
  names and every package those depend on or require, laid out as `ppk`
  stores them.
- **The live system's image,** at `/live.sqfs`. It is the live system's
  `/usr` as a read-only squashfs image: the packages the spec places in the
  live system and everything they need, with the inventory a composed
  `/usr` has. Each file and directory carries its security descriptor, and
  each signed file its signature, which the store's plain files on the
  medium can't, because ISO9660 holds no extended attributes.
- **The live system's profile and layout.** `/live.toml` is the environment
  profile the image is the closure of: it asks for the packages the spec
  places in the live system, each at exactly the version `peiso` chose.
  `peiso` checks, when it builds the ISO, that resolving it from the store
  gives exactly those packages. `/live.layout` lists the links and empty
  directories the spec gives the live system's root outside `/usr`. For
  profiles, see [Environments](~peios/packaging/environments).
- **What an installer composes an installed system from,** if the spec can
  be installed: `/install/system.toml` and `/install/initramfs.toml`, the
  requests of its `/usr` and its initramfs, each pinned to the version
  `peiso` chose, and checked as `/live.toml` is.
- **The medium's identifier,** at `/peios-medium`. It is the SHA-256 hash
  of the packages, the profiles and the image, so the same spec and
  packages always make the same medium.

## Where each package goes

A spec places each package in one or more of these places:

- **The live system,** whose `/usr` is the medium's image.
- **The initramfs,** which prelude, Peios's early-boot init, runs from.
  Anything that runs before the live system exists belongs here: prelude
  itself, the live-boot hook that mounts the live system, and what the
  hook runs, such as a shell. The initramfs is held in memory,
  so it holds only what boot needs.
- **The installed system,** which an installer composes from the medium,
  and its initramfs, which mounts the installed system's disk. A spec that
  places nothing there can't be installed.
- **The medium only,** carried in the store and nowhere else. Such a
  package can be fetched from the medium later, which makes an ISO a
  library of packages for a machine with no network.

By default, a package goes in the live system and the installed system.
Every package is in the store, wherever else it goes.

A package comes with what it needs. `peiso` resolves the packages the spec
names together, with `ppk`'s resolver, and carries one version of each
package that they depend on or require. Each place then holds the
packages placed there and everything they need, at the versions the store
holds.

A dependency goes where the package that needs it goes. A package meant
for the initramfs, such as a boot hook, must be placed there by the spec,
even when a package of the live system depends on it. For the details,
see [ISO Specs](~peios/building-isos/iso-specs).

## How it boots

1. UEFI firmware starts the UKI from the medium's EFI system partition,
   and the kernel starts prelude from the initramfs.
2. prelude runs the live-boot hook, which finds the medium: it tries each
   disc, disk and stick with an ISO9660 filesystem, waiting a few seconds
   for a USB stick to appear, and keeps the one whose `/peios-medium`
   matches the identifier the initramfs holds. Another Peios medium
   attached alongside is never taken for it.
3. The hook checks the medium's image against the SHA-256 hash the
   initramfs holds, and mounts it read-only as the system's `/usr`, under
   a root on a fresh tmpfs. The system's `/usr/inventory` records what the
   spec asked for and what was chosen. The hook makes what `/live.layout`
   lists, and applies the system's root skeleton file, which gives the
   system its directories outside `/usr` and the `/lib64` link. It moves
   the medium's mount to `/media/peios` in the system, where the image is
   read from and an installer reads the store.
4. prelude switches to the live system and starts its init.

If the hook can't find the medium, or the image isn't the one the
initramfs names, prelude opens its recovery shell when the kernel command
line holds `rd.shell`.

The live system reads its `/usr` from the medium, so the medium must stay
attached while it runs. Everything else lives in memory, so nothing the
system writes survives a restart.
