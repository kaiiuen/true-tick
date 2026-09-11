use std::fs;
use std::path::{Path, PathBuf};

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
        None => Slot::A,
    };
    let fallback = match preferred {
        Slot::A => Slot::B,
        Slot::B => Slot::A,
    };
    for slot in [preferred, fallback] {
        let path = slots.join(slot.name());
        if path.is_dir() && path.join("true-tick.exe").is_file() {
            return Selection::Selected { slot, path };
        }
    }
    Selection::RepairRequired("neither slot contains a recognizable internal executable".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn invalid_active_slot_requires_repair() {
        let root = std::env::temp_dir().join(format!(
            "true-tick-ab-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("active-slot.txt"), "C\n").unwrap();
        assert!(matches!(select(&root), Selection::RepairRequired(_)));
        fs::remove_dir_all(root).unwrap();
    }
}
