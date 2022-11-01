use std::sync::LazyLock;

pub(crate) static FILE_EXTENSION: LazyLock<[(u64, &'static str); 49]> = LazyLock::new(|| {
    let mut a = [
        "animation",
        "animation_curves",
        "bik",
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

pub(crate) const fn murmurhash64(key: &[u8]) -> u64 {
    murmur_hash64a(key, 0)
}

#[allow(clippy::identity_op)]
// Copyright (c) 2014-2016, Jan-Erik Rediger
// https://github.com/badboy/murmurhash64-rs/blob/3f9a5821650de6ee12f3cc45701444171ce30ebf/LICENSE
//
// https://github.com/badboy/murmurhash64-rs/blob/3f9a5821650de6ee12f3cc45701444171ce30ebf/src/lib.rs#L44
pub(crate) const fn murmur_hash64a(key: &[u8], seed: u64) -> u64 {
    let m : u64 = 0xc6a4a7935bd1e995;
    let r : u8 = 47;

    let len = key.len();
    let mut h : u64 = seed ^ ((len as u64).wrapping_mul(m));

    let endpos = len-(len&7);
    let mut i = 0;
    while i != endpos {
        let mut k : u64;

        k  = key[i+0] as u64;
        k |= (key[i+1] as u64) << 8;
        k |= (key[i+2] as u64) << 16;
        k |= (key[i+3] as u64) << 24;
        k |= (key[i+4] as u64) << 32;
        k |= (key[i+5] as u64) << 40;
        k |= (key[i+6] as u64) << 48;
        k |= (key[i+7] as u64) << 56;

        k = k.wrapping_mul(m);
        k ^= k >> r;
        k = k.wrapping_mul(m);
        h ^= k;
        h = h.wrapping_mul(m);

        i += 8;
    };

    let over = len & 7;
    if over == 7 { h ^= (key[i+6] as u64) << 48; }
    if over >= 6 { h ^= (key[i+5] as u64) << 40; }
    if over >= 5 { h ^= (key[i+4] as u64) << 32; }
    if over >= 4 { h ^= (key[i+3] as u64) << 24; }
    if over >= 3 { h ^= (key[i+2] as u64) << 16; }
    if over >= 2 { h ^= (key[i+1] as u64) << 8; }
    if over >= 1 { h ^= key[i+0] as u64; }
    if over >  0 { h = h.wrapping_mul(m); }

    h ^= h >> r;
    h = h.wrapping_mul(m);
    h ^= h >> r;
    h
}

