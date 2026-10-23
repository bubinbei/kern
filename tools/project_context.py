"""Small offline Markdown context manager for KERN (Python standard library)."""
from __future__ import annotations

import argparse
from contextlib import contextmanager
from datetime import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
STATE_LIMIT, RECENT_LIMIT, OUTPUT_LIMIT = 2500, 6500, 12000
HEADER = re.compile(r"^## ([^\n]+) \| ([a-z0-9][a-z0-9-]{0,95})\n", re.M)
DIGEST = re.compile(r"\n<!-- round-digest: ([a-f0-9]{64}) -->\n?$")


def safe_path(root: Path, name: str) -> Path:
    if not isinstance(name, str) or not name or "\\" in name or Path(name).is_absolute():
        raise ValueError("Use a relative repository path with forward slashes")
    path = root / name
    if ".." in Path(name).parts or not path.resolve().is_relative_to(root.resolve()):
        raise ValueError("Path leaves the repository")
    for part in [path, *path.parents]:
        if part == root:
            break
        if part.exists() and (part.is_symlink() or bool(getattr(part.stat(), "st_file_attributes", 0) & 0x400)):
            raise ValueError("Linked context sources are not allowed")
    return path


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8-sig").replace("\r\n", "\n")


def atomic_write(path: Path, value: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=".context-", suffix=".tmp", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf8", newline="\n") as stream:
            stream.write(value)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


@contextmanager
def locked(root: Path):
    path = root / "context/.record.lock"
    try:
        fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
    except FileExistsError as error:
        raise ValueError("Context is locked; see context/README.md before removing a stale lock") from error
    try:
        os.write(fd, str(os.getpid()).encode("ascii"))
        os.close(fd)
        yield
    finally:
        path.unlink(missing_ok=True)


def projects(root: Path) -> dict:
    value = json.loads(read(safe_path(root, "context/projects.json")))
    if not isinstance(value, dict) or not value:
        raise ValueError("Expected a nonempty project map")
    for name, item in value.items():
        if not re.fullmatch(r"[a-z][a-z0-9-]*", name) or not isinstance(item, dict):
            raise ValueError("Invalid project definition")
        for field in ("title", "skill", "sources"):
            if field not in item:
                raise ValueError(f"Missing {field} for {name}")
        if not isinstance(item["title"], str) or not item["title"] or "\n" in item["title"]:
            raise ValueError("Invalid project title")
        if not isinstance(item["sources"], list) or not item["sources"]:
            raise ValueError("Specify project sources")
        for source in [item["skill"], *item["sources"]]:
            if not safe_path(root, source).is_file():
                raise ValueError(f"Missing configured source: {source}")
    return value


def base(root: Path, name: str) -> Path:
    return safe_path(root, f"obsidian/Projects/{name}")


def timestamp(value: str) -> datetime:
    if not re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:Z|[+-]\d{2}:\d{2})", value):
        raise ValueError("Use ISO date YYYY-MM-DDTHH:MM:SS with Z or a timezone offset")
    result = datetime.fromisoformat(value)
    if result.tzinfo is None or result.utcoffset() is None:
        raise ValueError("Round date must include a timezone")
    return result


def entries(root: Path, name: str) -> list[tuple[datetime, str, str]]:
    found, ids = [], set()
    folder = base(root, name) / "rounds"
    if folder.exists():
        safe_path(root, folder.relative_to(root).as_posix())
    for path in sorted(folder.glob("*.md")):
        safe_path(root, path.relative_to(root).as_posix())
        text = read(path)
        headers = list(HEADER.finditer(text))
        if not headers and text.strip() != "# Completed rounds":
            raise ValueError(f"Unrecognized round history: {path.name}")
        if headers and text[:headers[0].start()].strip() != "# Completed rounds":
            raise ValueError(f"Invalid history preamble: {path.name}")
        for index, match in enumerate(headers):
            date, identity = match.groups()
            dt = timestamp(date)
            if path.stem != date[:7] or identity in ids:
                raise ValueError("Round month mismatch or duplicate ID")
            entry = text[match.start():headers[index + 1].start() if index + 1 < len(headers) else len(text)].strip() + "\n"
            marker = DIGEST.search(entry)
            if not marker or hashlib.sha256(entry[:marker.start()].encode("utf8")).hexdigest() != marker[1]:
                raise ValueError(f"Round integrity mismatch: {identity}; add corrections as a new round")
            ids.add(identity)
            found.append((dt, identity, entry))
    return sorted(found, key=lambda item: (item[0], item[1]))


