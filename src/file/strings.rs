use std::fmt;
use super::*;

#[repr(u32)]
enum Language {
    English,
    Spanish,
    French,
    Polish,
    German,
    Japanese,
    English2,
    Italian,
    Korean,
    ChineseTraditional,
    Russian,
    Portuguese,
    ChineseSimplified,
}

impl Language {
    fn from_code(code: u32) -> Option<Self> {
        debug_assert!(code == 0 || code.is_power_of_two());
        Some(match code {
            0    => Self::English,
            1    => Self::Spanish,
            2    => Self::French,
            4    => Self::Polish,
            8    => Self::German,
            16   => Self::Japanese,
            32   => Self::English2,
            64   => Self::Italian,
            128  => Self::Korean,
            256  => Self::ChineseTraditional,
            512  => Self::Russian,
            1024 => Self::Portuguese,
            2048 => Self::ChineseSimplified,
            _ => return None,
        })
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match *self {
            Self::English            => "english",
            Self::English2           => "english2",
            Self::ChineseSimplified  => "chinese_simplified",
            Self::Italian            => "italian",
            Self::ChineseTraditional => "chinese_traditional",
            Self::Portuguese         => "portuguese",
            Self::Polish             => "polish",
            Self::Russian            => "russian",
            Self::Korean             => "korean",
            Self::Spanish            => "spanish",
            Self::German             => "german",
            Self::Japanese           => "japanese",
            Self::French             => "french",
        })
    }
}

pub(crate) struct StringsParser;

impl Extractor for StringsParser {
    fn extract(
        &self,
        entry: &mut Entry<'_, '_>,
        file_path: &Path,
        shared: &mut [u8],
        shared_flex: &mut Vec<u8>,
        options: &ExtractOptions<'_>,
    ) -> io::Result<u64> {
        let mut wrote = 0;
        let mut variant_i = 0;
        while let Some(variant) = entry.variants().get(variant_i) {
            let mut shared = &mut shared[..];
            variant_i += 1;
            let kind = variant.kind;
            let variant_size = variant.body_size;

            let _unk = entry.read_u32::<LE>()?;
            //assert_eq!(_unk, 0x3e85f3ae);
            let num_items = entry.read_u32::<LE>()?;
            let mut offset = 8;
            let size_needed = num_items as usize * 8;
            assert!(shared.len() > (size_needed + 0x1000), "{}, {size_needed}", shared.len());
            let (hashes, buffer) = shared.split_at_mut(size_needed);
            let mut hashes_into = &mut hashes[..];
            let mut last = None;
            for _ in 0..num_items {
                let short_hash = entry.read_u32::<LE>()?;
                let string_offset = entry.read_u32::<LE>()?;
                if let Some((last_hash, last_offset)) = last {
                    hashes_into.write_u32::<LE>(last_hash)?;
                    // store length
                    hashes_into.write_u32::<LE>(string_offset - last_offset)?;
                }
                last = Some((short_hash, string_offset));
                offset += 8;
            }
            if let Some((last_hash, last_offset)) = last {
                hashes_into.write_u32::<LE>(last_hash)?;
                hashes_into.write_u32::<LE>(variant_size - last_offset)?;
            }

            let mut hashes = &hashes[..];
            shared_flex.clear();
            let mut is_trailing = false;
            write!(shared_flex, "{{")?;
            for _ in 0..num_items {
                let short_hash = hashes.read_u32::<LE>()?;
                let string_len = hashes.read_u32::<LE>()? as usize;
                let do_print = if let Some(key) = options.dictionary_short.get(&short_hash.into()) {
                    if is_trailing {
                        write!(shared_flex, ",")?;
                    }
                    is_trailing = true;
                    write!(shared_flex, "{key:?}:\"")?;
                    true
                } else if !options.skip_unknown {
                    if is_trailing {
                        write!(shared_flex, ",")?;
                    }
                    is_trailing = true;
                    write!(shared_flex, "\"{short_hash:08x}\":\"")?;
                    true
                } else {
                    false
                };

                assert!(buffer.len() >= string_len);
                entry.read_exact(&mut buffer[..string_len])?;
                assert_eq!(0, buffer[string_len - 1]);
                if do_print {
                    shared_flex.reserve(string_len * 2);
                    for c in std::str::from_utf8(&buffer[..string_len - 2]).unwrap().chars() {
                        match c {
                            '\0' => {
                                // characters with a nul before the end have
                                // trailing "[Narrative]" or "[Dev]" text

                                break;
                            }
                            '\t'
                            | '\n'
                            | '\r'
                            | '"' => {
                                write!(shared_flex, "\\{}", match c {
                                    '\t' => 't',
                                    '\n' => 'n',
                                    '\r' => 'r',
                                    '"'  => '"',
                                    _ => unreachable!(),
                                })?;
                            }
                            _ => {
                                write!(shared_flex, "{c}")?;
                            }
                        }
                    }
                    write!(shared_flex, "\"")?;
                }
                offset += string_len;
            }

            assert_eq!(offset, variant_size as usize);
            write!(shared_flex, "}}")?;

            let lang = if let Some(lang) = Language::from_code(kind) {
                write_help!(&mut shared, "{lang}")
            } else {
                write_help!(&mut shared, "{kind:016x}")
            };

            let stem = file_path.file_stem().unwrap().to_str().unwrap();
            let file = write_help!(&mut shared, "{stem}.{lang}");
            let parent = file_path.parent().unwrap();
            let path = path_concat(parent, &mut shared, file, Some("json"));
            options.out.write(path, &shared_flex)?;

            wrote += shared_flex.len() as u64;
        }

        Ok(wrote)
    }
}
