//! The ISO spec: the TOML that says which packages an ISO carries, where
//! they come from, and where each goes.
//!
//! The spec's whole form is settled, but parts of it need what does not
//! exist yet: the default repository needs pkgs.peios.org, and the install
//! flags need an installer. Those are read, so that a spec written for the
//! whole form is understood, and refused with the reason.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use ppk::{Constraint, EnvDir, EnvSymlink, Profile};

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
    /// Registry policy staged on the medium for the live-root hook.
    pub seeds: Vec<Seed>,
}

/// A seed master and the policy queue that receives it.
#[derive(Debug)]
pub struct Seed {
    pub name: String,
    /// A spec-relative file, or a master from the live closure when absent.
    pub file: Option<PathBuf>,
    pub queue: &'static str,
}

/// The kernel the ISO boots.
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
    /// spec's packages, with its image at `usr/lib/modules/vmlinuz`.
    Package(String),
}

/// The prefix an edition's name, lowercased, is given to name its package,
/// as `dev.peios.peios-experimental` for `edition = "Experimental"`.
pub const EDITION_PREFIX: &str = "dev.peios.peios-";

/// A package the spec names: one it gives with `[[package]]`, its edition's,
/// or its kernel's.
#[derive(Debug)]
pub struct PackageSpec {
    pub name: String,
    /// The versions that will do, as a version constraint (PSPU 5.7), in
    /// which a bare version is that version exactly; `None` for any. The
    /// newest that will do is carried.
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
    /// The initramfs's layout unless the spec gives one: mount points for
    /// what prelude mounts. Prelude's package claims `/init` itself.
    pub fn initramfs() -> Self {
        Self::of(&[], &["proc", "sys", "tmp"])
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
                "seeds",
            ],
        )?;
        let edition = match (
            string(&doc, "edition", "")?,
            string(&doc, "edition_package", "")?,
        ) {
            (Some(edition), None) => Some(format!("{EDITION_PREFIX}{}", edition.to_lowercase())),
            (None, Some(package)) => Some(package.to_owned()),
            (Some(_), Some(_)) => bail!("give an edition or an edition_package, not both"),
            (None, None) => None,
        };
        let edition_version = string(&doc, "version", "")?.map(str::to_owned);
        if edition.is_none() && edition_version.is_some() {
            bail!("version is the edition's, but there is no edition or edition_package");
        }
        if let Some(version) = &edition_version {
            Constraint::parse(version).context("version")?;
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
        if let Some(version) = &kernel_version {
            Constraint::parse(version).context("kernel: version")?;
        }
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

        // An edition is the live system: the package naming it, and all it
        // depends on.
        let mut packages: Vec<PackageSpec> = Vec::new();
        if let Some(name) = edition {
            packages.push(PackageSpec {
                name,
                version: edition_version,
                liveboot: true,
                initramfs: false,
            });
        }
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
            let version = string(package, "version", &what)?.map(str::to_owned);
            if let Some(version) = &version {
                Constraint::parse(version).with_context(|| format!("{what}: version"))?;
            }
            packages.push(PackageSpec {
                version,
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
            initramfs: layout(&doc, "initramfs", dir)?.unwrap_or_else(Layout::initramfs),
            liveboot: layout(&doc, "liveboot", dir)?.unwrap_or_else(Layout::liveboot),
            seeds: seeds(&doc, dir)?,
        })
    }
}

/// Names without `.reg` select live package masters; `.reg` paths select files.
fn seeds(doc: &toml::Table, dir: &Path) -> Result<Vec<Seed>> {
    let Some(table) = doc.get("seeds") else {
        return Ok(Vec::new());
    };
    let table = table
        .as_table()
        .ok_or_else(|| anyhow!("seeds is not a table"))?;
    known(table, "seeds", &["apply", "live", "install"])?;
    let mut seeds: Vec<Seed> = Vec::new();
    for (key, queue) in [
        ("apply", "autoapply.d"),
        ("live", "autoapply.live.d"),
        ("install", "autoapply.install.d"),
    ] {
        let Some(values) = table.get(key) else {
            continue;
        };
        let values = values
            .as_array()
            .ok_or_else(|| anyhow!("seeds: {key} is not a list"))?;
        for value in values {
            let value = value
                .as_str()
                .ok_or_else(|| anyhow!("seeds: {key} entries must be strings"))?;
            let (name, file) = if value.ends_with(".reg") {
                let path = Path::new(value);
                if path.is_absolute() {
                    bail!("registry seed {value:?}: file paths must be relative to the spec");
                }
                let name = path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or("");
                (name, Some(dir.join(path)))
            } else {
                (value, None)
            };
            if name.is_empty()
                || name == "."
                || name == ".."
                || !name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
            {
                bail!("registry seed {value:?}: invalid seed name");
            }
            if seeds.iter().any(|seed| seed.name == name) {
                bail!("registry seed {name:?}: given more than once; a seed belongs to one queue");
            }
            seeds.push(Seed {
                name: name.to_owned(),
                file,
                queue,
            });
        }
    }
    seeds.sort_by(|a, b| (a.queue, &a.name).cmp(&(b.queue, &b.name)));
    Ok(seeds)
}

/// The layout the table `key` of `doc` gives, written as an environment
/// profile's `symlinks`, `dirs` and `include` are, with each fragment found
/// relative to `dir`, or `None` if there is no table.
fn layout(doc: &toml::Table, key: &str, dir: &Path) -> Result<Option<Layout>> {
    let Some(table) = doc.get(key) else {
        return Ok(None);
    };
    let table = table
        .as_table()
        .ok_or_else(|| anyhow!("{key} is not a table"))?;
    known(table, key, &["symlinks", "dirs", "include"])?;
    let profile = Profile::parse_in(&table.to_string(), dir).context(key.to_owned())?;
    // A spec's packages are its own, so a fragment may give only a layout.
    if !profile.packages.is_empty() || profile.include_artifacts {
        bail!("{key}: an included fragment gives packages or artifacts, not only a layout");
    }
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
