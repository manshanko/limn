limn
=====

Extractor for the bundle format used in the game Warhammer 40k Darktide.

## Examples

Extract all files:
```
limn.exe "C:\Program Files (x86)\Steam\steamapps\common\Warhammer 40,000 Darktide\bundle"
```

Extract only lua files:
```
limn.exe "C:\Program Files (x86)\Steam\steamapps\common\Warhammer 40,000 Darktide\bundle" lua
```

## Dictionary

If a file named `dictionary.txt` is placed next `limn.exe` it will be used for reverse hash lookup.

Currently when limn is using a dictionary it will only extract files that it is able to find a name for.

## Supported File Types

limn only supports a few file types used in Darktide bundles.

### lua

Fatshark uses a private fork of LuaJIT in Darktide. All `lua` files are stored as LuaJIT bytecode that, aside from a header version change, is compatible with existing tooling for LuaJIT (like any decompilers).

### texture

`texture` files are stored as DDS. For mipmap levels 64KiB or larger Darktide deduplicates them to a resource file at `data/**/*`.

limn will export the highest quality mipmap level found.

For converting DDS to PNG [texconv](https://github.com/Microsoft/DirectXTex/wiki/Texconv) and [ffmpeg](https://ffmpeg.org/) can be used:
```bash
texconv -ft bmp -f B8G8R8A8_UNORM -y texture_file.dds
ffmpeg -i texture_file.BMP texture_file.png
```