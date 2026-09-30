//! Open every log component relative to a held directory handle. A path-based
//! symlink check followed by an ordinary open would leave a replacement window.

use std::{ffi::OsStr, fs::File, io, path::Path};

#[cfg(unix)]
use rustix::fs::{Mode, OFlags, mkdirat, open, openat};

#[cfg(unix)]
const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

#[cfg(unix)]
fn child_directory(parent: &File, name: &OsStr) -> io::Result<File> {
    match mkdirat(parent, name, Mode::RWXU) {
        Ok(()) | Err(rustix::io::Errno::EXIST) => {}
        Err(error) => return Err(error.into()),
    }
    openat(parent, name, DIRECTORY_FLAGS, Mode::empty())
        .map(File::from)
        .map_err(Into::into)
}

#[cfg(unix)]
pub(super) fn directory(root: &Path, relative: &Path) -> io::Result<File> {
    use std::path::Component;

    if relative
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(io::Error::other("invalid log directory"));
    }
    let root = std::path::absolute(root)?;
    let mut ancestor = root.as_path();
    let mut names = Vec::new();
    // Resolve only ancestors above the configured root (e.g. macOS /var ->
    // /private/var). The root itself and every child must be opened NOFOLLOW.
    names.push(
        ancestor
            .file_name()
            .ok_or_else(|| io::Error::other("invalid log root"))?
            .to_os_string(),
    );
    ancestor = ancestor
        .parent()
        .ok_or_else(|| io::Error::other("invalid log root"))?;
    let canonical_parent = loop {
        match std::fs::canonicalize(ancestor) {
            Ok(path) => break path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                names.push(
                    ancestor
                        .file_name()
                        .ok_or_else(|| io::Error::other("invalid log root"))?
                        .to_os_string(),
                );
                ancestor = ancestor
                    .parent()
                    .ok_or_else(|| io::Error::other("invalid log root"))?;
            }
            Err(error) => return Err(error),
        }
    };
    let mut directory = File::from(open(&canonical_parent, DIRECTORY_FLAGS, Mode::empty())?);
    for name in names.iter().rev() {
        directory = child_directory(&directory, name)?;
    }
    for component in relative.components() {
        directory = child_directory(&directory, component.as_os_str())?;
    }
    Ok(directory)
}

#[cfg(unix)]
pub(super) fn append_file(parent: &File, name: &OsStr) -> io::Result<File> {
    use std::os::unix::fs::MetadataExt;

    let file = File::from(openat(
        parent,
        name,
        OFlags::WRONLY
            | OFlags::CREATE
            | OFlags::APPEND
            | OFlags::NOFOLLOW
            | OFlags::CLOEXEC
            | OFlags::NONBLOCK,
        Mode::RUSR | Mode::WUSR,
    )?);
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(io::Error::other("log must be a single-link regular file"));
    }
    Ok(file)
}

// The application targets macOS. Do not silently use link-following I/O on an
// unsupported platform if the core is built independently.
#[cfg(not(unix))]
pub(super) fn directory(_root: &Path, _relative: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "safe log I/O requires Unix",
    ))
}

#[cfg(not(unix))]
pub(super) fn append_file(_parent: &File, _name: &OsStr) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "safe log I/O requires Unix",
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, io::Write, os::unix::fs::symlink};

    #[test]
    fn replacing_a_parent_path_cannot_redirect_an_open_directory_handle() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("logs");
        let outside = fixture.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let held = directory(&root, Path::new("2026/09")).unwrap();
        let month = root.join("2026/09");
        fs::rename(&month, root.join("2026/original")).unwrap();
        symlink(&outside, &month).unwrap();

        append_file(&held, OsStr::new("daily.log"))
            .unwrap()
            .write_all(b"event")
            .unwrap();

        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
        assert_eq!(
            fs::read(root.join("2026/original/daily.log")).unwrap(),
            b"event"
        );
    }
}
