"""Verify screenshot completeness/provenance without launching or pixel-testing an app."""
import argparse
import json
import pathlib

from readme_screenshots import REPO, digest, png_size, ui_inputs, verify_capture


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--capture', type=pathlib.Path)
    args = parser.parse_args()
    root = args.capture or REPO / 'assets/screenshots'
    report = json.loads((root / 'capture.json').read_bytes())
    verify_capture(root, report)
    if not args.capture:
        hero = report['hero']
        path = REPO / 'assets/screenshot.png'
        if digest(path) != hero['sha256'] or png_size(path.read_bytes()) != (hero['width'], hero['height']):
            raise ValueError('Lead image does not match its generated composition')
    changed = sorted(path for path, value in ui_inputs().items() if report.get('ui_inputs', {}).get(path) != value)
    if changed:
        print('UI source review needed (not a pixel failure): ' + ', '.join(changed))
    print('PASS: complete real-frame PNG set, fixed paths, checksums, dimensions and successful cleanup')


if __name__ == '__main__':
    main()
