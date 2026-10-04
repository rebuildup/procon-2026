#!/usr/bin/env python3
"""Release metadata regressions using isolated fixture manifests."""
import importlib.util
import pathlib
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("release_version", pathlib.Path(__file__).with_name("check-release-version.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ReleaseVersionTests(unittest.TestCase):
    def test_version_and_lock_must_match_target(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            (root / "crate").mkdir()
            (root / "crate/Cargo.toml").write_text('[package]\nname="fixture"\n')
            (root / "Cargo.toml").write_text('[workspace]\nmembers=["crate"]\n[workspace.package]\nversion="0.2.0"\n')
            (root / "Cargo.lock").write_text('[[package]]\nname="fixture"\nversion="0.2.0"\n')
            module.check("release-0-2-0", root)
            for branch in ["290", "release-0-1-0"]:
                with self.assertRaises(ValueError):
                    module.check(branch, root)
            (root / "Cargo.lock").write_text('[[package]]\nname="fixture"\nversion="0.1.0"\n')
            with self.assertRaises(ValueError):
                module.check("release-0-2-0", root)


if __name__ == "__main__":
    unittest.main()
