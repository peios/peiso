//! Finding each package a spec names in its repositories, at an exact
//! version.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use ppk::PackageId;

use crate::spec::Spec;

/// A package the ISO carries, found.
#[derive(Debug)]
pub struct Resolved {
    pub id: PackageId,
    /// The repository it is taken from: the first that holds it.
    pub repository: PathBuf,
    pub liveboot: bool,
    pub initramfs: bool,
}

/// Finds every package of `spec`, for `arch`. A package named without a
/// version must have one version in all the repositories together, since
/// versions have no order to choose the newest by. The repositories must
/// agree on what each version is.
pub fn resolve(spec: &Spec, arch: &str) -> Result<Vec<Resolved>> {
    let mut resolved = Vec::new();
    for package in &spec.packages {
        let version = match &package.version {
            Some(version) => version.clone(),
            None => only_version(&package.name, arch, &spec.repositories)?,
        };
        let id = PackageId::new(&package.name, &version, arch)?;
        let mut found: Option<(&Path, String)> = None;
        for repository in &spec.repositories {
            let Some(identity) = ppk::packed(&id, repository)? else {
                continue;
            };
            let identity = identity.to_string();
            match &found {
                None => found = Some((repository, identity)),
                Some((first, held)) if *held != identity => bail!(
                    "{id} is {held} in {}, but {identity} in {}",
                    first.display(),
                    repository.display()
                ),
                Some(_) => {}
            }
        }
        let Some((repository, _)) = found else {
            bail!("{id} is in no repository");
        };
        resolved.push(Resolved {
            repository: repository.to_path_buf(),
            liveboot: package.liveboot,
            initramfs: package.initramfs,
            id,
        });
    }
    Ok(resolved)
}

/// The one version of `name` packed for `arch` in `repositories`.
fn only_version(name: &str, arch: &str, repositories: &[PathBuf]) -> Result<String> {
    let mut versions: Vec<String> = Vec::new();
    for repository in repositories {
        for version in ppk::versions(name, arch, repository)? {
            if !versions.contains(&version) {
                versions.push(version);
            }
        }
    }
    match versions.as_slice() {
        [version] => Ok(version.clone()),
        [] => bail!("{name} is in no repository for {arch}"),
        _ => {
            versions.sort();
            bail!(
                "{name} has several versions, {}: give one with version = \"...\"",
                versions.join(", ")
            )
        }
    }
}

#[cfg(test)]
mod tests;