def recent(name: str, item: dict, history: list) -> str:
    heading = f"# Recent — {item['title']}\n\n> Generated from rounds/. Keep permanent constraints in STATE.md. Do not edit this file.\n\n"
    content = "\n".join(DIGEST.sub("\n", row[2]) for row in reversed(history[-4:]))
    result = heading + (content or "No completed rounds yet. This does not imply implementation or validation.\n")
    if len(result) > RECENT_LIMIT:
        raise ValueError(f"RECENT exceeds {RECENT_LIMIT} characters: {name}")
    return result


def state(root: Path, name: str) -> str:
    result = read(safe_path(root, f"obsidian/Projects/{name}/STATE.md"))
    if not result.strip() or len(result) > STATE_LIMIT:
        raise ValueError(f"STATE must contain 1–{STATE_LIMIT} characters: {name}")
    return result


def check(root: Path) -> None:
    for name, item in projects(root).items():
        current = state(root, name)
        expected = recent(name, item, entries(root, name))
        if read(safe_path(root, f"obsidian/Projects/{name}/RECENT.md")) != expected:
            raise ValueError(f"Stale RECENT for {name}; run refresh")
        if len(current + expected + json.dumps(item, ensure_ascii=False)) > OUTPUT_LIMIT:
            raise ValueError(f"Context output budget exceeded: {name}")


def paragraph(value: dict, key: str, limit: int, optional=False) -> str:
    text = value.get(key, "" if optional else None)
    if not isinstance(text, str) or (not optional and not text.strip()) or len(text) > limit or any(c in text for c in "\r\n\0"):
        raise ValueError(f"Invalid {key}: single paragraph, at most {limit} characters")
    if text.startswith(("#", "<!--")) or "round-digest:" in text:
        raise ValueError("Reserved Markdown structure in round field")
    return text.strip()


def record(root: Path, name: str, value: dict) -> bool:
    if not isinstance(value, dict):
        raise ValueError("Round input must be an object")
    config = projects(root)
    if name not in config:
        raise ValueError("Unknown project")
    identity = paragraph(value, "id", 96)
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]{0,95}", identity):
        raise ValueError("Use a lowercase round ID with digits and hyphens")
    date = paragraph(value, "date", 40)
    timestamp(date)
    summary = paragraph(value, "summary", 750)
    validation = paragraph(value, "validation", 350)
    next_step = paragraph(value, "next", 200, optional=True)
    sources = value.get("sources")
    if not isinstance(sources, list) or not 1 <= len(sources) <= 4 or any(not isinstance(s, str) for s in sources) or len(set(sources)) != len(sources):
        raise ValueError("Specify 1–4 distinct repository source paths")
    lines = []
    for source in sources:
        if not isinstance(source, str) or not re.fullmatch(r"[A-Za-z0-9_./-]+", source):
            raise ValueError("Use ASCII repository paths without Markdown punctuation")
        path = safe_path(root, source)
        if not path.is_file() or path.stat().st_size > 1024 * 1024 or path.suffix != ".md":
            raise ValueError("Round evidence must be a Markdown document up to 1 MiB")
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        lines.append(f"- `{source}` — `sha256:{digest}`")
    text = f"## {date} | {identity}\n\n{summary}\n\n**Validation:** {validation}\n\n**Sources:**\n" + "\n".join(lines) + "\n"
    if next_step:
        text += f"\n**Next:** {next_step}\n"
    text = text.rstrip()  # Digest protects history against accidental edits, not malicious tampering.
    entry = text + f"\n<!-- round-digest: {hashlib.sha256(text.encode('utf8')).hexdigest()} -->\n"
    if len(entry) > 1800:
        raise ValueError("Round entry too large; move details to a source document")
    with locked(root):
        state(root, name)
        history = entries(root, name)
        same = [row for row in history if row[1] == identity]
        if same:
            if same[0][2] != entry:
                raise ValueError("Round ID already exists with different content or evidence; use a new ID")
            atomic_write(base(root, name) / "RECENT.md", recent(name, config[name], history))
            return False
        combined = sorted(history + [(timestamp(date), identity, entry)], key=lambda row: (row[0], row[1]))
        expected = recent(name, config[name], combined)
        path = base(root, name) / "rounds" / f"{date[:7]}.md"
        safe_path(root, path.relative_to(root).as_posix())
        previous = read(path) if path.exists() else "# Completed rounds\n"
        atomic_write(path, previous.rstrip() + "\n\n" + entry)
        atomic_write(base(root, name) / "RECENT.md", expected)
    return True


