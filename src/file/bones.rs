use super::*;

pub(crate) struct BonesParser;

impl Extractor for BonesParser {
    fn extract(
        &self,
        entry: &mut Entry<'_, '_>,
        file_path: &Path,
        mut shared: &mut [u8],
        mut shared_flex: &mut Vec<u8>,
        _options: &ExtractOptions<'_>,
    ) -> io::Result<u64> {
        let variants = entry.variants();
        assert_eq!(1, variants.len());
        shared_flex.clear();

        let num_bones = entry.read_u32::<LE>().unwrap();
        let num_lods = entry.read_u32::<LE>().unwrap();
        for _ in 0..num_bones {
            let _short_hash = entry.read_u32::<LE>().unwrap();
        }

        write!(&mut shared_flex, "{{\"lod\":[").unwrap();
        for i in 0..num_lods {
            let lod = entry.read_u32::<LE>().unwrap();
            if i > 0 {
                write!(&mut shared_flex, ",").unwrap();
            }
            write!(&mut shared_flex, "{lod}").unwrap();
        }
        write!(&mut shared_flex, "],\"bones\":[").unwrap();
        let mut len = 0;
        for i in 0..num_bones {
            loop {
                let b = entry.read_u8().unwrap();
                if b == 0 {
                    break;
                } else {
                    shared[len] = b;
                    len += 1;
                }
            }

            if i > 0 {
                write!(&mut shared_flex, ",").unwrap();
            }
            write!(&mut shared_flex, "\"{}\"", std::str::from_utf8(&shared[..len]).unwrap()).unwrap();
            len = 0;
        }
        assert!(entry.read_u8().is_err());
        write!(&mut shared_flex, "]}}").unwrap();

        let parent = file_path.parent().unwrap();
        let name = file_path.file_name().unwrap().to_str().unwrap();
        let path = path_concat(parent, &mut shared, name, Some("json"));
        fs::create_dir_all(parent).unwrap();
        fs::write(path, &shared_flex).unwrap();

        Ok(shared_flex.len() as u64)
    }
}