"""Critical screenshot publication guards; no app or image renderer is started."""
import copy
import json
import pathlib
import struct
import tempfile
import unittest
import zlib

from readme_screenshots import CONTRACT, digest, png_size, verify_capture


def png():
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 400, 600, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress((b'\0' + b'\0' * 1600) * 600)) + chunk(b'IEND', b''))


class PublicationGuards(unittest.TestCase):
    def fixture(self, root):
        report = {'schema': 1, 'status': 'pass', 'cleanup': 'pass',
                  'contract_sha256': digest(CONTRACT), 'captures': {}}
        data = png()
        for scene in json.loads(CONTRACT.read_bytes())['scenes']:
            path = root / scene['file']
            path.write_bytes(data)
            report['captures'][scene['id']] = {'file': scene['file'], 'sha256': digest(path), 'width': 400, 'height': 600}
        return report

    def test_publication_refuses_failed_cleanup_missing_scenes_and_changed_pixels(self):
        with tempfile.TemporaryDirectory() as folder:
            root = pathlib.Path(folder)
            report = self.fixture(root)
            verify_capture(root, report)
            for invalid in [dict(report, cleanup='fail'), dict(report, status='fail')]:
                with self.assertRaises(ValueError):
                    verify_capture(root, invalid)
            missing = copy.deepcopy(report)
            del missing['captures']['topology']
            with self.assertRaises(ValueError):
                verify_capture(root, missing)
            (root / 'main-dark.png').write_bytes(png() + b'changed')
            with self.assertRaises(ValueError):
                verify_capture(root, report)

    def test_png_requires_a_complete_bounded_frame_and_paths_are_whitelisted(self):
        self.assertEqual(png_size(png()), (400, 600))
        with self.assertRaises(ValueError):
            png_size(png()[:-12])
        with tempfile.TemporaryDirectory() as folder:
            root = pathlib.Path(folder)
            report = self.fixture(root)
            report['captures']['main-dark']['file'] = '../state.json'
            with self.assertRaises(ValueError):
                verify_capture(root, report)


if __name__ == '__main__':
    unittest.main()
