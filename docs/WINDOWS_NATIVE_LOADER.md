# Windows Native Loader and Manifest Specification

## 1. Background and Failure Diagnosis
Early internal builds of `true-tick.exe` crashed immediately upon execution on Windows 10 LTSC with native exception `STATUS_ORDINAL_NOT_FOUND` (0xC0000138).

Binary inspection of the Portable Executable (PE) export/import tables revealed:
- `true-tick.exe` imported ordinal 345 from `COMCTL32.dll` (`TaskDialogIndirect`).
- The application binary lacked an embedded Windows application manifest.
- By default, modern Windows assigns unmanifested executables to Common Controls version 5.82 for legacy compatibility.
- Common Controls version 5.82 does not export ordinal 345, causing the Windows NT kernel loader to terminate the process before `main` could execute.

## 2. Embedded Manifest Architecture
To resolve the loader failure, `apps/true-tick/build.rs` was implemented to link and embed an official application manifest directly into the compiled PE resource section:

```rust
// apps/true-tick/build.rs snippet
fn set_manifest_link_args() {
    let embed = "cargo:rustc-link-arg-bin:true-tick=/MANIFEST:EMBED"
    let input = "cargo:rustc-link-arg-bin:true-tick=/MANIFESTINPUT:app.manifest"
    println!("{embed}")
    println!("{input}")
}
```

The embedded manifest explicitly declares:
1. **Common Controls Version 6 Activation**:
   ```xml
   <dependency>
     <dependentAssembly>
       <assemblyIdentity
         type="win32"
         name="Microsoft.Windows.Common-Controls"
         version="6.0.0.0"
         processorArchitecture="*"
         publicKeyToken="6595b64144ccf1df"
         language="*" />
     </dependentAssembly>
   </dependency>
   ```
2. **OS Compatibility GUIDs**: Explicit compatibility tags for Windows 10 and Windows 11.
3. **Execution Level**: `asInvoker` execution level without requesting administrative elevation or UI access privileges.

## 3. Runtime Common Controls Initialization
In addition to the embedded manifest, `apps/true-tick/src/tray/controller.rs` performs explicit runtime initialization before any window or control is created:

```rust
let init = InitCommonControlsEx {
    size: std::mem::size_of::<InitCommonControlsEx>() as u32,
    classes: ICC_LISTVIEW_CLASSES | ICC_BAR_CLASSES,
}
InitCommonControlsEx(&init)
```
This guarantees that `SysListView32` (the diagnostic log grid) and tooltip control classes are registered and available before UI instantiation.
