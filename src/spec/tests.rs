use super::*;

const KERNEL: &str = "[kernel]\nfile = \"vmlinuz\"\ncmdline = \"quiet\"\n";
const REPOSITORY: &str = "disable_default_repos = true\n[[repository]]\nfilepath = \"repo\"\n";

/// Parses `text` within an otherwise complete spec. It comes first, so that
/// its top-level keys are not read as part of a table.
fn parse(text: &str) -> Result<Spec> {
    Spec::parse(
        &format!(
            "disable_default_repos = true\n{text}\n[[repository]]\nfilepath = \"repo\"\n{KERNEL}"
        ),
        Path::new("/specs"),
    )
}

fn error(text: &str) -> String {
    format!("{:#}", parse(text).unwrap_err())
}

#[test]
fn a_spec_names_its_repositories_kernel_and_packages() {
    let spec = parse(
        "[[package]]\nname = \"net.busybox.busybox\"\n\n\
         [[package]]\nname = \"dev.peios.prelude\"\nversion = \"0.0.7-1\"\n\
         liveboot = false\ninitramfs = true\n",
    )
    .unwrap();
    // Paths are relative to the spec's directory.
    assert_eq!(spec.repositories, [PathBuf::from("/specs/repo")]);
    assert_eq!(spec.kernel.file, PathBuf::from("/specs/vmlinuz"));
    assert_eq!(spec.kernel.cmdline, "quiet");
    let packages: Vec<_> = (spec.packages.iter())
        .map(|p| {
            (
                p.name.as_str(),
                p.version.as_deref(),
                p.liveboot,
                p.initramfs,
            )
        })
        .collect();
    assert_eq!(
        packages,
        [
            ("net.busybox.busybox", None, true, false),
            ("dev.peios.prelude", Some("0.0.7-1"), false, true),
        ]
    );
    assert_eq!(spec.initramfs, Layout::initramfs());
    assert_eq!(spec.liveboot, Layout::liveboot());
}

#[test]
fn a_layout_replaces_the_default() {
    let spec =
        parse("[initramfs]\ndirs = [\"proc\"]\nsymlinks = { \"init\" = \":bin/sh\" }\n").unwrap();
    assert_eq!(spec.initramfs.dirs, ["proc".parse::<EnvDir>().unwrap()]);
    let links: Vec<_> = (spec.initramfs.symlinks.iter())
        .map(|link| (link.path(), link.target()))
        .collect();
    assert_eq!(links, [("init", "usr/bin/sh")]);
    assert_eq!(spec.liveboot, Layout::liveboot());
    assert!(error("[liveboot]\npackages = {}").starts_with("liveboot: unknown key \"packages\""));
}

#[test]
fn what_needs_more_than_exists_is_refused_with_the_reason() {
    for (text, message) in [
        (
            "edition = \"Experimental\"",
            "edition: editions need package dependencies",
        ),
        (
            "edition_package = \"com.example.peios-inhouse\"",
            "edition_package: editions need",
        ),
        ("version = \"0.1.0\"", "version: editions need"),
        (
            "[[package]]\nname = \"a.b.c\"\nautoinstall = true",
            "package a.b.c: autoinstall: there is no installer yet",
        ),
        (
            "[[package]]\nname = \"a.b.c\"\nofferinstall = true",
            "package a.b.c: offerinstall: there is no installer yet",
        ),
        (
            "[[repository]]\nurl = \"https://pkgs.peios.org\"",
            "repository 1: url: repositories on the network are not supported yet",
        ),
    ] {
        let found = error(text);
        assert!(found.starts_with(message), "{text}: {found}");
    }
    let found = format!(
        "{:#}",
        Spec::parse(KERNEL, Path::new("/specs")).unwrap_err()
    );
    assert!(
        found.starts_with("the default repository, pkgs.peios.org, is not available yet"),
        "{found}"
    );
}

#[test]
fn a_malformed_spec_is_refused() {
    for (text, message) in [
        ("editon = \"x\"", "unknown key \"editon\""),
        ("[[package]]\nversion = \"1\"", "package 1: no name"),
        (
            "[[package]]\nname = \"a.b.c\"\nlivebot = false",
            "package 1: unknown key \"livebot\"",
        ),
        (
            "[[package]]\nname = \"a.b.c\"\n[[package]]\nname = \"a.b.c\"",
            "package a.b.c: given more than once",
        ),
        (
            "[package]\nname = \"a.b.c\"",
            "package is not a list of tables",
        ),
        (
            "[[package]]\nname = \"a.b.c\"\nliveboot = \"no\"",
            "package a.b.c: liveboot is",
        ),
    ] {
        let found = error(text);
        assert!(found.starts_with(message), "{text}: {found}");
    }
    let no_kernel = Spec::parse(REPOSITORY, Path::new("/specs")).unwrap_err();
    assert_eq!(format!("{no_kernel:#}"), "no [kernel]");
    let no_repository = Spec::parse(
        &format!("disable_default_repos = true\n{KERNEL}"),
        Path::new("/specs"),
    )
    .unwrap_err();
    assert_eq!(
        format!("{no_repository:#}"),
        "no [[repository]]: there is nowhere to find packages"
    );
}
