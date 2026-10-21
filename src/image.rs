//! Turning a unified kernel image and a store into a bootable ISO: the UKI
//! goes into an EFI system partition image, and that image and the store
//! go onto the ISO. Nothing here needs privilege: mtools fills the FAT
//! image without mounting it.

use std::fs::{self, File};
use std::io;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

/// The EFI system partition image's path on the ISO.
const ESP_IMAGE: &str = "boot/efi.img";

/// Where UEFI firmware looks for a removable medium's loader, in the EFI
/// system partition.
const LOADER: &str = "EFI/BOOT/BOOTX64.EFI";

/// The ISO9660 volume label.
const LABEL: &str = "PEIOS";

const MIB: u64 = 1 << 20;

/// Writes `out`, an EFI system partition image holding `uki` as the
/// removable medium's loader. It is FAT32, sized for the UKI with room for
/// the filesystem's own records.
pub fn esp_image(uki: &Path, out: &Path) -> Result<()> {
    let size = fs::metadata(uki)
        .with_context(|| uki.display().to_string())?
        .len();
    let file = File::create(out).with_context(|| out.display().to_string())?;
    file.set_len(esp_size(size))
        .with_context(|| out.display().to_string())?;
    drop(file);
    tool(Command::new("mkfs.vfat").args(["-F", "32"]).arg(out))?;
    tool(
        Command::new("mmd")
            .arg("-i")
            .arg(out)
            .args(["::/EFI", "::/EFI/BOOT"]),
    )?;
    tool(
        Command::new("mcopy")
            .arg("-i")
            .arg(out)
            .arg(uki)
            .arg(format!("::/{LOADER}")),
    )
}

/// The size of an EFI system partition image for `content` bytes: a fifth
/// more and 2 MiB for FAT's own records, in whole MiB, and at least the
/// 40 MiB FAT32 needs.
fn esp_size(content: u64) -> u64 {
    let size = content + content / 5 + 2 * MIB;
    (size.div_ceil(MIB) * MIB).max(40 * MIB)
}

/// Writes the ISO `out` from `esp`, the EFI system partition image,
/// `artifacts`, a store, `live`, the live system's profile, and `medium`,
/// the medium's identifier, staging them in `staging`, a directory that
/// must not exist. The store is placed at `/artifacts`, the profile at
/// `/live.toml`, and the identifier at `/peios-medium`.
///
/// The ESP image lives inside the ISO9660 tree, not appended after it.
/// El Torito's load size is 16 bits of 512-byte sectors, so an image over
/// 32 MiB, as one holding a UKI is, is recorded as size 0, which EDK2 reads
/// as "to the end of the volume". An image appended after the volume
/// starts exactly at its end, so that came to nothing, and OVMF refused
/// the disc (PEI-506). Inside the volume the same reading covers the whole
/// image, and `-efi-boot-part --efi-boot-image` marks that extent in a GPT
/// as the EFI system partition too, so the ISO also boots written to a
/// USB stick, with no second copy.
///
/// Rock Ridge (`-r`) keeps the store's long names and execute bits, which
/// plain ISO9660 cannot hold, with every file owned by root and readable
/// by all.
pub fn iso(
    esp: &Path,
    artifacts: &Path,
    live: &str,
    medium: &str,
    staging: &Path,
    out: &Path,
) -> Result<()> {
    let image = staging.join(ESP_IMAGE);
    let parent = image.parent().expect("the image is in a directory");
    fs::create_dir_all(parent).with_context(|| parent.display().to_string())?;
    link_or_copy(esp, &image)?;
    link_tree(artifacts, &staging.join("artifacts"))?;
    for (name, content) in [
        ("live.toml", live.to_owned()),
        ("peios-medium", format!("{medium}\n")),
    ] {
        let path = staging.join(name);
        fs::write(&path, content).with_context(|| path.display().to_string())?;
    }
    tool(
        Command::new("xorriso")
            .args(["-as", "mkisofs", "-V", LABEL, "-iso-level", "3", "-r"])
            .args(["-e", ESP_IMAGE, "-no-emul-boot"])
            .args(["-efi-boot-part", "--efi-boot-image"])
            .arg("-o")
            .arg(out)
            .arg(staging),
    )
}

/// Places the tree `from` at `to`, linking each file where it can.
fn link_tree(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to).with_context(|| to.display().to_string())?;
    for entry in fs::read_dir(from).with_context(|| from.display().to_string())? {
        let entry = entry.with_context(|| from.display().to_string())?;
        let (source, dest) = (entry.path(), to.join(entry.file_name()));
        let kind = entry.file_type()?;
        if kind.is_dir() {
            link_tree(&source, &dest)?;
        } else if kind.is_file() {
            link_or_copy(&source, &dest)?;
        } else {
            // A store holds only files and directories.
            bail!("{}: not a file or directory", source.display());
        }
    }
    Ok(())
}

fn link_or_copy(from: &Path, to: &Path) -> Result<()> {
    if fs::hard_link(from, to).is_ok() {
        return Ok(());
    }
    fs::copy(from, to).with_context(|| format!("{} to {}", from.display(), to.display()))?;
    Ok(())
}

/// Runs `command`, keeping its output for when it fails.
pub fn tool(command: &mut Command) -> Result<()> {
    let program = command.get_program().to_string_lossy().into_owned();
    let output = match command.output() {
        Ok(output) => output,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            bail!("{program}: not found; install it, or give its path")
        }
        Err(error) => return Err(error).with_context(|| format!("cannot run {program}")),
    };
    if !output.status.success() {
        let mut said = String::from_utf8_lossy(&output.stderr).into_owned();
        said.push_str(&String::from_utf8_lossy(&output.stdout));
        bail!("{program} failed ({}): {}", output.status, said.trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_esp_image_is_at_least_what_fat32_needs_in_whole_mib() {
        assert_eq!(esp_size(0), 40 * MIB);
        assert_eq!(esp_size(100 * MIB), 122 * MIB);
        assert_eq!(esp_size(100 * MIB + 1) % MIB, 0);
    }
}