def search(root: Path, name: str, item: dict, query: str, archive: bool) -> dict:
    terms = query.casefold().split()
    if not terms:
        raise ValueError("Search query is empty")
    files = [base(root, name) / "STATE.md", base(root, name) / "RECENT.md"]
    files += [safe_path(root, source) for source in item["sources"]]
    if archive:
        files += sorted((base(root, name) / "rounds").glob("*.md"))
        if name == "workspace":
            files.append(root / "obsidian/Archive/BOOTSTRAP.md")
    result, budget, omitted = [], 0, False
    for path in dict.fromkeys(files):
        safe_path(root, path.relative_to(root).as_posix())
        text = read(path)
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        for number, line in enumerate(text.splitlines(), 1):
            if all(term in line.casefold() for term in terms):
                match = {"path": path.relative_to(root).as_posix(), "line": number, "sha256": digest,
                         "text": line[:400], "excerpt_truncated": len(line) > 400}
                size = len(json.dumps(match, ensure_ascii=False)) + 3
                if budget + size > OUTPUT_LIMIT - 500:
                    omitted = True
                    break
                result.append(match)
                budget += size
        if omitted:
            break
    return {"project": name, "archive": archive, "matches": result, "more_matches_omitted": omitted}


def main() -> int:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf8")
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for command in ("list", "check", "refresh"):
        commands.add_parser(command)
    for command in ("start", "search", "record"):
        part = commands.add_parser(command)
        part.add_argument("project")
        if command == "search":
            part.add_argument("query")
            part.add_argument("--archive", action="store_true")
        if command == "record":
            part.add_argument("--input", type=Path, required=True)
    args = parser.parse_args()
    try:
        config = projects(ROOT)
        if hasattr(args, "project") and args.project not in config:
            raise ValueError("Unknown project; run list")
        if args.command == "list":
            output = {name: item["title"] for name, item in config.items()}
        elif args.command == "check":
            check(ROOT)
            output = {"ok": True, "projects": len(config)}
        elif args.command == "refresh":
            with locked(ROOT):
                rendered = {name: recent(name, item, entries(ROOT, name)) for name, item in config.items()}
                for name, content in rendered.items():
                    atomic_write(base(ROOT, name) / "RECENT.md", content)
            check(ROOT)
            output = {"ok": True, "refreshed": len(config)}
        elif args.command == "record":
            changed = record(ROOT, args.project, json.loads(read(args.input)))
            output = {"ok": True, "added": changed}
        elif args.command == "search":
            output = search(ROOT, args.project, config[args.project], args.query, args.archive)
        else:
            name = args.project
            expected = recent(name, config[name], entries(ROOT, name))
            if read(base(ROOT, name) / "RECENT.md") != expected:
                raise ValueError("Stale RECENT; run refresh")
            output = {"project": name, "state": state(ROOT, name), "recent": expected, **config[name]}
        rendered = json.dumps(output, ensure_ascii=False, indent=2)
        # JSON quoting has overhead; the documented packet budget applies to decoded text.
        if args.command == "start" and sum(len(str(value)) for value in output.values()) > OUTPUT_LIMIT:
            raise ValueError("Context packet exceeds its output budget")
        print(rendered)
        return 0
    except (ValueError, OSError, TypeError) as error:
        print(f"CONTEXT ERROR: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
