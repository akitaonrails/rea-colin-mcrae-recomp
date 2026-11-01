use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cmr_assets::{
    ReviewedAuxControlProfile, ReviewedCarSelectorProfile, ReviewedCompiledInitializerProfile,
    ReviewedEngineAudioTablesProfile, ReviewedProfile, ReviewedRivalRecordsProfile,
    ReviewedStageWeatherTablesProfile, ReviewedStaticTablesProfile,
    ReviewedSurfaceAudioTablesProfile, ReviewedSurfaceCapabilityProfile, ReviewedSurfaceProfile,
    ReviewedTyreTablesProfile, catalog_standard_source, extract_aux_control_constants,
    extract_car_selectors, extract_compiled_car_initializer, extract_engine_audio_tables,
    extract_rival_records, extract_stage_weather_tables, extract_static_tables,
    extract_surface_audio_tables, extract_surface_capabilities, extract_surface_tables,
    extract_tyre_surface_tables, import_source, verify_source,
};

fn main() -> ExitCode {
    match run(std::env::args_os().skip(1)) {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("cmr-assets: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: impl Iterator<Item = OsString>) -> Result<String, String> {
    let mut args = args;
    let command = args.next().ok_or_else(usage)?;
    if command == "--version" {
        if args.next().is_some() {
            return Err(usage());
        }
        return Ok(format!("cmr-assets {}", env!("CARGO_PKG_VERSION")));
    }
    let options = parse_options(args)?;
    match command.to_str() {
        Some("catalog") => {
            let source = required_path(&options, "--source")?;
            let id = required_text(&options, "--id")?;
            let output = required_path(&options, "--output-profile")?;
            reject_unknown(&options, &["--source", "--id", "--output-profile"])?;
            let profile =
                catalog_standard_source(&source, id).map_err(|error| error.to_string())?;
            let mut document =
                serde_json::to_vec_pretty(&profile).map_err(|error| error.to_string())?;
            document.push(b'\n');
            write_new(&output, &document)?;
            Ok(format!(
                "profile={} files={} output={}",
                profile.id,
                profile.files.len(),
                output.display()
            ))
        }
        Some("verify") => {
            let source = required_path(&options, "--source")?;
            let profile_path = required_path(&options, "--profile")?;
            reject_unknown(&options, &["--source", "--profile"])?;
            let reviewed = load_profile(&profile_path)?;
            let summary =
                verify_source(&source, reviewed.profile()).map_err(|error| error.to_string())?;
            serde_json::to_string(&summary).map_err(|error| error.to_string())
        }
        Some("import") => {
            let source = required_path(&options, "--source")?;
            let profile_path = required_path(&options, "--profile")?;
            let output = required_path(&options, "--output")?;
            reject_unknown(&options, &["--source", "--profile", "--output"])?;
            let reviewed = load_profile(&profile_path)?;
            let summary =
                import_source(&source, &output, &reviewed).map_err(|error| error.to_string())?;
            serde_json::to_string(&summary).map_err(|error| error.to_string())
        }
        Some("extract-surface-tables") => {
            let source = required_path(&options, "--source")?;
            let profile_path = required_path(&options, "--profile")?;
            let output = required_path(&options, "--output")?;
            reject_unknown(&options, &["--source", "--profile", "--output"])?;
            let reviewed = load_surface_profile(&profile_path)?;
            let summary = extract_surface_tables(&source, &output, &reviewed)
                .map_err(|error| error.to_string())?;
            serde_json::to_string(&summary).map_err(|error| error.to_string())
        }
        Some("extract-surface-capabilities") => {
            let source = required_path(&options, "--source")?;
            let profile_path = required_path(&options, "--profile")?;
            let output = required_path(&options, "--output")?;
            reject_unknown(&options, &["--source", "--profile", "--output"])?;
            let reviewed = load_surface_capability_profile(&profile_path)?;
            let summary = extract_surface_capabilities(&source, &output, &reviewed)
                .map_err(|error| error.to_string())?;
            serde_json::to_string(&summary).map_err(|error| error.to_string())
        }
        Some("extract-compiled-car-initializer") => {
            let source = required_path(&options, "--source")?;
            let profile_path = required_path(&options, "--profile")?;
            let output = required_path(&options, "--output")?;
            reject_unknown(&options, &["--source", "--profile", "--output"])?;
            let reviewed = load_compiled_initializer_profile(&profile_path)?;
            let summary = extract_compiled_car_initializer(&source, &output, &reviewed)
                .map_err(|error| error.to_string())?;
            serde_json::to_string(&summary).map_err(|error| error.to_string())
        }
        Some("extract-car-selectors") => {
            let source = required_path(&options, "--source")?;
            let profile_path = required_path(&options, "--profile")?;
            let output = required_path(&options, "--output")?;
            reject_unknown(&options, &["--source", "--profile", "--output"])?;
            let reviewed = load_car_selector_profile(&profile_path)?;
            let summary = extract_car_selectors(&source, &output, &reviewed)
                .map_err(|error| error.to_string())?;
            serde_json::to_string(&summary).map_err(|error| error.to_string())
        }
        Some("extract-tyre-surface-tables") => {
            let source = required_path(&options, "--source")?;
            let profile_path = required_path(&options, "--profile")?;
            let output = required_path(&options, "--output")?;
            reject_unknown(&options, &["--source", "--profile", "--output"])?;
            let reviewed = load_tyre_tables_profile(&profile_path)?;
            let summary = extract_tyre_surface_tables(&source, &output, &reviewed)
                .map_err(|error| error.to_string())?;
            serde_json::to_string(&summary).map_err(|error| error.to_string())
        }
        Some("extract-engine-audio-tables") => {
            let source = required_path(&options, "--source")?;
            let profile_path = required_path(&options, "--profile")?;
            let output = required_path(&options, "--output")?;
            reject_unknown(&options, &["--source", "--profile", "--output"])?;
            let reviewed = load_engine_audio_tables_profile(&profile_path)?;
            let summary = extract_engine_audio_tables(&source, &output, &reviewed)
                .map_err(|error| error.to_string())?;
            serde_json::to_string(&summary).map_err(|error| error.to_string())
        }
        Some("extract-surface-audio-tables") => {
            let source = required_path(&options, "--source")?;
            let profile_path = required_path(&options, "--profile")?;
            let output = required_path(&options, "--output")?;
            reject_unknown(&options, &["--source", "--profile", "--output"])?;
            let reviewed = load_surface_audio_tables_profile(&profile_path)?;
            let summary = extract_surface_audio_tables(&source, &output, &reviewed)
                .map_err(|error| error.to_string())?;
            serde_json::to_string(&summary).map_err(|error| error.to_string())
        }
        Some("extract-stage-weather-tables") => {
            let source = required_path(&options, "--source")?;
            let profile_path = required_path(&options, "--profile")?;
            let output = required_path(&options, "--output")?;
            reject_unknown(&options, &["--source", "--profile", "--output"])?;
            let reviewed = load_stage_weather_tables_profile(&profile_path)?;
            let summary = extract_stage_weather_tables(&source, &output, &reviewed)
                .map_err(|error| error.to_string())?;
            serde_json::to_string(&summary).map_err(|error| error.to_string())
        }
        Some("extract-rival-records") => {
            let source = required_path(&options, "--source")?;
            let profile_path = required_path(&options, "--profile")?;
            let output = required_path(&options, "--output")?;
            reject_unknown(&options, &["--source", "--profile", "--output"])?;
            let reviewed = load_rival_records_profile(&profile_path)?;
            let summary = extract_rival_records(&source, &output, &reviewed)
                .map_err(|error| error.to_string())?;
            serde_json::to_string(&summary).map_err(|error| error.to_string())
        }
        Some("extract-static-tables") => {
            let source = required_path(&options, "--source")?;
            let profile_path = required_path(&options, "--profile")?;
            let output = required_path(&options, "--output")?;
            reject_unknown(&options, &["--source", "--profile", "--output"])?;
            let reviewed = load_static_tables_profile(&profile_path)?;
            let summary = extract_static_tables(&source, &output, &reviewed)
                .map_err(|error| error.to_string())?;
            serde_json::to_string(&summary).map_err(|error| error.to_string())
        }
        Some("extract-aux-control-constants") => {
            let source = required_path(&options, "--source")?;
            let profile_path = required_path(&options, "--profile")?;
            let output = required_path(&options, "--output")?;
            reject_unknown(&options, &["--source", "--profile", "--output"])?;
            let reviewed = load_aux_control_profile(&profile_path)?;
            let summary = extract_aux_control_constants(&source, &output, &reviewed)
                .map_err(|error| error.to_string())?;
            serde_json::to_string(&summary).map_err(|error| error.to_string())
        }
        _ => Err(usage()),
    }
}

fn parse_options(
    mut args: impl Iterator<Item = OsString>,
) -> Result<BTreeMap<String, OsString>, String> {
    let mut options = BTreeMap::new();
    while let Some(option) = args.next() {
        let option = option
            .into_string()
            .map_err(|_| "option name is not UTF-8".to_owned())?;
        if !option.starts_with("--") {
            return Err(format!("unexpected positional argument {option:?}"));
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {option}"))?;
        if options.insert(option.clone(), value).is_some() {
            return Err(format!("duplicate option {option}"));
        }
    }
    Ok(options)
}

fn required_path(options: &BTreeMap<String, OsString>, name: &str) -> Result<PathBuf, String> {
    options
        .get(name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing required option {name}"))
}

fn required_text(options: &BTreeMap<String, OsString>, name: &str) -> Result<String, String> {
    options
        .get(name)
        .ok_or_else(|| format!("missing required option {name}"))?
        .clone()
        .into_string()
        .map_err(|_| format!("{name} is not UTF-8"))
}

fn reject_unknown(options: &BTreeMap<String, OsString>, allowed: &[&str]) -> Result<(), String> {
    if let Some(name) = options
        .keys()
        .find(|name| !allowed.contains(&name.as_str()))
    {
        return Err(format!("unknown option {name}"));
    }
    Ok(())
}

fn load_profile(path: &Path) -> Result<ReviewedProfile, String> {
    let document = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    ReviewedProfile::from_json(&document).map_err(|error| format!("{}: {error}", path.display()))
}

fn load_surface_profile(path: &Path) -> Result<ReviewedSurfaceProfile, String> {
    let document = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    ReviewedSurfaceProfile::from_json(&document)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn load_surface_capability_profile(
    path: &Path,
) -> Result<ReviewedSurfaceCapabilityProfile, String> {
    let document = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    ReviewedSurfaceCapabilityProfile::from_json(&document)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn load_compiled_initializer_profile(
    path: &Path,
) -> Result<ReviewedCompiledInitializerProfile, String> {
    let document = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    ReviewedCompiledInitializerProfile::from_json(&document)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn load_car_selector_profile(path: &Path) -> Result<ReviewedCarSelectorProfile, String> {
    let document = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    ReviewedCarSelectorProfile::from_json(&document)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn load_tyre_tables_profile(path: &Path) -> Result<ReviewedTyreTablesProfile, String> {
    let document = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    ReviewedTyreTablesProfile::from_json(&document)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn load_engine_audio_tables_profile(
    path: &Path,
) -> Result<ReviewedEngineAudioTablesProfile, String> {
    let document = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    ReviewedEngineAudioTablesProfile::from_json(&document)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn load_surface_audio_tables_profile(
    path: &Path,
) -> Result<ReviewedSurfaceAudioTablesProfile, String> {
    let document = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    ReviewedSurfaceAudioTablesProfile::from_json(&document)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn load_stage_weather_tables_profile(
    path: &Path,
) -> Result<ReviewedStageWeatherTablesProfile, String> {
    let document = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    ReviewedStageWeatherTablesProfile::from_json(&document)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn load_rival_records_profile(path: &Path) -> Result<ReviewedRivalRecordsProfile, String> {
    let document = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    ReviewedRivalRecordsProfile::from_json(&document)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn load_static_tables_profile(path: &Path) -> Result<ReviewedStaticTablesProfile, String> {
    let document = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    ReviewedStaticTablesProfile::from_json(&document)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn load_aux_control_profile(path: &Path) -> Result<ReviewedAuxControlProfile, String> {
    let document = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    ReviewedAuxControlProfile::from_json(&document)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn usage() -> String {
    "usage: cmr-assets <catalog|verify|import|extract-surface-tables|extract-surface-capabilities|extract-compiled-car-initializer|extract-car-selectors|extract-tyre-surface-tables|extract-engine-audio-tables|extract-surface-audio-tables|extract-stage-weather-tables|extract-rival-records|extract-static-tables|extract-aux-control-constants> [options]".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cmr_assets::{
        AUX_CONTROL_DATA_FILE, AUX_CONTROL_STATE_FILE, AuxControlConstantsStore,
        AuxControlImportSummary, AuxControlProfile, CompiledInitializerEncoding,
        CompiledInitializerExecutable, CompiledInitializerProfile, CompiledInitializerSpec,
        CompiledInitializerStore, ReviewedAuxControlProfile, SURFACE_CAPABILITY_DATA_FILE,
        SURFACE_CAPABILITY_STATE_FILE, SurfaceCapabilityImportSummary, SurfaceCapabilityProfile,
    };
    use cmr_formats::pe_aux_control_initializer::{
        AUX_CONTROL_ENCODED_LEN, test_support::synthetic_aux_control_pe,
    };
    use cmr_formats::pe_compiled_initializer::{
        COMPILED_CASE_COUNT, COMPILED_CONTROL_WORD_OFFSETS, COMPILED_STATE_WORD_OFFSETS,
        OFFICIAL_EU_COMPILED_INITIALIZER_LAYOUT,
    };
    use sha2::{Digest, Sha256};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct Temp(PathBuf);

    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "cmr-compiled-cli-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn args(values: &[&str]) -> impl Iterator<Item = OsString> {
        values.iter().map(OsString::from)
    }

    fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn version_command_is_stable_and_rejects_extra_arguments() {
        assert_eq!(
            run(args(&["--version"])).unwrap(),
            format!("cmr-assets {}", env!("CARGO_PKG_VERSION"))
        );
        assert!(run(args(&["--version", "extra"])).is_err());
    }

    fn compiled_cli_synthetic_pe() -> Vec<u8> {
        const PE_OFFSET: usize = 0x80;
        const OPTIONAL_SIZE: usize = 0xe0;
        const SECTION_TABLE: usize = PE_OFFSET + 4 + 20 + OPTIONAL_SIZE;
        const RAW_OFFSET: usize = 0x400;
        const SECTION_RVA: u32 = 0x4000;
        const RAW_SIZE: usize = 0x3400;
        let layout = OFFICIAL_EU_COMPILED_INITIALIZER_LAYOUT;
        let section_va = layout.image_base + SECTION_RVA;
        let file_offset = |va: u32| RAW_OFFSET + usize::try_from(va - section_va).unwrap();

        let mut bytes = vec![0_u8; RAW_OFFSET + RAW_SIZE];
        bytes[..2].copy_from_slice(b"MZ");
        put_u32(&mut bytes, 0x3c, PE_OFFSET as u32);
        bytes[PE_OFFSET..PE_OFFSET + 4].copy_from_slice(b"PE\0\0");
        put_u16(&mut bytes, PE_OFFSET + 4, 0x14c);
        put_u16(&mut bytes, PE_OFFSET + 6, 1);
        put_u16(&mut bytes, PE_OFFSET + 20, OPTIONAL_SIZE as u16);
        put_u16(&mut bytes, PE_OFFSET + 24, 0x10b);
        put_u32(&mut bytes, PE_OFFSET + 24 + 28, layout.image_base);
        put_u32(&mut bytes, SECTION_TABLE + 8, RAW_SIZE as u32);
        put_u32(&mut bytes, SECTION_TABLE + 12, SECTION_RVA);
        put_u32(&mut bytes, SECTION_TABLE + 16, RAW_SIZE as u32);
        put_u32(&mut bytes, SECTION_TABLE + 20, RAW_OFFSET as u32);
        put_u32(&mut bytes, SECTION_TABLE + 36, 0x6000_0020);

        let proof_start = file_offset(layout.zero_seed_va);
        let proof_end = file_offset(layout.dispatch_va);
        bytes[proof_start..proof_end].fill(0x50);
        bytes[proof_start..proof_start + 2].copy_from_slice(&[0x33, 0xdb]);
        let esi = file_offset(layout.esi_seed_va);
        bytes[esi] = 0xbe;
        put_u32(&mut bytes, esi + 1, i32::MIN as u32);
        let edi = file_offset(layout.edi_seed_va);
        bytes[edi] = 0xbf;
        put_u32(&mut bytes, edi + 1, i32::MAX as u32);

        let dispatch = file_offset(layout.dispatch_va);
        bytes[dispatch..dispatch + 19].copy_from_slice(&[
            0x8b, 0x45, 0x14, 0x83, 0xf8, 0x0d, 0x0f, 0x87, 0, 0, 0, 0, 0xff, 0x24, 0x85, 0, 0, 0,
            0,
        ]);
        let default = i64::from(layout.common_join_va) - i64::from(layout.dispatch_va + 12);
        put_u32(&mut bytes, dispatch + 8, default as i32 as u32);
        put_u32(&mut bytes, dispatch + 15, layout.jump_table_va);

        let chronological: Vec<(u32, u16)> = COMPILED_STATE_WORD_OFFSETS[..3]
            .iter()
            .copied()
            .map(|offset| (layout.state_pointer_va, offset))
            .chain(
                COMPILED_CONTROL_WORD_OFFSETS
                    .iter()
                    .copied()
                    .map(|offset| (layout.control_pointer_va, offset)),
            )
            .chain(
                COMPILED_STATE_WORD_OFFSETS[3..]
                    .iter()
                    .copied()
                    .map(|offset| (layout.state_pointer_va, offset)),
            )
            .collect();
        assert_eq!(chronological.len(), 31);

        let mut immediates_left = 192_usize;
        let mut target = layout.first_case_va;
        let mut targets = [0_u32; COMPILED_CASE_COUNT];
        for (selector, case_target) in targets.iter_mut().enumerate() {
            *case_target = target;
            let mut body = Vec::new();
            body.push(0xb9);
            body.extend_from_slice(&(selector as i32 * 101 - 700).to_le_bytes());
            if selector < 3 {
                body.push(0xb8);
                body.extend_from_slice(&(selector as i32 + 17).to_le_bytes());
            }
            for (index, (pointer, displacement)) in chronological.iter().copied().enumerate() {
                body.extend_from_slice(&[0x8b, 0x15]);
                body.extend_from_slice(&pointer.to_le_bytes());
                if immediates_left > 0 {
                    immediates_left -= 1;
                    body.extend_from_slice(&[0xc7, 0x82]);
                    body.extend_from_slice(&u32::from(displacement).to_le_bytes());
                    body.extend_from_slice(
                        &(selector as i32 * 1_000 + index as i32 - 300).to_le_bytes(),
                    );
                } else {
                    let source = [3_u8, 6, 7][index % 3];
                    body.extend_from_slice(&[0x89, 0x80 | (source << 3) | 2]);
                    body.extend_from_slice(&u32::from(displacement).to_le_bytes());
                }
            }
            let body_offset = file_offset(target);
            bytes[body_offset..body_offset + body.len()].copy_from_slice(&body);
            target += body.len() as u32;
            if selector + 1 != COMPILED_CASE_COUNT {
                let jump_offset = file_offset(target);
                bytes[jump_offset] = 0xe9;
                let displacement = i64::from(layout.common_join_va) - i64::from(target + 5);
                put_u32(&mut bytes, jump_offset + 1, displacement as i32 as u32);
                target += 5;
            }
        }
        assert_eq!(immediates_left, 0);
        assert_eq!(target, layout.common_join_va);
        for (selector, target) in targets.into_iter().enumerate() {
            put_u32(
                &mut bytes,
                file_offset(layout.jump_table_va) + selector * 4,
                target,
            );
        }
        bytes
    }

    fn capability_cli_synthetic_pe() -> Vec<u8> {
        let mut bytes = vec![0; 0x1000];
        let pe = 0x80;
        let optional = pe + 24;
        let sections = optional + 0xe0;
        bytes[..2].copy_from_slice(b"MZ");
        put_u32(&mut bytes, 0x3c, pe as u32);
        bytes[pe..pe + 4].copy_from_slice(b"PE\0\0");
        put_u16(&mut bytes, pe + 4, 0x14c);
        put_u16(&mut bytes, pe + 6, 1);
        put_u16(&mut bytes, pe + 20, 0xe0);
        put_u16(&mut bytes, optional, 0x10b);
        put_u32(&mut bytes, optional + 28, 0x400000);
        put_u32(&mut bytes, sections + 8, 0x900);
        put_u32(&mut bytes, sections + 12, 0xec000);
        put_u32(&mut bytes, sections + 16, 0x900);
        put_u32(&mut bytes, sections + 20, 0x200);
        for (index, byte) in bytes[0x2f8..0x2f8 + 1920].iter_mut().enumerate() {
            *byte = (index % 253) as u8;
        }
        bytes
    }

    fn digest(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    #[test]
    fn rejects_missing_duplicate_and_unknown_options() {
        assert!(run(args(&["verify", "--source", "somewhere"])).is_err());
        assert!(
            run(args(&[
                "verify",
                "--source",
                "a",
                "--source",
                "b",
                "--profile",
                "p"
            ]))
            .unwrap_err()
            .contains("duplicate")
        );
        assert!(
            run(args(&[
                "verify",
                "--source",
                "a",
                "--profile",
                "p",
                "--surprise",
                "x"
            ]))
            .unwrap_err()
            .contains("unknown")
        );
    }

    #[test]
    fn every_command_requires_an_explicit_source_path() {
        for arguments in [
            &[
                "catalog",
                "--id",
                "local",
                "--output-profile",
                "profile.json",
            ][..],
            &["verify", "--profile", "profile.json"],
            &[
                "import",
                "--profile",
                "profile.json",
                "--output",
                "runtime-data",
            ],
            &[
                "extract-surface-tables",
                "--profile",
                "surface-profile.json",
                "--output",
                "runtime-data/surface",
            ],
            &[
                "extract-surface-capabilities",
                "--profile",
                "surface-capability-profile.json",
                "--output",
                "runtime-data/surface-capabilities",
            ],
            &[
                "extract-compiled-car-initializer",
                "--profile",
                "compiled-profile.json",
                "--output",
                "runtime-data/compiled-cases",
            ],
            &[
                "extract-car-selectors",
                "--profile",
                "selector-profile.json",
                "--output",
                "runtime-data/car-selectors",
            ],
            &[
                "extract-tyre-surface-tables",
                "--profile",
                "tyre-profile.json",
                "--output",
                "runtime-data/tyre-surface-tables",
            ],
            &[
                "extract-engine-audio-tables",
                "--profile",
                "engine-audio-profile.json",
                "--output",
                "runtime-data/engine-audio-tables",
            ],
            &[
                "extract-surface-audio-tables",
                "--profile",
                "surface-audio-profile.json",
                "--output",
                "runtime-data/surface-audio-tables",
            ],
            &[
                "extract-stage-weather-tables",
                "--profile",
                "stage-weather-profile.json",
                "--output",
                "runtime-data/stage-weather-tables",
            ],
            &[
                "extract-rival-records",
                "--profile",
                "rival-profile.json",
                "--output",
                "runtime-data/rival-records",
            ],
            &[
                "extract-static-tables",
                "--profile",
                "static-tables-profile.json",
                "--output",
                "runtime-data/static-tables",
            ],
            &[
                "extract-aux-control-constants",
                "--profile",
                "aux-profile.json",
                "--output",
                "runtime-data/aux-control",
            ],
        ] {
            assert_eq!(
                run(args(arguments)),
                Err("missing required option --source".to_owned())
            );
        }
    }

    #[test]
    fn surface_extraction_requires_profile_and_output() {
        assert_eq!(
            run(args(&["extract-surface-tables", "--source", "source"])),
            Err("missing required option --profile".to_owned())
        );
        assert_eq!(
            run(args(&[
                "extract-surface-tables",
                "--source",
                "source",
                "--profile",
                "profile.json",
            ])),
            Err("missing required option --output".to_owned())
        );
    }

    #[test]
    fn surface_capability_cli_extracts_reviewed_synthetic_data() {
        assert_eq!(
            run(args(&[
                "extract-surface-capabilities",
                "--source",
                "source"
            ])),
            Err("missing required option --profile".to_owned())
        );
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let executable = capability_cli_synthetic_pe();
        fs::write(source.join("Rally.exe"), &executable).unwrap();
        let mut profile: SurfaceCapabilityProfile = serde_json::from_slice(include_bytes!(
            "../../../profiles/official-eu-surface-capabilities-v1.json"
        ))
        .unwrap();
        profile.id = "synthetic-cli-capabilities".into();
        profile.executable.size = executable.len() as u64;
        profile.executable.sha256 = digest(&executable);
        let profile_path = temp.0.join("capability-profile.json");
        fs::write(&profile_path, serde_json::to_vec(&profile).unwrap()).unwrap();
        let output = temp.0.join("capability-output");
        let arguments = [
            OsString::from("extract-surface-capabilities"),
            OsString::from("--source"),
            source.into_os_string(),
            OsString::from("--profile"),
            profile_path.into_os_string(),
            OsString::from("--output"),
            output.clone().into_os_string(),
        ];
        let result = run(arguments.into_iter()).unwrap();
        let summary: SurfaceCapabilityImportSummary = serde_json::from_str(&result).unwrap();
        assert_eq!(summary.output.size, 1920);
        let data = fs::read(output.join(SURFACE_CAPABILITY_DATA_FILE)).unwrap();
        for (index, byte) in data.iter().enumerate() {
            assert_eq!(*byte, (index % 253) as u8);
        }
        let state = fs::read_to_string(output.join(SURFACE_CAPABILITY_STATE_FILE)).unwrap();
        assert!(!state.contains(temp.0.to_str().unwrap()));
    }

    #[test]
    fn compiled_initializer_extraction_requires_profile_and_output() {
        assert_eq!(
            run(args(&[
                "extract-compiled-car-initializer",
                "--source",
                "source",
            ])),
            Err("missing required option --profile".to_owned())
        );
        assert_eq!(
            run(args(&[
                "extract-compiled-car-initializer",
                "--source",
                "source",
                "--profile",
                "profile.json",
            ])),
            Err("missing required option --output".to_owned())
        );
    }

    #[test]
    fn car_selector_extraction_requires_profile_and_output() {
        assert_eq!(
            run(args(&["extract-car-selectors", "--source", "source"])),
            Err("missing required option --profile".to_owned())
        );
        assert_eq!(
            run(args(&[
                "extract-car-selectors",
                "--source",
                "source",
                "--profile",
                "profile.json",
            ])),
            Err("missing required option --output".to_owned())
        );
    }

    #[test]
    fn tyre_tables_extraction_requires_profile_and_output() {
        assert_eq!(
            run(args(&["extract-tyre-surface-tables", "--source", "source"])),
            Err("missing required option --profile".to_owned())
        );
        assert_eq!(
            run(args(&[
                "extract-tyre-surface-tables",
                "--source",
                "source",
                "--profile",
                "profile.json",
            ])),
            Err("missing required option --output".to_owned())
        );
    }

    #[test]
    fn engine_audio_tables_extraction_requires_profile_and_output() {
        assert_eq!(
            run(args(&["extract-engine-audio-tables", "--source", "source"])),
            Err("missing required option --profile".to_owned())
        );
        assert_eq!(
            run(args(&[
                "extract-engine-audio-tables",
                "--source",
                "source",
                "--profile",
                "profile.json",
            ])),
            Err("missing required option --output".to_owned())
        );
    }

    #[test]
    fn engine_audio_tables_cli_authenticates_then_rejects_a_non_pe_without_output() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let executable = b"identified synthetic non-PE";
        fs::write(source.join("Rally.exe"), executable).unwrap();
        let mut profile: cmr_assets::EngineAudioTablesProfile = serde_json::from_slice(
            include_bytes!("../../../profiles/official-eu-engine-audio-tables-v1.json"),
        )
        .unwrap();
        profile.id = "synthetic-cli-engine-audio".into();
        profile.executable.size = executable.len() as u64;
        profile.executable.sha256 = digest(executable);
        let profile_path = temp.0.join("engine-audio-profile.json");
        fs::write(&profile_path, serde_json::to_vec(&profile).unwrap()).unwrap();
        let output = temp.0.join("engine-audio-output");
        let error = run([
            OsString::from("extract-engine-audio-tables"),
            OsString::from("--source"),
            source.into_os_string(),
            OsString::from("--profile"),
            profile_path.into_os_string(),
            OsString::from("--output"),
            output.clone().into_os_string(),
        ]
        .into_iter())
        .unwrap_err();
        assert!(
            error.contains("cannot extract engine audio tables"),
            "{error}"
        );
        assert!(!output.exists());
    }

    #[test]
    fn surface_audio_tables_extraction_requires_profile_and_output() {
        assert_eq!(
            run(args(&[
                "extract-surface-audio-tables",
                "--source",
                "source"
            ])),
            Err("missing required option --profile".to_owned())
        );
        assert_eq!(
            run(args(&[
                "extract-surface-audio-tables",
                "--source",
                "source",
                "--profile",
                "profile.json",
            ])),
            Err("missing required option --output".to_owned())
        );
    }

    #[test]
    fn surface_audio_tables_cli_authenticates_then_rejects_a_non_pe_without_output() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let executable = b"identified synthetic non-PE";
        fs::write(source.join("Rally.exe"), executable).unwrap();
        let mut profile: cmr_assets::SurfaceAudioTablesProfile = serde_json::from_slice(
            include_bytes!("../../../profiles/official-eu-surface-audio-tables-v1.json"),
        )
        .unwrap();
        profile.id = "synthetic-cli-surface-audio".into();
        profile.executable.size = executable.len() as u64;
        profile.executable.sha256 = digest(executable);
        let profile_path = temp.0.join("surface-audio-profile.json");
        fs::write(&profile_path, serde_json::to_vec(&profile).unwrap()).unwrap();
        let output = temp.0.join("surface-audio-output");
        let error = run([
            OsString::from("extract-surface-audio-tables"),
            OsString::from("--source"),
            source.into_os_string(),
            OsString::from("--profile"),
            profile_path.into_os_string(),
            OsString::from("--output"),
            output.clone().into_os_string(),
        ]
        .into_iter())
        .unwrap_err();
        assert!(
            error.contains("cannot extract surface audio tables"),
            "{error}"
        );
        assert!(!output.exists());
    }

    #[test]
    fn stage_weather_tables_extraction_requires_profile_and_output() {
        assert_eq!(
            run(args(&[
                "extract-stage-weather-tables",
                "--source",
                "source"
            ])),
            Err("missing required option --profile".to_owned())
        );
        assert_eq!(
            run(args(&[
                "extract-stage-weather-tables",
                "--source",
                "source",
                "--profile",
                "profile.json",
            ])),
            Err("missing required option --output".to_owned())
        );
        assert_eq!(
            run(args(&[
                "extract-stage-weather-tables",
                "--source",
                "source",
                "--profile",
                "profile.json",
                "--output",
                "out",
                "--extra",
                "x",
            ])),
            Err("unknown option --extra".to_owned())
        );
    }

    #[test]
    fn stage_weather_tables_cli_authenticates_then_rejects_a_non_pe_without_output() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let executable = b"identified synthetic non-PE";
        fs::write(source.join("Rally.exe"), executable).unwrap();
        let mut profile: cmr_assets::StageWeatherTablesProfile = serde_json::from_slice(
            include_bytes!("../../../profiles/official-eu-stage-weather-tables-v1.json"),
        )
        .unwrap();
        profile.id = "synthetic-cli-stage-weather".into();
        profile.executable.size = executable.len() as u64;
        profile.executable.sha256 = digest(executable);
        let profile_path = temp.0.join("stage-weather-profile.json");
        fs::write(&profile_path, serde_json::to_vec(&profile).unwrap()).unwrap();
        let output = temp.0.join("stage-weather-output");
        let error = run([
            OsString::from("extract-stage-weather-tables"),
            OsString::from("--source"),
            source.into_os_string(),
            OsString::from("--profile"),
            profile_path.into_os_string(),
            OsString::from("--output"),
            output.clone().into_os_string(),
        ]
        .into_iter())
        .unwrap_err();
        assert!(
            error.contains("cannot extract stage weather tables"),
            "{error}"
        );
        assert!(!output.exists());
    }

    #[test]
    fn rival_records_extraction_requires_profile_and_output() {
        assert_eq!(
            run(args(&["extract-rival-records", "--source", "source"])),
            Err("missing required option --profile".to_owned())
        );
        assert_eq!(
            run(args(&[
                "extract-rival-records",
                "--source",
                "source",
                "--profile",
                "profile.json",
            ])),
            Err("missing required option --output".to_owned())
        );
        assert_eq!(
            run(args(&[
                "extract-rival-records",
                "--source",
                "source",
                "--profile",
                "profile.json",
                "--output",
                "out",
                "--extra",
                "x",
            ])),
            Err("unknown option --extra".to_owned())
        );
    }

    #[test]
    fn rival_records_cli_authenticates_then_rejects_a_non_pe_without_output() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let executable = b"identified synthetic non-PE";
        fs::write(source.join("Rally.exe"), executable).unwrap();
        let mut profile: cmr_assets::RivalRecordsProfile = serde_json::from_slice(include_bytes!(
            "../../../profiles/official-eu-rival-records-v1.json"
        ))
        .unwrap();
        profile.id = "synthetic-cli-rival-records".into();
        profile.executable.size = executable.len() as u64;
        profile.executable.sha256 = digest(executable);
        let profile_path = temp.0.join("rival-profile.json");
        fs::write(&profile_path, serde_json::to_vec(&profile).unwrap()).unwrap();
        let output = temp.0.join("rival-output");
        let error = run([
            OsString::from("extract-rival-records"),
            OsString::from("--source"),
            source.into_os_string(),
            OsString::from("--profile"),
            profile_path.into_os_string(),
            OsString::from("--output"),
            output.clone().into_os_string(),
        ]
        .into_iter())
        .unwrap_err();
        assert!(error.contains("cannot extract rival records"), "{error}");
        assert!(!output.exists());
    }

    #[test]
    fn static_tables_extraction_requires_profile_and_output() {
        assert_eq!(
            run(args(&["extract-static-tables", "--source", "source"])),
            Err("missing required option --profile".to_owned())
        );
        assert_eq!(
            run(args(&[
                "extract-static-tables",
                "--source",
                "source",
                "--profile",
                "profile.json",
            ])),
            Err("missing required option --output".to_owned())
        );
        assert_eq!(
            run(args(&[
                "extract-static-tables",
                "--source",
                "source",
                "--profile",
                "profile.json",
                "--output",
                "out",
                "--extra",
                "x",
            ])),
            Err("unknown option --extra".to_owned())
        );
    }

    #[test]
    fn static_tables_cli_authenticates_then_rejects_a_non_pe_without_output() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let executable = b"identified synthetic non-PE";
        fs::write(source.join("Rally.exe"), executable).unwrap();
        let mut profile: cmr_assets::StaticTablesProfile = serde_json::from_slice(include_bytes!(
            "../../../profiles/official-eu-static-tables-v1.json"
        ))
        .unwrap();
        profile.id = "synthetic-cli-static-tables".into();
        profile.executable.size = executable.len() as u64;
        profile.executable.sha256 = digest(executable);
        let profile_path = temp.0.join("static-tables-profile.json");
        fs::write(&profile_path, serde_json::to_vec(&profile).unwrap()).unwrap();
        let output = temp.0.join("static-tables-output");
        let error = run([
            OsString::from("extract-static-tables"),
            OsString::from("--source"),
            source.into_os_string(),
            OsString::from("--profile"),
            profile_path.into_os_string(),
            OsString::from("--output"),
            output.clone().into_os_string(),
        ]
        .into_iter())
        .unwrap_err();
        assert!(error.contains("cannot extract static tables"), "{error}");
        assert!(!output.exists());
    }

    #[test]
    fn aux_control_extraction_requires_profile_and_output() {
        assert_eq!(
            run(args(&[
                "extract-aux-control-constants",
                "--source",
                "source"
            ])),
            Err("missing required option --profile".to_owned())
        );
        assert_eq!(
            run(args(&[
                "extract-aux-control-constants",
                "--source",
                "source",
                "--profile",
                "profile.json",
            ])),
            Err("missing required option --output".to_owned())
        );
    }

    #[test]
    fn aux_control_cli_authenticates_then_rejects_a_malformed_synthetic_pe_without_output() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let executable = b"identified synthetic non-PE";
        fs::write(source.join("Rally.exe"), executable).unwrap();
        let mut profile: AuxControlProfile = serde_json::from_slice(include_bytes!(
            "../../../profiles/official-eu-aux-control-constants-v1.json"
        ))
        .unwrap();
        profile.id = "synthetic-cli-aux-control".into();
        profile.executable.size = executable.len() as u64;
        profile.executable.sha256 = digest(executable);
        let profile_path = temp.0.join("aux-profile.json");
        fs::write(&profile_path, serde_json::to_vec(&profile).unwrap()).unwrap();
        let output = temp.0.join("aux-output");

        let error = run([
            OsString::from("extract-aux-control-constants"),
            OsString::from("--source"),
            source.into_os_string(),
            OsString::from("--profile"),
            profile_path.into_os_string(),
            OsString::from("--output"),
            output.clone().into_os_string(),
        ]
        .into_iter())
        .unwrap_err();
        assert!(error.contains("cannot extract aux-control constants"));
        assert!(!output.exists());
    }

    #[test]
    fn aux_control_cli_extracts_real_semantic_fixture_and_store_reopens_exact_words() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let (executable, expected) = synthetic_aux_control_pe();
        fs::write(source.join("Rally.exe"), &executable).unwrap();
        let mut profile: AuxControlProfile = serde_json::from_slice(include_bytes!(
            "../../../profiles/official-eu-aux-control-constants-v1.json"
        ))
        .unwrap();
        profile.id = "synthetic-cli-aux-control".into();
        profile.executable.size = executable.len() as u64;
        profile.executable.sha256 = digest(&executable);
        let profile_bytes = serde_json::to_vec(&profile).unwrap();
        let profile_path = temp.0.join("aux-profile.json");
        fs::write(&profile_path, &profile_bytes).unwrap();
        let output = temp.0.join("aux-output");

        let output_text = run([
            OsString::from("extract-aux-control-constants"),
            OsString::from("--source"),
            source.into_os_string(),
            OsString::from("--profile"),
            profile_path.into_os_string(),
            OsString::from("--output"),
            output.clone().into_os_string(),
        ]
        .into_iter())
        .unwrap();
        let summary: AuxControlImportSummary = serde_json::from_str(&output_text).unwrap();
        assert_eq!(summary.output.path, AUX_CONTROL_DATA_FILE);
        assert_eq!(summary.output.size, AUX_CONTROL_ENCODED_LEN as u64);
        assert_eq!(
            fs::metadata(output.join(AUX_CONTROL_DATA_FILE))
                .unwrap()
                .len(),
            AUX_CONTROL_ENCODED_LEN as u64
        );
        assert!(output.join(AUX_CONTROL_STATE_FILE).is_file());

        let reviewed = ReviewedAuxControlProfile::from_json(&profile_bytes).unwrap();
        let store = AuxControlConstantsStore::open(&output, &reviewed).unwrap();
        assert_eq!(store.constants(), &expected);
    }

    #[test]
    fn compiled_initializer_cli_extracts_and_reopens_synthetic_cases() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let executable = compiled_cli_synthetic_pe();
        fs::write(source.join("Rally.exe"), &executable).unwrap();
        let layout = OFFICIAL_EU_COMPILED_INITIALIZER_LAYOUT;
        let profile = CompiledInitializerProfile {
            schema_version: 1,
            id: "synthetic-cli-compiled-cases".into(),
            executable: CompiledInitializerExecutable {
                path: "Rally.exe".into(),
                size: executable.len() as u64,
                sha256: digest(&executable),
            },
            initializer: CompiledInitializerSpec {
                image_base: layout.image_base,
                zero_seed_va: layout.zero_seed_va,
                esi_seed_va: layout.esi_seed_va,
                edi_seed_va: layout.edi_seed_va,
                seed_call_targets: layout.seed_call_targets,
                dispatch_va: layout.dispatch_va,
                jump_table_va: layout.jump_table_va,
                first_case_va: layout.first_case_va,
                common_join_va: layout.common_join_va,
                state_pointer_va: layout.state_pointer_va,
                control_pointer_va: layout.control_pointer_va,
                selectors: COMPILED_CASE_COUNT as u32,
                state_offsets: layout.state_offsets,
                control_offsets: layout.control_offsets,
                output_encoding: CompiledInitializerEncoding::CmrCcaseV1,
            },
        };
        let profile_path = temp.0.join("profile.json");
        let profile_bytes = serde_json::to_vec(&profile).unwrap();
        fs::write(&profile_path, &profile_bytes).unwrap();
        let output = temp.0.join("runtime-data");
        let output_text = run([
            OsString::from("extract-compiled-car-initializer"),
            OsString::from("--source"),
            source.into_os_string(),
            OsString::from("--profile"),
            profile_path.into_os_string(),
            OsString::from("--output"),
            output.clone().into_os_string(),
        ]
        .into_iter())
        .unwrap();
        let summary: cmr_assets::CompiledInitializerImportSummary =
            serde_json::from_str(&output_text).unwrap();
        assert_eq!(summary.output.size, 1_760);
        let reviewed = ReviewedCompiledInitializerProfile::from_json(&profile_bytes).unwrap();
        let store = CompiledInitializerStore::open(&output, &reviewed).unwrap();
        assert_eq!(store.cases().len(), COMPILED_CASE_COUNT);
        assert_eq!(store.case(0).unwrap().control_words[0], -297);
        assert_eq!(store.case(13).unwrap().state_words[1], i32::MIN);
    }
}
