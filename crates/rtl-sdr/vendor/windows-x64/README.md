Prebuilt Windows x64 `librtlsdr` binaries.

- `librtlsdr.dll`: from `rtlsdr-bin-w64_dlldep.zip` at
  https://github.com/librtlsdr/librtlsdr/releases/tag/v0.9.0
- `rtlsdr.lib`: MSVC import library generated from `librtlsdr.dll`'s export
  table (`llvm-lib /def:rtlsdr.def /out:rtlsdr.lib /machine:x64`), since the
  release zip only ships the DLL. Named `rtlsdr.lib` to match
  `#[link(name = "rtlsdr")]`.
- `librtlsdr.dll.a`: MinGW-w64 import library for the same DLL, generated with:
  ```
  x86_64-w64-mingw32-objdump -p librtlsdr.dll | \
    sed -n '/\[Ordinal\/Name Pointer\] Table/,/^$/p' | \
    grep -oP '^\s*\[\s*\d+\]\s*\+base\[\s*\d+\]\s+[0-9a-f]+\s+\K\S+' | \
    { echo EXPORTS; cat; } > librtlsdr.def
  x86_64-w64-mingw32-dlltool --input-def librtlsdr.def --dllname librtlsdr.dll --output-lib librtlsdr.dll.a
  ```
  Named with the `lib` prefix (unlike the MSVC `.lib`) because MinGW's linker
  resolves `-lrtlsdr` to `librtlsdr.dll.a`/`librtlsdr.a`, not `rtlsdr.dll.a`.

To update: download a newer `rtlsdr-bin-w64_dlldep.zip`, regenerate both
`rtlsdr.lib` and `librtlsdr.dll.a` the same way, and bump the version here.
