//! Lists the packages a spec resolves to, with where each goes and why it
//! is carried.

use std::fs;
use std::path::Path;
use std::process::Command;

/// Packs `id`, holding `bin/<file>` and depending on each of
/// `dependencies`, into the store `dir`.
fn pack(dir: &Path, id: &str, file: &str, dependencies: &[&str]) {
    let tree = tempfile::tempdir().unwrap();
    fs::create_dir(tree.path().join("bin")).unwrap();
    fs::write(tree.path().join("bin").join(file), file).unwrap();
    let metadata = ppk::Metadata {
        dependencies: (dependencies.iter())
            .map(|name| ((*name).to_owned(), ppk::Constraint::default()))
            .collect(),
        ..ppk::Metadata::default()
    };
    ppk::pack_with(&id.parse().unwrap(), &metadata, tree.path(), dir).unwrap();
}

#[test]
fn resolve_lists_each_package_with_where_it_goes() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    pack(
        &repo,
        "dev.peios.live@1.0-1?arch=x86_64",
        "live",
        &["dev.peios.library"],
    );
    pack(
        &repo,
        "dev.peios.boot@1.0-1?arch=x86_64",
        "boot",
        &["dev.peios.library"],
    );
    pack(&repo, "dev.peios.library@1.0-1?arch=x86_64", "library", &[]);
    pack(&repo, "dev.peios.kernel@1.0-1?arch=x86_64", "kernel", &[]);
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
    let output = Command::new(env!("CARGO_BIN_EXE_peiso"))
        .arg("resolve")
        .arg(&spec)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "dev.peios.boot@1.0-1?arch=x86_64 initramfs (the request for dev.peios.boot)\n\
         dev.peios.kernel@1.0-1?arch=x86_64 store (the request for dev.peios.kernel)\n\
         dev.peios.library@1.0-1?arch=x86_64 liveboot,initramfs \
         (dev.peios.live@1.0-1 depends on dev.peios.library)\n\
         dev.peios.live@1.0-1?arch=x86_64 liveboot (the request for dev.peios.live)\n"
    );
}
