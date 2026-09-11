"""Reject em dash and semicolon characters in active documentation."""
from pathlib import Path
import sys


def files_under(root: Path):
    for path in root.rglob("*"):
        if path.is_file() and path.suffix.lower() in {".md", ".markdown", ".txt"}:
            if "archive" not in path.parts:
                yield path


def main() -> int:
    arguments = [argument for argument in sys.argv[1:] if argument != "--fix"]
    fix = "--fix" in sys.argv[1:]
    violations = []
    for argument in arguments:
        root = Path(argument)
        paths = [root] if root.is_file() else files_under(root)
        for path in paths:
            text = path.read_text(encoding="utf-8")
            if fix:
                text = text.replace("—", "-").replace(";", ",")
                path.write_text(text, encoding="utf-8")
            for number, line in enumerate(text.splitlines(), 1):
                if "—" in line or ";" in line:
                    violations.append((path, number, line))
    for path, number, line in violations:
        print(f"{path}:{number}: {line}")
    if violations:
        print(f"Found {len(violations)} punctuation violations")
        return 1
    print("Documentation punctuation check passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
