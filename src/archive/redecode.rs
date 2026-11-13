use super::{Summary, parent};
use crate::{
    capture::{CSV_METADATA, csv_header},
    decode::Field,
    profile::Profile,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FileDigest {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

pub(super) const IDENTITY_VERIFICATION: &str = "not performed; new interpretation of saved payloads, not a new acquisition or device verification";

pub(super) fn digest(name: &str, bytes: &[u8]) -> FileDigest {
    FileDigest {
        name: name.into(),
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(bytes)),
    }
}

pub(super) struct DigestReader {
    file: File,
    hash: Option<Sha256>,
    bytes: u64,
}

impl DigestReader {
    pub fn new(file: File, hash: bool) -> Self {
        Self {
            file,
            hash: hash.then(Sha256::new),
            bytes: 0,
        }
    }

    pub fn receipt(&self, name: &str) -> Result<FileDigest> {
        Ok(FileDigest {
            name: name.into(),
            bytes: self.bytes,
            sha256: format!(
                "{:x}",
                self.hash
                    .as_ref()
                    .context("source hashing was not enabled")?
                    .clone()
                    .finalize()
            ),
        })
    }
}

impl Read for DigestReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.file.read(buffer)?;
        if let Some(hash) = &mut self.hash {
            hash.update(&buffer[..count]);
        }
        self.bytes += count as u64;
        Ok(count)
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Redecode {
    pub format_version: u32,
    pub kind: String,
    pub workbench_version: String,
    pub created_utc: String,
    pub output_directory: PathBuf,
    pub source_directory: PathBuf,
    pub identity_verification: String,
    pub source_capture: Summary,
    pub mapping_profile: String,
    pub input_bytes: usize,
    pub exported_fields: Vec<Field>,
    pub rows_written: u64,
    pub typed_rows: u64,
    pub blank_rows: u64,
    pub source_files: Vec<FileDigest>,
    pub exported_files: Vec<FileDigest>,
}

pub(super) struct Exporter {
    // Close the writer before TempDir cleanup on any early failure.
    writer: Option<csv::Writer<File>>,
    staging: tempfile::TempDir,
    destination: PathBuf,
    source_directory: PathBuf,
    mapping: Profile,
    fields: Vec<Field>,
    source_files: Vec<FileDigest>,
    exported_files: Vec<FileDigest>,
    rows: u64,
    typed_rows: u64,
}

fn save(directory: &Path, name: &str, bytes: &[u8]) -> Result<FileDigest> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(name))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(digest(name, bytes))
}

impl Exporter {
    pub fn new(directory: &Path, mapping: &Profile, output: &Path) -> Result<Self> {
        mapping.validate()?;
        ensure!(
            !mapping.log_fields().is_empty(),
            "re-decode mapping has no selected typed fields"
        );
        let source_directory = directory.canonicalize()?;
        let parent = parent(output).canonicalize()?;
        ensure!(
            !parent.starts_with(&source_directory),
            "export must be outside the source capture directory"
        );
        let destination = parent.join(
            output
                .file_name()
                .context("export needs a new directory name")?,
        );
        ensure!(
            fs::symlink_metadata(&destination).is_err(),
            "export destination already exists: {}",
            destination.display()
        );
        let staging = tempfile::Builder::new()
            .prefix(".iolink-redecode-")
            .tempdir_in(parent)?;
        let exported_files = vec![save(
            staging.path(),
            "mapping-profile.toml",
            toml::to_string_pretty(mapping)?.as_bytes(),
        )?];
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(staging.path().join("data.csv"))?;
        let mut writer = csv::WriterBuilder::new()
            .terminator(csv::Terminator::Any(b'\n'))
            .from_writer(file);
        writer.write_record(csv_header(mapping))?;
        Ok(Self {
            staging,
            destination,
            source_directory,
            mapping: mapping.clone(),
            fields: mapping.log_fields().into_iter().cloned().collect(),
            writer: Some(writer),
            source_files: Vec::new(),
            exported_files,
            rows: 0,
            typed_rows: 0,
        })
    }

