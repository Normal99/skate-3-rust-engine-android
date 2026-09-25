"""Xbox 360 game disc (XDVDFS / XISO) reader.

Replaces the Windows-only extract-xiso download where setup runs elsewhere
(phones via Termux, Linux, macOS). Reads only the player's own image.
"""
from pathlib import Path
import struct

SECTOR = 2048
MAGIC = b'MICROSOFT*XBOX*MEDIA'
# Game partition offsets probed by extract-xiso: plain XISO, XGD3, XGD2, XGD1.
PARTITION_OFFSETS = (0, 0x2080000, 0xFD90000, 0x18300000)
DIRECTORY = 0x10
NO_ENTRY = 0xFFFF
MAX_ENTRIES = 200_000
MAX_DEPTH = 64


def find_partition(stream):
    """Return (partition byte offset, root table sector, root table size)."""
    for offset in PARTITION_OFFSETS:
        stream.seek(offset + 32 * SECTOR)
        header = stream.read(28)
        if len(header) == 28 and header[:20] == MAGIC:
            sector, size = struct.unpack_from('<II', header, 20)
            return offset, sector, size
    raise ValueError('Not an Xbox 360 game image: no XDVDFS volume found')


def _entries(stream, partition, sector, size, image_size):
    """Yield (name, sector, size, is_directory) from one directory table.

    Tables are binary trees: each entry holds left/right child offsets in
    4-byte units from the table start. Unused space is 0xFF padding.
    """
    if size == 0:
        return
    start = partition + sector * SECTOR
    if start + size > image_size:
        raise ValueError('Directory table lies outside the image')
    stream.seek(start)
    table = stream.read(size)
    pending, seen = [0], set()
    while pending:
        offset = pending.pop()
        if offset in seen:
            continue
        seen.add(offset)
        if offset + 14 > len(table):
            raise ValueError('Directory entry lies outside its table')
        left, right, child_sector, length, attributes, name_length = struct.unpack_from('<HHIIBB', table, offset)
        if left == NO_ENTRY and right == NO_ENTRY:
            # An empty table starts with padding instead of an entry.
            continue
        raw = table[offset + 14:offset + 14 + name_length]
        if len(raw) != name_length or not raw:
            raise ValueError('Truncated directory entry name')
        name = raw.decode('latin-1')
        if name in ('.', '..') or any(c in name for c in '/\\\0'):
            raise ValueError('Unsafe name on disc: ' + repr(name))
        yield name, child_sector, length, bool(attributes & DIRECTORY)
        for child in (left, right):
            if child not in (0, NO_ENTRY):
                pending.append(child * 4)


def extract(image, destination, report=None):
    """Extract every file of an Xbox 360 image into destination."""
    image, destination = Path(image), Path(destination)
    destination.mkdir(parents=True, exist_ok=True)
    root = destination.resolve()
    image_size = image.stat().st_size
    count = 0
    with image.open('rb') as stream:
        partition, sector, size = find_partition(stream)
        pending = [(sector, size, root, 0)]
        while pending:
            sector, size, directory, depth = pending.pop()
            if depth > MAX_DEPTH:
                raise ValueError('Directory nesting is too deep')
            for name, child_sector, length, is_directory in _entries(stream, partition, sector, size, image_size):
                count += 1
                if count > MAX_ENTRIES:
                    raise ValueError('Too many entries on disc')
                target = directory / name
                if not target.resolve().is_relative_to(root):
                    raise ValueError('Unsafe path on disc: ' + name)
                if is_directory:
                    target.mkdir(exist_ok=True)
                    pending.append((child_sector, length, target, depth + 1))
                    continue
                begin = partition + child_sector * SECTOR
                if begin + length > image_size:
                    raise ValueError('File lies outside the image: ' + name)
                if report and length >= 64 * 1024 * 1024:
                    report(f'Extracting {name} ({length // (1024 * 1024)} MB)')
                stream.seek(begin)
                remaining = length
                with target.open('wb') as output:
                    while remaining:
                        chunk = stream.read(min(remaining, 4 * 1024 * 1024))
                        if not chunk:
                            raise ValueError('Image ended inside ' + name)
                        output.write(chunk)
                        remaining -= len(chunk)
    return count
