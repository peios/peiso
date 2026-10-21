//! Builds an ISO end to end, with the host's mkfs.vfat, mtools and xorriso,
//! and stand-ins for mkirf and mkuki that record what peiso gives them.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

/// Packs `id`, holding `bin/<file>`, into the store `dir`.
fn pack(dir: &Path, id: &str, file: &str) {
    let tree = tempfile::tempdir().unwrap();
    fs::create_dir(tree.path().join("bin")).unwrap();
    fs::write(tree.path().join("bin").join(file), file).unwrap();
    ppk::pack(&id.parse().unwrap(), tree.path(), dir).unwrap();
}

/// Writes an executable script `text` at `path`.
fn script(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// The paths under `/` in the ISO `iso` that `find` finds under `under`.
fn on_iso(iso: &Path, under: &str) -> Vec<String> {
    let output = Command::new("xorriso")
        .args(["-indev"])
        .arg(iso)
        .args(["-find", under, "-type", "d"])
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| line.trim_matches('\'').to_owned())
        .collect()
}

#[test]
fn build_writes_an_iso_carrying_every_package_and_booting_the_initramfs() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    pack(&repo, "dev.peios.live@1.0?arch=x86_64", "live");
    pack(&repo, "dev.peios.boot@1.0?arch=x86_64", "boot");
    pack(&repo, "dev.peios.library@1.0?arch=x86_64", "library");
    fs::write(dir.path().join("vmlinuz"), "kernel").unwrap();

    let capture = dir.path().join("capture");
    fs::create_dir(&capture).unwrap();
    let mkirf = dir.path().join("mkirf");
    script(
        &mkirf,
        "#!/bin/sh\n\
         (cd \"$1\" && find . | sort) >\"$CAPTURE/tree\"\n\
         cp \"$1/system/live.toml\" \"$CAPTURE/live.toml\"\n\
         printf cpio >\"$2\"\n",
    );
    let mkuki = dir.path().join("mkuki");
    script(
        &mkuki,
        "#!/bin/sh\n\
         while [ $# -gt 0 ]; do\n\
             case $1 in\n\
                 --out) out=$2 ;;\n\
                 --cmdline) printf '%s' \"$2\" >\"$CAPTURE/cmdline\" ;;\n\
             esac\n\
             shift 2\n\
         done\n\
         head -c 1048576 /dev/zero >\"$out\"\n",
    );

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        "disable_default_repos = true\n\n\
         [[repository]]\nfilepath = \"repo\"\n\n\
         [kernel]\nfile = \"vmlinuz\"\ncmdline = \"console=ttyS0 rd.shell\"\n\n\
         [[package]]\nname = \"dev.peios.live\"\n\n\
         [[package]]\nname = \"dev.peios.boot\"\nliveboot = false\ninitramfs = true\n\n\
         [[package]]\nname = \"dev.peios.library\"\nliveboot = false\n",
    )
    .unwrap();
    let iso = dir.path().join("peios.iso");
    let output = Command::new(env!("CARGO_BIN_EXE_peiso"))
        .arg("build")
        .arg(&spec)
        .arg("--out")
        .arg(&iso)
        .arg("--mkirf")
        .arg(&mkirf)
        .arg("--mkuki")
        .arg(&mkuki)
        .env("CAPTURE", &capture)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("{}\n", iso.display())
    );

    // The ISO's store holds every package, the medium-only one too, and
    // the ISO boots the ESP image.
    let refs = on_iso(&iso, "/artifacts/refs");
    for name in ["dev.peios.live", "dev.peios.boot", "dev.peios.library"] {
        assert!(
            refs.contains(&format!("/artifacts/refs/{name}")),
            "{refs:?}"
        );
    }
    let report = Command::new("xorriso")
        .arg("-indev")
        .arg(&iso)
        .args(["-report_el_torito", "plain"])
        .output()
        .unwrap();
    let report = String::from_utf8_lossy(&report.stdout);
    assert!(report.contains("/boot/efi.img"), "{report}");
    assert!(report.contains("UEFI"), "{report}");

    // The initramfs holds the live and boot packages and their store, but
    // not the medium-only one, with prelude as init.
    let tree = fs::read_to_string(capture.join("tree")).unwrap();
    for path in [
        "./usr/bin/live",
        "./usr/bin/boot",
        "./init",
        "./proc",
        "./sys",
        "./tmp",
        "./system/artifacts/refs/dev.peios.live",
        "./system/artifacts/refs/dev.peios.boot",
    ] {
        assert!(tree.lines().any(|line| line == path), "{path}: {tree}");
    }
    assert!(!tree.contains("library"), "{tree}");

    // The live system's profile names only the live packages.
    let live =
        ppk::Profile::parse(&fs::read_to_string(capture.join("live.toml")).unwrap()).unwrap();
    let names: Vec<&str> = live.packages.iter().map(|id| id.name()).collect();
    assert_eq!(names, ["dev.peios.live"]);
    let links: Vec<&str> = live.symlinks.iter().map(|link| link.path()).collect();
    assert_eq!(links, ["bin", "sbin"]);
    assert_eq!(
        fs::read_to_string(capture.join("cmdline")).unwrap(),
        "console=ttyS0 rd.shell"
    );

    // The work directory beside the ISO is gone.
    let left: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.to_string_lossy().starts_with(".peiso-"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn build_refuses_a_spec_with_no_live_system() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    pack(&repo, "dev.peios.boot@1.0?arch=x86_64", "boot");
    fs::write(dir.path().join("vmlinuz"), "kernel").unwrap();
    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        "disable_default_repos = true\n[[repository]]\nfilepath = \"repo\"\n\
         [kernel]\nfile = \"vmlinuz\"\n\
         [[package]]\nname = \"dev.peios.boot\"\nliveboot = false\ninitramfs = true\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_peiso"))
        .arg("build")
        .arg(&spec)
        .arg("--out")
        .arg(dir.path().join("peios.iso"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "peiso: no package has liveboot = true, so the live system would be empty\n"
    );
}
