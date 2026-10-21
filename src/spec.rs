//! The ISO spec: the TOML that says which packages an ISO carries, where
//! they come from, and where each goes.
//!
//! The spec's whole form is settled, but parts of it need what does not
//! exist yet: editions need package dependencies, the default repository
//! needs pkgs.peios.org, and the install flags need an installer. Those
//! are read, so that a spec written for the whole form is understood, and
//! refused with the reason.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use ppk::{EnvDir, EnvSymlink, Profile};

/// A read spec.
#[derive(Debug)]
pub struct Spec {
    /// Where packages come from, in the order they are searched.
    pub repositories: Vec<PathBuf>,
    pub kernel: Kernel,
    pub packages: Vec<PackageSpec>,
    /// What the initramfs holds besides its packages.
    pub initramfs: Layout,
    /// What the live system holds besides its packages.
    pub liveboot: Layout,
}

/// The kernel the ISO boots, until the kernel is packaged.
#[derive(Debug)]
pub struct Kernel {
    pub image: KernelImage,
    pub cmdline: String,
}

/// Where the kernel image comes from.
#[derive(Debug, PartialEq, Eq)]
pub enum KernelImage {
    /// A file on this machine.
    File(PathBuf),
    /// The package of this name, which the ISO carries as it carries the
    /// spec's packages, with its image at `usr/lib/modules/*/vmlinuz-*`.
    Package(String),
}

/// A package the ISO carries.
#[derive(Debug)]
pub struct PackageSpec {
    pub name: String,
    /// The version to carry, or `None` for the only one the repositories
    /// hold.
    pub version: Option<String>,
    /// Whether the live system holds it.
    pub liveboot: bool,
    /// Whether the initramfs holds it.
    pub initramfs: bool,
}

/// The links and empty directories an environment holds outside `usr`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub symlinks: Vec<EnvSymlink>,
    pub dirs: Vec<EnvDir>,
}

impl Layout {
    /// The initramfs's layout unless the spec gives one: prelude as init,
    /// and mount points for what it mounts.
    pub fn initramfs() -> Self {
        Self::of(&[("init", ":sbin/prelude")], &["proc", "sys", "tmp"])
    }

    /// The live system's layout unless the spec gives one: `/bin` and
    /// `/sbin` in `usr`, and a `/tmp`.
    pub fn liveboot() -> Self {
        Self::of(&[("bin", ":bin"), ("sbin", ":sbin")], &["tmp"])
    }

    fn of(symlinks: &[(&str, &str)], dirs: &[&str]) -> Self {
        Self {
            symlinks: symlinks
                .iter()
                .map(|(path, target)| format!("{path}:{target}").parse().expect("a valid link"))
                .collect(),
            dirs: dirs
                .iter()
                .map(|dir| dir.parse().expect("a valid path"))
                .collect(),
        }
    }
}

