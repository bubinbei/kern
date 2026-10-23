import importlib.util
from pathlib import Path
import tempfile
import subprocess
import unittest

PROJECT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("docs_sync", PROJECT / "tools/docs_sync.py")
sync = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sync)


class DocumentationSyncTests(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        self.root = Path(self.folder.name)
        (self.root / "docs/en").mkdir(parents=True)
        self.write("README.ru.md", "Русский оригинал\n")
        self.write("README.md", "English translation\n")
        self.write("docs/VISION.md", "Русские требования\n")
        self.write("docs/en/VISION.md", "Translated requirements\n")

    def write(self, name, value):
        (self.root / name).write_text(value, encoding="utf8")

    def test_changed_original_or_translation_requires_explicit_new_receipt(self):
        self.assertTrue(sync.check(self.root))
        self.assertEqual(sync.record(self.root, None), 2)
        self.assertEqual(sync.check(self.root), [])
        self.write("docs/VISION.md", "Новая версия\n")
        self.assertTrue(sync.check(self.root))
        self.write("docs/en/VISION.md", "Updated English version\n")
        self.assertTrue(sync.check(self.root))
        self.assertEqual(sync.record(self.root, ["docs/VISION.md"]), 1)
        self.assertEqual(sync.check(self.root), [])
        self.write("docs/en/VISION.md", "Changed after review\n")
        self.assertTrue(sync.check(self.root))

    def test_new_missing_and_removed_documents_cannot_silently_pass(self):
        sync.record(self.root, None)
        saved = (self.root / "docs/translations.json").read_bytes()
        self.write("docs/NEW.md", "Новый документ\n")
        self.assertTrue(sync.check(self.root))
        with self.assertRaises(ValueError):
            sync.record(self.root, None)
        self.assertEqual((self.root / "docs/translations.json").read_bytes(), saved)
        self.write("docs/en/NEW.md", "New document\n")
        sync.record(self.root, None)
        (self.root / "docs/NEW.md").unlink()
        self.assertTrue(sync.check(self.root))
        sync.record(self.root, None)
        self.assertTrue(any("no Russian original" in error for error in sync.check(self.root)))
        (self.root / "docs/en/NEW.md").unlink()
        self.assertEqual(sync.check(self.root), [])

    def test_line_endings_do_not_make_false_translation_changes(self):
        sync.record(self.root, None)
        path = self.root / "docs/en/VISION.md"
        path.write_bytes(path.read_text(encoding="utf8").replace("\n", "\r\n").encode("utf8"))
        self.assertEqual(sync.check(self.root), [])
        with self.assertRaises(ValueError):
            sync.record(self.root, ["../outside.md"])

    def test_corrected_worktree_does_not_hide_an_outdated_staged_translation(self):
        def git(*args):
            subprocess.check_output(["git", "-C", str(self.root), *args], stderr=subprocess.PIPE)
        git("init", "-b", "main")
        sync.record(self.root, None)
        git("add", ".")
        self.assertEqual(sync.check_staged(self.root), [])
        self.write("docs/VISION.md", "Новое требование\n")
        git("add", "docs/VISION.md")
        self.write("docs/en/VISION.md", "New requirement\n")
        sync.record(self.root, ["docs/VISION.md"])
        self.assertEqual(sync.check(self.root), [])
        self.assertTrue(sync.check_staged(self.root))
        git("add", "docs/en/VISION.md", "docs/translations.json")
        self.assertEqual(sync.check_staged(self.root), [])
