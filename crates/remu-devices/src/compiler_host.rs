use super::*;
use std::fs::{self, File, OpenOptions};
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

const COMMAND: u64 = 0x00;
const ARG0: u64 = 0x04;
const ARG1: u64 = 0x08;
const RESULT: u64 = 0x0c;
const DATA: u64 = 0x10;
const ARG2: u64 = 0x14;

const PATH_RESET: u32 = 1;
const OPEN: u32 = 2;
const CLOSE: u32 = 3;
const READ: u32 = 4;
const WRITE_BEGIN: u32 = 5;
const CHMOD: u32 = 6;
const READ_AT: u32 = 7;
const WRITE_AT_BEGIN: u32 = 8;

enum HostFile {
    File(File),
    Directory(Cursor<Vec<u8>>),
}

#[derive(Clone, Copy)]
enum InputMode {
    Idle,
    Path,
    Write {
        fd: u32,
        remaining: usize,
        written: usize,
        restore: Option<u64>,
    },
}

/// Explicit, root-confined file bridge used by compiler self-host tests.
///
/// The byte-oriented register contract avoids guest-memory DMA and only
/// exposes Renvo's existing open/close/read/write/chmod operations.
pub struct CompilerHost {
    name: String,
    root: PathBuf,
    arg0: u32,
    arg1: u32,
    arg2: u32,
    result: i32,
    path: Vec<u8>,
    read_data: VecDeque<u8>,
    mode: InputMode,
    files: BTreeMap<u32, HostFile>,
    next_fd: u32,
    console: UartHandle,
}

impl CompilerHost {
    /// Creates a disabled-by-default bridge rooted at an explicit directory.
    pub fn new(
        name: impl Into<String>,
        root: impl AsRef<Path>,
        console: UartHandle,
    ) -> Result<Self, DeviceError> {
        let root = fs::canonicalize(root.as_ref()).map_err(|error| {
            DeviceError::new(format!("compiler host root could not be resolved: {error}"))
        })?;
        Ok(Self {
            name: name.into(),
            root,
            arg0: 0,
            arg1: 0,
            arg2: 0,
            result: 0,
            path: Vec::new(),
            read_data: VecDeque::new(),
            mode: InputMode::Idle,
            files: BTreeMap::new(),
            next_fd: 3,
            console,
        })
    }

