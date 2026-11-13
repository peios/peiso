//! Choosing the packages an ISO carries: what the spec names, and every
//! package those depend on or require, through ppk's resolver.
//!
//! Everything the spec names is resolved together, so the ISO's store holds
//! one version of each name, which satisfies every package in it. Each
//! place a package can go, the live system and the initramfs, then holds
//! the closure of the packages placed there: what ppk's resolver chooses
//! for them from the store's packages alone, so that a place can't choose
//! another version, or another provider of a capability, than the store
//! holds.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ppk::{Constraint, PackageId, Request};

use crate::spec::{PackageSpec, Spec};

/// The packages an ISO carries.
#[derive(Debug)]
pub struct Resolved {
    /// Every package the ISO's store holds, sorted by name: what the spec
    /// names and everything they need.
    pub store: Vec<Package>,
    /// The packages the live system holds, sorted by name.
    pub liveboot: Vec<PackageId>,
    /// What the live system asks for: each package the spec places there,
    /// exactly at the version chosen. Resolving these from the store gives
    /// [`Resolved::liveboot`].
    pub liveboot_requests: Vec<Request>,
    /// The packages the initramfs holds, sorted by name.
    pub initramfs: Vec<PackageId>,
    /// What the initramfs asks for, as [`Resolved::liveboot_requests`] is
    /// for the live system.
    pub initramfs_requests: Vec<Request>,
}

/// A package the ISO's store holds.
#[derive(Debug)]
pub struct Package {
    pub id: PackageId,
    /// The repository it is taken from: the first that holds it.
    pub repository: PathBuf,
    /// Why it is carried: what first asked for it.
    pub reason: String,
}

/// Chooses the packages of `spec`, for `arch`. The store's packages are
/// indexed in `index`, an empty directory, as an artifact directory holding
/// only their refs and manifests, which the places are resolved from.
pub fn resolve(spec: &Spec, arch: &str, index: &Path) -> Result<Resolved> {
    let placed = |place: fn(&PackageSpec) -> bool| -> Vec<&PackageSpec> {
        spec.packages.iter().filter(|p| place(p)).collect()
    };
    let (liveboot, initramfs) = (placed(|p| p.liveboot), placed(|p| p.initramfs));
    if liveboot.is_empty() {
        bail!("no package has liveboot = true, so the live system would be empty");
    }
    if initramfs.is_empty() {
        bail!("no package has initramfs = true, so nothing would boot");
    }

    let all: Vec<&PackageSpec> = spec.packages.iter().collect();
    let store = ppk::resolve(&requests(&all)?, &spec.repositories, arch)?;
    for package in &store.packages {
        let manifest = (package.artifact_dir.join("manifests"))
            .join(ppk::ALGORITHM)
            .join(package.identity.hex());
        let to = index.join("manifests").join(ppk::ALGORITHM);
        fs::create_dir_all(&to).with_context(|| to.display().to_string())?;
        fs::copy(&manifest, to.join(package.identity.hex()))
            .with_context(|| manifest.display().to_string())?;
        let at = (index.join("refs"))
            .join(package.id.name())
            .join(package.id.version());
        fs::create_dir_all(&at).with_context(|| at.display().to_string())?;
        let reference = at.join(package.id.arch());
        fs::write(&reference, format!("{}\n", package.identity))
            .with_context(|| reference.display().to_string())?;
    }

    let index = [index.to_path_buf()];
    let place = |packages: &[&PackageSpec], what: &str| -> Result<(Vec<PackageId>, Vec<Request>)> {
        let ids = ppk::resolve(&requests(packages)?, &index, arch)
            .context(what.to_owned())?
            .ids();
        let pinned = (packages.iter())
            .map(|package| {
                let id = (ids.iter())
                    .find(|id| id.name() == package.name)
                    .expect("a package asked for is among those chosen");
                Request::package(
                    id.name(),
                    Constraint::parse(&format!("= {}", id.version()))?,
                )
            })
            .collect::<Result<_>>()?;
        Ok((ids, pinned))
    };
    let (liveboot, liveboot_requests) = place(&liveboot, "the live system")?;
    let (initramfs, initramfs_requests) = place(&initramfs, "the initramfs")?;
    Ok(Resolved {
        liveboot,
        liveboot_requests,
        initramfs,
        initramfs_requests,
        store: (store.packages.into_iter())
            .map(|p| Package {
                id: p.id,
                repository: p.artifact_dir,
                reason: p.reason,
            })
            .collect(),
    })
}

/// A request for each of `packages`, at the versions its `version` allows,
/// or any.
fn requests(packages: &[&PackageSpec]) -> Result<Vec<Request>> {
    packages
        .iter()
        .map(|package| {
            let constraint = match &package.version {
                Some(version) => Constraint::parse(version)?,
                None => Constraint::default(),
            };
            Request::package(&package.name, constraint)
        })
        .collect::<Result<_>>()
}

#[cfg(test)]
mod tests;
