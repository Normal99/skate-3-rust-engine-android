"""Synthetic XDVDFS images; no game content."""
from pathlib import Path
import struct, tempfile, unittest

from tools.asset_pipeline import xiso


class Image:
    """Tiny XDVDFS writer: directories chain entries through right links."""

    def __init__(self, partition):
        self.partition = partition
        self.sectors = {}
        self.next = 40

    def allocate(self, data):
        sector = self.next
        self.sectors[sector] = data
        self.next += max(1, -(-len(data) // xiso.SECTOR))
        return sector

    def directory(self, children, extra=b''):
        """children: list of (name, bytes | list) in table order."""
        placed = []
        for name, value in children:
            if isinstance(value, list):
                sector, size = self.directory(value)
                placed.append((name, sector, size, xiso.DIRECTORY))
            else:
                placed.append((name, self.allocate(value), len(value), 0))
        if not placed:
            return 0, 0
        records, offsets, offset = [], [], 0
        for name, *_ in placed:
            offsets.append(offset)
            offset += (14 + len(name) + 3) & ~3
        for index, (name, sector, size, attributes) in enumerate(placed):
            right = offsets[index + 1] // 4 if index + 1 < len(placed) else 0
            record = struct.pack('<HHIIBB', 0, right, sector, size, attributes, len(name)) + name.encode()
            records.append(record + b'\xff' * (-len(record) % 4))
        table = b''.join(records) + extra
        return self.allocate(table), len(table)

    def write(self, path, root_children, root_extra=b''):
        root_sector, root_size = self.directory(root_children, root_extra)
        with path.open('wb') as out:
            out.seek(self.partition + 32 * xiso.SECTOR)
            out.write(xiso.MAGIC + struct.pack('<II', root_sector, root_size) + b'\0' * 8)
            for sector, data in self.sectors.items():
                out.seek(self.partition + sector * xiso.SECTOR)
                out.write(data)
            out.seek(self.partition + self.next * xiso.SECTOR)
            out.write(b'\0')


class XisoTests(unittest.TestCase):
    def tree(self):
        return [
            ('default.xex', b'XEX2' + b'x' * 5000),
            ('data', [('big', [('miscload.big', b'BIGF' * 700), ('db.big', b'')]),
                      ('content', [('worldDIST_University.big', b'u' * 3000)]),
                      ('empty', [])]),
        ]

    def check(self, partition):
        with tempfile.TemporaryDirectory() as temp:
            temp = Path(temp)
            Image(partition).write(temp / 'game.iso', self.tree())
            count = xiso.extract(temp / 'game.iso', temp / 'out')
            self.assertEqual(count, 8)
            self.assertEqual((temp / 'out/default.xex').read_bytes(), b'XEX2' + b'x' * 5000)
            self.assertEqual((temp / 'out/data/big/miscload.big').read_bytes(), b'BIGF' * 700)
            self.assertEqual((temp / 'out/data/big/db.big').read_bytes(), b'')
            self.assertEqual((temp / 'out/data/content/worldDIST_University.big').stat().st_size, 3000)
            self.assertTrue((temp / 'out/data/empty').is_dir())

    def test_plain_image(self):
        self.check(0)

    def test_retail_partition_offset(self):
        # XGD2 layout; the file is sparse, so this stays small on disk.
        self.check(0xFD90000)

    def test_rejects_non_xbox_image_and_unsafe_names(self):
        with tempfile.TemporaryDirectory() as temp:
            temp = Path(temp)
            (temp / 'plain.iso').write_bytes(b'\0' * (40 * xiso.SECTOR))
            with self.assertRaises(ValueError):
                xiso.extract(temp / 'plain.iso', temp / 'out')
            Image(0).write(temp / 'bad.iso', [('..', b'x')])
            with self.assertRaises(ValueError):
                xiso.extract(temp / 'bad.iso', temp / 'out2')

    def test_rejects_file_beyond_image(self):
        with tempfile.TemporaryDirectory() as temp:
            temp = Path(temp)
            image = Image(0)
            image.write(temp / 'short.iso', [('default.xex', b'x' * 10)])
            data = bytearray((temp / 'short.iso').read_bytes())
            root = image.partition + 32 * xiso.SECTOR
            table_sector = struct.unpack_from('<I', data, root + 20)[0]
            struct.pack_into('<I', data, table_sector * xiso.SECTOR + 8, 1 << 30)
            (temp / 'short.iso').write_bytes(bytes(data))
            with self.assertRaises(ValueError):
                xiso.extract(temp / 'short.iso', temp / 'out')


if __name__ == '__main__':
    unittest.main()
