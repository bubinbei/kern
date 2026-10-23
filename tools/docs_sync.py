"""Track Russian originals and English translations; no automatic translation."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def expected_pairs(names) -> dict[str, str]:
    result = {"README.ru.md": "README.md"}
    for name in sorted(names):
        path = PurePosixPath(name)
        if path.suffix == ".md" and len(path.parts) > 1 and path.parts[0] == "docs" and path.parts[1] != "en":
            result[name] = "docs/en/" + "/".join(path.parts[1:])
    return result


def pairs(root: Path) -> dict[str, str]:
    return expected_pairs(path.relative_to(root).as_posix() for path in (root / "docs").rglob("*.md"))


def digest(path: Path) -> str:
    # Normalize line endings so Windows and Unix checkouts have the same receipt.
    return hashlib.sha256(path.read_text(encoding="utf-8-sig").encode("utf8")).hexdigest()


def receipt(root: Path, source: str, target: str) -> dict:
    for name in (source, target):
        path = root / name
        if not path.is_file() or not path.read_text(encoding="utf-8-sig").strip():
            raise ValueError(f"Missing or empty document: {name}")
    return {"translation": target, "source_sha256": digest(root / source),
            "translation_sha256": digest(root / target)}


def load(root: Path) -> dict:
    path = root / "docs/translations.json"
    value = json.loads(path.read_text(encoding="utf8")) if path.exists() else {}
    if not isinstance(value, dict):
        raise ValueError("Translation registry must be an object")
    return value


def check(root: Path) -> list[str]:
    expected, saved = pairs(root), load(root)
    errors = []
    for source, target in expected.items():
        try:
            actual = receipt(root, source, target)
        except (OSError, ValueError) as error:
            errors.append(str(error))
            continue
        if saved.get(source) != actual:
            errors.append(f"Translation pair needs a content review and record: {source}")
    for source in saved.keys() - expected.keys():
        errors.append(f"Removed source still in translation registry: {source}")
    targets = set(expected.values())
    for path in (root / "docs/en").rglob("*.md"):
        name = path.relative_to(root).as_posix()
        if name not in targets:
            errors.append(f"English document has no Russian original: {name}")
    return errors


def check_staged(root: Path) -> list[str]:
    """Read index blobs, so a partial commit cannot hide behind a corrected worktree."""
    names = subprocess.check_output(["git", "-C", str(root), "ls-files", "-z"]).decode("utf8").split("\0")
    expected = expected_pairs(name for name in names if name)
    def read_index(name):
        data = subprocess.check_output(["git", "-C", str(root), "show", ":" + name], stderr=subprocess.PIPE)
        return data.decode("utf-8-sig").replace("\r\n", "\n").replace("\r", "\n")
    saved = json.loads(read_index("docs/translations.json"))
    if not isinstance(saved, dict):
        raise ValueError("Staged translation registry must be an object")
    errors = []
    for source, target in expected.items():
        if source not in names or target not in names:
            errors.append(f"Staged document pair is incomplete: {source}")
            continue
        source_text, target_text = read_index(source), read_index(target)
        actual = {"translation": target, "source_sha256": hashlib.sha256(source_text.encode("utf8")).hexdigest(),
                  "translation_sha256": hashlib.sha256(target_text.encode("utf8")).hexdigest()}
        if not source_text.strip() or not target_text.strip() or saved.get(source) != actual:
            errors.append(f"Staged translation pair needs review and record: {source}")
    for source in saved.keys() - expected.keys():
        errors.append(f"Staged registry contains a removed source: {source}")
    for name in names:
        if name.startswith("docs/en/") and name.endswith(".md") and name not in expected.values():
            errors.append(f"Staged English document has no Russian original: {name}")
    return errors


def record(root: Path, selected: list[str] | None) -> int:
    expected, saved = pairs(root), load(root)
    names = list(expected) if selected is None else selected
    if not names or any(name not in expected for name in names):
        raise ValueError("Specify a Russian original from docs/ or README.ru.md")
    # Validate every selected pair before changing the registry.
    updates = {source: receipt(root, source, expected[source]) for source in names}
    saved.update(updates)
    if selected is None:
        saved = {source: saved[source] for source in expected}
    value = json.dumps(saved, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    path = root / "docs/translations.json"
    fd, temporary = tempfile.mkstemp(prefix=".translations-", suffix=".tmp", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf8", newline="\n") as stream:
            stream.write(value)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)
    return len(updates)


def main() -> int:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf8")
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("check").add_argument("--staged", action="store_true", help="Also validate Git index blobs")
    part = sub.add_parser("record")
    scope = part.add_mutually_exclusive_group(required=True)
    scope.add_argument("--source", action="append")
    scope.add_argument("--all", action="store_true")
    args = parser.parse_args()
    try:
        if args.command == "record":
            count = record(ROOT, None if args.all else args.source)
            print(f"Recorded {count} translation pairs; hashes do not prove semantic equivalence.")
            return 0
        errors = check(ROOT)
        if args.staged:
            errors += check_staged(ROOT)
        if errors:
            print("DOCUMENTATION SYNC FAILED\n" + "\n".join(errors))
            return 1
        print(f"DOCUMENTATION SYNC PASS ({len(pairs(ROOT))} pairs; Russian originals, English translations)")
        return 0
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f"DOCUMENTATION ERROR: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
