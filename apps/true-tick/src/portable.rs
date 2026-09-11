use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LauncherPathError {
    NotAnAbSlotExecutable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Slot {
    A,
    B,
}

impl Slot {
    fn name(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
        }
    }
}

#[derive(Debug)]
pub enum Selection {
    Selected { slot: Slot, path: PathBuf },
    RepairRequired(String),
}

pub fn portable_root_from_slot_executable(executable: &Path) -> Result<PathBuf, LauncherPathError> {
    let slot = executable
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str());
    if !matches!(slot, Some("A" | "B")) {
        return Err(LauncherPathError::NotAnAbSlotExecutable);
    }
    executable
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or(LauncherPathError::NotAnAbSlotExecutable)
}

pub fn launcher_path_from_slot_executable(executable: &Path) -> Result<PathBuf, LauncherPathError> {
    Ok(portable_root_from_slot_executable(executable)?.join("Launcher.exe"))
}

pub fn select(root: &Path) -> Selection {
    let slots = root.join("Slots");
    let active_file = root.join("active-slot.txt");
    let active = fs::read_to_string(&active_file)
        .ok()
        .map(|value| value.trim().to_owned());
    let preferred = match active.as_deref() {
        Some("A") => Slot::A,
        Some("B") => Slot::B,
        Some(_) => return Selection::RepairRequired("active-slot.txt is not A or B".into()),
        None => {
            return Selection::RepairRequired(
                "active-slot.txt is missing, explicit initialization is required".into(),
            )
        }
    };
    let path = slots.join(preferred.name());
    if path.is_dir() && path.join("true-tick.exe").is_file() {
        return Selection::Selected {
            slot: preferred,
            path,
        };
    }
    Selection::RepairRequired("active slot lacks a recognizable internal executable".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn launcher_path_uses_portable_root_not_slot_payload() {
        let executable = PathBuf::from("portable-root")
            .join("Slots")
            .join("B")
            .join("true-tick.exe");
        assert_eq!(
            launcher_path_from_slot_executable(&executable).unwrap(),
            PathBuf::from("portable-root").join("Launcher.exe")
        );
    }

    #[test]
    fn launcher_path_rejects_unbounded_executable_shape() {
        assert_eq!(
            launcher_path_from_slot_executable(
                &PathBuf::from("portable-root").join("true-tick.exe"),
            ),
            Err(LauncherPathError::NotAnAbSlotExecutable)
        );
    }

    #[test]
    fn invalid_or_missing_active_slot_requires_repair() {
        let root = std::env::temp_dir().join(format!(
            "true-tick-ab-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        assert!(matches!(select(&root), Selection::RepairRequired(_)));
        fs::write(root.join("active-slot.txt"), "C\n").unwrap();
        assert!(matches!(select(&root), Selection::RepairRequired(_)));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn portable_root_matches_launcher_root() {
        let executable = PathBuf::from("portable-root")
            .join("Slots")
            .join("A")
            .join("true-tick.exe");
        assert_eq!(
            portable_root_from_slot_executable(&executable).unwrap(),
            PathBuf::from("portable-root")
        );
    }
}
