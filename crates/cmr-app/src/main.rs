use std::fs;
use std::path::{Path, PathBuf};
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

const USAGE: &str = "\
usage: cmr-app-modern [command] [options]

Double-click (or run with no arguments) for guided first-time setup and launch.

Commands:
  --setup                   choose your original CMR folder and import all assets
  --windowed-root-frame     launch the full game (front-end menus and racing)
  --windowed-game-session   jump straight into a stage (needs --stage and --car-code)
  --windowed-stage-frame    render a single stage frame in a window (needs --stage)
  --headless-root-frame     render one front-end frame headlessly
  --headless-stage-frame    render one stage frame headlessly (needs --stage)
  --headless-game-session   run one headless session check

Options:
  --runtime-data <dir>      imported runtime-data directory (or CMR_RUNTIME_DATA)
  --source <dir>            original game folder (or CMR_SOURCE_DIR)
  --stage <name>            stage identifier for direct-to-stage commands
  --car-code <code>         car code for --windowed-game-session
  --renderer <auto|gpu|software>
  --language <english|french|german|italian|spanish>
  --version                 print version and exit
  --help, -h                print this help and exit";

fn user_data_runtime_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(local).join("CMRRecomp").join("runtime-data");
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(xdg) = std::env::var_os("XDG_DATA_HOME")
            && !xdg.is_empty()
        {
            return PathBuf::from(xdg).join("cmr-recomp").join("runtime-data");
        }
        if let Some(home) = std::env::var_os("HOME")
            && !home.is_empty()
        {
            return PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("cmr-recomp")
                .join("runtime-data");
        }
    }
    PathBuf::from("runtime-data")
}

fn dir_is_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".cmr-write-test-{}", std::process::id()));
    if fs::write(&probe, b"").is_ok() {
        let _ = fs::remove_file(&probe);
        true
    } else {
        false
    }
}

fn default_runtime_data_dir(exe: Option<&Path>) -> PathBuf {
    if let Some(env_dir) = std::env::var_os(RUNTIME_DATA_ENV)
        && !env_dir.is_empty()
    {
        return PathBuf::from(env_dir);
    }
    // Inside a Linux AppImage, APPDIR is mounted read-only: always store
    // imported data in the user's local data directory.
    if std::env::var_os("APPDIR").is_some() {
        return user_data_runtime_dir();
    }
    if let Some(exe_dir) = exe.and_then(Path::parent)
        && dir_is_writable(exe_dir)
    {
        return exe_dir.join("runtime-data");
    }
    user_data_runtime_dir()
}

fn run_guided_launch(force_setup: bool) -> ExitCode {
    let exe = std::env::current_exe().ok();
    let runtime_data = default_runtime_data_dir(exe.as_deref());

    let needs_setup = force_setup || !cmr_assets::is_runtime_data_ready(&runtime_data);
    let source = if needs_setup {
        let Some(profiles_dir) = cmr_assets::find_profiles_dir(exe.as_deref()) else {
            let msg = "Could not find the bundled profiles/ directory.\n\n\
                       Keep the profiles/ folder next to cmr-app-modern.exe (extract the full release ZIP).";
            eprintln!("cmr-app: {msg}");
            cmr_platform_sdl3::dialog::show_error("Colin McRae Rally Modern — Setup Error", msg);
            return ExitCode::FAILURE;
        };

        let env_source = std::env::var_os(SOURCE_ENV)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        let chosen_source = if let Some(dir) = env_source.filter(|d| !force_setup && d.is_dir()) {
            dir
        } else {
            cmr_platform_sdl3::dialog::show_info(
                "Colin McRae Rally Modern — First-Time Setup",
                &format!(
                    "Select your original Colin McRae Rally (1998, EU v1.5) game folder \
                     (the folder containing Rally.exe).\n\n\
                     All game assets and lookup tables will be imported automatically into:\n{}\n\n\
                     This only needs to be done once.",
                    runtime_data.display()
                ),
            );
            let default_loc = cmr_assets::read_saved_source(&runtime_data);
            match cmr_platform_sdl3::dialog::pick_folder(default_loc.as_deref()) {
                Ok(Some(folder)) => folder,
                Ok(None) => return ExitCode::SUCCESS,
                Err(error) => {
                    let msg = format!(
                        "Could not open the folder picker ({error}).\n\n\
                         You can run one-step setup from a terminal instead:\n\
                         cmr-assets setup --source <path-to-CMRally>"
                    );
                    eprintln!("cmr-app: {msg}");
                    cmr_platform_sdl3::dialog::show_error(
                        "Colin McRae Rally Modern — Setup Error",
                        &msg,
                    );
                    return ExitCode::FAILURE;
                }
            }
        };

        eprintln!(
            "cmr-app: importing game data from {} into {}...",
            chosen_source.display(),
            runtime_data.display()
        );
        if let Err(error) = cmr_assets::setup_all(
            &chosen_source,
            &profiles_dir,
            &runtime_data,
            force_setup,
            |step, total, label| {
                eprintln!("[{step}/{total}] {label}");
            },
        ) {
            let msg = format!(
                "Asset import failed:\n\n{error}\n\n\
                 Make sure you selected a clean, unpatched EU v1.5 Colin McRae Rally folder \
                 containing Rally.exe, Game/, New_Fe/, and ini/."
            );
            eprintln!("cmr-app: {msg}");
            cmr_platform_sdl3::dialog::show_error("Colin McRae Rally Modern — Setup Error", &msg);
            return ExitCode::FAILURE;
        }
        Some(chosen_source)
    } else {
        cmr_assets::read_saved_source(&runtime_data).or_else(|| {
            std::env::var_os(SOURCE_ENV)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        })
    };

    match run_windowed_root_frame(&runtime_data, source.as_deref()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let msg = format!("Could not start the game: {}", error.kind());
            eprintln!("cmr-app: windowed-root-frame failed: {}", error.kind());
            cmr_platform_sdl3::dialog::show_error("Colin McRae Rally Modern", &msg);
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if arguments.is_empty() {
        return run_guided_launch(false);
    }
    if arguments.as_slice() == ["--setup"] {
        return run_guided_launch(true);
    }
    if arguments.as_slice() == ["--version"] {
        println!("cmr-app {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if arguments.len() == 1 && (arguments[0] == "--help" || arguments[0] == "-h") {
        println!("{USAGE}");
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