    fn confined_path(&self) -> Result<PathBuf, DeviceError> {
        let text = std::str::from_utf8(&self.path)
            .map_err(|_| DeviceError::new("compiler host path is not UTF-8"))?;
        let mut relative = PathBuf::new();
        for component in Path::new(text.trim_start_matches('/')).components() {
            match component {
                Component::CurDir => {}
                Component::Normal(value) => relative.push(value),
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(DeviceError::new("compiler host path escapes its root"));
                }
            }
        }
        let joined = self.root.join(relative);
        let confined = if joined.exists() {
            fs::canonicalize(&joined).map_err(|error| {
                DeviceError::new(format!(
                    "compiler host path {} could not be resolved: {error}",
                    joined.display()
                ))
            })?
        } else {
            let parent = joined
                .parent()
                .ok_or_else(|| DeviceError::new("compiler host path has no parent"))?;
            let parent = fs::canonicalize(parent).map_err(|error| {
                DeviceError::new(format!(
                    "compiler host parent {} could not be resolved: {error}",
                    parent.display()
                ))
            })?;
            parent.join(
                joined
                    .file_name()
                    .ok_or_else(|| DeviceError::new("compiler host path has no filename"))?,
            )
        };
        if !confined.starts_with(&self.root) {
            return Err(DeviceError::new("compiler host path escapes its root"));
        }
        Ok(confined)
    }

    fn open(&mut self) -> Result<i32, DeviceError> {
        let path = self.confined_path()?;
        let flags = self.arg0;
        let file = if path.is_dir() {
            HostFile::Directory(Cursor::new(encode_directory(&path)?))
        } else {
            let writable = flags & 3 != 0 || flags & 64 != 0 || flags & 512 != 0;
            let file = OpenOptions::new()
                .read(!writable || flags & 2 != 0)
                .write(writable)
                .create(flags & 64 != 0)
                .truncate(flags & 512 != 0)
                .open(&path)
                .map_err(|error| {
                    DeviceError::new(format!("compiler host open {}: {error}", path.display()))
                })?;
            HostFile::File(file)
        };
        let fd = self.next_fd;
        self.next_fd = self.next_fd.saturating_add(1);
        self.files.insert(fd, file);
        Ok(fd as i32)
    }

    fn read(&mut self) -> Result<i32, DeviceError> {
        let fd = self.arg0;
        let count = usize::try_from(self.arg1).unwrap_or(usize::MAX);
        let file = self
            .files
            .get_mut(&fd)
            .ok_or_else(|| DeviceError::new(format!("compiler host read invalid fd {fd}")))?;
        self.read_data.clear();
        let mut data = vec![0; count];
        let used = match file {
            HostFile::File(file) => file.read(&mut data),
            HostFile::Directory(directory) => directory.read(&mut data),
        }
        .map_err(|error| DeviceError::new(format!("compiler host read fd {fd}: {error}")))?;
        self.read_data.extend(data[..used].iter().copied());
        Ok(i32::try_from(used).unwrap_or(i32::MAX))
    }

    fn read_at(&mut self) -> Result<i32, DeviceError> {
        let fd = self.arg0;
        let offset = u64::from(self.arg2);
        let count = usize::try_from(self.arg1).unwrap_or(usize::MAX);
        let file = self
            .files
            .get_mut(&fd)
            .ok_or_else(|| DeviceError::new(format!("compiler host seek invalid fd {fd}")))?;
        self.read_data.clear();
        let mut data = vec![0; count];
        let used = match file {
            HostFile::File(file) => {
                let current = file.stream_position();
                let result = current.and_then(|current| {
                    file.seek(SeekFrom::Start(offset))?;
                    let used = file.read(&mut data)?;
                    file.seek(SeekFrom::Start(current))?;
                    Ok(used)
                });
                result
            }
            HostFile::Directory(directory) => {
                let current = directory.position();
                directory.set_position(offset);
                let result = directory.read(&mut data);
                directory.set_position(current);
                result
            }
        }
        .map_err(|error| DeviceError::new(format!("compiler host read-at fd {fd}: {error}")))?;
        self.read_data.extend(data[..used].iter().copied());
        Ok(i32::try_from(used).unwrap_or(i32::MAX))
    }

    fn chmod(&mut self) -> Result<i32, DeviceError> {
        let fd = self.arg0;
        let HostFile::File(file) = self
            .files
            .get_mut(&fd)
            .ok_or_else(|| DeviceError::new(format!("compiler host chmod invalid fd {fd}")))?
        else {
            return Err(DeviceError::new("compiler host cannot chmod a directory"));
        };
        let mut permissions = file
            .metadata()
            .map_err(|error| DeviceError::new(format!("compiler host stat fd {fd}: {error}")))?
            .permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(self.arg1);
        }
        #[cfg(not(unix))]
        permissions.set_readonly(self.arg1 & 0o222 == 0);
        file.set_permissions(permissions)
            .map_err(|error| DeviceError::new(format!("compiler host chmod fd {fd}: {error}")))?;
        Ok(0)
    }

    fn write_byte(&mut self, byte: u8) -> Result<(), DeviceError> {
        let InputMode::Write {
            fd,
            remaining,
            written,
            restore,
        } = self.mode
        else {
            return Err(DeviceError::new(
                "compiler host data write has no active operation",
            ));
        };
        if remaining == 0 {
            return Err(DeviceError::new(
                "compiler host write exceeds declared length",
            ));
        }
        if fd == 1 || fd == 2 {
            self.console.transmit(&[byte]);
        } else {
            let file = self
                .files
                .get_mut(&fd)
                .ok_or_else(|| DeviceError::new(format!("compiler host write invalid fd {fd}")))?;
            match file {
                HostFile::File(file) => file.write_all(&[byte]).map_err(|error| {
                    DeviceError::new(format!("compiler host write fd {fd}: {error}"))
                })?,
                HostFile::Directory(_) => {
                    return Err(DeviceError::new("compiler host cannot write a directory"));
                }
            }
        }
        let remaining = remaining - 1;
        let written = written + 1;
        self.result = i32::try_from(written).unwrap_or(i32::MAX);
        if remaining == 0 {
            if let Some(position) = restore {
                let file = self.files.get_mut(&fd).ok_or_else(|| {
                    DeviceError::new(format!("compiler host write-at invalid fd {fd}"))
                })?;
                match file {
                    HostFile::File(file) => file.seek(SeekFrom::Start(position)),
                    HostFile::Directory(directory) => directory.seek(SeekFrom::Start(position)),
                }
                .map_err(|error| {
                    DeviceError::new(format!("compiler host restore fd {fd}: {error}"))
                })?;
            }
            self.mode = InputMode::Idle;
        } else {
            self.mode = InputMode::Write {
                fd,
                remaining,
                written,
                restore,
            };
        }
        Ok(())
    }

    fn command(&mut self, command: u32) -> Result<(), DeviceError> {
        self.result = match command {
            PATH_RESET => {
                self.path.clear();
                self.mode = InputMode::Path;
                0
            }
            OPEN => {
                self.mode = InputMode::Idle;
                self.open().unwrap_or(-1)
            }
            CLOSE => {
                if self.files.remove(&self.arg0).is_some() {
                    0
                } else {
                    -1
                }
            }
            READ => self.read().unwrap_or(-1),
            READ_AT => self.read_at().unwrap_or(-1),
            WRITE_BEGIN => {
                self.mode = InputMode::Write {
                    fd: self.arg0,
                    remaining: usize::try_from(self.arg1).unwrap_or(usize::MAX),
                    written: 0,
                    restore: None,
                };
                0
            }
            WRITE_AT_BEGIN => {
                let fd = self.arg0;
                let offset = u64::from(self.arg2);
                let file = self.files.get_mut(&fd).ok_or_else(|| {
                    DeviceError::new(format!("compiler host write-at invalid fd {fd}"))
                })?;
                let restore = match file {
                    HostFile::File(file) => {
                        let current = file.stream_position().map_err(|error| {
                            DeviceError::new(format!("compiler host tell fd {fd}: {error}"))
                        })?;
                        file.seek(SeekFrom::Start(offset)).map_err(|error| {
                            DeviceError::new(format!("compiler host seek fd {fd}: {error}"))
                        })?;
                        current
                    }
                    HostFile::Directory(directory) => {
                        let current = directory.position();
                        directory.set_position(offset);
                        current
                    }
                };
                self.mode = InputMode::Write {
                    fd,
                    remaining: usize::try_from(self.arg1).unwrap_or(usize::MAX),
                    written: 0,
                    restore: Some(restore),
                };
                0
            }
            CHMOD => self.chmod().unwrap_or(-1),
            _ => {
                return Err(DeviceError::new(format!(
                    "unknown compiler host command {command}"
                )));
            }
        };
        Ok(())
    }
}

