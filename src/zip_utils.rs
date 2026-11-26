use crate::{constants::*, Result};
use cap_std::{ambient_authority, fs::Dir};
use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
use walkdir::WalkDir;
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

fn portable_name(name: &str) -> Result<()> {
    let parts: Vec<_> = name.strip_suffix('/').unwrap_or(name).split('/').collect();
    if name.len() > 4096 || parts.len() > 64 {
        return Err("Archive path too long".into());
    }
    for part in parts {
        let stem = part
            .split('.')
            .next()
            .unwrap_or("")
            .trim_end()
            .to_ascii_uppercase();
        if part.is_empty()
            || part.ends_with(['.', ' '])
            || part.contains(['\\', ':', '<', '>', '"', '|', '?', '*'])
            || part.chars().any(char::is_control)
            || matches!(
                stem.as_str(),
                "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
            )
            || ["COM", "LPT"].iter().any(|prefix| {
                stem.strip_prefix(prefix).is_some_and(|s| {
                    s.len() == 1 && s.as_bytes()[0].is_ascii_digit() || matches!(s, "¹" | "²" | "³")
                })
            })
        {
            return Err("Unsafe archive path".into());
        }
    }
    Ok(())
}

pub fn create_zip(input: &Path) -> Result<File> {
    let meta = input.symlink_metadata()?;
    if !(meta.is_dir() || meta.is_file()) {
        return Err("Expected a regular file or directory".into());
    }
    let absolute = input.canonicalize()?;
    let parent = absolute
        .parent()
        .ok_or("Select a file or folder below the filesystem root")?;
    let root = Dir::open_ambient_dir(
        if meta.is_dir() { &absolute } else { parent },
        ambient_authority(),
    )?;
    let mut writer = ZipWriter::new(tempfile::tempfile()?);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o600);
    for (i, item) in WalkDir::new(&absolute)
        .follow_root_links(false)
        .into_iter()
        .enumerate()
    {
        if i >= MAX_ENTRIES {
            return Err("Too many archive entries".into());
        }
        let entry = item?;
        let relative = entry.path().strip_prefix(parent)?;
        let name = relative
            .to_str()
            .ok_or("Expected UTF-8 filename")?
            .replace(std::path::MAIN_SEPARATOR, "/");
        portable_name(&name)?;
        if entry.file_type().is_dir() {
            writer.add_directory(name, options)?;
            continue;
        }
        if !entry.file_type().is_file() {
            return Err("Backup contains a link or special file".into());
        }
        let scoped = if meta.is_dir() {
            entry.path().strip_prefix(&absolute)?
        } else {
            relative
        };
        let file = root.open(scoped)?;
        if !file.metadata()?.is_file() {
            return Err("Expected regular file".into());
        }
        writer.start_file(name, options.large_file(true))?;
        io::copy(&mut &file, &mut writer)?;
    }
    let mut file = writer.finish()?;
    file.seek(SeekFrom::Start(0))?;
    Ok(file)
}

pub fn restore(
    mut data: impl Read + Seek,
    output: &Path,
    password: Option<&str>,
    max_bytes: u64,
) -> Result<()> {
    // Read the count before ZipArchive builds its in-memory directory index.
    if zip::unstable::read::ZipIterable::try_new(&mut data, Default::default())?.len() > MAX_ENTRIES
    {
        return Err("Too many archive entries".into());
    }
    data.rewind()?;
    let archive = ZipArchive::new(data)?;
    if archive.len() > MAX_ENTRIES {
        return Err("Too many archive entries".into());
    }
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(output)?;
    // Own only the newly created directory. Roll it back on any extraction error.
    if let Err(error) = extract_entries(archive, output, password, max_bytes) {
        fs::remove_dir_all(output)?;
        return Err(error);
    }
    Ok(())
}

fn extract_entries(
    mut archive: ZipArchive<impl Read + Seek>,
    output: &Path,
    password: Option<&str>,
    mut remaining: u64,
) -> Result<()> {
    for i in 0..archive.len() {
        let mut entry = match password {
            Some(s) => archive.by_index_decrypt(i, s.as_bytes())?,
            None => archive.by_index(i)?,
        };
        portable_name(&entry.name()?)?;
        let path: PathBuf = output.join(entry.enclosed_name().ok_or("Unsafe ZIP path")?);
        if entry.is_symlink() {
            return Err("Links are unsupported".into());
        }
        if entry.is_dir() {
            fs::create_dir_all(path)?;
            continue;
        }
        fs::create_dir_all(path.parent().ok_or("Invalid ZIP entry")?)?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        if entry.size() > remaining {
            return Err("Expanded backup exceeds limit".into());
        }
        let mut target = options.open(path)?;
        let copied = io::copy(
            &mut entry.by_ref().take(remaining.saturating_add(1)),
            &mut target,
        )?;
        if copied != entry.size() {
            return Err("ZIP size mismatch".into());
        }
        remaining = remaining
            .checked_sub(copied)
            .ok_or("Expanded backup exceeds limit")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restores_in_a_new_directory_only() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let source = temp.path().join("source");
        fs::create_dir(&source)?;
        fs::write(source.join("notes"), b"test")?;
        let archive = create_zip(&source)?;
        let limited = temp.path().join("limited");
        assert!(restore(archive.try_clone()?, &limited, None, 3).is_err());
        assert!(!limited.exists());
        let out = temp.path().join("restore");
        restore(archive.try_clone()?, &out, None, u64::MAX)?;
        assert_eq!(fs::read(out.join("source/notes"))?, b"test");
        assert!(restore(archive, &out, None, u64::MAX).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn refuses_links_in_backup() -> Result<()> {
        let temp = tempfile::tempdir()?;
        fs::write(temp.path().join("real"), b"test")?;
        std::os::unix::fs::symlink("real", temp.path().join("alias"))?;
        assert!(create_zip(temp.path()).is_err());
        Ok(())
    }

    #[test]
    fn validates_portable_archive_names() {
        for name in ["../notes", "CON.txt", "/notes", "dir/file:stream", "file."] {
            assert!(portable_name(name).is_err());
        }
        assert!(portable_name("folder/notes.txt").is_ok());
    }
}
