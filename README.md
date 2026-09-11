# True™ Tick

True™ Tick is an internal v1 tray-only Windows application. It is not a
production release and is not authorized for publication.

The v1 uses a narrow native timer adapter with raw status preservation, a
serialized owned-request lifecycle, conservative power policy, a native tray
surface, explicit local configuration, per-user boot startup registration, and
a bounded portable A/B launcher scaffold. Per-user boot registration targets the
portable `Launcher.exe` entry point, never a slot payload. The launcher remains
responsible for selecting and validating A/B before activation. Boot registration
is opt-in through local config and is distinct from automatic timer activation
after launch. The current path resolver only derives `Launcher.exe` from a `Slots\A` or
`Slots\B` executable shape. It does not claim runtime path or file validation. It
does not include profiles, process detection, True™ Time, NTP, an installer, a
secure updater, or release signing.

Unsupported platform behavior remains explicit. Native API acceptance and the
adapter postcondition are not claims about a universal effective system value.

Active documentation is checked with `scripts/check_doc_punctuation.py`. The
checker rejects em dash and semicolon characters and skips historical archive
material.

See [`ARCHITECTURE.md`](ARCHITECTURE.md) and [`STATUS.md`](STATUS.md).
