use std::fs;

use super::*;
use crate::spec::Spec;

/// What a packed package needs and provides, each as `name` and
/// `constraint`, or `name` and version.
#[derive(Default)]
struct Needs<'a> {
    dependencies: &'a [(&'a str, &'a str)],
    requires: &'a [(&'a str, &'a str)],
    provides: &'a [(&'a str, &'a str)],
}

/// Packs `id`, holding one file of `content` and with `needs`, into the
/// store `dir`.
fn pack(dir: &Path, id: &str, content: &str, needs: Needs) {
    let tree = tempfile::tempdir().unwrap();
    fs::write(tree.path().join("file"), content).unwrap();
    let constraints = |list: &[(&str, &str)]| {
        (list.iter())
            .map(|(name, constraint)| {
                let constraint = Constraint::from_manifest_string(constraint).unwrap();
                ((*name).to_owned(), constraint)
            })
            .collect()
    };
    let metadata = ppk::Metadata {
        dependencies: constraints(needs.dependencies),
        requires: constraints(needs.requires),
        provides: (needs.provides.iter())
            .map(|(name, version)| {
                let version = ppk::Version::parse_relaxed(version).unwrap();
                ((*name).to_owned(), Some(version))
            })
            .collect(),
        ..ppk::Metadata::default()
    };
    ppk::pack_with(&id.parse().unwrap(), &metadata, tree.path(), dir).unwrap();
}

/// A spec searching `repositories`, in order, with `rest` after them.
fn spec(repositories: &[&Path], rest: &str) -> Spec {
    let mut text = String::from("disable_default_repos = true\n");
    text.push_str(rest);
    for repository in repositories {
        text.push_str(&format!(
            "\n[[repository]]\nfilepath = \"{}\"\n",
            repository.display()
        ));
    }
    if !text.contains("[kernel]") {
        text.push_str("[kernel]\nfile = \"vmlinuz\"\n");
    }
    Spec::parse(&text, Path::new("/")).unwrap()
}

fn resolved(spec: &Spec) -> Resolved {
    let index = tempfile::tempdir().unwrap();
    resolve(spec, "x86_64", index.path()).unwrap()
}

fn error(spec: &Spec) -> String {
    let index = tempfile::tempdir().unwrap();
    format!("{:#}", resolve(spec, "x86_64", index.path()).unwrap_err())
}

/// Each package as `name@version`.
fn short<'a>(ids: impl IntoIterator<Item = &'a PackageId>) -> Vec<String> {
    (ids.into_iter())
        .map(|id| format!("{}@{}", id.name(), id.version()))
        .collect()
}

fn store(resolved: &Resolved) -> Vec<String> {
    short(resolved.store.iter().map(|p| &p.id))
}

/// A live package, `a.b.live`, depending on `a.b.lib`, and a boot package,
/// `a.b.boot`, requiring `sh`, which `a.b.shell` provides and which depends on
/// `a.b.lib` too.
fn closures(dir: &Path) {
    let lib = [("a.b.lib", "")];
    pack(
        dir,
        "a.b.live@1.0-1?arch=x86_64",
        "live",
        Needs {
            dependencies: &lib,
            ..Needs::default()
        },
    );
    pack(
        dir,
        "a.b.boot@1.0-1?arch=x86_64",
        "boot",
        Needs {
            requires: &[("sh", ">= 1")],
            ..Needs::default()
        },
    );
    pack(
        dir,
        "a.b.shell@1.0-1?arch=x86_64",
        "shell",
        Needs {
            dependencies: &lib,
            provides: &[("sh", "1")],
            ..Needs::default()
        },
    );
    pack(dir, "a.b.lib@1.0-1?arch=x86_64", "lib", Needs::default());
}

const PLACED: &str = "[[package]]\nname = \"a.b.live\"\n\
                      [[package]]\nname = \"a.b.boot\"\nliveboot = false\ninitramfs = true\n";

#[test]
fn each_place_holds_the_closure_of_its_packages() {
    let repo = tempfile::tempdir().unwrap();
    closures(repo.path());
    let resolved = resolved(&spec(&[repo.path()], PLACED));
    assert_eq!(
        store(&resolved),
        [
            "a.b.boot@1.0-1",
            "a.b.lib@1.0-1",
            "a.b.live@1.0-1",
            "a.b.shell@1.0-1"
        ]
    );
    assert_eq!(
        short(&resolved.liveboot),
        ["a.b.lib@1.0-1", "a.b.live@1.0-1"]
    );
    assert_eq!(
        short(&resolved.initramfs),
        ["a.b.boot@1.0-1", "a.b.lib@1.0-1", "a.b.shell@1.0-1"]
    );
    let reason = |name: &str| {
        let package = resolved.store.iter().find(|p| p.id.name() == name);
        package.unwrap().reason.clone()
    };
    assert_eq!(reason("a.b.live"), "the request for a.b.live");
    assert_eq!(reason("a.b.shell"), "a.b.boot@1.0-1 requires sh >= 1");
    assert_eq!(resolved.store[0].repository, repo.path());
}

