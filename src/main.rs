//! peiso builds bootable Peios ISOs from ppk packages.
//!
//! A spec names the packages an ISO carries and where each goes. peiso
//! resolves them, with everything they need, from the spec's repositories
//! and gathers every one into the ISO's store. It composes the initramfs from the initramfs's packages,
//! wraps the initramfs and the kernel into a unified kernel image (UKI),
//! and writes an ISO that boots the UKI under UEFI and carries the store
//! and the live system's profile. At boot, prelude's live-root hook finds
//! the ISO by the identifier peiso puts in both, and composes the live
//! system from it.

mod image;
mod resolve;
mod seeds;
mod spec;
mod squash;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use ppk::{Extras, PackageId, Profile, RootArtifacts};
use sha2::{Digest, Sha256};

use crate::resolve::resolve;
use crate::spec::{KernelImage, Spec};

/// Where the initramfs holds the identifier of the medium it boots from,
/// which the live-root hook looks for.
const MEDIUM_ID: &str = "system/medium";

/// Where the medium holds what an installer composes an installed system
/// from: the profiles of its `SYSTEM` and `INITRAMFS` closures.
const INSTALL_DIR: &str = "install";
const INSTALL_SYSTEM: &str = "system.toml";
const INSTALL_INITRAMFS: &str = "initramfs.toml";

/// Where the medium holds the live system's `/usr` as a squashfs image,
/// which the live-root hook mounts.
const LIVE_IMAGE: &str = "live.sqfs";

/// Where the medium holds what the live system's root has outside `/usr`
/// besides its skeleton: a line `l <path> <target>` for each link and
/// `d <path>` for each directory, for the live-root hook to make.
const LIVE_LAYOUT: &str = "live.layout";

/// Where the initramfs holds the SHA-256 of the medium's live image, which
/// the live-root hook checks before it mounts the image.
const LIVE_IMAGE_HASH: &str = "system/live-image";

/// Build bootable Peios ISOs from ppk packages
#[derive(Parser)]
#[command(name = "peiso", version, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build the ISO a spec describes
    Build {
        /// The spec
        #[arg(value_name = "spec")]
        spec: PathBuf,
        /// Write the ISO to <file>, replacing any there
        #[arg(long, value_name = "file")]
        out: PathBuf,
        /// Build the initramfs with this mkirf (default: mkirf on PATH)
        #[arg(long, value_name = "file", default_value = "mkirf")]
        mkirf: PathBuf,
        /// Build the UKI with this mkuki (default: mkuki on PATH)
        #[arg(long, value_name = "file", default_value = "mkuki")]
        mkuki: PathBuf,
        /// Build the live system's image with this mksquashfs (default:
        /// mksquashfs on PATH)
        #[arg(long, value_name = "file", default_value = "mksquashfs")]
        mksquashfs: PathBuf,
    },
    /// List the packages the ISO a spec describes would carry
    Resolve {
        /// The spec
        #[arg(value_name = "spec")]
        spec: PathBuf,
    },
}

