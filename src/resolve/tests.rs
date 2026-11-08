use std::fs;

use super::*;
use crate::spec::Spec;

/// Packs `id`, holding one file of `content`, into the store `dir`.
fn pack(dir: &Path, id: &str, content: &str) {
    let tree = tempfile::tempdir().unwrap();
    fs::write(tree.path().join("file"), content).unwrap();
    ppk::pack(&id.parse().unwrap(), tree.path(), dir).unwrap();
}

/// A spec searching `repositories`, in order, for `packages`.
fn spec(repositories: &[&Path], packages: &str) -> Spec {
    let mut text = String::from("disable_default_repos = true\n");
    for repository in repositories {
        text.push_str(&format!(
            "[[repository]]\nfilepath = \"{}\"\n",
            repository.display()
        ));
    }
    text.push_str("[kernel]\nfile = \"vmlinuz\"\n");
    text.push_str(packages);
    Spec::parse(&text, Path::new("/")).unwrap()
}

fn error(spec: &Spec) -> String {
    format!("{:#}", resolve(spec, "x86_64").unwrap_err())
}

#[test]
fn a_package_is_found_at_its_one_version_in_the_first_repository() {
    let (first, second) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    pack(second.path(), "a.b.c@1.0-1?arch=x86_64", "a");
    pack(first.path(), "d.e.f@2.0-1?arch=x86_64", "d");
    pack(second.path(), "d.e.f@2.0-1?arch=x86_64", "d");
    // Another arch's version is no choice.
    pack(second.path(), "a.b.c@9.0-1?arch=aarch64", "a");
    let spec = spec(
        &[first.path(), second.path()],
        "[[package]]\nname = \"a.b.c\"\n[[package]]\nname = \"d.e.f\"\nversion = \"2.0-1\"\n",
    );
    let resolved = resolve(&spec, "x86_64").unwrap();
    let found: Vec<_> = (resolved.iter())
        .map(|r| (r.id.to_string(), r.repository.clone()))
        .collect();
    assert_eq!(
        found,
        [
            (
                "a.b.c@1.0-1?arch=x86_64".to_owned(),
                second.path().to_path_buf()
            ),
            (
                "d.e.f@2.0-1?arch=x86_64".to_owned(),
                first.path().to_path_buf()
            ),
        ]
    );
}

#[test]
fn a_version_must_be_found_and_unambiguous() {
    let (first, second) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    pack(first.path(), "a.b.c@1.0-1?arch=x86_64", "a");
    pack(second.path(), "a.b.c@2.0-1?arch=x86_64", "a");
    let repositories = [first.path(), second.path()];
    assert_eq!(
        error(&spec(&repositories, "[[package]]\nname = \"a.b.c\"\n")),
        "a.b.c has several versions, 1.0-1, 2.0-1: give one with version = \"...\""
    );
    assert_eq!(
        error(&spec(&repositories, "[[package]]\nname = \"x.y.z\"\n")),
        "x.y.z is in no repository for x86_64"
    );
    assert_eq!(
        error(&spec(
            &repositories,
            "[[package]]\nname = \"a.b.c\"\nversion = \"3.0-1\"\n"
        )),
        "a.b.c@3.0-1?arch=x86_64 is in no repository"
    );
}

#[test]
fn repositories_must_agree_on_what_a_version_is() {
    let (first, second) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    pack(first.path(), "a.b.c@1.0-1?arch=x86_64", "one thing");
    pack(second.path(), "a.b.c@1.0-1?arch=x86_64", "another");
    let found = error(&spec(
        &[first.path(), second.path()],
        "[[package]]\nname = \"a.b.c\"\n",
    ));
    assert!(
        found.starts_with("a.b.c@1.0-1?arch=x86_64 is sha256:"),
        "{found}"
    );
}
