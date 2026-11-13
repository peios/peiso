//! Stage the spec's registry policy beside the live profile and store.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result};
use ppk::{Extras, PackageId, Request};

use crate::spec::Seed;

const DRAIN: &str = "#!/bin/sh\n\
# Placed by peiso. Apply the image's registry seeds, draining each after it
# applies (--once-delete). Run every boot by peinit; a no-op once the queues are
# empty.
#
# Two directories, in this order: autoapply.d is what this system applies
# wherever it is running, autoapply.live.d is what only the boot medium applies.
# An installer deletes the second from the target, so a disk drains the first
# alone. autoapply.install.d is deliberately absent -- it is the installer's to
# promote, and applying it here would run a first-boot flow on the medium.
set -e
/bin/reg apply --dir /lcl/policy/autoapply.d --once-delete
exec /bin/reg apply --dir /lcl/policy/autoapply.live.d --once-delete
";

/// Stage only requested policy, returning its content fingerprint for the
/// medium identifier. Named masters come only from the live closure, the
/// packages `live` made for `requests`.
pub fn stage(
    seeds: &[Seed],
    live: &[PackageId],
    requests: &[Request],
    store: &Path,
    work: &Path,
    medium: &Path,
) -> Result<String> {
    if seeds.is_empty() {
        return Ok(String::new());
    }
    let root = work.join("seed-masters");
    if seeds.iter().any(|seed| seed.file.is_none()) {
        let composed = ppk::compose_env_for(live, requests, store, &root, &Extras::default())
            .context("composing the live closure for registry seed masters")?;
        super::warn_unstamped(composed.unstamped, "the seed masters' live closure");
    }
    let mut fingerprint = String::new();
    for seed in seeds {
        let source = seed.file.clone().unwrap_or_else(|| {
            root.join("usr/share/regim")
                .join(format!("{}.reg", seed.name))
        });
        let content = fs::read(&source).with_context(|| match &seed.file {
            Some(_) => format!("registry seed {:?}: reading {}", seed.name, source.display()),
            None => format!(
                "registry seed {:?}: no readable usr/share/regim/{}.reg in the live closure (a live package must ship it)",
                seed.name, seed.name
            ),
        })?;
        let path = format!("lcl/policy/{}/{}.reg", seed.queue, seed.name);
        write(medium, &path, &content, 0o644)?;
        fingerprint.push_str(&format!("{path} 644 {}\n", ppk::Identity::of(&content)));
        eprintln!("peiso: staged seed {} -> {}", seed.name, seed.queue);
    }
    let path = "lcl/policy/autorun.d/10-apply-seeds.sh";
    write(medium, path, DRAIN.as_bytes(), 0o755)?;
    fingerprint.push_str(&format!(
        "{path} 755 {}\n",
        ppk::Identity::of(DRAIN.as_bytes())
    ));
    Ok(fingerprint)
}

fn write(root: &Path, path: &str, content: &[u8], mode: u32) -> Result<()> {
    let path = root.join(path);
    fs::create_dir_all(path.parent().expect("policy files have a parent"))?;
    fs::write(&path, content).with_context(|| path.display().to_string())?;
    fs::set_permissions(&path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::Spec;

    #[test]
    fn policy_tracks_content_and_queue_and_uses_only_live_masters() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("store");
        let tree = dir.path().join("package");
        fs::create_dir_all(tree.join("share/regim")).unwrap();
        fs::write(tree.join("share/regim/master.reg"), "master content").unwrap();
        let id: PackageId = "dev.peios.seed@1-1?arch=x86_64".parse().unwrap();
        ppk::pack(&id, &tree, &store).unwrap();
        let other: PackageId = "dev.peios.other@1-1?arch=x86_64".parse().unwrap();
        fs::remove_file(tree.join("share/regim/master.reg")).unwrap();
        ppk::pack(&other, &tree, &store).unwrap();
        fs::write(dir.path().join("custom.reg"), "custom content").unwrap();
        let spec = |policy: &str| {
            Spec::parse(&format!(
            "disable_default_repos = true\n[[repository]]\nfilepath = \"store\"\n[kernel]\nfile = \"kernel\"\n{policy}"
        ), dir.path()).unwrap()
        };
        let run = |policy: &str, live: &[PackageId]| {
            let work = tempfile::tempdir_in(dir.path()).unwrap();
            let medium = work.path().join("iso");
            let requests = ppk::exact_requests(live).unwrap();
            let fingerprint = stage(
                &spec(policy).seeds,
                live,
                &requests,
                &store,
                work.path(),
                &medium,
            );
            (work, medium, fingerprint)
        };
        let (_, medium, empty) = run("", &[]);
        assert_eq!(empty.unwrap(), "");
        assert!(!medium.exists());
        let policy = "[seeds]\napply = [\"master\"]\nlive = [\"custom.reg\"]";
        let (_work, medium, fingerprint) = run(policy, std::slice::from_ref(&id));
        let fingerprint = fingerprint.unwrap();
        for (path, content, mode) in [
            ("autoapply.d/master.reg", "master content", 0o644),
            ("autoapply.live.d/custom.reg", "custom content", 0o644),
            ("autorun.d/10-apply-seeds.sh", DRAIN, 0o755),
        ] {
            let path = medium.join("lcl/policy").join(path);
            assert_eq!(fs::read_to_string(&path).unwrap(), content);
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                mode
            );
        }
        assert_eq!(
            run(policy, std::slice::from_ref(&id)).2.unwrap(),
            fingerprint
        );
        fs::write(dir.path().join("custom.reg"), "changed content").unwrap();
        assert_ne!(
            run(policy, std::slice::from_ref(&id)).2.unwrap(),
            fingerprint
        );
        let install = run("[seeds]\ninstall = [\"custom.reg\"]", &[]);
        assert!(
            install
                .1
                .join("lcl/policy/autoapply.install.d/custom.reg")
                .is_file()
        );
        assert_ne!(
            install.2.unwrap(),
            run("[seeds]\nlive = [\"custom.reg\"]", &[]).2.unwrap()
        );
        let error = format!(
            "{:#}",
            run("[seeds]\napply = [\"master\"]", &[other])
                .2
                .unwrap_err()
        );
        assert!(error.contains("registry seed \"master\""), "{error}");
        assert!(error.contains("live closure"), "{error}");
        let error = format!(
            "{:#}",
            run("[seeds]\nlive = [\"missing.reg\"]", &[]).2.unwrap_err()
        );
        assert!(error.contains("registry seed \"missing\""), "{error}");
    }
}
