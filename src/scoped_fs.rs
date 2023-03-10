use std::fs;
use std::io;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

pub(crate) struct ScopedFs(PathBuf);

impl ScopedFs {
    pub(crate) fn new(root: &Path) -> Self {
        fs::create_dir_all(root).unwrap();
        Self(root.canonicalize().unwrap())
    }

    fn format_path(&self, path: &Path) -> io::Result<PathBuf> {
        let out = self.0.join(path);
        for part in out.components() {
            match part {
                //Component::RootDir => return false,
                Component::ParentDir => panic!(),
                _ => (),
            }
        }
        assert!(out.starts_with(&self.0));
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(out)
    }

    pub(crate) fn write(&self, path: &Path, data: &[u8]) -> io::Result<()> {
        let path = self.format_path(path)?;
        fs::write(path, data)
    }

    pub(crate) fn create(&self, path: &Path) -> io::Result<fs::File> {
        let path = self.format_path(path)?;
        fs::File::create(path)
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