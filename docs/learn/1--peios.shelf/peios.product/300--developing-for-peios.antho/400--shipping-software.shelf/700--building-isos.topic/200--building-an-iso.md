---
title: Building an ISO
type: how-to
description: Write a spec naming an ISO's packages and kernel, check what
  it brings with peiso resolve, build the ISO with peiso build, and boot it
  in QEMU under UEFI.
related:
  - peios/building-isos/isos
  - peios/building-isos/iso-specs
  - pekit/shipping/packaging
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
  [`pekit`](~pekit/packaging) packs into.
  The live system needs a shell and an init, and the initramfs needs
  prelude, the live-boot hook, `ppk`, and a shell. Packages bring what
  they depend on or require, so the artifact directory needs those
  packages too.
- Have the kernel package, `dev.peios.kernel`, in the artifact directory
  too, or a Peios kernel image built with its EFI stub.

## Write the spec

Write the spec, such as `peios.toml`, beside the artifact directory. Paths
in it are relative to the spec:

```toml
disable_default_repos = true

[[repository]]
filepath = "artifacts"

[kernel]
package = "dev.peios.kernel"
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

The kernel comes from its package, which the ISO carries in its store.
The live system is busybox and `ppk`, and the initramfs holds both,
since the live-boot hook runs them, with prelude and the hook itself.
Each place also holds every package its packages depend on or require,
and each package is carried at the newest version the artifact directory
holds, unless the spec gives a `version`. For every key, see
[ISO Specs](~peios/building-isos/iso-specs).

## Check what the spec brings

Run `peiso resolve` to list every package the ISO would carry, without
building it:

```sh
peiso resolve peios.toml
```

Each line is a package, the places that hold it, and what first asked for
it. A package in neither place is in the store only:

```text
dev.peios.kernel@0.25.0-indev-1?arch=x86_64 store (the request for dev.peios.kernel)
dev.peios.prelude@0.0.7-1?arch=x86_64 initramfs (the request for dev.peios.prelude)
org.gnu.glibc@2.44-1?arch=x86_64 liveboot,initramfs (net.busybox.busybox@1.37.0-1 requires libc.so.6)
```

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
peiso: fetching dev.peios.kernel@0.25.0-indev-1?arch=x86_64
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

`peiso` checks the spec and resolves every package before it builds
anything. It refuses a package, or a dependency, that no repository
holds at a version that will do, and says what needed it:

```text
peiso: dev.peios.peios-experimental@2026.8-11 depends on dev.peios.bootconfig >= 0.21.0-alpha11-1, but no artifact directory holds dev.peios.bootconfig for x86_64: artifacts
```

It refuses a spec with no package in the live system, or none in the
initramfs, since such an ISO couldn't boot. It also refuses what a spec
can say but Peios can't do yet, such as a repository on the network, and
says why.

If a tool it runs fails, `peiso` stops and shows what the tool said.
