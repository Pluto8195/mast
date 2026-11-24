//! Per-preset template: built once, cloned (`cp -c -R`, copy-on-write) for each
//! run. Reset = drop the app, restore the template, reopen.
//!
//! The template lives inside the scratch root and the live `data/` and `repos/`
//! stay at their own fixed paths, so the absolute paths stored in the engine
//! and in git worktree pointers remain valid after every restore.

use std::io;
use std::path::{Path, PathBuf};

use crate::environment::ScratchDirectories;
use crate::materialize;
use crate::world::SmallWorld;

pub struct Template {
    preset: &'static str,
    seed: u64,
    hash: String,
    dir: PathBuf,
}

fn no_links(source: &Path) -> io::Result<()> {
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            no_links(&entry.path())?;
        } else if !kind.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "template contains a link or special file",
            ));
        }
    }
    Ok(())
}

fn clone_dir(from: &Path, to: &Path) -> io::Result<()> {
    no_links(from)?;
    let status = std::process::Command::new("/bin/cp")
        .args(["-c", "-R"])
        .arg(from.join("."))
        .arg(to)
        .status()?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| io::Error::other("template clone failed"))
}

/// Empty `dir` (which must be a real directory inside `root`) without
/// following links.
fn empty_inside(root: &Path, dir: &Path) -> io::Result<()> {
    let canonical = dir.canonicalize()?;
    if canonical == root || !canonical.starts_with(root) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "refusing to empty a directory outside the scratch root",
        ));
    }
    for entry in std::fs::read_dir(&canonical)? {
        let path = entry?.path();
        if std::fs::symlink_metadata(&path)?.is_dir() {
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

impl Template {
    /// Materialize `world` into the scratch root, hash it from a reopened
    /// store, and keep a copy-on-write snapshot as the template.
    pub async fn build(scratch: &ScratchDirectories, world: &SmallWorld) -> Result<Self, String> {
        let io = |e: io::Error| e.to_string();
        materialize::materialize(scratch, world).await?;
        let core = materialize::open_core(scratch.data())?;
        let hash = materialize::snapshot_hash(scratch, world, &core);
        core.shutdown().await;
        drop(core);
        let hash = hash?;
        let dir = scratch.root().join("template");
        std::fs::create_dir(&dir).map_err(io)?;
        for name in ["data", "repos"] {
            std::fs::create_dir(dir.join(name)).map_err(io)?;
        }
        clone_dir(scratch.data(), &dir.join("data")).map_err(io)?;
        clone_dir(scratch.repositories(), &dir.join("repos")).map_err(io)?;
        Ok(Self {
            preset: world.preset,
            seed: world.seed,
            hash,
            dir,
        })
    }

    pub fn hash(&self) -> &str {
        &self.hash
    }

    /// Header that starts every tree dump.
    pub fn header(&self) -> String {
        format!(
            "preset={} seed={} template={}",
            self.preset, self.seed, self.hash
        )
    }

    /// Replace the run's `data/` and `repos/` with fresh clones of the
    /// template. Nothing may have the engine open.
    pub fn restore(&self, scratch: &ScratchDirectories) -> io::Result<()> {
        let root = scratch.root().canonicalize()?;
        for (name, live) in [("data", scratch.data()), ("repos", scratch.repositories())] {
            empty_inside(&root, live)?;
            clone_dir(&self.dir.join(name), live)?;
        }
        Ok(())
    }
}
