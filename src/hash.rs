use std::sync::LazyLock;
use std::hash::Hash;
use std::hash::Hasher;

pub(crate) static FILE_EXTENSION: LazyLock<[(u64, &'static str); 50]> = LazyLock::new(|| {
    let mut a = [
        "animation",
        "animation_curves",
        "bik",
        "bk2",
        "blend_set",
        "bones",
        "chroma",
        "common_package",
        "config",
        "data",
        "entity",
        "flow",
        "font",
        "ies",
        "ini",
        "ivf",
        "keys",
        "level",
        "lua",
        "material",
        "mod",
        "mouse_cursor",
        "navdata",
        "network_config",
        "oodle_net",
        "package",
        "particles",
        "physics_properties",
        "render_config",
        "rt_pipeline",
        "scene",
        "shader",
        "shader_library",
        "shader_library_group",
        "shading_environment",
        "shading_environment_mapping",
        "slug",
        "slug_album",
        "state_machine",
        "strings",
        "texture",
        "theme",
        "tome",
        "unit",
        "vector_field",
        "wwise_bank",
        "wwise_dep",
        "wwise_event",
        "wwise_metadata",
        "wwise_stream",
    ].map(|s| (murmurhash64(s.as_bytes()), s));
    a.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    a
});

#[derive(Debug, PartialEq, Eq)]
pub struct MurmurHash(pub(crate) u64);

impl MurmurHash {
    pub fn new<T: AsRef<[u8]>>(key: T) -> Self {
        Self(murmurhash64(key.as_ref()))
    }

    pub fn clone_short(&self) -> MurmurHash32 {
        MurmurHash32(((self.0 >> 32) & 0xffffffff) as u32)
    }
}

impl Hash for MurmurHash {
    fn hash<H: Hasher>(&self, h: &mut H) {
        h.write_u64(self.0)
    }
}

impl From<u64> for MurmurHash {
    fn from(key: u64) -> Self {
        Self(key)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct MurmurHash32(pub(crate) u32);

impl Hash for MurmurHash32 {
    fn hash<H: Hasher>(&self, h: &mut H) {
        h.write_u32(self.0)
    }
}

impl From<u32> for MurmurHash32 {
    fn from(key: u32) -> Self {
        Self(key)
    }
}

pub(crate) const fn murmurhash64(key: &[u8]) -> u64 {
    murmur_hash64a(key, 0)
}

#[inline]
pub const fn murmur_hash64a(mut key: &[u8], seed: u64) -> u64 {
    const MAGIC: u64 = 0xc6a4a7935bd1e995;
    const ROLL: u8 = 47;

    let mut hash = seed ^ (key.len() as u64).wrapping_mul(MAGIC);

    while key.len() > 7 {
        let chunk;
        (chunk, key) = key.split_at(8);

        // <[u8; 8]>::try_from() not usable in const
        let mut k = u64::from_le_bytes([
            chunk[0],
            chunk[1],
            chunk[2],
            chunk[3],
            chunk[4],
            chunk[5],
            chunk[6],
            chunk[7],
        ]);

        k = k.wrapping_mul(MAGIC);
        k ^= k >> ROLL;
        k = k.wrapping_mul(MAGIC);
        hash ^= k;
        hash = hash.wrapping_mul(MAGIC);
    }

    if !key.is_empty() {
        let mut xor = u64::to_ne_bytes(0);
        let rem = key.len();
        if rem >= 4 {
            xor[0] = key[0];
            xor[1] = key[1];
            xor[2] = key[2];
            xor[3] = key[3];
            if rem >= 6 {
                xor[4] = key[4];
                xor[5] = key[5];
                if rem == 7 {
                    xor[6] = key[6];
                }
            } else if rem == 5 {
                xor[4] = key[4];
            }
        } else if rem >= 2 {
            xor[0] = key[0];
            xor[1] = key[1];
            if rem == 3 {
                xor[2] = key[2];
            }
        } else if rem == 1 {
            xor[0] = key[0];
        }

        hash ^= u64::from_le_bytes(xor);
        hash = hash.wrapping_mul(MAGIC);
    }

    hash ^= hash >> ROLL;
    hash = hash.wrapping_mul(MAGIC);
    hash ^= hash >> ROLL;
    hash
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn mmh64a() {
        const CHECK: &[(&[u8], u64)] = &[
            (b"", 0),
            (b"t", 0xa9f7b29f271e2bf0),
            (b"te", 0x09a5c91602af86bf),
            (b"tes", 0xdd890a49d3dbcc17),
            (b"test", 0x2f4a8724618f4c63),
            (b"testh", 0x897d3d790c864055),
            (b"testha", 0xbc03666f652e7504),
            (b"testhas", 0xc9735c8662b71bf6),
            (b"testhash", 0x78409ab9ed54c450),
        ];

        for (key, hash) in CHECK {
            assert_eq!(*hash, murmur_hash64a(key, 0));
        }
    }
}
