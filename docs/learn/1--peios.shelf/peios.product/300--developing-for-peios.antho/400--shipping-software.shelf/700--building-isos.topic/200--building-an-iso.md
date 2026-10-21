---
title: Building an ISO
type: how-to
description: Write a spec naming an ISO's packages and kernel, build the
  ISO with peiso build, and boot it in QEMU under UEFI.
related:
  - peios/building-isos/isos
  - peios/building-isos/iso-specs
  - peios/packaging/building-a-package-from-a-recipe
---

`peiso build` builds a bootable Peios ISO from packages you have built.
Do this to try a set of packages as a whole system, or to make a medium
to boot a machine from. For what an ISO holds, see
[Peios ISOs](~peios/building-isos/isos).

## Before you start

- Install `peiso`, and `mkirf` and `mkuki` from peiosutils, or know where
  they are.
- Install `mkfs.vfat`, mtools, and `xorriso`. `peiso` runs them, without
  any privilege.
- Have the packages in an artifact directory, such as one that
  [`pekit`](~peios/packaging/building-a-package-from-a-recipe) packs into.
  The live system needs a shell and an init, and the initramfs needs
  prelude, the live-boot hook, `ppk`, and a shell.
- Have a Peios kernel built with its EFI stub.

## Write the spec

Write the spec, such as `peios.toml`, beside the artifact directory. Paths
in it are relative to the spec:

```toml
disable_default_repos = true

[[repository]]
filepath = "artifacts"

[kernel]
file = "vmlinuz"
cmdline = "console=ttyS0 rd.shell"

[[package]]
name = "net.busybox.busybox"
initramfs = true

[[package]]
name = "dev.peios.ppk"
initramfs = true

[[package]]
name = "dev.peios.prelude"
liveboot = false
initramfs = true

[[package]]
name = "dev.peios.live-root"
liveboot = false
initramfs = true
```

Here the live system is busybox and `ppk`, and the initramfs holds both,
since the live-boot hook runs them, with prelude and the hook itself. A
package without a version must have only one version in the artifact
directory. For every key, see [ISO Specs](~peios/building-isos/iso-specs).

## Build the ISO

Run `peiso build` with the spec and where to write the ISO:

```sh
peiso build peios.toml --out peios.iso
```

Give `--mkirf` and `--mkuki` with their paths if they aren't on `PATH`.
`peiso` says what it is doing, and prints the ISO's path when it is done:

```text
peiso: fetching net.busybox.busybox@1.37.0-1?arch=x86_64
peiso: fetching dev.peios.ppk@0.1.0-1?arch=x86_64
peiso: fetching dev.peios.prelude@0.0.7-1?arch=x86_64
peiso: fetching dev.peios.live-root@0.1.0-1?arch=x86_64
peiso: composing the initramfs
peiso: building the UKI
peiso: writing the ISO
peios.iso
```

## Boot it

Boot the ISO in QEMU with OVMF, its UEFI firmware. OVMF writes to its
variables, so give it a copy of them:

```sh
cp /usr/share/OVMF/OVMF_VARS_4M.fd vars.fd
qemu-system-x86_64 -enable-kvm -m 1G -machine q35 -nographic \
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd \
    -drive if=pflash,format=raw,file=vars.fd \
    -cdrom peios.iso
```

prelude reports each step of boot on the console, and starts the live
system's init. The live-boot hook says where it found the medium:

```text
ppk-live-root: found the medium on /dev/sr0
```

To boot a computer, write the ISO to a USB stick and start the computer
from it. The ISO is also a disk with an EFI system partition, so it boots
from a stick as it does from a disc.

## If peiso refuses

`peiso` checks the spec and every package before it builds anything. It
refuses a package that isn't in any repository, or that has several
versions and no version given:

```text
peiso: dev.peios.ppk has several versions, 0.1.0-1, 0.1.0-2: give one with version = "..."
```

It refuses a spec with no package in the live system, or none in the
initramfs, since such an ISO couldn't boot. It also refuses what a spec
can say but Peios can't do yet, such as an edition, and says why.

If a tool it runs fails, `peiso` stops and shows what the tool said.
