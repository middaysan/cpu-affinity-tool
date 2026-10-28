"""Check resolved normal dependency graphs, without compiling either target."""
import pathlib
import subprocess
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]


def packages(target, feature):
    result = subprocess.run(
        ["cargo", "tree", "--locked", "--target", target, "--features", feature,
         "--edges", "normal", "--prefix", "none", "--format", "{p}"],
        cwd=ROOT, check=True, capture_output=True, text=True,
    )
    return {line.split()[0] for line in result.stdout.splitlines() if line.strip()}


class RendererDependencies(unittest.TestCase):
    def test_windows_has_wgpu_without_opengl(self):
        resolved = packages("x86_64-pc-windows-msvc", "windows")
        self.assertIn("egui-wgpu", resolved)
        self.assertIn("wgpu", resolved)
        self.assertFalse(resolved & {"glow", "egui_glow", "glutin", "glutin-winit"},
                         "Windows release must not link the unused OpenGL renderer")

    def test_linux_keeps_glow_without_wgpu(self):
        resolved = packages("x86_64-unknown-linux-gnu", "linux")
        self.assertTrue({"glow", "egui_glow", "glutin"} <= resolved)
        self.assertFalse(resolved & {"wgpu", "egui-wgpu"})


if __name__ == "__main__":
    unittest.main()
