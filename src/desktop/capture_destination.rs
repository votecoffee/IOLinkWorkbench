//! Check a recording destination without creating anything.
use super::App;
use anyhow::{Context, Result, bail, ensure};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

pub(super) struct Destination {
    pub path: PathBuf,
    default_parent: Option<PathBuf>,
}

impl Destination {
    pub(super) fn check(input: &str) -> Result<Self> {
        let input = input.trim();
        ensure!(!input.is_empty(), "Enter a new capture folder");
        let path = std::path::absolute(input).context("resolve capture folder")?;
        let parent = path
            .parent()
            .context("Choose a new folder inside an existing directory")?;
        let default_parent = match fs::metadata(parent) {
            Ok(metadata) => {
                ensure!(
                    metadata.is_dir(),
                    "Capture parent is not a folder: {}",
                    parent.display()
                );
                None
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                ensure!(
                    parent == std::path::absolute(Path::new(".local"))?,
                    "Capture parent folder does not exist: {}",
                    parent.display()
                );
                Some(parent.to_path_buf())
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("inspect capture parent {}", parent.display()));
            }
        };
        match fs::symlink_metadata(&path) {
            Ok(_) => bail!(
                "Capture folder already exists. Enter a new folder: {}",
                path.display()
            ),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("inspect capture folder {}", path.display()));
            }
        }
        Ok(Self {
            path,
            default_parent,
        })
    }

    pub fn prepare_parent(&self) -> Result<()> {
        if let Some(parent) = &self.default_parent {
            fs::create_dir_all(parent)
                .with_context(|| format!("create default capture parent {}", parent.display()))?;
        }
        Ok(())
    }

    fn availability(&self) -> String {
        let path = self.path.display();
        if self.default_parent.is_some() {
            format!(
                "Folder name available at check time · {path}. Start creates the default parent."
            )
        } else {
            format!("Folder name available at check time · {path}")
        }
    }
}

pub(super) struct FolderCheck {
    input: String,
    pub message: String,
    pub available: bool,
}

impl App {
    pub(super) fn check_capture_folder(&mut self) -> Result<Destination> {
        let result = Destination::check(&self.capture_path);
        self.capture_folder_check = Some(FolderCheck {
            input: self.capture_path.clone(),
            message: match &result {
                Ok(destination) => destination.availability(),
                Err(error) => format!("{error:#}"),
            },
            available: result.is_ok(),
        });
        result
    }

    pub(super) fn current_folder_check(&self) -> Option<&FolderCheck> {
        self.capture_folder_check
            .as_ref()
            .filter(|check| check.input == self.capture_path)
    }
}
