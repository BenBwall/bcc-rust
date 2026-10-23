"""The COFF oracle must enforce invariants even under python -O."""
import struct
import tempfile
import unittest
from pathlib import Path

from compare_coff_objects import view


class CoffValidationTests(unittest.TestCase):
    def fixture(self):
        # One initialized section with one relocation and one external symbol.
        header = struct.pack('<HHIIIHH', 0x8664, 1, 0, 78, 1, 0, 0)
        section = struct.pack('<8sIIIIIIHHI', b'.data', 0, 0, 8, 60, 68, 0, 1, 0, 0xC0100040)
        return bytearray(header + section + bytes(8) + struct.pack('<IIH', 0, 0, 1)
                         + struct.pack('<8sIhHBB', b'ext', 0, 0, 0, 2, 0)
                         + struct.pack('<I', 4))

    def parse(self, data):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'test.obj'
            path.write_bytes(data)
            return view(path)

    def test_valid_object(self):
        self.assertEqual(self.parse(self.fixture())['header'][0], 0x8664)

    def test_rejects_malformed_fields(self):
        for offset, kind, value in [
            (0, 'H', 0), (16, 'H', 1), (96, 'I', 3),
            (28, 'I', 1), (32, 'I', 1), (48, 'I', 1), (54, 'H', 1),
            (40, 'I', 1), (68, 'I', 8), (95, 'B', 1),
        ]:
            with self.subTest(offset=offset):
                data = self.fixture()
                struct.pack_into('<' + kind, data, offset, value)
                with self.assertRaises(ValueError):
                    self.parse(data)

    def test_rejects_invalid_overflow_sentinel(self):
        for count, target, kind, nrel in [(65535, 0, 0, 65535),
                                           (65536, 1, 0, 65535),
                                           (65536, 0, 1, 65535),
                                           (65536, 0, 0, 1)]:
            with self.subTest(count=count, target=target, kind=kind, nrel=nrel):
                data = self.fixture()
                struct.pack_into('<I', data, 56, 0xC1100040)
                struct.pack_into('<H', data, 52, nrel)
                struct.pack_into('<IIH', data, 68, count, target, kind)
                with self.assertRaises(ValueError):
                    self.parse(data)


if __name__ == '__main__':
    unittest.main()
