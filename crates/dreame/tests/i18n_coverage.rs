//! Every text the library shows to people has a Brazilian Portuguese translation: the enum
//! and part labels, the room type names, and each literal passed to `tr`, `trf` or `trn` in
//! the library crates.

use dreame_i18n::{literal_keys, lookup, Language};
use dreame_model::codec::consumables::Consumable;
use dreame_model::enums::*;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn labels<T: CodeEnum>(out: &mut BTreeSet<String>) {
    out.extend(T::ALL.iter().map(|v| v.label().to_string()));
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// The keys the library needs, from code.
fn needed() -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    macro_rules! all { ($($t:ty),* $(,)?) => { $(labels::<$t>(&mut keys);)* } }
    all!(
        State, VacuumStatus, TaskStatus, TaskType, ErrorCode, ChargingStatus, SuctionLevel, CleaningMode,
        WaterTank, WaterVolume, MopPadHumidity, CarpetSensitivity, CarpetCleaning, RelocationStatus,
        DustCollection, AutoEmptyStatus, SelfWashBaseStatus, MopWashLevel, MoppingType, CleaningRoute,
        SelfCleanFrequency, MopExtendFrequency, AutoEmptyMode, AutoEmptyModeV2, CleanGenius, CleanGeniusMode, SecondCleaning,
        FloorMaterial, LowWaterWarning, DrainageStatus, MapRecoveryStatus, MapBackupStatus,
        CleanWaterTankStatus, DirtyWaterTankStatus, DustBagStatus, DetergentStatus, HotWaterStatus,
        StationDrainageStatus, DustBagDryingStatus, WaterTemperature, CleanLogStatus,
    );
    keys.extend(Consumable::ALL.iter().map(|c| c.label().to_string()));
    keys.extend(dreame_model::codec::ai_obstacle::AiFlags::NAMES.iter().map(|(_, n)| n.to_string()));
    keys.extend(dreame_map::model::ROOM_TYPE_NAMES.iter().map(|s| s.to_string()));
    // `history::outcome`, translated by the apps.
    for o in ["completed", "ended by hand", "interrupted", "failed", "ended"] {
        keys.insert(o.to_string());
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for c in ["dreame/src", "dreame-map/src", "dreame-ffi/src"] {
        rust_files(&root.join(c), &mut files);
    }
    for f in files {
        keys.extend(literal_keys(&std::fs::read_to_string(f).unwrap()));
    }
    keys
}

fn missing() -> Vec<String> {
    needed().into_iter().filter(|k| lookup(Language::PortugueseBrazil, k).is_none()).collect()
}

#[test]
fn library_text_has_brazilian_portuguese() {
    let missing = missing();
    assert!(missing.is_empty(), "{} keys lack pt-BR in crates/dreame-i18n/locales/pt-BR.toml:\n{}", missing.len(), missing.join("\n"));
}

/// Prints the missing keys as catalog lines with empty values, for a translator to fill:
/// `cargo test -p dreame --test i18n_coverage -- --ignored print_missing --nocapture`.
#[test]
#[ignore]
fn print_missing() {
    for k in missing() {
        println!("{k:?} = \"\"");
    }
}
