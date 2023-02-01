use crate::hash::FILE_EXTENSION;
use crate::hash::MurmurHash;
use super::*;

pub(crate) struct PackageParser;

impl Extractor for PackageParser {
    fn extract(
        &self,
        entry: &mut Entry<'_, '_>,
        file_path: &Path,
        mut shared: &mut [u8],
        mut shared_flex: &mut Vec<u8>,
        options: &ExtractOptions<'_>,
    ) -> io::Result<u64> {
        let variants = entry.variants();
        assert_eq!(1, variants.len());
        shared_flex.clear();

        assert_eq!(43, entry.read_u32::<LE>().unwrap());
        let num_files = entry.read_u32::<LE>().unwrap();

        write!(&mut shared_flex, "[").unwrap();
        for i in 0..num_files {
            let ext_hash = entry.read_u64::<LE>().unwrap();
            let name_hash = entry.read_u64::<LE>().unwrap();
            let ext = FILE_EXTENSION
                .binary_search_by(|(probe, _)| probe.cmp(&ext_hash))
                .map(|i| FILE_EXTENSION[i].1)
                .ok();
            let name = options.dictionary.get(&MurmurHash(name_hash));

            if i > 0 {
                write!(&mut shared_flex, ",").unwrap();
            }
            write!(&mut shared_flex, "{{\"name_hash\":\"{name_hash:016x}\",").unwrap();
            if let Some(name) = name {
                write!(&mut shared_flex, "\"name\":\"{name}\",").unwrap();
            }
            if let Some(ext) = ext {
                write!(&mut shared_flex, "\"ext\":\"{ext}\"}}").unwrap();
            } else {
                write!(&mut shared_flex, "{{\"ext_hash\":\"{ext_hash:016x}\",").unwrap();
            }
        }
        write!(&mut shared_flex, "]").unwrap();

        let parent = file_path.parent().unwrap();
        let name = file_path.file_name().unwrap().to_str().unwrap();
        let path = path_concat(parent, &mut shared, name, Some("json"));
        fs::create_dir_all(parent).unwrap();
        fs::write(path, &shared_flex).unwrap();

        Ok(shared_flex.len() as u64)
    }
}