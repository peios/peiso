//! The live system's `/usr` as a squashfs image, which the live-root hook
//! mounts in place of composing the system from the medium's store.
//!
//! ISO9660 holds no extended attributes, so the store's files on the medium
//! carry none of the descriptors and signatures the kernel reads. The image
//! carries them: peiso streams the live closure as a tar archive, each
//! entry's attributes in PAX `SCHILY.xattr` records, into `mksquashfs
//! -tar`, which stores them as they are. Writing them that way needs no
//! privilege and no Peios. The image's root, which a tar archive can't
//! describe, gets its attributes from a pseudo definition.

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use base64::prelude::{BASE64_STANDARD, Engine as _};
use ppk::{ClosureImage, ImageEntry, ImageKind};

/// The prefix of a PAX record holding an extended attribute.
const XATTR_RECORD: &str = "SCHILY.xattr.";

/// Writes `image` as a squashfs image at `out`, with `mksquashfs`.
///
/// The image is the same for the same closure: every time and owner is
/// fixed, and entries go in sorted. zstd reads back quickly, and costs only
/// a little more than xz.
pub fn write(image: &ClosureImage, mksquashfs: &Path, out: &Path) -> Result<()> {
    let mut command = Command::new(mksquashfs);
    command
        .arg("-")
        .arg(out)
        .args([
            "-tar",
            "-noappend",
            "-no-recovery",
            "-quiet",
            "-no-progress",
        ])
        .args(["-comp", "zstd", "-b", "1M"])
        .args(["-mkfs-time", "0", "-inode-time", "0", "-root-time", "0"])
        .args(["-all-root", "-root-mode", "755"]);
    for (name, value) in &image.root.attributes {
        // Base64: mksquashfs 4.7.5 corrupts its memory decoding a hex value
        // of 40 bytes or more.
        command
            .arg("-p")
            .arg(format!("/ x {name}=0s{}", BASE64_STANDARD.encode(value)));
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("cannot run {}", mksquashfs.display()))?;
    let stdin = child.stdin.take().expect("stdin is piped");
    let streamed = stream(image, stdin);
    let output = child
        .wait_with_output()
        .with_context(|| mksquashfs.display().to_string())?;
    if !output.status.success() {
        bail!(
            "{} failed: {}",
            mksquashfs.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    streamed.context("writing the live system's image")
}

/// Writes `image`'s entries to `to` as a tar archive.
fn stream(image: &ClosureImage, to: impl Write) -> Result<()> {
    let mut tar = tar::Builder::new(io::BufWriter::new(to));
    tar.mode(tar::HeaderMode::Deterministic);
    for entry in &image.entries {
        append(&mut tar, entry).with_context(|| format!("/usr/{}", entry.path))?;
    }
    tar.into_inner()?.flush()?;
    Ok(())
}

fn append(tar: &mut tar::Builder<impl Write>, entry: &ImageEntry) -> Result<()> {
    let records: Vec<(String, &[u8])> = (entry.attributes.iter())
        .map(|(name, value)| (format!("{XATTR_RECORD}{name}"), value.as_slice()))
        .collect();
    tar.append_pax_extensions(records.iter().map(|(key, value)| (key.as_str(), *value)))?;

    let mut header = tar::Header::new_gnu();
    header.set_mode(entry.mode);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    match &entry.kind {
        ImageKind::Dir => {
            header.set_entry_type(tar::EntryType::Directory);
            header.set_size(0);
            tar.append_data(&mut header, format!("{}/", entry.path), io::empty())?;
        }
        ImageKind::Blob(blob) => {
            let file = File::open(blob).with_context(|| blob.display().to_string())?;
            header.set_entry_type(tar::EntryType::Regular);
            header.set_size(file.metadata()?.len());
            tar.append_data(&mut header, &entry.path, file)?;
        }
        ImageKind::Bytes(bytes) => {
            header.set_entry_type(tar::EntryType::Regular);
            header.set_size(bytes.len() as u64);
            tar.append_data(&mut header, &entry.path, bytes.as_slice())?;
        }
        ImageKind::Symlink(target) => {
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            tar.append_link(&mut header, &entry.path, target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn entry(path: &str, mode: u32, kind: ImageKind) -> ImageEntry {
        ImageEntry {
            path: path.to_owned(),
            mode,
            kind,
            attributes: vec![
                (
                    ppk::DESCRIPTOR_ATTRIBUTE,
                    vec![1, 0, 0x80, 0xff, b'\n', b'='],
                ),
                (ppk::SIGNATURE_ATTRIBUTE, vec![0; 3310]),
            ],
        }
    }

    #[test]
    fn each_entry_streams_with_its_attributes_as_they_are() {
        let long = format!("share/{}/file", "d".repeat(120));
        let image = ClosureImage {
            identity: ppk::Identity::of(b""),
            root: entry("", 0o755, ImageKind::Dir),
            entries: vec![
                entry("bin", 0o755, ImageKind::Dir),
                entry("bin/hello", 0o555, ImageKind::Bytes(b"binary".to_vec())),
                entry("bin/hi", 0o777, ImageKind::Symlink("hello".to_owned())),
                entry(&long, 0o444, ImageKind::Bytes(b"docs".to_vec())),
            ],
            root_links: Vec::new(),
        };
        let mut tar = Vec::new();
        stream(&image, &mut tar).unwrap();

        // Raw, with each PAX header an entry of its own, read here as the
        // length-prefixed records it is: the tar crate's own reader splits
        // them at newlines, which a binary value may hold.
        let mut archive = tar::Archive::new(tar.as_slice());
        let mut entries = archive.entries().unwrap().raw(true);
        let mut seen = Vec::new();
        while let Some(pax) = entries.next() {
            let mut pax = pax.unwrap();
            assert_eq!(pax.header().entry_type(), tar::EntryType::XHeader);
            let mut records = Vec::new();
            pax.read_to_end(&mut records).unwrap();
            let attributes = pax_records(&records);
            let mut each = entries.next().unwrap().unwrap();
            let mut long_name = None;
            if each.header().entry_type() == tar::EntryType::GNULongName {
                let mut name = Vec::new();
                each.read_to_end(&mut name).unwrap();
                let name = String::from_utf8(name).unwrap();
                long_name = Some(name.trim_end_matches('\0').to_owned());
                each = entries.next().unwrap().unwrap();
            }
            let header = each.header();
            let path =
                long_name.unwrap_or_else(|| header.path().unwrap().to_string_lossy().into_owned());
            let (mode, mtime, uid) = (
                header.mode().unwrap(),
                header.mtime().unwrap(),
                header.uid().unwrap(),
            );
            let mut content = Vec::new();
            each.read_to_end(&mut content).unwrap();
            seen.push((
                path.trim_end_matches('/').to_owned(),
                mode,
                mtime,
                uid,
                attributes,
                content,
            ));
        }

        let expected = &image.entries[0].attributes;
        let paths: Vec<_> = seen.iter().map(|each| (each.0.as_str(), each.1)).collect();
        assert_eq!(
            paths,
            [
                ("bin", 0o755),
                ("bin/hello", 0o555),
                ("bin/hi", 0o777),
                (long.as_str(), 0o444)
            ]
        );
        for (path, _, mtime, uid, attributes, _) in &seen {
            assert_eq!((*mtime, *uid), (0, 0), "{path}");
            let names: Vec<_> = attributes
                .iter()
                .map(|(name, value)| (name.as_str(), value.clone()))
                .collect();
            let wanted: Vec<_> = expected
                .iter()
                .map(|(name, value)| (*name, value.clone()))
                .collect();
            assert_eq!(names, wanted, "{path}");
        }
        assert_eq!(seen[1].5, b"binary");
        assert_eq!(seen[3].5, b"docs");
    }

    /// The `SCHILY.xattr` records of a PAX header: `<length> <key>=<value>\n`,
    /// the length counting the whole record.
    fn pax_records(mut bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut records = Vec::new();
        while !bytes.is_empty() {
            let space = bytes.iter().position(|b| *b == b' ').unwrap();
            let length: usize = std::str::from_utf8(&bytes[..space])
                .unwrap()
                .parse()
                .unwrap();
            let record = &bytes[space + 1..length - 1];
            let equals = record.iter().position(|b| *b == b'=').unwrap();
            let key = std::str::from_utf8(&record[..equals]).unwrap();
            let name = key.strip_prefix(XATTR_RECORD).unwrap();
            records.push((name.to_owned(), record[equals + 1..].to_vec()));
            assert_eq!(bytes[length - 1], b'\n');
            bytes = &bytes[length..];
        }
        records
    }
}
