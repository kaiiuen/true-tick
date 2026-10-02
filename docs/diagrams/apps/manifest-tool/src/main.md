# manifest-tool main.rs

Source: `true-tick/apps/manifest-tool/src/main.rs`

```mermaid
flowchart TD
    A["main: collect argv minus program name"] --> B["run: split_first"]
    B -->|"no subcommand"| U1["Usage error"] --> E1["stderr message + exit 1"]
    B --> C{"subcommand dispatch"}

    C -->|"gen-key"| G1["take_value --out-dir"]
    G1 --> G2["has_flag --force"]
    G2 --> G3["create_dir_all out-dir"]
    G3 --> G4{"not force and anchor or secret exists?"}
    G4 -->|"yes"| GE["Exists error"] --> E1
    G4 -->|"no"| G5["generate_keypair Ed25519"]
    G5 --> G6["write_raw_key_file trust-anchor.pub: 32 raw bytes, no newline"]
    G6 --> G7["write_key_file signing-key.sec: lowercase hex plus newline"]
    G7 --> G8["restrict_secret_permissions"]
    G8 --> OK["stdout message + exit 0"]

    C -->|"sign"| S1["take_value --package-dir, --key, --counter"]
    S1 --> S2["parse counter as u64"]
    S2 -->|"invalid"| U1
    S2 --> S3["take_optional_value --out, else package-dir/manifest.sig"]
    S3 --> S4["load_secret_key: raw bytes or hex text"]
    S4 --> S5["build_entries package-dir and out path"]
    S5 --> S6["sign_manifest Ed25519 over counter and entries"]
    S6 --> S7["atomic_write manifest.sig via tmp file and rename"]
    S7 --> S8["render_sha256sums then atomic_write SHA256SUMS.txt"]
    S8 --> OK

    C -->|"verify"| V1["take_value --package-dir, --anchor"]
    V1 --> V2["load_anchor: exactly 32 raw bytes"]
    V2 -->|"wrong length"| V3["InvalidAnchorLength"] --> E1
    V2 --> V4["read package-dir/manifest.sig and parse_signed_manifest"]
    V4 -->|"malformed"| V5["Crypto error"] --> E1
    V4 --> V6["verify_manifest against anchor"]
    V6 --> V7["per entry: read_bounded then verify_entry_digest"]
    V7 --> V8{"failures empty?"}
    V8 -->|"yes"| OK
    V8 -->|"no"| V9["VerifyFailed list"] --> E1

    C -->|"unknown"| U1

    subgraph PARSE["argument parsing helpers"]
        P1["take_value: first flag match, next arg must not start with double dash"] --> P2["missing flag or value gives Usage"]
        P3["take_optional_value: absent flag gives None, else same value rules"]
        P4["has_flag: true when any arg equals flag"]
    end

    subgraph ENTRIES["build_entries canonical collection"]
        B1["collect_files: read_dir, sort children, recurse directories"] --> B2["retain paths where not is_excluded"]
        B2 --> B3["is_excluded drops manifest.sig, SHA256SUMS.txt, any .toml, and the out path itself"]
        B3 --> B4{"file count over MAX_MANIFEST_ENTRIES?"}
        B4 -->|"yes"| B5["TooManyEntries"] --> E1
        B4 -->|"no"| B6["per file: backslashes to slashes, reject empty, whitespace, control chars"]
        B6 --> B7{"entry overhead plus path over MAX_MANIFEST_LINE_BYTES?"}
        B7 -->|"yes"| B8["LineTooLong"] --> E1
        B7 -->|"no"| B9["hash_file: streaming SHA-256, reject over MAX_ENTRY_BYTES"]
        B9 --> B10["ManifestEntry path plus sha256, order follows sorted traversal"]
    end
```

## Notes

- Single binary with three subcommands dispatched by `run`, namely `gen-key`, `sign`, and `verify`, plus an unknown subcommand usage error.
- `gen-key` requires `--out-dir`, accepts `--force`, and refuses to overwrite an existing `trust-anchor.pub` or `signing-key.sec` without it.
- The trust anchor is written through `write_raw_key_file` as exactly `PUBLIC_KEY_LEN` raw bytes with no encoding and no trailing newline.
- The secret key is written as lowercase hex text plus a single newline, then permissions are restricted through `icacls` on Windows or mode `0o600` on unix.
- `load_key_bytes` accepts either raw key bytes or lowercase hex text, while `load_anchor` accepts only a file of exactly 32 raw bytes and rejects any other length.
- `build_entries` collects files with a recursive `collect_files` walk that sorts each directory's children before recursion, so entry order follows sorted traversal.
- Exclusion covers `manifest.sig`, `SHA256SUMS.txt`, every file whose name ends in `.toml` as mutable runtime configuration, and the resolved output path.
- Entry paths are normalized from backslashes to forward slashes, then validated to reject empty paths and any whitespace or control characters.
- Hashing is a streaming `Sha256` over a 64 KiB buffer, and any file above `MAX_ENTRY_BYTES` of 512 MiB is rejected before reading.
- `sign` loads the secret key, builds canonical entries, signs with the release counter through `sign_manifest`, and writes both `manifest.sig` and a `SHA256SUMS.txt` of `hex  path` lines with two space separators.
- Writes are atomic through `atomic_write`, which writes a sibling `tmp` file keyed by process id then renames over the target, falling back to remove and rename.
- `verify` reads `package-dir/manifest.sig`, checks the manifest signature against the anchor, then rehashes every listed entry and reports a `VerifyFailed` aggregate listing the signature and each mismatching path.