    pub fn source(
        &mut self,
        profile: &Profile,
        profile_bytes: &[u8],
        manifest_bytes: &[u8],
    ) -> Result<()> {
        ensure!(
            self.mapping.input_bytes == profile.input_bytes,
            "mapping input width differs from the saved capture"
        );
        self.source_files
            .push(digest("profile.toml", profile_bytes));
        self.source_files
            .push(digest("capture.toml", manifest_bytes));
        self.exported_files.push(save(
            self.staging.path(),
            "source-profile.toml",
            profile_bytes,
        )?);
        self.exported_files.push(save(
            self.staging.path(),
            "source-capture.toml",
            manifest_bytes,
        )?);
        Ok(())
    }

    pub fn row(&mut self, original: &csv::StringRecord, payload: &[u8], typed: bool) -> Result<()> {
        let mut row = original
            .iter()
            .take(CSV_METADATA.len())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        for field in &self.fields {
            row.push(if typed {
                field.decode(payload)?.value
            } else {
                String::new()
            });
        }
        self.writer
            .as_mut()
            .expect("open export writer")
            .write_record(row)?;
        self.rows += 1;
        self.typed_rows += u64::from(typed);
        Ok(())
    }

    pub fn source_part(&mut self, file: FileDigest) {
        self.source_files.push(file);
    }

    pub fn finish(mut self, summary: Summary) -> Result<Redecode> {
        ensure!(
            self.rows == summary.rows_checked,
            "export row accounting differs from verification"
        );
        let mut writer = self.writer.take().context("export writer closed")?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        drop(writer);
        let mut file = DigestReader::new(File::open(self.staging.path().join("data.csv"))?, true);
        io::copy(&mut file, &mut io::sink())?;
        self.exported_files.push(file.receipt("data.csv")?);
        drop(file);
        let receipt = Redecode {
            format_version: 1,
            kind: "offline_redecode".into(),
            workbench_version: env!("CARGO_PKG_VERSION").into(),
            created_utc: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            output_directory: self.destination.clone(),
            source_directory: self.source_directory.clone(),
            identity_verification: IDENTITY_VERIFICATION.into(),
            source_capture: summary,
            mapping_profile: self.mapping.name.clone(),
            input_bytes: self.mapping.input_bytes,
            exported_fields: self.fields,
            rows_written: self.rows,
            typed_rows: self.typed_rows,
            blank_rows: self.rows - self.typed_rows,
            source_files: self.source_files,
            exported_files: self.exported_files,
        };
        save(
            self.staging.path(),
            "receipt.toml",
            toml::to_string_pretty(&receipt)?.as_bytes(),
        )?;
        publish(self.staging.path(), &self.destination)?;
        Ok(receipt)
    }
}

impl super::Observer for Exporter {
    fn source(
        &mut self,
        profile: &Profile,
        profile_bytes: &[u8],
        manifest_bytes: &[u8],
    ) -> Result<()> {
        Exporter::source(self, profile, profile_bytes, manifest_bytes)
    }
    fn row(&mut self, original: &csv::StringRecord, payload: &[u8], typed: bool) -> Result<()> {
        Exporter::row(self, original, payload, typed)
    }
    fn source_part(&mut self, file: FileDigest) -> Result<()> {
        Exporter::source_part(self, file);
        Ok(())
    }
}

#[cfg(windows)]
fn publish(staging: &Path, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
    let wide = |path: &Path| -> Result<Vec<u16>> {
        let mut value: Vec<_> = path.as_os_str().encode_wide().collect();
        ensure!(!value.contains(&0), "export path contains NUL");
        value.push(0);
        Ok(value)
    };
    let source = wide(staging)?;
    let target = wide(destination)?;
    // Same-parent rename. Zero flags forbid replacement and cross-volume copying.
    if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 0) } == 0 {
        return Err(io::Error::last_os_error())
            .context("publishing new re-decode export directory");
    }
    Ok(())
}

#[cfg(not(windows))]
fn publish(_staging: &Path, _destination: &Path) -> Result<()> {
    anyhow::bail!("re-decode directory publication requires Windows")
}

#[cfg(test)]
pub(super) mod tests;
