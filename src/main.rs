//! peiso builds bootable Peios ISOs from ppk packages.
//!
//! A spec names the packages an ISO carries and where each goes. peiso
//! finds them in the spec's repositories and gathers every one into the
//! ISO's store. It composes the initramfs from the initramfs's packages,
//! wraps the initramfs and the kernel into a unified kernel image (UKI),
//! and writes an ISO that boots the UKI under UEFI and carries the store
//! and the live system's profile. At boot, prelude's live-root hook finds
//! the ISO by the identifier peiso puts in both, and composes the live
//! system from it.

mod image;
mod resolve;
mod spec;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use ppk::{Extras, Profile, RootArtifacts};

use crate::resolve::resolve;
use crate::spec::Spec;

/// Where the initramfs holds the identifier of the medium it boots from,
/// which the live-root hook looks for.
const MEDIUM_ID: &str = "system/medium";

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
    },
}

fn main() -> ExitCode {
    let Cmd::Build {
        spec,
        out,
        mkirf,
        mkuki,
    } = Cli::parse().command;
    match build(&spec, &out, &mkirf, &mkuki) {
        Ok(()) => {
            println!("{}", out.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("peiso: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Builds the ISO the spec at `spec_path` describes, at `out`.
fn build(spec_path: &Path, out: &Path, mkirf: &Path, mkuki: &Path) -> Result<()> {
    let spec = Spec::read(spec_path)?;
    if !spec.kernel.file.is_file() {
        bail!("{}: no such kernel", spec.kernel.file.display());
    }
    let packages = resolve(&spec, ppk::HOST_ARCH)?;
    let live: Vec<_> = (packages.iter())
        .filter(|package| package.liveboot)
        .map(|package| package.id.clone())
        .collect();
    if live.is_empty() {
        bail!("no package has liveboot = true, so the live system would be empty");
    }
    if !packages.iter().any(|package| package.initramfs) {
        bail!("no package has initramfs = true, so nothing would boot");
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

    // The ISO's store holds every package the spec names, checked as
    // fetched.
    let store = work.join("artifacts");
    let mut identities = String::new();
    for package in &packages {
        eprintln!("peiso: fetching {}", package.id);
        let fetched = ppk::fetch(&package.id, &package.repository, &store)?;
        identities.push_str(&format!("{} {}\n", package.id, fetched.identity));
    }

    // The live system, which the live-root hook composes from the medium.
    let live = Profile {
        packages: live,
        symlinks: spec.liveboot.symlinks.clone(),
        dirs: spec.liveboot.dirs.clone(),
        include_artifacts: false,
    }
    .render()?;
    // What the medium holds names it, so an ISO built again from the same
    // spec and packages is the same medium, and one with any other content
    // is another.
    let medium = ppk::Identity::of(format!("{identities}\n{live}").as_bytes())
        .hex()
        .to_owned();

    eprintln!("peiso: composing the initramfs");
    let initramfs: Vec<_> = (packages.iter())
        .filter(|package| package.initramfs)
        .map(|package| package.id.clone())
        .collect();
    let root = work.join("initramfs");
    let extras = Extras {
        symlinks: spec.initramfs.symlinks.clone(),
        dirs: spec.initramfs.dirs.clone(),
        artifacts: RootArtifacts::None,
    };
    ppk::compose_env(&initramfs, &store, &root, &extras)?;
    let id_file = root.join(MEDIUM_ID);
    let system = id_file.parent().expect("the identifier is in a directory");
    fs::create_dir_all(system).with_context(|| system.display().to_string())?;
    fs::write(&id_file, format!("{medium}\n")).with_context(|| id_file.display().to_string())?;

    let cpio = work.join("initramfs.cpio.zst");
    image::tool(Command::new(mkirf).arg(&root).arg(&cpio))?;
    eprintln!("peiso: building the UKI");
    let uki = work.join("uki.efi");
    image::tool(
        Command::new(mkuki)
            .arg("--kernel")
            .arg(&spec.kernel.file)
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
    image::iso(&esp, &store, &live, &medium, &work.join("iso"), &iso)?;
    fs::rename(&iso, out).with_context(|| out.display().to_string())?;
    Ok(())
}
