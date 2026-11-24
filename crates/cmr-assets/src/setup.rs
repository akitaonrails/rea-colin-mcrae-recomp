//! One-step setup that runs the full media import and all 11 executable-backed
//! table extracts into a single `runtime-data/` directory.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{
    ReviewedAuxControlProfile, ReviewedCarSelectorProfile, ReviewedCompiledInitializerProfile,
    ReviewedEngineAudioTablesProfile, ReviewedProfile, ReviewedRivalRecordsProfile,
    ReviewedStageWeatherTablesProfile, ReviewedStaticTablesProfile,
    ReviewedSurfaceAudioTablesProfile, ReviewedSurfaceCapabilityProfile, ReviewedSurfaceProfile,
    ReviewedTyreTablesProfile, extract_aux_control_constants, extract_car_selectors,
    extract_compiled_car_initializer, extract_engine_audio_tables, extract_rival_records,
    extract_stage_weather_tables, extract_static_tables, extract_surface_audio_tables,
    extract_surface_capabilities, extract_surface_tables, extract_tyre_surface_tables,
    import_source,
};

/// File inside `runtime-data/` that remembers the user's original game folder
/// so subsequent double-click launches don't have to ask again.
pub const SAVED_SOURCE_FILE: &str = "source-dir.txt";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StepKind {
    MediaImport,
    SurfaceTables,
    SurfaceCapabilities,
    CompiledCarInitializer,
    CarSelectors,
    TyreSurfaceTables,
    EngineAudioTables,
    SurfaceAudioTables,
    StageWeatherTables,
    RivalRecords,
    StaticTables,
    AuxControlConstants,
}

#[derive(Clone, Copy, Debug)]
struct SetupStep {
    label: &'static str,
    profile_file: &'static str,
    output_subdir: &'static str,
    kind: StepKind,
}

const SETUP_STEPS: &[SetupStep] = &[
    SetupStep {
        label: "Game media (stages, cars, menus, audio, fonts)",
        profile_file: "installed-eu-derived-assets-v2.json",
        output_subdir: "installed-eu-derived-assets-v2",
        kind: StepKind::MediaImport,
    },
    SetupStep {
        label: "Surface physics tables",
        profile_file: "official-eu-surface-tables-v1.json",
        output_subdir: "official-eu-surface-coefficients",
        kind: StepKind::SurfaceTables,
    },
    SetupStep {
        label: "Surface capability tables",
        profile_file: "official-eu-surface-capabilities-v1.json",
        output_subdir: "official-eu-surface-capabilities",
        kind: StepKind::SurfaceCapabilities,
    },
    SetupStep {
        label: "Compiled car initializer tables",
        profile_file: "official-eu-compiled-car-initializer-v1.json",
        output_subdir: "official-eu-compiled-car-initializer",
        kind: StepKind::CompiledCarInitializer,
    },
    SetupStep {
        label: "Car selector map",
        profile_file: "official-eu-car-selector-map-v1.json",
        output_subdir: "official-eu-car-selector-map",
        kind: StepKind::CarSelectors,
    },
    SetupStep {
        label: "Tyre surface tables",
        profile_file: "official-eu-tyre-surface-tables-v1.json",
        output_subdir: "official-eu-tyre-surface-tables",
        kind: StepKind::TyreSurfaceTables,
    },
    SetupStep {
        label: "Engine audio tables",
        profile_file: "official-eu-engine-audio-tables-v1.json",
        output_subdir: "official-eu-engine-audio-tables",
        kind: StepKind::EngineAudioTables,
    },
    SetupStep {
        label: "Surface audio tables",
        profile_file: "official-eu-surface-audio-tables-v1.json",
        output_subdir: "official-eu-surface-audio-tables",
        kind: StepKind::SurfaceAudioTables,
    },
    SetupStep {
        label: "Stage weather tables",
        profile_file: "official-eu-stage-weather-tables-v1.json",
        output_subdir: "official-eu-stage-weather-tables",
        kind: StepKind::StageWeatherTables,
    },
    SetupStep {
        label: "Rival records",
        profile_file: "official-eu-rival-records-v1.json",
        output_subdir: "official-eu-rival-records",
        kind: StepKind::RivalRecords,
    },
    SetupStep {
        label: "Static lookup tables",
        profile_file: "official-eu-static-tables-v1.json",
        output_subdir: "official-eu-static-tables",
        kind: StepKind::StaticTables,
    },
    SetupStep {
        label: "Auxiliary control constants",
        profile_file: "official-eu-aux-control-constants-v1.json",
        output_subdir: "official-eu-aux-control-constants",
        kind: StepKind::AuxControlConstants,
    },
];

