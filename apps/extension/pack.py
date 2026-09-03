"""Pack an extension directory into a byte-reproducible zip.

Two builds of one tree must produce one artefact, because that is what lets a
reviewer check the package they were sent against the source they were given.
Three things in a zip are otherwise free to vary: entry order, timestamps and
permission bits. All three are fixed here.

Usage: pack.py <out.zip> <root> <name>...

Members are named explicitly rather than walked, so a file appears in the
package because someone listed it and not because it was left in the build
directory.
"""

import hashlib
import sys
import zipfile
from pathlib import Path

# The earliest instant a zip timestamp can express. Chosen because it is the
# one value that carries no timezone and so cannot vary by builder.
ZIP_EPOCH = (1980, 1, 1, 0, 0, 0)

# `create_system` 3 marks the entry as Unix so the mode below is what an
# extractor honours; 0o644 is the mode every member gets, since nothing in an
# extension package is executable.
UNIX = 3
MODE = 0o644


def pack(out: Path, root: Path, names: list[str]) -> str:
    with zipfile.ZipFile(out, "w") as archive:
        for name in sorted(names):
            entry = zipfile.ZipInfo(name, date_time=ZIP_EPOCH)
            entry.compress_type = zipfile.ZIP_DEFLATED
            entry.create_system = UNIX
            entry.external_attr = MODE << 16
            archive.writestr(entry, (root / name).read_bytes())
    return hashlib.sha256(out.read_bytes()).hexdigest()


def main(argv: list[str]) -> int:
    if len(argv) < 4:
        print(__doc__, file=sys.stderr)
        return 2
    out, root, names = Path(argv[1]), Path(argv[2]), argv[3:]
    out.parent.mkdir(parents=True, exist_ok=True)
    missing = [name for name in names if not (root / name).is_file()]
    if missing:
        print(f"pack: not in {root}: {' '.join(missing)}", file=sys.stderr)
        return 1
    digest = pack(out, root, names)
    print(f"{digest}  {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
