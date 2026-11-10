//! Builds an ISO end to end, with the host's mkfs.vfat, mtools and xorriso,
//! and stand-ins for mkirf and mkuki that record what peiso gives them.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

/// Packs `id`, holding `bin/<file>`, into the store `dir`.
fn pack(dir: &Path, id: &str, file: &str) {
    pack_claiming(dir, id, file, &[]);
}

/// Packs `id`, holding `bin/<file>` and claiming each of `claims`, a path
/// and its link's target, into the store `dir`.
fn pack_claiming(dir: &Path, id: &str, file: &str, claims: &[(&str, &str)]) {
    let tree = tempfile::tempdir().unwrap();
    fs::create_dir(tree.path().join("bin")).unwrap();
    fs::write(tree.path().join("bin").join(file), file).unwrap();
    let metadata = ppk::Metadata {
        claims: (claims.iter())
            .map(|(path, target)| ((*path).to_owned(), (*target).to_owned()))
            .collect(),
        ..ppk::Metadata::default()
    };
    ppk::pack_with(&id.parse().unwrap(), &metadata, tree.path(), dir).unwrap();
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

/// The text of the file `path` on the ISO `iso`, extracted into `dir`.
fn from_iso(iso: &Path, path: &str, dir: &Path) -> String {
    let out = dir.join("extracted");
    let _ = fs::remove_file(&out);
    let status = Command::new("xorriso")
        .args(["-osirrox", "on", "-indev"])
        .arg(iso)
        .args(["-extract", path])
        .arg(&out)
        .output()
        .unwrap()
        .status;
    assert!(status.success(), "{path}");
    fs::read_to_string(&out).unwrap()
}

#[test]
fn build_writes_an_iso_carrying_every_package_and_booting_the_initramfs() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    pack(&repo, "dev.peios.live@1.0-1?arch=x86_64", "live");
    // The initramfs's init, claimed by its package as prelude claims it.
    pack_claiming(
        &repo,
        "dev.peios.boot@1.0-1?arch=x86_64",
        "boot",
        &[("/init", "usr/bin/boot")],
    );
    pack(&repo, "dev.peios.library@1.0-1?arch=x86_64", "library");
    fs::write(dir.path().join("vmlinuz"), "kernel").unwrap();

    let capture = dir.path().join("capture");
    fs::create_dir(&capture).unwrap();
    let mkirf = dir.path().join("mkirf");
    script(
        &mkirf,
        "#!/bin/sh\n\
         (cd \"$1\" && find . | sort) >\"$CAPTURE/tree\"\n\
         cp \"$1/system/medium\" \"$CAPTURE/medium\"\n\
         readlink \"$1/init\" >\"$CAPTURE/init\"\n\
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

    // The initramfs holds only its own package, with the init it claims,
    // and the identifier of its medium, which the medium holds too.
    let tree = fs::read_to_string(capture.join("tree")).unwrap();
    for path in [
        "./usr/bin/boot",
        "./init",
        "./proc",
        "./sys",
        "./tmp",
        "./system/medium",
    ] {
        assert!(tree.lines().any(|line| line == path), "{path}: {tree}");
    }
    for absent in ["live", "library", "artifacts"] {
        assert!(!tree.contains(absent), "{absent}: {tree}");
    }
    assert_eq!(
        fs::read_to_string(capture.join("init")).unwrap(),
        "usr/bin/boot\n"
    );
    let medium = fs::read_to_string(capture.join("medium")).unwrap();
    assert_eq!(medium.trim().len(), 64, "{medium}");
    assert_eq!(from_iso(&iso, "/peios-medium", dir.path()), medium);

    // The medium holds the live system's profile, naming only the live
    // packages.
    let live = ppk::Profile::parse(&from_iso(&iso, "/live.toml", dir.path())).unwrap();
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
fn build_refuses_a_spec_with_no_live_system_or_nothing_to_boot() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    pack(&repo, "dev.peios.boot@1.0-1?arch=x86_64", "boot");
    fs::write(dir.path().join("vmlinuz"), "kernel").unwrap();
    for (flags, message) in [
        (
            "liveboot = false\ninitramfs = true",
            "no package has liveboot = true, so the live system would be empty",
        ),
        ("", "no package has initramfs = true, so nothing would boot"),
    ] {
        let spec = dir.path().join("spec.toml");
        fs::write(
            &spec,
            format!(
                "disable_default_repos = true\n[[repository]]\nfilepath = \"repo\"\n\
                 [kernel]\nfile = \"vmlinuz\"\n\
                 [[package]]\nname = \"dev.peios.boot\"\n{flags}\n"
            ),
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
            format!("peiso: {message}\n")
        );
    }
}

#[test]
fn build_boots_the_kernel_a_package_holds() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    pack(&repo, "dev.peios.live@1.0-1?arch=x86_64", "live");
    pack(&repo, "dev.peios.boot@1.0-1?arch=x86_64", "boot");
    let tree = tempfile::tempdir().unwrap();
    let modules = tree.path().join("lib/modules");
    fs::create_dir_all(&modules).unwrap();
    fs::write(modules.join("vmlinuz"), "the packaged kernel").unwrap();
    fs::write(modules.join("config"), "config").unwrap();
    ppk::pack(
        &"dev.peios.kernel@1-1?arch=x86_64".parse().unwrap(),
        tree.path(),
        &repo,
    )
    .unwrap();

    let capture = dir.path().join("capture");
    fs::create_dir(&capture).unwrap();
    let mkirf = dir.path().join("mkirf");
    script(&mkirf, "#!/bin/sh\nprintf cpio >\"$2\"\n");
    let mkuki = dir.path().join("mkuki");
    script(
        &mkuki,
        "#!/bin/sh\n\
         while [ $# -gt 0 ]; do\n\
             case $1 in\n\
                 --out) out=$2 ;;\n\
                 --kernel) cp \"$2\" \"$CAPTURE/kernel\" ;;\n\
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
         [kernel]\npackage = \"dev.peios.kernel\"\n\n\
         [[package]]\nname = \"dev.peios.live\"\n\n\
         [[package]]\nname = \"dev.peios.boot\"\nliveboot = false\ninitramfs = true\n",
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

    // The UKI is made from the package's image, and the ISO carries the
    // package in its store.
    assert_eq!(
        fs::read_to_string(capture.join("kernel")).unwrap(),
        "the packaged kernel"
    );
    let refs = on_iso(&iso, "/artifacts/refs");
    assert!(
        refs.contains(&"/artifacts/refs/dev.peios.kernel".to_owned()),
        "{refs:?}"
    );
}