#[test]
fn a_place_holds_the_versions_the_store_does() {
    let repo = tempfile::tempdir().unwrap();
    pack(
        repo.path(),
        "a.b.lib@1.0-1?arch=x86_64",
        "1",
        Needs::default(),
    );
    pack(
        repo.path(),
        "a.b.lib@2.0-1?arch=x86_64",
        "2",
        Needs::default(),
    );
    pack(
        repo.path(),
        "a.b.boot@1.0-1?arch=x86_64",
        "boot",
        Needs {
            dependencies: &[("a.b.lib", "")],
            ..Needs::default()
        },
    );
    // Resolved on its own, the initramfs would take a.b.lib's newest version,
    // but the live system needs the older one, which the store holds.
    let resolved = resolved(&spec(
        &[repo.path()],
        "[[package]]\nname = \"a.b.lib\"\nversion = \"1.0-1\"\n\
         [[package]]\nname = \"a.b.boot\"\nliveboot = false\ninitramfs = true\n",
    ));
    assert_eq!(store(&resolved), ["a.b.boot@1.0-1", "a.b.lib@1.0-1"]);
    assert_eq!(
        short(&resolved.initramfs),
        ["a.b.boot@1.0-1", "a.b.lib@1.0-1"]
    );
}

#[test]
fn a_version_is_the_newest_that_will_do() {
    let (first, second) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    for version in ["1.0-1", "2.0-1", "3.0-1"] {
        let id = format!("a.b.c@{version}?arch=x86_64");
        pack(second.path(), &id, version, Needs::default());
    }
    pack(
        first.path(),
        "a.b.c@2.0-1?arch=x86_64",
        "2.0-1",
        Needs::default(),
    );
    // Another arch's version is no choice.
    pack(
        second.path(),
        "a.b.c@9.0-1?arch=aarch64",
        "9",
        Needs::default(),
    );
    for (version, chosen, from) in [
        ("", "a.b.c@3.0-1", second.path()),
        ("version = \"2.0-1\"", "a.b.c@2.0-1", first.path()),
        ("version = \"< 2\"", "a.b.c@1.0-1", second.path()),
    ] {
        let resolved = resolved(&spec(
            &[first.path(), second.path()],
            &format!("[[package]]\nname = \"a.b.c\"\ninitramfs = true\n{version}\n"),
        ));
        assert_eq!(store(&resolved), [chosen], "{version}");
        assert_eq!(resolved.store[0].repository, from, "{version}");
    }
}

#[test]
fn what_is_needed_must_be_found() {
    let repo = tempfile::tempdir().unwrap();
    pack(
        repo.path(),
        "a.b.c@1.0-1?arch=x86_64",
        "a",
        Needs::default(),
    );
    let found = error(&spec(
        &[repo.path()],
        "[[package]]\nname = \"a.b.c\"\ninitramfs = true\nversion = \"3.0-1\"\n",
    ));
    assert!(found.contains("a.b.c = 3.0-1"), "{found}");
    let found = error(&spec(
        &[repo.path()],
        "[[package]]\nname = \"x.y.z\"\ninitramfs = true\n",
    ));
    assert!(found.contains("x.y.z"), "{found}");
}

#[test]
fn repositories_must_agree_on_what_a_version_is() {
    let (first, second) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let id = "a.b.c@1.0-1?arch=x86_64";
    pack(first.path(), id, "one thing", Needs::default());
    pack(second.path(), id, "another", Needs::default());
    let found = error(&spec(
        &[first.path(), second.path()],
        "[[package]]\nname = \"a.b.c\"\ninitramfs = true\n",
    ));
    assert!(
        found.starts_with("a.b.c@1.0-1?arch=x86_64 is sha256:"),
        "{found}"
    );
}

#[test]
fn an_edition_is_the_live_system_and_all_it_needs() {
    let repo = tempfile::tempdir().unwrap();
    closures(repo.path());
    pack(
        repo.path(),
        "dev.peios.peios-experimental@2026.8-1?arch=x86_64",
        "edition",
        Needs {
            dependencies: &[("a.b.live", ">= 1.0-1")],
            ..Needs::default()
        },
    );
    let resolved = resolved(&spec(
        &[repo.path()],
        "edition = \"Experimental\"\n\
         [[package]]\nname = \"a.b.boot\"\nliveboot = false\ninitramfs = true\n",
    ));
    assert_eq!(
        short(&resolved.liveboot),
        [
            "a.b.lib@1.0-1",
            "a.b.live@1.0-1",
            "dev.peios.peios-experimental@2026.8-1"
        ]
    );
    assert_eq!(
        short(&resolved.initramfs),
        ["a.b.boot@1.0-1", "a.b.lib@1.0-1", "a.b.shell@1.0-1"]
    );
}

#[test]
fn a_kernel_package_is_carried_but_not_placed() {
    let repo = tempfile::tempdir().unwrap();
    closures(repo.path());
    pack(
        repo.path(),
        "dev.peios.kernel@1.0-1?arch=x86_64",
        "kernel",
        Needs::default(),
    );
    let resolved = resolved(&spec(
        &[repo.path()],
        &format!("[kernel]\npackage = \"dev.peios.kernel\"\n{PLACED}"),
    ));
    assert!(store(&resolved).contains(&"dev.peios.kernel@1.0-1".to_owned()));
    for place in [&resolved.liveboot, &resolved.initramfs] {
        assert!(!place.iter().any(|id| id.name() == "dev.peios.kernel"));
    }
}

#[test]
fn a_place_must_have_a_package() {
    let repo = tempfile::tempdir().unwrap();
    closures(repo.path());
    for (packages, message) in [
        (
            "[[package]]\nname = \"a.b.boot\"\nliveboot = false\ninitramfs = true\n",
            "no package has liveboot = true, so the live system would be empty",
        ),
        (
            "[[package]]\nname = \"a.b.live\"\n",
            "no package has initramfs = true, so nothing would boot",
        ),
    ] {
        assert_eq!(error(&spec(&[repo.path()], packages)), message);
    }
}
