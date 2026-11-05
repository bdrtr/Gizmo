# Test fonts

Small subsets for the shaping tests in `../mod.rs`, which need real `GSUB` and `GPOS` tables and must
not depend on what a machine has installed. They are only read under `#[cfg(test)]`.

| file | from | keeps |
|---|---|---|
| `noto-sans-arabic-subset.ttf` | Noto Sans Arabic 2.013 | the letters of سلام and مرحبا |
| `noto-sans-devanagari-subset.ttf` | Noto Sans Devanagari 2.007 | क ि ् ष and a space |
| `noto-sans-thai-subset.ttf` | Noto Sans Thai | ฤ and ๅ, which it joins with `liga` |

Built with fontTools:

```bash
pyftsubset NotoSansArabic-Regular.ttf --text="سلامرحبا" --layout-features='*' --no-hinting \
  --desubroutinize --output-file=noto-sans-arabic-subset.ttf
pyftsubset NotoSansDevanagari-Regular.ttf --unicodes="U+0915,U+093F,U+094D,U+0937,U+0020" \
  --layout-features='*' --no-hinting --desubroutinize --output-file=noto-sans-devanagari-subset.ttf
pyftsubset NotoSansThai-Regular.ttf --unicodes="U+0E24,U+0E45" --layout-features='*' --no-hinting \
  --desubroutinize --output-file=noto-sans-thai-subset.ttf
```

Copyright 2022 The Noto Project Authors (<https://github.com/notofonts/arabic>,
<https://github.com/notofonts/devanagari>, <https://github.com/notofonts/thai>). Licensed under the SIL Open Font License 1.1; the licence
text is `assets/fonts/OFL.txt` in this crate.
