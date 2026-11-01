use std::fs;
use std::path::PathBuf;
use std::process::Command;

use cmr_app::{
    HeadlessRootFrameError, RUNTIME_DATA_ENV, render_root_frame_with_profile,
    run_headless_root_frame, run_headless_root_frame_with_profile,
};
use cmr_core::localization::Language;
use cmr_core::root_menu_assets::VisualVariant;
use cmr_core::root_menu_c3d_runtime::{
    C3dMode2CornerSource, C3dMode2LoadState, C3dSurfaceLayoutGates, RootMenuC3dMode2LoadStates,
};
use cmr_platform_sdl3::present_software_frame_once;
use cmr_test_support::SyntheticRootFrameFixture;
use cmr_video::software_root_menu_session::SoftwareRootMenuSessionRequest;

fn synthetic_request() -> SoftwareRootMenuSessionRequest {
    let load_states = RootMenuC3dMode2LoadStates::splat(C3dMode2LoadState {
        prepared_resource_count: 0,
        corner_source: C3dMode2CornerSource::OrdinaryDefaults,
        surface_layout: C3dSurfaceLayoutGates::default(),
    });
    SoftwareRootMenuSessionRequest::new(Language::English, VisualVariant::Normal, load_states)
}

#[test]
fn shared_fixture_runs_one_complete_repeatable_root_frame() {
    let fixture = SyntheticRootFrameFixture::new();

    let first = run_headless_root_frame_with_profile(
        fixture.runtime_root(),
        fixture.reviewed_profile(),
        synthetic_request(),
    )
    .unwrap();
    let second = run_headless_root_frame_with_profile(
        fixture.runtime_root(),
        fixture.reviewed_profile(),
        synthetic_request(),
    )
    .unwrap();

    assert_eq!(first, second);
    assert_eq!(first.profile_id, "synthetic-root-frame-vertical");
    assert_eq!(first.frame_digest.extent.width, 640);
    assert_eq!(first.frame_digest.extent.height, 480);
    assert_eq!(first.frame_digest.indexed_pixel_count, 640 * 480);
    assert_eq!(first.bitmap_blits, 5);
    assert_eq!(first.bitmap_no_ops, 0);
    assert!(first.footer_rendered);
    assert_eq!(first.c3d_commands, 1);
    assert!(first.scene_completed);
    assert!(first.palette_committed);
    assert!(first.presentation_completed);
    assert!(
        !first
            .stable_summary()
            .contains(&fixture.runtime_root().display().to_string())
    );
}

#[test]
fn headless_digest_and_native_presenter_share_the_completed_frame_source() {
    let fixture = SyntheticRootFrameFixture::new();
    let rendered = render_root_frame_with_profile(
        fixture.runtime_root(),
        fixture.reviewed_profile(),
        synthetic_request(),
    )
    .unwrap();
    let headless = run_headless_root_frame_with_profile(
        fixture.runtime_root(),
        fixture.reviewed_profile(),
        synthetic_request(),
    )
    .unwrap();

    assert_eq!(rendered.frame.digest_summary(), headless.frame_digest);
    assert_eq!(
        rendered.frame.pixels().len(),
        usize::from(rendered.frame.extent().width) * usize::from(rendered.frame.extent().height)
    );
    assert_eq!(rendered.profile_id, headless.profile_id);
    assert_eq!(rendered.bitmap_blits, headless.bitmap_blits);
    assert_eq!(rendered.c3d_commands, headless.c3d_commands);
    assert_eq!(rendered.palette_committed, headless.palette_committed);
    assert_eq!(
        rendered.presentation_completed,
        headless.presentation_completed
    );
}

#[test]
fn corrupt_runtime_data_reports_the_store_boundary() {
    let fixture = SyntheticRootFrameFixture::new();
    fs::write(
        fixture.runtime_root().join("import-state.json"),
        b"not json",
    )
    .unwrap();

    let error = run_headless_root_frame_with_profile(
        fixture.runtime_root(),
        fixture.reviewed_profile(),
        synthetic_request(),
    )
    .unwrap_err();

    assert!(matches!(error, HeadlessRootFrameError::Store(_)));
    assert_eq!(error.kind(), "store");
}

#[test]
#[ignore = "requires an SDL video driver; run with SDL_VIDEODRIVER=dummy"]
fn sdl_host_opens_uploads_and_presents_the_shared_synthetic_frame() {
    let fixture = SyntheticRootFrameFixture::new();
    let rendered = render_root_frame_with_profile(
        fixture.runtime_root(),
        fixture.reviewed_profile(),
        synthetic_request(),
    )
    .unwrap();

    present_software_frame_once(&rendered.frame).unwrap();
}

#[test]
fn binary_reports_a_path_free_typed_store_failure() {
    let fixture = SyntheticRootFrameFixture::new();
    let result = Command::new(env!("CARGO_BIN_EXE_cmr-app"))
        .args(["--headless-root-frame", "--runtime-data"])
        .arg(fixture.runtime_root())
        .env_remove(RUNTIME_DATA_ENV)
        .output()
        .unwrap();

    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert_eq!(
        String::from_utf8(result.stderr).unwrap(),
        "cmr-app: headless-root-frame failed: store\n"
    );
}

#[test]
fn binary_reports_its_release_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_cmr-app"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("cmr-app {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

#[test]
#[ignore = "requires a user-owned import selected with CMR_RUNTIME_DATA"]
fn reviewed_import_produces_one_complete_official_root_frame() {
    let runtime_root = std::env::var_os(RUNTIME_DATA_ENV)
        .map(PathBuf::from)
        .expect("CMR_RUNTIME_DATA must name an imported runtime-data directory");

    let output = run_headless_root_frame(&runtime_root)
        .expect("reviewed import must produce the official initial root frame");

    assert_eq!(output.profile_id, "installed-eu-derived-assets-v2");
    assert_eq!(output.frame_digest.extent.width, 640);
    assert_eq!(output.frame_digest.extent.height, 480);
    assert!(output.footer_rendered);
    assert!(output.scene_completed);
    assert!(output.palette_committed);
    assert!(output.presentation_completed);
}
