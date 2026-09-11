# True™ Tick

True™ Tick is an internal v1 tray-only Windows application. It is not a
production release and is not authorized for publication.

The v1 uses a narrow native timer adapter with raw status preservation, a
serialized owned-request lifecycle, conservative power policy, a native tray
surface, explicit local configuration, and a bounded portable A/B launcher
scaffold. It does not include profiles, process detection, True™ Time, NTP,
an installer, a secure updater, or release signing.

Unsupported platform behavior remains explicit. Native API acceptance and the
adapter postcondition are not claims about a universal effective system value.

See [`ARCHITECTURE.md`](ARCHITECTURE.md) and [`STATUS.md`](STATUS.md).