fn main() -> ExitCode {
    let done = match Cli::parse().command {
        Cmd::Build {
            spec,
            out,
            mkirf,
            mkuki,
            mksquashfs,
        } => build(
            &spec,
            &out,
            &Tools {
                mkirf,
                mkuki,
                mksquashfs,
            },
        )
        .map(|()| println!("{}", out.display())),
        Cmd::Resolve { spec } => list(&spec),
    };
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("peiso: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Warns that `unstamped` files and directories lack the security metadata
/// their manifests record, as ppk does: a host that isn't Peios won't let
/// ppk give it.
fn warn_unstamped(unstamped: usize, what: &str) {
    match unstamped {
        0 => {}
        1 => eprintln!(
            "peiso: warning: 1 file or directory of {what} lacks security metadata this \
             system would not let ppk give; its manifest records it"
        ),
        n => eprintln!(
            "peiso: warning: {n} files and directories of {what} lack security metadata \
             this system would not let ppk give; their manifests record it"
        ),
    }
}

/// Prints each package the ISO the spec at `spec_path` describes would
/// carry, with where it goes and why it is carried.
fn list(spec_path: &Path) -> Result<()> {
    let spec = Spec::read(spec_path)?;
    let index = tempfile::tempdir()?;
    let resolved = resolve(&spec, ppk::HOST_ARCH, index.path())?;
    for package in &resolved.store {
        let places: Vec<&str> = [
            (&resolved.liveboot, "liveboot"),
            (&resolved.initramfs, "initramfs"),
        ]
        .into_iter()
        .filter(|(place, _)| place.contains(&package.id))
        .map(|(_, name)| name)
        .collect();
        let places = match places.is_empty() {
            true => "store".to_owned(),
            false => places.join(","),
        };
        println!("{} {places} ({})", package.id, package.reason);
    }
    Ok(())
}

/// The kernel image the package `id` in `store` holds, at
/// `usr/lib/modules/vmlinuz`, from the package laid
/// out on its own at `root`.
fn kernel_image(id: &PackageId, store: &Path, root: &Path) -> Result<PathBuf> {
    ppk::compose_env(std::slice::from_ref(id), store, root, &Extras::default())?;
    let image = root.join("usr/lib/modules/vmlinuz");
    if !image.is_file() {
        bail!("{id} holds no kernel at usr/lib/modules/vmlinuz");
    }
    Ok(image)
}

/// Applies the root skeleton file (PSPU 11) the initramfs's closure holds,
/// fsbase-irf's, to the composed initramfs at `root`: its directories
/// outside `/usr` and its `/lib64` link. The live system's is applied at
/// boot, by the live-root hook. An initramfs is a cpio archive, which holds
/// no security descriptors, so each directory is made plainly and the
/// descriptors the file gives are left to the system the initramfs builds.
/// A closure without a skeleton file gets only what the spec's layout gives.
fn apply_skeleton(root: &Path) -> Result<()> {
    let file = root.join(peios_skeleton::PATH);
    if !file.exists() {
        eprintln!(
            "peiso: warning: the initramfs has no skeleton file, /{}; it holds nothing \
             outside /usr but what the spec's layout gives",
            peios_skeleton::PATH
        );
        return Ok(());
    }
    let text = fs::read_to_string(&file).with_context(|| file.display().to_string())?;
    let skeleton = peios_skeleton::parse(&text)
        .with_context(|| format!("the initramfs's skeleton file, /{}", peios_skeleton::PATH))?;
    peios_skeleton::apply(root, &skeleton, |at, _| fs::create_dir(at))
        .context("applying the initramfs's skeleton file")?;
    Ok(())
}

/// The SHA-256 of the file at `path`, in hexadecimal, as `sha256sum` gives it.
fn file_hash(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).with_context(|| path.display().to_string())?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).with_context(|| path.display().to_string())?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// The live system's root outside `/usr` besides its skeleton, as the
/// medium's `live.layout` holds it.
fn live_layout(layout: &spec::Layout) -> String {
    let mut out = String::new();
    for link in &layout.symlinks {
        out.push_str(&format!("l {} {}\n", link.path(), link.target()));
    }
    for dir in &layout.dirs {
        out.push_str(&format!("d {}\n", dir.path()));
    }
    out
}

/// The programs peiso runs to build an ISO's images.
struct Tools {
    mkirf: PathBuf,
    mkuki: PathBuf,
    mksquashfs: PathBuf,
}

/// Builds the ISO the spec at `spec_path` describes, at `out`.
fn build(spec_path: &Path, out: &Path, tools: &Tools) -> Result<()> {
    let spec = Spec::read(spec_path)?;
    if let KernelImage::File(file) = &spec.kernel.image
        && !file.is_file()
    {
        bail!("{}: no such kernel", file.display());
    }

    // Beside the ISO, so that what is built links rather than copies, and
    // the ISO is renamed into place whole.
    let parent = match out.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let work = tempfile::Builder::new()
        .prefix(".peiso-")
        .tempdir_in(parent)
        .with_context(|| parent.display().to_string())?;
    let work = work.path();
    let index = work.join("index");
    fs::create_dir(&index).with_context(|| index.display().to_string())?;
    let packages = resolve(&spec, ppk::HOST_ARCH, &index)?;

    // The ISO's store holds every package chosen, checked as fetched.
    let store = work.join("artifacts");
    let mut identities = String::new();
    let mut unstamped = 0;
    for package in &packages.store {
        eprintln!("peiso: fetching {}", package.id);
        let fetched = ppk::fetch(&package.id, &package.repository, &store)?;
        identities.push_str(&format!("{} {}\n", package.id, fetched.identity));
        unstamped += fetched.unstamped;
    }
    warn_unstamped(unstamped, "the ISO's store");

    // The live-root hook resolves the live system's requests from the
    // medium's store at boot, and the installer an installed system's, so
    // each must give exactly the packages chosen for it here.
    let places = [
        (
            "the live system",
            &packages.liveboot_requests,
            &packages.liveboot,
        ),
        (
            "an installed system",
            &packages.installed_requests,
            &packages.installed,
        ),
        (
            "an installed system's initramfs",
            &packages.installed_initramfs_requests,
            &packages.installed_initramfs,
        ),
    ];
    for (what, requests, chosen) in places {
        if requests.is_empty() {
            continue;
        }
        let from_medium = ppk::resolve(requests, std::slice::from_ref(&store), ppk::HOST_ARCH)
            .with_context(|| format!("resolving {what} from the medium's store"))?
            .ids();
        if from_medium != *chosen {
            bail!(
                "{what}'s requests resolve to other packages from the medium's store than \
                 were chosen for it"
            );
        }
    }

    // The live system, which the live-root hook composes from the medium.
    let live = Profile {
        arch: Some(ppk::HOST_ARCH.to_owned()),
        packages: packages.liveboot_requests.clone(),
        symlinks: spec.liveboot.symlinks.clone(),
        dirs: spec.liveboot.dirs.clone(),
        include_artifacts: false,
    }
    .render()?;
    let staging = work.join("iso");
    fs::create_dir_all(&staging).with_context(|| staging.display().to_string())?;
    // The live system's /usr, as an image carrying its files' descriptors
    // and signatures, which the medium's ISO9660 can't, and what its root
    // has outside /usr besides its skeleton.
    eprintln!("peiso: writing the live system's image");
    let image = ppk::closure_image(&packages.liveboot, &packages.liveboot_requests, &store)?;
    let image_path = staging.join(LIVE_IMAGE);
    squash::write(&image, &tools.mksquashfs, &image_path)?;
    let image_hash = file_hash(&image_path)?;
    let layout = live_layout(&spec.liveboot);
    let layout_path = staging.join(LIVE_LAYOUT);
    fs::write(&layout_path, &layout).with_context(|| layout_path.display().to_string())?;
    // What an installer composes an installed system from, if the spec can
    // be installed: its /usr and its initramfs, each with what it holds
    // outside /usr.
    let mut install = String::new();
    if !packages.installed.is_empty() {
        let dir = staging.join(INSTALL_DIR);
        fs::create_dir_all(&dir).with_context(|| dir.display().to_string())?;
        for (file, requests, layout) in [
            (INSTALL_SYSTEM, &packages.installed_requests, &spec.liveboot),
            (
                INSTALL_INITRAMFS,
                &packages.installed_initramfs_requests,
                &spec.initramfs,
            ),
        ] {
            let profile = Profile {
                arch: Some(ppk::HOST_ARCH.to_owned()),
                packages: requests.clone(),
                symlinks: layout.symlinks.clone(),
                dirs: layout.dirs.clone(),
                include_artifacts: false,
            }
            .render()?;
            let path = dir.join(file);
            fs::write(&path, &profile).with_context(|| path.display().to_string())?;
            install.push_str(&format!("{file}\n{profile}"));
        }
    }
    let policy = seeds::stage(
        &spec.seeds,
        &packages.liveboot,
        &packages.liveboot_requests,
        &store,
        work,
        &staging,
    )?;
    // What the medium holds names it, so an ISO built again from the same
    // spec and packages is the same medium, and one with any other content
    // is another.
    let medium = ppk::Identity::of(
        format!("{identities}\n{live}{policy}{install}{image_hash}\n{layout}").as_bytes(),
    )
    .hex()
    .to_owned();

    eprintln!("peiso: composing the initramfs");
    let root = work.join("initramfs");
    let extras = Extras {
        symlinks: spec.initramfs.symlinks.clone(),
        dirs: spec.initramfs.dirs.clone(),
        artifacts: RootArtifacts::None,
        ..Extras::default()
    };
    let composed = ppk::compose_env_for(
        &packages.initramfs,
        &packages.initramfs_requests,
        &store,
        &root,
        &extras,
    )?;
    warn_unstamped(composed.unstamped, "the initramfs");
    apply_skeleton(&root)?;
    let id_file = root.join(MEDIUM_ID);
    let system = id_file.parent().expect("the identifier is in a directory");
    fs::create_dir_all(system).with_context(|| system.display().to_string())?;
    fs::write(&id_file, format!("{medium}\n")).with_context(|| id_file.display().to_string())?;
    let hash_file = root.join(LIVE_IMAGE_HASH);
    fs::write(&hash_file, format!("{image_hash}\n"))
        .with_context(|| hash_file.display().to_string())?;

    let kernel = match &spec.kernel.image {
        KernelImage::File(file) => file.clone(),
        KernelImage::Package(name) => {
            let package = (packages.store.iter())
                .find(|package| package.id.name() == name)
                .expect("the kernel package is among the store's packages");
            kernel_image(&package.id, &store, &work.join("kernel"))?
        }
    };

    let cpio = work.join("initramfs.cpio.zst");
    image::tool(Command::new(&tools.mkirf).arg(&root).arg(&cpio))?;
    eprintln!("peiso: building the UKI");
    let uki = work.join("uki.efi");
    image::tool(
        Command::new(&tools.mkuki)
            .arg("--kernel")
            .arg(&kernel)
            .arg("--initramfs")
            .arg(&cpio)
            .arg("--cmdline")
            .arg(&spec.kernel.cmdline)
            .arg("--out")
            .arg(&uki),
    )?;
    let esp = work.join("efi.img");
    image::esp_image(&uki, &esp)?;
    eprintln!("peiso: writing the ISO");
    let iso = work.join("out.iso");
    image::iso(&esp, &store, &live, &medium, &staging, &iso)?;
    fs::rename(&iso, out).with_context(|| out.display().to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_live_layout_lists_links_then_directories() {
        let layout = spec::Layout {
            symlinks: vec!["bin::bin".parse().unwrap(), "lib64:lib".parse().unwrap()],
            dirs: vec!["tmp".parse().unwrap()],
        };
        assert_eq!(live_layout(&layout), "l bin usr/bin\nl lib64 lib\nd tmp\n");
    }

    #[test]
    fn the_initramfs_gets_its_closures_skeleton() {
        let root = tempfile::tempdir().unwrap();
        let share = root.path().join("usr/share");
        fs::create_dir_all(&share).unwrap();
        fs::create_dir(root.path().join("proc")).unwrap();
        fs::write(
            share.join("skeleton"),
            "/proc\n/var\n/var/state/ O:SYG:SYD:P(A;OICI;GA;;;SY)\n",
        )
        .unwrap();
        let err = format!("{:#}", apply_skeleton(root.path()).unwrap_err());
        assert!(err.contains("line 3"), "{err}");
        assert!(!root.path().join("var").exists());

        fs::write(
            share.join("skeleton"),
            "/proc\n/var\n/var/state O:SYG:SYD:P(A;OICI;GA;;;SY)\n/lib64 -> usr/lib\n",
        )
        .unwrap();
        apply_skeleton(root.path()).unwrap();
        apply_skeleton(root.path()).unwrap();
        assert!(root.path().join("var/state").is_dir());
        assert_eq!(
            fs::read_link(root.path().join("lib64")).unwrap(),
            Path::new("usr/lib")
        );
    }

    #[test]
    fn an_initramfs_without_a_skeleton_is_left_as_it_is() {
        let root = tempfile::tempdir().unwrap();
        apply_skeleton(root.path()).unwrap();
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
}
