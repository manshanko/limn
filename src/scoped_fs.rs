use std::fs;
use std::io;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

pub(crate) struct ScopedFs {
    root: PathBuf,
    is_null: bool,
}

impl ScopedFs {
    fn new_(root: &Path, is_null: bool) -> Self {
        let root = if is_null {
            root.to_path_buf()
        } else {
            fs::create_dir_all(root).unwrap();
            root.canonicalize().unwrap()
        };

        Self {
            root,
            is_null,
        }
    }

    pub(crate) fn new(root: &Path) -> Self {
        Self::new_(root, false)
    }

    #[allow(dead_code)]
    pub(crate) fn new_null(root: &Path) -> Self {
        Self::new_(root, true)
    }

    fn format_path(&self, path: &Path) -> io::Result<PathBuf> {
        let out = self.root.join(path);
        for part in out.components() {
            match part {
                //Component::RootDir => return false,
                Component::ParentDir => panic!(),
                _ => (),
            }
        }
        assert!(out.starts_with(&self.root));
        if !self.is_null {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
        }
        Ok(out)
    }

    pub(crate) fn write(&self, path: &Path, data: &[u8]) -> io::Result<()> {
        if self.is_null {
            Ok(())
        } else {
            let path = self.format_path(path)?;
            fs::write(path, data)
        }
    }

    pub(crate) fn create(&self, path: &Path) -> io::Result<impl io::Write> {
        if self.is_null {
            Ok(ScopedFd(None))
        } else {
            let path = self.format_path(path)?;
            Ok(ScopedFd(Some(fs::File::create(path)?)))
        }
    }
}

pub(crate) struct ScopedFd(Option<fs::File>);

impl io::Write for ScopedFd {
    #[inline]
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if let Some(fd) = self.0.as_mut() {
            fd.write(data)
        } else {
            Ok(data.len())
        }
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        if let Some(fd) = self.0.as_mut() {
            fd.flush()
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    #[should_panic]
    fn scope() {
        let scope = ScopedFs(PathBuf::from("sandbox"));
        let _ = scope.create(Path::new("../target/test.bin"));
    }
}
