//! Opt-in smoke for a user-owned official executable.
//!
//! Run explicitly with `CMR_ORIGINAL_DIR` set and `--ignored`. The test never
//! logs extracted bytes or strings or retains the caller's source path.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use cmr_assets::{ReviewedStaticTablesProfile, StaticTablesStore, extract_static_tables};
use cmr_formats::pe_static_tables::STATIC_TABLES_FORMAT_VERSION;

const OFFICIAL_PROFILE: &[u8] =
    include_bytes!("../../../profiles/official-eu-static-tables-v1.json");
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "cmr-official-static-tables-smoke-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).expect("fresh smoke root can be created");
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "requires user-owned media selected only by CMR_ORIGINAL_DIR"]
fn reviewed_official_static_tables_extract_and_reopen_without_exposing_values() {
    let Some(source) = std::env::var_os("CMR_ORIGINAL_DIR") else {
        panic!("CMR_ORIGINAL_DIR must select the user-owned source root");
    };
    let reviewed = ReviewedStaticTablesProfile::from_json(OFFICIAL_PROFILE)
        .expect("repository static-tables profile must remain valid");
    let temp = Temp::new();
    let output = temp.0.join("store");

    let result = extract_static_tables(PathBuf::from(source).as_path(), &output, &reviewed);
    assert!(
        result.is_ok(),
        "official static-tables extraction failed: {:?}",
        result.as_ref().err().map(std::mem::discriminant)
    );
    let summary = result.expect("checked successful extraction");
    assert_eq!(summary.shape.format_version, STATIC_TABLES_FORMAT_VERSION);
    assert_eq!(summary.shape.entry_count, 122);

    let store = StaticTablesStore::open(&output, &reviewed);
    assert!(store.is_ok(), "official static-tables store reopen failed");
    let store = store.expect("checked successful reopen");
    let tables = store.tables();
    for (name, len) in [
        ("frontend_setup_layout_championship", 0x50),
        ("frontend_setup_layout_rally", 0x50),
        ("frontend_setup_layout_time_trial", 0x50),
        ("frontend_camera_targets_and_records", 0x128),
        ("frontend_country_table", 0x50),
        ("results_save_schedule", 9),
        ("bonus_car_countries", 8),
        ("frontend_arrow_oscillation", 16),
        ("car_stats", 168),
        ("car_lamp_tables", 0x1730),
    ] {
        assert_eq!(tables.block(name).map(<[u8]>::len), Some(len), "{name}");
    }
    assert_eq!(tables.strings("car_names").map(<[_]>::len), Some(16));
    assert_eq!(tables.strings("stage_names").map(<[_]>::len), Some(54));
    assert!(tables.string("school_stage_name").is_some());
    assert!(tables.string("frontend_arrow_left").is_some());
    assert!(tables.string("frontend_arrow_right").is_some());
    assert_eq!(tables.strings("car_short_names").map(<[_]>::len), Some(14));
    assert_eq!(
        tables.strings("stage_select_labels").map(<[_]>::len),
        Some(7)
    );
    assert!(tables.string("stage_select_row_format").is_some());
    assert!(tables.string("frontend_sound_prefix").is_some());
    assert!(tables.string("frontend_sound_18").is_some());
}
