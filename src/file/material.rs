use super::*;

pub(crate) struct MaterialParser;

impl Extractor for MaterialParser {
    fn extract(
        &self,
        entry: &mut Entry<'_, '_>,
        file_path: &Path,
        shared: &mut [u8],
        _shared_flex: &mut Vec<u8>,
        options: &ExtractOptions<'_>,
    ) -> io::Result<u64> {
        let variants = entry.variants();
        assert_eq!(1, variants.len());
        let prime = &variants[0];
        assert_eq!(prime.body_size, 30);
        assert_eq!(prime.tail_size, 0);

        let mut data_res = {
            let (data_path, scope_shared) = shared.split_at_mut(prime.body_size as usize);
            entry.read_exact(data_path).unwrap();
            file_from_data_path(scope_shared, options.target, data_path).unwrap()
        };

        let mut out_fd = options.out.create(file_path)?;
        io::copy(&mut data_res, &mut out_fd)
    }
}