impl Device for CompilerHost {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, _width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        match offset {
            ARG0 => Ok(u64::from(self.arg0)),
            ARG1 => Ok(u64::from(self.arg1)),
            ARG2 => Ok(u64::from(self.arg2)),
            RESULT => Ok(u64::from(self.result as u32)),
            DATA => Ok(u64::from(self.read_data.pop_front().unwrap_or(0))),
            _ => Err(DeviceError::new(format!(
                "compiler host read at {offset:#x}"
            ))),
        }
    }

    fn write(
        &mut self,
        offset: u64,
        _width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        match offset {
            COMMAND => self.command(value as u32),
            ARG0 => {
                self.arg0 = value as u32;
                Ok(())
            }
            ARG1 => {
                self.arg1 = value as u32;
                Ok(())
            }
            ARG2 => {
                self.arg2 = value as u32;
                Ok(())
            }
            DATA => match self.mode {
                InputMode::Path => {
                    self.path.push(value as u8);
                    Ok(())
                }
                InputMode::Write { .. } => self.write_byte(value as u8),
                InputMode::Idle => Err(DeviceError::new("compiler host data write while idle")),
            },
            _ => Err(DeviceError::new(format!(
                "compiler host write at {offset:#x}"
            ))),
        }
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.arg0 = 0;
        self.arg1 = 0;
        self.arg2 = 0;
        self.result = 0;
        self.path.clear();
        self.read_data.clear();
        self.mode = InputMode::Idle;
        self.files.clear();
        self.next_fd = 3;
    }
}

fn encode_directory(path: &Path) -> Result<Vec<u8>, DeviceError> {
    let mut entries = fs::read_dir(path)
        .map_err(|error| {
            DeviceError::new(format!(
                "compiler host read directory {}: {error}",
                path.display()
            ))
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            DeviceError::new(format!(
                "compiler host read directory {}: {error}",
                path.display()
            ))
        })?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut out = Vec::new();
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_string_lossy().as_bytes().to_vec();
        let record_len = 19 + name.len() + 1;
        let aligned = (record_len + 7) & !7;
        let start = out.len();
        out.resize(start + aligned, 0);
        out[start + 16..start + 18].copy_from_slice(&(aligned as u16).to_le_bytes());
        out[start + 18] = if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            4
        } else {
            8
        };
        out[start + 19..start + 19 + name.len()].copy_from_slice(&name);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_test_file(host: &mut CompilerHost, path: &[u8]) -> u32 {
        host.command(PATH_RESET).unwrap();
        host.path.extend_from_slice(path);
        let fd = host.command(OPEN).map(|_| host.result).unwrap();
        assert!(fd >= 3);
        fd as u32
    }

    #[test]
    fn positional_read_preserves_stream_position() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("unit"), b"RNVO-body").unwrap();
        let mut host = CompilerHost::new("test", root.path(), UartHandle::default()).unwrap();
        let fd = open_test_file(&mut host, b"unit");

        host.arg0 = fd;
        host.arg1 = 4;
        host.arg2 = 0;
        host.command(READ_AT).unwrap();
        assert_eq!(host.read_data.drain(..).collect::<Vec<_>>(), b"RNVO");

        host.arg1 = 9;
        host.command(READ).unwrap();
        assert_eq!(host.read_data.drain(..).collect::<Vec<_>>(), b"RNVO-body");
    }

    #[test]
    fn missing_files_are_reported_to_the_guest() {
        let root = tempfile::tempdir().unwrap();
        let mut host = CompilerHost::new("test", root.path(), UartHandle::default()).unwrap();
        host.command(PATH_RESET).unwrap();
        host.path.extend_from_slice(b"missing");
        host.command(OPEN).unwrap();
        assert_eq!(host.result, -1);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_cannot_escape_the_explicit_root() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), b"nope").unwrap();
        symlink(outside.path(), root.path().join("escape")).unwrap();
        let mut host = CompilerHost::new("test", root.path(), UartHandle::default()).unwrap();
        host.command(PATH_RESET).unwrap();
        host.path.extend_from_slice(b"escape/secret");
        host.command(OPEN).unwrap();
        assert_eq!(host.result, -1);
    }
}
