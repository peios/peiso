---
title: Peios ISOs
type: concept
description: What a Peios ISO holds, covering the unified kernel image and
  its initramfs, the medium's package store and live system profile, and
  how the live system is composed from the medium at boot.
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

An ISO holds four things:

- **A unified kernel image (UKI).** One EFI program holding the kernel,
  its command line, and the initramfs, the small filesystem the kernel
  starts from. It sits in an EFI system partition image, where UEFI
  firmware looks for the loader of a removable medium, so the firmware
  starts it directly.
- **A package store,** at `/artifacts`. It holds every package the spec
  names, laid out as `ppk` stores them.
- **The live system's environment profile,** at `/live.toml`. It names the
  packages of the live system, at exact versions, and what the system
  holds outside `/usr`. For profiles, see
  [Environments](~peios/packaging/environments).
- **The medium's identifier,** at `/peios-medium`. It is the SHA-256 hash
  of the packages and the profile, so the same spec and packages always
  make the same medium.

## Where each package goes

A spec places each package in one or more of three places:

- **The live system,** the default, composed at boot from the medium.
- **The initramfs,** which prelude, Peios's early-boot init, runs from.
  Anything that runs before the live system exists belongs here: prelude
  itself, the live-boot hook that composes the live system, and what the
  hook runs, such as `ppk` and a shell. The initramfs is held in memory,
  so it holds only what boot needs.
- **The medium only,** carried in the store and nowhere else. Such a
  package can be fetched from the medium later, which makes an ISO a
  library of packages for a machine with no network.

Every package is in the store, wherever else it goes.

## How it boots

1. UEFI firmware starts the UKI from the medium's EFI system partition,
   and the kernel starts prelude from the initramfs.
2. prelude runs the live-boot hook, which finds the medium: it tries each
   disc, disk and stick with an ISO9660 filesystem, waiting a few seconds
   for a USB stick to appear, and keeps the one whose `/peios-medium`
   matches the identifier the initramfs holds. Another Peios medium
   attached alongside is never taken for it.
3. The hook composes the live system from `/live.toml` and the store, on a
   fresh tmpfs. It copies the system's packages into the system's own
   store, checking each, so the system needs nothing more from the medium,
   which the hook unmounts.
4. prelude switches to the live system and starts its init.

If the hook can't find the medium or compose the system, prelude opens its
recovery shell when the kernel command line holds `rd.shell`.

A live system lives in memory, so nothing in it survives a restart.