impl Spec {
    /// The spec in the file at `path`, whose relative paths are relative
    /// to its directory.
    pub fn read(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).with_context(|| path.display().to_string())?;
        let dir = path.parent().unwrap_or(Path::new("."));
        Self::parse(&text, dir).with_context(|| path.display().to_string())
    }

    /// The spec `text` describes, with relative paths resolved against
    /// `dir`.
    pub fn parse(text: &str, dir: &Path) -> Result<Self> {
        let doc: toml::Table =
            toml::from_str(text).map_err(|err| anyhow!("not valid TOML: {}", err.message()))?;
        known(
            &doc,
            "",
            &[
                "edition",
                "edition_package",
                "version",
                "disable_default_repos",
                "repository",
                "kernel",
                "package",
                "initramfs",
                "liveboot",
            ],
        )?;
        for key in ["edition", "edition_package", "version"] {
            if doc.contains_key(key) {
                bail!(
                    "{key}: editions need package dependencies, which ppk does not support \
                     yet; name every package with [[package]]"
                );
            }
        }
        if !boolean(&doc, "disable_default_repos", false)? {
            bail!(
                "the default repository, pkgs.peios.org, is not available yet; set \
                 disable_default_repos = true and give a [[repository]]"
            );
        }

        let mut repositories = Vec::new();
        for (at, repository) in tables(&doc, "repository")?.into_iter().enumerate() {
            let what = format!("repository {}", at + 1);
            known(repository, &what, &["filepath", "url"])?;
            if repository.contains_key("url") {
                bail!("{what}: url: repositories on the network are not supported yet");
            }
            let path = string(repository, "filepath", &what)?
                .ok_or_else(|| anyhow!("{what}: no filepath"))?;
            repositories.push(dir.join(path));
        }
        if repositories.is_empty() {
            bail!("no [[repository]]: there is nowhere to find packages");
        }

        let kernel = doc
            .get("kernel")
            .ok_or_else(|| anyhow!("no [kernel]"))?
            .as_table()
            .ok_or_else(|| anyhow!("kernel is not a table"))?;
        known(kernel, "kernel", &["file", "package", "version", "cmdline"])?;
        let kernel_version = string(kernel, "version", "kernel")?.map(str::to_owned);
        let image = match (
            string(kernel, "file", "kernel")?,
            string(kernel, "package", "kernel")?,
        ) {
            (Some(file), None) if kernel_version.is_none() => KernelImage::File(dir.join(file)),
            (Some(_), None) => bail!("kernel: version is for a package, not a file"),
            (None, Some(name)) => KernelImage::Package(name.to_owned()),
            (Some(_), Some(_)) => bail!("kernel: give a file or a package, not both"),
            (None, None) => bail!("kernel: no file or package"),
        };
        let kernel = Kernel {
            image,
            cmdline: string(kernel, "cmdline", "kernel")?
                .unwrap_or_default()
                .to_owned(),
        };

        let mut packages: Vec<PackageSpec> = Vec::new();
        for (at, package) in tables(&doc, "package")?.into_iter().enumerate() {
            let what = format!("package {}", at + 1);
            known(
                package,
                &what,
                &[
                    "name",
                    "version",
                    "liveboot",
                    "initramfs",
                    "autoinstall",
                    "offerinstall",
                ],
            )?;
            let name = string(package, "name", &what)?
                .ok_or_else(|| anyhow!("{what}: no name"))?
                .to_owned();
            let what = format!("package {name}");
            for key in ["autoinstall", "offerinstall"] {
                if boolean(package, key, false).context(what.clone())? {
                    bail!("{what}: {key}: there is no installer yet");
                }
            }
            if packages.iter().any(|known| known.name == name) {
                bail!("{what}: given more than once");
            }
            packages.push(PackageSpec {
                version: string(package, "version", &what)?.map(str::to_owned),
                liveboot: boolean(package, "liveboot", true).context(what.clone())?,
                initramfs: boolean(package, "initramfs", false).context(what)?,
                name,
            });
        }

        // A kernel package is carried in the store, and the [[package]]
        // naming it, if any, says where else it goes and at which version.
        if let KernelImage::Package(name) = &kernel.image {
            match packages.iter().find(|package| package.name == *name) {
                Some(package) => {
                    if let Some(version) = &kernel_version
                        && package.version.as_ref() != Some(version)
                    {
                        bail!(
                            "kernel: version is {version}, but package {name} does not give \
                             that version"
                        );
                    }
                }
                None => packages.push(PackageSpec {
                    name: name.clone(),
                    version: kernel_version,
                    liveboot: false,
                    initramfs: false,
                }),
            }
        }

        Ok(Self {
            repositories,
            kernel,
            packages,
            initramfs: layout(&doc, "initramfs")?.unwrap_or_else(Layout::initramfs),
            liveboot: layout(&doc, "liveboot")?.unwrap_or_else(Layout::liveboot),
        })
    }
}

/// The layout the table `key` of `doc` gives, written as an environment
/// profile's `symlinks` and `dirs` are, or `None` if there is no table.
fn layout(doc: &toml::Table, key: &str) -> Result<Option<Layout>> {
    let Some(table) = doc.get(key) else {
        return Ok(None);
    };
    let table = table
        .as_table()
        .ok_or_else(|| anyhow!("{key} is not a table"))?;
    known(table, key, &["symlinks", "dirs"])?;
    let profile = Profile::parse(&table.to_string()).context(key.to_owned())?;
    Ok(Some(Layout {
        symlinks: profile.symlinks,
        dirs: profile.dirs,
    }))
}

/// Refuses any key of `table` that is not in `keys`, so that a misspelt one
/// cannot silently change nothing.
fn known(table: &toml::Table, what: &str, keys: &[&str]) -> Result<()> {
    for key in table.keys() {
        if !keys.contains(&key.as_str()) {
            match what {
                "" => bail!("unknown key {key:?}"),
                _ => bail!("{what}: unknown key {key:?}"),
            }
        }
    }
    Ok(())
}

/// The array of tables `key` of `doc`, empty if there is none.
fn tables<'a>(doc: &'a toml::Table, key: &str) -> Result<Vec<&'a toml::Table>> {
    let Some(value) = doc.get(key) else {
        return Ok(Vec::new());
    };
    let not = || anyhow!("{key} is not a list of tables: write it as [[{key}]]");
    value
        .as_array()
        .ok_or_else(not)?
        .iter()
        .map(|item| item.as_table().ok_or_else(not))
        .collect()
}

fn string<'a>(table: &'a toml::Table, key: &str, what: &str) -> Result<Option<&'a str>> {
    match table.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| anyhow!("{what}: {key} is not a string")),
    }
}

fn boolean(table: &toml::Table, key: &str, default: bool) -> Result<bool> {
    match table.get(key) {
        None => Ok(default),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| anyhow!("{key} is not true or false")),
    }
}

#[cfg(test)]
mod tests;
