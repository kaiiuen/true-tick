"""Reject captured timer-resolution values in live Rust code.

Captured values are valid in tests and in the explicitly named legacy config
migration fixture. Handoff timing policy is separate and is not checked here.
"""

from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
FILES = (
    ROOT / "crates/tick-core/src/lib.rs",
    ROOT / "crates/tick-platform-windows/src/lib.rs",
    ROOT / "crates/tick-ownership/src/lib.rs",
    ROOT / "apps/true-tick/src/config.rs",
    ROOT / "apps/true-tick/src/tray.rs",
    ROOT / "apps/true-tick/src/tray_surface.rs",
)
CAPTURED = re.compile(r"(?<![0-9_])(?:5000|5_000|9966|9_966|4966|4_966|156250|156_250|10000|10_000)(?![0-9_])")


def production_lines(text: str):
    for line in text.splitlines():
        if line.strip().startswith("#[cfg(test)]"):
            break
        yield line


def main() -> int:
    violations = []
    for path in FILES:
        for line_number, line in enumerate(production_lines(path.read_text(encoding="utf-8")), 1):
            if "LEGACY_ONE_MILLISECOND_REQUEST_INTERVAL" in line:
                continue
            if "HNS_PER_MILLISECOND" in line:
                continue
            if CAPTURED.search(line):
                violations.append(f"{path.relative_to(ROOT)}:{line_number}: {line.strip()}")
    if violations:
        print("Live timer-resolution values found:")
        print("\n".join(violations))
        return 1
    print("Live timer-resolution value check passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