/// Summary returned by [`setup_all`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SetupSummary {
    pub total_steps: usize,
    pub imported_steps: usize,
    pub skipped_steps: usize,
    pub output_dir: String,
    pub source_dir: String,
}

fn dir_has_entries(path: &Path) -> bool {
    fs::read_dir(path)
        .ok()
        .and_then(|mut it| it.next())
        .is_some()
}

/// Returns `true` when all 12 imported asset/table directories exist and are
/// non-empty inside `output_dir`.
pub fn is_runtime_data_ready(output_dir: &Path) -> bool {
    SETUP_STEPS
        .iter()
        .all(|step| dir_has_entries(&output_dir.join(step.output_subdir)))
}

/// Reads the remembered original game folder from `output_dir/source-dir.txt`,
/// returning `Some(path)` only if that path still exists as a directory.
pub fn read_saved_source(output_dir: &Path) -> Option<PathBuf> {
    let raw = fs::read_to_string(output_dir.join(SAVED_SOURCE_FILE)).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = PathBuf::from(trimmed);
    path.is_dir().then_some(path)
}

/// Writes the original game folder path to `output_dir/source-dir.txt`.
pub fn write_saved_source(output_dir: &Path, source: &Path) -> Result<(), String> {
    fs::create_dir_all(output_dir)
        .map_err(|error| format!("create {}: {error}", output_dir.display()))?;
    let canonical = fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
    let text = format!("{}\n", canonical.display());
    fs::write(output_dir.join(SAVED_SOURCE_FILE), text)
        .map_err(|error| format!("write {SAVED_SOURCE_FILE}: {error}"))
}

/// Searches standard locations for the `profiles/` directory containing the
/// reviewed JSON profiles.
pub fn find_profiles_dir(exe_path: Option<&Path>) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(appdir) = std::env::var_os("APPDIR") {
        candidates.push(PathBuf::from(appdir).join("usr/share/cmr/profiles"));
    }
    if let Some(exe) = exe_path
        && let Some(exe_dir) = exe.parent()
    {
        candidates.push(exe_dir.join("profiles"));
        candidates.push(exe_dir.join("../share/cmr/profiles"));
        candidates.push(exe_dir.join("../../profiles"));
    }
    candidates.push(PathBuf::from("profiles"));

    candidates.into_iter().find(|dir| {
        dir.join("installed-eu-derived-assets-v2.json").is_file()
            && dir.join("official-eu-surface-tables-v1.json").is_file()
    })
}

