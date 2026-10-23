"""History, interruption and scope tests using independent temporary vaults."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

PROJECT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("project_context", PROJECT / "tools/project_context.py")
context = importlib.util.module_from_spec(spec)
spec.loader.exec_module(context)


class ContextTests(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        self.root = Path(self.folder.name)
        for folder in ("context", "docs", "obsidian/Projects/workspace", "obsidian/Archive"):
            (self.root / folder).mkdir(parents=True)
        self.write("docs/canon.md", "# Contract\nFresh observation only.\n")
        self.write("docs/skill.md", "# Skill\n")
        self.write("obsidian/Archive/BOOTSTRAP.md", "Oldbootstrapword\n")
        self.item = {"title": "Workspace", "skill": "docs/skill.md", "sources": ["docs/canon.md"]}
        self.write("context/projects.json", json.dumps({"workspace": self.item}))
        self.write("obsidian/Projects/workspace/STATE.md", "# Current\nNot hardware-validated.\n")
        self.refresh()

    def write(self, name, text):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf8")

    def refresh(self):
        rows = context.entries(self.root, "workspace")
        self.write("obsidian/Projects/workspace/RECENT.md", context.recent("workspace", self.item, rows))

    def value(self, number=1):
        return {"id": f"round-{number}", "date": f"2026-10-{number:02d}T10:00:00+04:00",
                "summary": f"Observed synthetic outcome {number}.", "validation": "Temporary fixture, no hardware.",
                "sources": ["docs/canon.md"], "next": "Review the contract."}

    def test_four_recent_rounds_do_not_delete_monthly_history(self):
        for number in range(1, 8):
            value = self.value(number)
            if number < 3:
                value["date"] = value["date"].replace("2026-10", "2026-09")
            context.record(self.root, "workspace", value)
        rows = context.entries(self.root, "workspace")
        self.assertEqual(len(rows), 7)
        recent = context.read(self.root / "obsidian/Projects/workspace/RECENT.md")
        self.assertNotIn("| round-3\n", recent)
        self.assertIn("| round-4\n", recent)
        self.assertLess(recent.index("round-7"), recent.index("round-6"))
        self.assertEqual(len(list((self.root / "obsidian/Projects/workspace/rounds").glob("*.md"))), 2)
        context.check(self.root)

    def test_idempotence_and_conflict_never_rewrite_previous_record(self):
        value = self.value()
        self.assertTrue(context.record(self.root, "workspace", value))
        self.assertFalse(context.record(self.root, "workspace", value))
        original = context.entries(self.root, "workspace")[0][2]
        value["summary"] = "Different result."
        with self.assertRaisesRegex(ValueError, "already exists"):
            context.record(self.root, "workspace", value)
        self.write("docs/canon.md", "Changed evidence\n")
        with self.assertRaisesRegex(ValueError, "already exists"):
            context.record(self.root, "workspace", self.value())
        self.assertEqual(context.entries(self.root, "workspace")[0][2], original)
        context.check(self.root)  # Historical evidence hashes do not require current evidence to match.

    def test_stale_recent_and_history_edit_are_detected_separately(self):
        context.record(self.root, "workspace", self.value())
        self.write("obsidian/Projects/workspace/RECENT.md", "Edited derived file\n")
        with self.assertRaisesRegex(ValueError, "Stale RECENT"):
            context.check(self.root)
        self.refresh()
        context.check(self.root)
        path = self.root / "obsidian/Projects/workspace/rounds/2026-10.md"
        path.write_text(context.read(path).replace("Observed", "Invented"), encoding="utf8")
        with self.assertRaisesRegex(ValueError, "integrity mismatch"):
            self.refresh()

    def test_invalid_input_cannot_create_a_round(self):
        bad = [{"date": "2026-10-21T10:00:00"}, {"date": "20261021T100000+0400"},
               {"summary": "first\n## forged header"}, {"sources": ["../outside.md"]},
               {"sources": ["docs/missing.md"]}, {"sources": []}, {"sources": [{}]},
               {"validation": ""}, {"id": "UPPERCASE"}, {"summary": "x" * 751}]
        for change in bad:
            with self.subTest(change=change), self.assertRaises(ValueError):
                context.record(self.root, "workspace", {**self.value(), **change})
        self.assertEqual(context.entries(self.root, "workspace"), [])

    def test_state_and_recent_budgets_fail_explicitly(self):
        self.write("obsidian/Projects/workspace/STATE.md", "x" * 2501)
        with self.assertRaisesRegex(ValueError, "STATE"):
            context.check(self.root)
        self.write("obsidian/Projects/workspace/STATE.md", "Small state\n")
        rows = [(None, str(number), "x" * 1700) for number in range(4)]
        with self.assertRaisesRegex(ValueError, "RECENT exceeds"):
            context.recent("workspace", self.item, rows)

    def test_search_scope_and_archive_are_explicit(self):
        for number in range(1, 6):
            value = self.value(number)
            value["summary"] = "Onlyoldword" if number == 1 else "Currentword"
            context.record(self.root, "workspace", value)
        self.write("docs/not-configured.md", "Outsideword\n")
        search = lambda word, archive=False: context.search(self.root, "workspace", self.item, word, archive)["matches"]
        self.assertFalse(search("Onlyoldword"))
        self.assertTrue(search("Onlyoldword", True))
        self.assertFalse(search("Oldbootstrapword"))
        self.assertTrue(search("Oldbootstrapword", True))
        self.assertFalse(search("Outsideword", True))
        match = search("fresh OBSERVATION")[0]
        self.assertEqual(match["line"], 2)
        self.assertEqual(len(match["sha256"]), 64)

    def test_concurrent_writer_is_rejected_without_new_data(self):
        with context.locked(self.root):
            with self.assertRaisesRegex(ValueError, "locked"):
                context.record(self.root, "workspace", self.value())
            self.assertEqual(context.entries(self.root, "workspace"), [])
        self.assertTrue(context.record(self.root, "workspace", self.value()))

    def test_interruption_after_journal_commit_is_recoverable(self):
        original = context.atomic_write
        def interrupted(path, value):
            if path.name == "RECENT.md":
                raise OSError("Simulated interrupted write")
            original(path, value)
        with patch.object(context, "atomic_write", interrupted):
            with self.assertRaises(OSError):
                context.record(self.root, "workspace", self.value())
        self.assertEqual(len(context.entries(self.root, "workspace")), 1)
        self.assertFalse((self.root / "context/.record.lock").exists())
        with self.assertRaisesRegex(ValueError, "Stale RECENT"):
            context.check(self.root)
        self.assertFalse(context.record(self.root, "workspace", self.value()))
        context.check(self.root)

    def test_symlink_cannot_read_external_context(self):
        path = self.root / "docs/link.md"
        try:
            path.symlink_to(PROJECT / "README.md")
        except OSError:
            self.skipTest("Symlink creation unavailable")
        with self.assertRaises(ValueError):
            context.record(self.root, "workspace", {**self.value(), "sources": ["docs/link.md"]})
