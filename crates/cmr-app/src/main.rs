use std::path::Path;
use std::process::ExitCode;

use cmr_app::{
    AppCommand, RUNTIME_DATA_ENV, RuntimeConfig, SOURCE_ENV, run_headless_game_session,
    run_headless_root_frame, run_headless_stage_frame, run_windowed_game_session,
    run_windowed_root_frame, run_windowed_stage_frame, runtime_config,
};

/// The media root and stage name a stage command resolves with.
///
/// `RuntimeConfig::resolve` guarantees both are present for a stage command, so
/// these accessors are populated.
fn stage_inputs(config: &RuntimeConfig) -> (&Path, &str) {
    let media_root = config
        .media_root()
        .expect("stage command resolves with a media root");
    let stage = config.stage().expect("stage command resolves with a stage");
    (media_root, stage)
}

fn main() -> ExitCode {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if arguments.as_slice() == ["--version"] {
        println!("cmr-app {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    match runtime_config(
        arguments,
        std::env::var_os(RUNTIME_DATA_ENV),
        std::env::var_os(SOURCE_ENV),
    ) {
        Ok(config) => match config.command() {
            AppCommand::HeadlessRootFrame => match run_headless_root_frame(config.runtime_data()) {
                Ok(output) => {
                    println!("{}", output.stable_summary());
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("cmr-app: headless-root-frame failed: {}", error.kind());
                    ExitCode::FAILURE
                }
            },
            AppCommand::WindowedRootFrame => {
                match run_windowed_root_frame(config.runtime_data(), config.media_root()) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(error) => {
                        eprintln!("cmr-app: windowed-root-frame failed: {}", error.kind());
                        ExitCode::FAILURE
                    }
                }
            }
            AppCommand::HeadlessStageFrame => {
                // resolve() guarantees both a stage name and a media root for
                // this command, so these accessors are populated.
                let (media_root, stage) = stage_inputs(&config);
                match run_headless_stage_frame(media_root, stage, config.stage_renderer()) {
                    Ok(output) => {
                        println!("{}", output.stable_summary());
                        ExitCode::SUCCESS
                    }
                    Err(error) => {
                        eprintln!("cmr-app: headless-stage-frame failed: {}", error.kind());
                        ExitCode::FAILURE
                    }
                }
            }
            AppCommand::WindowedStageFrame => {
                let (media_root, stage) = stage_inputs(&config);
                match run_windowed_stage_frame(media_root, stage, config.stage_renderer()) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(error) => {
                        eprintln!("cmr-app: windowed-stage-frame failed: {}", error.kind());
                        ExitCode::FAILURE
                    }
                }
            }
            AppCommand::HeadlessGameSession => {
                let output = run_headless_game_session();
                println!("{}", output.stable_summary());
                ExitCode::SUCCESS
            }
            AppCommand::WindowedGameSession => {
                let (media_root, stage) = stage_inputs(&config);
                let car_code = config
                    .car_code()
                    .expect("windowed game session resolves with a car code");
                match run_windowed_game_session(
                    media_root,
                    config.runtime_data(),
                    stage,
                    car_code,
                    config.stage_renderer(),
                    config.language(),
                ) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(error) => {
                        eprintln!("cmr-app: windowed-game-session failed: {}", error.kind());
                        ExitCode::FAILURE
                    }
                }
            }
        },
        Err(error) => {
            eprintln!("cmr-app: {error}");
            ExitCode::FAILURE
        }
    }
}