/// Runs all 12 import/extraction steps from `source` using profiles in
/// `profiles_dir`, writing into `output_dir`.
///
/// Steps whose output directory is already populated are skipped unless `force`
/// is `true` (in which case each leaf directory is cleared before re-importing).
pub fn setup_all<F>(
    source: &Path,
    profiles_dir: &Path,
    output_dir: &Path,
    force: bool,
    mut on_step: F,
) -> Result<SetupSummary, String>
where
    F: FnMut(usize, usize, &str),
{
    if !source.is_dir() {
        return Err(format!(
            "game folder does not exist or is not a directory: {}",
            source.display()
        ));
    }
    fs::create_dir_all(output_dir)
        .map_err(|error| format!("cannot create {}: {error}", output_dir.display()))?;

    let total_steps = SETUP_STEPS.len();
    let mut imported_steps = 0usize;
    let mut skipped_steps = 0usize;

    for (index, step) in SETUP_STEPS.iter().enumerate() {
        let step_out = output_dir.join(step.output_subdir);
        if !force && dir_has_entries(&step_out) {
            on_step(
                index + 1,
                total_steps,
                &format!("{} (already imported)", step.label),
            );
            skipped_steps += 1;
            continue;
        }
        if step_out.exists() {
            fs::remove_dir_all(&step_out).map_err(|error| {
                format!(
                    "cannot clear previous output {}: {error}",
                    step_out.display()
                )
            })?;
        }

        on_step(index + 1, total_steps, step.label);
        let profile_path = profiles_dir.join(step.profile_file);
        let document = fs::read(&profile_path)
            .map_err(|error| format!("cannot read profile {}: {error}", profile_path.display()))?;

        match step.kind {
            StepKind::MediaImport => {
                let reviewed = ReviewedProfile::from_json(&document)
                    .map_err(|error| format!("{}: {error}", profile_path.display()))?;
                import_source(source, &step_out, &reviewed)
                    .map_err(|error| format!("{}: {error}", step.label))?;
            }
            StepKind::SurfaceTables => {
                let reviewed = ReviewedSurfaceProfile::from_json(&document)
                    .map_err(|error| format!("{}: {error}", profile_path.display()))?;
                extract_surface_tables(source, &step_out, &reviewed)
                    .map_err(|error| format!("{}: {error}", step.label))?;
            }
            StepKind::SurfaceCapabilities => {
                let reviewed = ReviewedSurfaceCapabilityProfile::from_json(&document)
                    .map_err(|error| format!("{}: {error}", profile_path.display()))?;
                extract_surface_capabilities(source, &step_out, &reviewed)
                    .map_err(|error| format!("{}: {error}", step.label))?;
            }
            StepKind::CompiledCarInitializer => {
                let reviewed = ReviewedCompiledInitializerProfile::from_json(&document)
                    .map_err(|error| format!("{}: {error}", profile_path.display()))?;
                extract_compiled_car_initializer(source, &step_out, &reviewed)
                    .map_err(|error| format!("{}: {error}", step.label))?;
            }
            StepKind::CarSelectors => {
                let reviewed = ReviewedCarSelectorProfile::from_json(&document)
                    .map_err(|error| format!("{}: {error}", profile_path.display()))?;
                extract_car_selectors(source, &step_out, &reviewed)
                    .map_err(|error| format!("{}: {error}", step.label))?;
            }
            StepKind::TyreSurfaceTables => {
                let reviewed = ReviewedTyreTablesProfile::from_json(&document)
                    .map_err(|error| format!("{}: {error}", profile_path.display()))?;
                extract_tyre_surface_tables(source, &step_out, &reviewed)
                    .map_err(|error| format!("{}: {error}", step.label))?;
            }
            StepKind::EngineAudioTables => {
                let reviewed = ReviewedEngineAudioTablesProfile::from_json(&document)
                    .map_err(|error| format!("{}: {error}", profile_path.display()))?;
                extract_engine_audio_tables(source, &step_out, &reviewed)
                    .map_err(|error| format!("{}: {error}", step.label))?;
            }
            StepKind::SurfaceAudioTables => {
                let reviewed = ReviewedSurfaceAudioTablesProfile::from_json(&document)
                    .map_err(|error| format!("{}: {error}", profile_path.display()))?;
                extract_surface_audio_tables(source, &step_out, &reviewed)
                    .map_err(|error| format!("{}: {error}", step.label))?;
            }
            StepKind::StageWeatherTables => {
                let reviewed = ReviewedStageWeatherTablesProfile::from_json(&document)
                    .map_err(|error| format!("{}: {error}", profile_path.display()))?;
                extract_stage_weather_tables(source, &step_out, &reviewed)
                    .map_err(|error| format!("{}: {error}", step.label))?;
            }
            StepKind::RivalRecords => {
                let reviewed = ReviewedRivalRecordsProfile::from_json(&document)
                    .map_err(|error| format!("{}: {error}", profile_path.display()))?;
                extract_rival_records(source, &step_out, &reviewed)
                    .map_err(|error| format!("{}: {error}", step.label))?;
            }
            StepKind::StaticTables => {
                let reviewed = ReviewedStaticTablesProfile::from_json(&document)
                    .map_err(|error| format!("{}: {error}", profile_path.display()))?;
                extract_static_tables(source, &step_out, &reviewed)
                    .map_err(|error| format!("{}: {error}", step.label))?;
            }
            StepKind::AuxControlConstants => {
                let reviewed = ReviewedAuxControlProfile::from_json(&document)
                    .map_err(|error| format!("{}: {error}", profile_path.display()))?;
                extract_aux_control_constants(source, &step_out, &reviewed)
                    .map_err(|error| format!("{}: {error}", step.label))?;
            }
        }
        imported_steps += 1;
    }

    let _ = write_saved_source(output_dir, source);

    Ok(SetupSummary {
        total_steps,
        imported_steps,
        skipped_steps,
        output_dir: output_dir.display().to_string(),
        source_dir: source.display().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_steps_cover_all_twelve_profiles() {
        assert_eq!(SETUP_STEPS.len(), 12);
    }

    #[test]
    fn readiness_and_saved_source_round_trip() {
        let dir =
            std::env::temp_dir().join(format!("cmr-assets-setup-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let source = dir.join("game");
        let output = dir.join("runtime-data");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&output).unwrap();

        assert!(!is_runtime_data_ready(&output));
        for step in SETUP_STEPS {
            let sub = output.join(step.output_subdir);
            fs::create_dir_all(&sub).unwrap();
            fs::write(sub.join("marker.json"), b"{}").unwrap();
        }
        assert!(is_runtime_data_ready(&output));

        assert_eq!(read_saved_source(&output), None);
        write_saved_source(&output, &source).unwrap();
        assert!(read_saved_source(&output).is_some());

        let _ = fs::remove_dir_all(&dir);
    }
}
