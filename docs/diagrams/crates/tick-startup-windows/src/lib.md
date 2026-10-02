# tick-startup-windows lib architecture

Source: `crates/tick-startup-windows/src/lib.rs`

```mermaid
flowchart TD
    ST["startup_operation(enabled)"] --> EN{"enabled?"}
    EN -- "Yes" --> OPR["Register"]
    EN -- "No" --> OPM["Remove"]

    OPR --> RGL["register(path)"]
    OPR --> RGD["register_development(path)"]
    RGL -- "windows" --> VL{"validate_launcher_path"}
    RGL -- "other OS" --> UN1["Unsupported"]
    RGD -- "windows" --> VD{"is_development_executable"}
    RGD -- "other OS" --> UN1
    VL -- "name not Launcher.exe" --> ER1["NotLauncher"]
    VL -- "missing on disk" --> ER2["LauncherMissing"]
    VL -- "not a file" --> ER3["InvalidExecutablePath"]
    VL -- "accepted" --> RV["register_validated(path)"]
    VD -- "name or dir mismatch" --> ER4["NotDevelopmentExecutable"]
    VD -- "true-tick.exe in target/debug" --> VD2{"validate_development_path"}
    VD2 -- "missing on disk" --> ER2
    VD2 -- "not a file" --> ER3
    VD2 -- "accepted" --> RV

    RV --> SC{"startup_command valid?"}
    SC -- "empty path or quote char" --> ER3
    SC -- "wrap path in quotes" --> OK["open_run_key, RegCreateKeyExW HKCU Run"]
    OK -- "raw status" --> ER5["OpenKey raw_status"]
    OK -- "live handle" --> SV["RegSetValueExW TrueTick REG_SZ, overwrite is idempotent"]
    SV -- "raw status" --> ER6["SetValue raw_status"]
    SV -- "success" --> CK["close_run_key"]
    CK -- "raw status" --> ER7["CloseKey raw_status"]
    CK -- "success" --> DONE["Registered, per-user Run value TrueTick"]

    OPM --> RMV["remove()"]
    RMV -- "windows" --> OK2["open_run_key, RegCreateKeyExW HKCU Run"]
    RMV -- "other OS" --> UN1
    OK2 -- "raw status" --> ER5
    OK2 -- "live handle" --> DV["RegDeleteValueW TrueTick"]
    DV -- "success or missing" --> CK2["close_run_key"]
    DV -- "raw status" --> ER8["RemoveValue raw_status"]
    CK2 -- "raw status" --> ER7
    CK2 -- "success" --> DONE2["Value absent, key and other entries untouched"]

    ER1 --> SE["StartupError"]
    ER2 --> SE
    ER3 --> SE
    ER4 --> SE
    ER5 --> SE
    ER6 --> SE
    ER7 --> SE
    ER8 --> SE
```

## Notes

* Scope is the current user HKCU Run key only
* No service, scheduled task, machine-wide value, installer, or elevated state is created
* startup_operation maps enabled true to Register and false to Remove
* register requires the file name Launcher.exe, then checks existence and file type
* register_development requires true-tick.exe plus target and debug path components
* Non Windows targets return Unsupported for register, register_development, and remove
* startup_command wraps the path in double quotes so paths with spaces still launch
* Empty paths and paths containing a double quote return InvalidExecutablePath
* RegSetValueExW writes a REG_SZ value named TrueTick, an existing value is overwritten so the set is idempotent
* remove treats ERROR_SUCCESS and ERROR_FILE_NOT_FOUND alike, so a missing value counts as success
* Rollback is non destructive, only the TrueTick value is removed and the Run key and other entries stay
* Raw Win32 status codes are captured in OpenKey, SetValue, RemoveValue, and CloseKey variants, and tests exercise pure helpers only
