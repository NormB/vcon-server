# The vcon-server logo

The mark is a pair of braces around a voice waveform: a conversation, kept
as a structured record. It was drawn by hand for this project on a 32-unit
grid. The wordmark is Inter (SIL Open Font License 1.1) turned to outlines,
so no font is embedded.

| File | Use |
|---|---|
| `mark.svg` | The source. Every other file is drawn from it by `scripts/icons.py`. |
| `lockup-light.svg`, `lockup-dark.svg` | The mark and the name, for light and dark backgrounds. |
| `social-preview.png` | The repository's social preview, 1280 by 640. |

The site's `favicon.svg`, `favicon.ico`, `apple-touch-icon.png` and
`og-image.png` in `website/static/` are drawn by the same script. The two
social images carry the home page's headline, read from `headline` in
`website/content/_index.md`. After changing `mark.svg` or the headline, run
`python3 scripts/icons.py`; `--check` says whether any file is out of date. It needs `resvg` and the Python `fontTools` package
pinned in `scripts/icons-requirements.txt`; CI runs the check on every push.
`fonts/` holds the two Inter 4.0 files the wordmark is drawn from, unchanged
from the Inter release, under the SIL Open Font License (`fonts/OFL.txt`).

## Use of the name and logo

These files are not covered by the code's MIT or Apache-2.0 licence. You may
use the name and the logo, unchanged, to refer to the vcon-server project, for
example in an article, a talk or a list of integrations. Do not use them in a
way that suggests your product is vcon-server or is endorsed by it, and do not
change them. vcon-server is an independent implementation of the IETF vCon
drafts; it is not an IETF or vCon working-group product.
