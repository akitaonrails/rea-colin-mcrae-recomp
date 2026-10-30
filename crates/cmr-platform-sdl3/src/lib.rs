//! SDL3 host adapter for completed compatibility frames.
//!
//! SDL owns the native window, event queue, and opened gamepad handles. It
//! does not participate in recovered rasterization: callers supply one
//! completed indexed software frame, which is expanded to opaque RGBA8 before
//! upload to an SDL streaming texture.

pub mod audio;
pub mod input;

pub use audio::{AudioHost, SharedSoftwareMixer};
pub use input::{
    AXIS_DEAD_ZONE, FrontendAction, FrontendInputMapper, GamepadDrivingState, KeyboardVkState,
    action_for_button, action_for_key, axis_action, gamepad_action_bit, gamepad_trigger_pedal,
    sdl_axis_to_winmm, sdl_keycode_to_vk,
};

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;
use std::time::{Duration, Instant};

use cmr_core::audio_assets::{FRONTEND_MUSIC, MENU_WAVE_FILES};
use cmr_core::audio_menu::{MenuChannel, menu_sound_for_input, play_menu_sound};
use cmr_core::audio_mixer::{AudioMixer, ChannelId, WaveSlotId, apply_commands};
use cmr_core::audio_music_cues::FRONTEND_CONTEXT;
use cmr_core::display::DisplayExtent;
use cmr_core::frontend::FrontendInput;
use cmr_core::host_lifecycle::{HostLifecycle, PlatformLifecycleEvent};
use cmr_core::music_player::MusicController;
use cmr_core::present_scaling::{DEFAULT_SCALING, LogicalSize, PhysicalSize, fit_viewport};
use cmr_core::root_menu_interactive::RootInteraction;
use cmr_core::root_menu_visual::RootRenderInput;
use cmr_core::window::{WindowEffect, WindowMode};
use cmr_video::{SoftwarePresentedFrame, SoftwareRgba8Error, SoftwareRgba8Frame};
use sdl3::event::{Event, WindowEvent};
use sdl3::keyboard::Keycode;
use sdl3::pixels::PixelFormat;
use sdl3::render::{Canvas, FRect, ScaleMode, Texture};
use sdl3::video::Window;

/// Target frame interval for the interactive loop's event wait (~60 Hz).
const INTERACTIVE_FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// Recovered fixed simulation cadence for the native game session (25 Hz).
///
/// This is deliberately separate from [`INTERACTIVE_FRAME_INTERVAL`]: the root
/// menu host may redraw at its own cadence, while one game-session callback
/// advances exactly one recovered fixed simulation tick.
const GAME_SESSION_TICK_INTERVAL: Duration = Duration::from_millis(40);

/// Recovered regime-A front-end/menu periodic timer interval (`FUN_004a7790`
/// registration: `0x19 = 25` ms / 40 Hz; see [`cmr_core::host_loop::menu_timer_period_ms`]).
const FRONTEND_SESSION_TICK_INTERVAL: Duration = Duration::from_millis(25);

/// Monotonic one-tick deadline used by the SDL game-session host.
///
/// Input events may wake the host before the deadline, but cannot create an
/// extra simulation tick. If rendering overruns by one or more whole periods,
/// the next deadline is resynchronized from `now`; missed ticks are not replayed
/// in a catch-up spiral. Losing focus disarms the deadline and gaining focus
/// starts a fresh full interval.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FixedStepDeadline {
    interval: Duration,
    next: Option<Duration>,
}

impl FixedStepDeadline {
    fn new(now: Duration, interval: Duration) -> Self {
        Self {
            interval,
            next: Some(now.saturating_add(interval)),
        }
    }

    fn wait_duration(self, now: Duration) -> Option<Duration> {
        self.next.map(|next| next.saturating_sub(now))
    }

    fn set_active(&mut self, active: bool, now: Duration) {
        match (active, self.next) {
            (false, Some(_)) => self.next = None,
            (true, None) => self.next = Some(now.saturating_add(self.interval)),
            _ => {}
        }
    }

    fn set_interval(&mut self, interval: Duration) {
        self.interval = interval;
    }

    /// Consume at most one due tick and schedule the next deadline.
    fn consume_due(&mut self, now: Duration) -> bool {
        let Some(deadline) = self.next else {
            return false;
        };
        if now < deadline {
            return false;
        }

        let next_regular = deadline.saturating_add(self.interval);
        self.next = Some(if next_regular <= now {
            now.saturating_add(self.interval)
        } else {
            next_regular
        });
        true
    }
}

/// SDL's timeout bridge truncates [`Duration`] to whole milliseconds. Round a
/// positive remainder up so the final sub-millisecond interval does not become
/// a zero-time busy poll.
fn sdl_wait_duration(wait: Duration) -> Duration {
    if wait.is_zero() {
        return Duration::ZERO;
    }
    let whole = wait.as_millis();
    let rounded = whole + u128::from(!wait.subsec_nanos().is_multiple_of(1_000_000));
    Duration::from_millis(u64::try_from(rounded).unwrap_or(u64::MAX))
}

/// Query the actual drawable pixel dimensions of the SDL window.
#[must_use]
pub fn physical_drawable_size(canvas: &Canvas<Window>) -> PhysicalSize {
    let (width, height) = canvas
        .output_size()
        .unwrap_or_else(|_| canvas.window().size_in_pixels());
    PhysicalSize { width, height }
}

fn toggle_fullscreen(canvas: &mut Canvas<Window>) -> Result<(), SdlHostError> {
    let window = canvas.window_mut();
    let enable = window.fullscreen_state() == sdl3::video::FullscreenType::Off;
    window
        .set_fullscreen(enable)
        .map_err(|error| SdlHostError::sdl("fullscreen toggle", error))
}

fn is_fullscreen_toggle(event: &Event) -> bool {
    matches!(
        event,
        Event::KeyDown {
            keycode: Some(Keycode::F11),
            ..
        }
    )
}

/// Platform-neutral events relevant to the first native host loop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostEvent {
    QuitRequested,
    EscapePressed,
    GamepadAdded(u32),
    GamepadRemoved(u32),
    RedrawRequested,
    RendererReset,
    FocusGained,
    FocusLost,
    Ignored,
}

impl HostEvent {
    /// The backend-neutral [`PlatformLifecycleEvent`] this event drives, if any.
    /// This is how the SDL adapter feeds the recovered host lifecycle
    /// ([`cmr_core::host_lifecycle`]): a close/quit shuts down, focus changes
    /// pause and resume. Input and redraw events are not lifecycle events.
    pub fn as_lifecycle(self) -> Option<PlatformLifecycleEvent> {
        match self {
            HostEvent::QuitRequested => Some(PlatformLifecycleEvent::CloseRequested),
            HostEvent::FocusGained => Some(PlatformLifecycleEvent::FocusGained),
            HostEvent::FocusLost => Some(PlatformLifecycleEvent::FocusLost),
            _ => None,
        }
    }
}

/// Testable lifecycle state mirrored by the SDL event adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostEventState {
    exit_requested: bool,
    focused: bool,
    connected_gamepads: BTreeSet<u32>,
}

impl Default for HostEventState {
    fn default() -> Self {
        Self {
            exit_requested: false,
            // The window starts focused once SDL creates it; a later FocusLost
            // parks the loop and a FocusGained resumes it.
            focused: true,
            connected_gamepads: BTreeSet::new(),
        }
    }
}

impl HostEventState {
    #[must_use]
    pub fn with_connected_gamepads(ids: impl IntoIterator<Item = u32>) -> Self {
        Self {
            connected_gamepads: ids.into_iter().collect(),
            ..Self::default()
        }
    }

    pub fn apply(&mut self, event: HostEvent) {
        match event {
            HostEvent::QuitRequested | HostEvent::EscapePressed => self.exit_requested = true,
            HostEvent::GamepadAdded(id) => {
                self.connected_gamepads.insert(id);
            }
            HostEvent::GamepadRemoved(id) => {
                self.connected_gamepads.remove(&id);
            }
            HostEvent::FocusGained => self.focused = true,
            HostEvent::FocusLost => self.focused = false,
            HostEvent::RedrawRequested | HostEvent::RendererReset | HostEvent::Ignored => {}
        }
    }

    /// Whether the window currently holds input focus.
    pub fn focused(&self) -> bool {
        self.focused
    }

    #[must_use]
    pub const fn exit_requested(&self) -> bool {
        self.exit_requested
    }

    #[must_use]
    pub fn connected_gamepads(&self) -> &BTreeSet<u32> {
        &self.connected_gamepads
    }
}

/// Native presentation failures kept outside compatibility crates.
#[derive(Debug)]
pub enum SdlHostError {
    FrameConversion(SoftwareRgba8Error),
    InvalidUploadLayout {
        source_pitch: usize,
        destination_pitch: usize,
        height: usize,
    },
    Sdl {
        operation: &'static str,
        detail: String,
    },
}

impl SdlHostError {
    fn sdl(operation: &'static str, source: impl fmt::Display) -> Self {
        Self::Sdl {
            operation,
            detail: source.to_string(),
        }
    }
}

impl fmt::Display for SdlHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FrameConversion(source) => source.fmt(formatter),
            Self::InvalidUploadLayout {
                source_pitch,
                destination_pitch,
                height,
            } => write!(
                formatter,
                "invalid SDL upload layout: source pitch {source_pitch}, destination pitch {destination_pitch}, height {height}"
            ),
            Self::Sdl { operation, detail } => {
                write!(formatter, "SDL {operation} failed: {detail}")
            }
        }
    }
}

impl std::error::Error for SdlHostError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::FrameConversion(source) => Some(source),
            Self::InvalidUploadLayout { .. } | Self::Sdl { .. } => None,
        }
    }
}

/// Opens a resizable SDL3 window and presents a completed software frame until
/// the user closes the window or presses Escape.
///
/// Connected SDL gamepads are opened at startup and tracked across hotplug
/// events so later input mapping can consume stable live handles.
pub fn present_software_frame(frame: &SoftwarePresentedFrame) -> Result<(), SdlHostError> {
    present_software_frame_impl(frame, true)
}

/// Opens the SDL presenter, uploads and presents one frame, then returns.
///
/// This bounded entry point exists for display-server and dummy-video smoke
/// tests. The normal application uses [`present_software_frame`] so its event
/// and gamepad lifecycle remains active.
pub fn present_software_frame_once(frame: &SoftwarePresentedFrame) -> Result<(), SdlHostError> {
    present_software_frame_impl(frame, false)
}

fn present_software_frame_impl(
    frame: &SoftwarePresentedFrame,
    wait_for_exit: bool,
) -> Result<(), SdlHostError> {
    let rgba = frame
        .expand_rgba8()
        .map_err(SdlHostError::FrameConversion)?;
    let extent = rgba.extent();

    let sdl = sdl3::init().map_err(|error| SdlHostError::sdl("initialization", error))?;
    let video = sdl
        .video()
        .map_err(|error| SdlHostError::sdl("video initialization", error))?;
    let gamepad = sdl
        .gamepad()
        .map_err(|error| SdlHostError::sdl("gamepad initialization", error))?;
    let window = video
        .window(
            "Colin McRae Rally — reconstruction",
            u32::from(extent.width),
            u32::from(extent.height),
        )
        .position_centered()
        .resizable()
        .build()
        .map_err(|error| SdlHostError::sdl("window creation", error))?;
    let mut canvas = window.into_canvas();
    let texture_creator = canvas.texture_creator();
    let mut texture = texture_creator
        .create_texture_streaming(
            PixelFormat::RGBA32,
            u32::from(extent.width),
            u32::from(extent.height),
        )
        .map_err(|error| SdlHostError::sdl("streaming texture creation", error))?;
    texture.set_scale_mode(ScaleMode::Nearest);
    upload_rgba_frame(&mut texture, &rgba)?;
    redraw_present(&mut canvas, &texture, extent)?;
    if !wait_for_exit {
        return Ok(());
    }

    let mut open_gamepads = BTreeMap::new();
    for id in gamepad
        .gamepads()
        .map_err(|error| SdlHostError::sdl("gamepad enumeration", error))?
    {
        if let Ok(handle) = gamepad.open(id) {
            open_gamepads.insert(id.raw(), handle);
        }
    }
    let mut state = HostEventState::with_connected_gamepads(open_gamepads.keys().copied());
    let mut events = sdl
        .event_pump()
        .map_err(|error| SdlHostError::sdl("event pump creation", error))?;

    while !state.exit_requested() {
        let event = events.wait_event();
        if is_fullscreen_toggle(&event) {
            toggle_fullscreen(&mut canvas)?;
        }
        let host_event = map_event(&event);
        let apply_event = match host_event {
            HostEvent::GamepadAdded(id) => {
                if let Ok(handle) = gamepad.open(id.into()) {
                    open_gamepads.insert(id, handle);
                    true
                } else {
                    false
                }
            }
            HostEvent::GamepadRemoved(id) => {
                open_gamepads.remove(&id);
                true
            }
            HostEvent::RendererReset => {
                texture = texture_creator
                    .create_texture_streaming(
                        PixelFormat::RGBA32,
                        u32::from(extent.width),
                        u32::from(extent.height),
                    )
                    .map_err(|error| SdlHostError::sdl("streaming texture recreation", error))?;
                texture.set_scale_mode(ScaleMode::Nearest);
                upload_rgba_frame(&mut texture, &rgba)?;
                redraw_present(&mut canvas, &texture, extent)?;
                true
            }
            HostEvent::RedrawRequested => {
                redraw_present(&mut canvas, &texture, extent)?;
                true
            }
            _ => true,
        };
        if apply_event {
            state.apply(host_event);
        }
    }

    Ok(())
}

fn redraw_present(
    canvas: &mut Canvas<Window>,
    texture: &Texture<'_>,
    extent: DisplayExtent,
) -> Result<(), SdlHostError> {
    let logical = LogicalSize {
        width: u32::from(extent.width),
        height: u32::from(extent.height),
    };
    let viewport = fit_viewport(logical, physical_drawable_size(canvas), DEFAULT_SCALING);
    let destination = FRect::new(
        viewport.x as f32,
        viewport.y as f32,
        viewport.width as f32,
        viewport.height as f32,
    );
    canvas.clear();
    canvas
        .copy(texture, None, destination)
        .map_err(|error| SdlHostError::sdl("frame redraw", error))?;
    if !canvas.present() {
        return Err(SdlHostError::sdl("frame redraw", sdl3::get_error()));
    }
    Ok(())
}

fn upload_rgba_frame(
    texture: &mut Texture<'_>,
    frame: &SoftwareRgba8Frame,
) -> Result<(), SdlHostError> {
    let mut upload_result = Ok(());
    texture
        .with_lock(None, |destination, destination_pitch| {
            upload_result = copy_packed_rows(
                frame.pixels(),
                frame.row_pitch(),
                destination,
                destination_pitch,
                usize::from(frame.extent().height),
            );
        })
        .map_err(|error| SdlHostError::sdl("streaming texture lock", error))?;
    upload_result
}

fn map_event(event: &Event) -> HostEvent {
    match event {
        Event::Quit { .. } => HostEvent::QuitRequested,
        Event::KeyDown {
            keycode: Some(Keycode::Escape),
            ..
        } => HostEvent::EscapePressed,
        Event::GamepadAdded { which, .. } => HostEvent::GamepadAdded(which.raw()),
        Event::GamepadRemoved { which, .. } => HostEvent::GamepadRemoved(which.raw()),
        Event::Window {
            win_event: WindowEvent::FocusGained,
            ..
        } => HostEvent::FocusGained,
        Event::Window {
            win_event: WindowEvent::FocusLost | WindowEvent::Minimized,
            ..
        } => HostEvent::FocusLost,
        Event::Window {
            win_event:
                WindowEvent::Exposed
                | WindowEvent::Resized(_, _)
                | WindowEvent::PixelSizeChanged(_, _)
                | WindowEvent::Restored,
            ..
        }
        | Event::RenderTargetsReset { .. } => HostEvent::RedrawRequested,
        Event::RenderDeviceReset { .. } => HostEvent::RendererReset,
        _ => HostEvent::Ignored,
    }
}

fn copy_packed_rows(
    source: &[u8],
    source_pitch: usize,
    destination: &mut [u8],
    destination_pitch: usize,
    height: usize,
) -> Result<(), SdlHostError> {
    let source_len = source_pitch.checked_mul(height);
    let destination_len = destination_pitch.checked_mul(height);
    if destination_pitch < source_pitch
        || source_len != Some(source.len())
        || destination_len.is_none_or(|required| destination.len() < required)
    {
        return Err(SdlHostError::InvalidUploadLayout {
            source_pitch,
            destination_pitch,
            height,
        });
    }
    for row in 0..height {
        let source_start = row * source_pitch;
        let destination_start = row * destination_pitch;
        destination[destination_start..destination_start + source_pitch]
            .copy_from_slice(&source[source_start..source_start + source_pitch]);
    }
    Ok(())
}

/// A failure while running the interactive root-menu loop.
#[derive(Debug)]
pub enum InteractiveRootError<E> {
    /// A host (SDL) presentation or window-system failure.
    Host(SdlHostError),
    /// A failure returned by the caller's frame-render callback.
    Render(E),
}

impl<E: fmt::Display> fmt::Display for InteractiveRootError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Host(source) => source.fmt(formatter),
            Self::Render(source) => write!(formatter, "interactive render failed: {source}"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for InteractiveRootError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Host(source) => Some(source),
            Self::Render(source) => Some(source),
        }
    }
}

fn elapsed_ms(clock: Instant) -> u32 {
    clock.elapsed().as_millis() as u32
}

fn rendered_rgba<E>(
    frame: SoftwarePresentedFrame,
) -> Result<SoftwareRgba8Frame, InteractiveRootError<E>> {
    frame
        .expand_rgba8()
        .map_err(|error| InteractiveRootError::Host(SdlHostError::FrameConversion(error)))
}

/// Runs the interactive root-menu loop until the user exits.
///
/// SDL owns the window, event queue, gamepads, and presentation. Navigation
/// and exit come from the recovered [`RootInteraction`]; the `render` callback
/// turns each distinct recovered [`RootRenderInput`] into a completed software
/// frame (typically by driving the compatibility session). The frame is only
/// re-rendered when the controller's render input actually changes.
///
/// `Escape` and the window close button exit, as in the static presenter. The
/// controller supplies a monotonic millisecond clock so its recovered idle and
/// attract timing can elapse.
pub fn run_interactive_root_frame<E>(
    mut interaction: RootInteraction,
    save_allowed: bool,
    source_root: Option<&Path>,
    mut render: impl FnMut(&RootRenderInput) -> Result<SoftwarePresentedFrame, E>,
) -> Result<(), InteractiveRootError<E>> {
    let clock = Instant::now();

    // Settle the recovered controller into the interactive root screen and
    // render its first frame before opening a window.
    let mut current_input = loop {
        let tick = interaction.tick(FrontendInput::default(), elapsed_ms(clock), save_allowed);
        if tick.exit_requested {
            return Ok(());
        }
        if let Some(input) = tick.render_input {
            break input;
        }
    };
    let mut rgba = rendered_rgba(render(&current_input).map_err(InteractiveRootError::Render)?)?;
    let extent = rgba.extent();

    let host = InteractiveRootError::Host;
    let sdl = sdl3::init().map_err(|error| host(SdlHostError::sdl("initialization", error)))?;
    let video = sdl
        .video()
        .map_err(|error| host(SdlHostError::sdl("video initialization", error)))?;
    let gamepad = sdl
        .gamepad()
        .map_err(|error| host(SdlHostError::sdl("gamepad initialization", error)))?;
    let window = video
        .window(
            "Colin McRae Rally — reconstruction",
            u32::from(extent.width),
            u32::from(extent.height),
        )
        .position_centered()
        .resizable()
        .build()
        .map_err(|error| host(SdlHostError::sdl("window creation", error)))?;
    let mut canvas = window.into_canvas();
    let texture_creator = canvas.texture_creator();
    let mut texture = texture_creator
        .create_texture_streaming(
            PixelFormat::RGBA32,
            u32::from(extent.width),
            u32::from(extent.height),
        )
        .map_err(|error| host(SdlHostError::sdl("streaming texture creation", error)))?;
    texture.set_scale_mode(ScaleMode::Nearest);
    upload_rgba_frame(&mut texture, &rgba).map_err(InteractiveRootError::Host)?;
    redraw_present(&mut canvas, &texture, extent).map_err(InteractiveRootError::Host)?;

    let mut open_gamepads = BTreeMap::new();
    for id in gamepad
        .gamepads()
        .map_err(|error| host(SdlHostError::sdl("gamepad enumeration", error)))?
    {
        if let Ok(handle) = gamepad.open(id) {
            open_gamepads.insert(id.raw(), handle);
        }
    }
    let mut host_state = HostEventState::with_connected_gamepads(open_gamepads.keys().copied());
    // The window is created focused; drive focus/pause/restore/shutdown through
    // the recovered host lifecycle (B12) as SDL reports focus and close events.
    let mut lifecycle = HostLifecycle::new(WindowMode::Windowed);
    lifecycle.apply(PlatformLifecycleEvent::FocusGained);
    let mut mapper = FrontendInputMapper::new();
    let mut events = sdl
        .event_pump()
        .map_err(|error| host(SdlHostError::sdl("event pump creation", error)))?;

    // Best-effort front-end menu SFX: open the audio device and register the
    // recovered four menu waves (their install-root-relative leading separator
    // stripped for the source-rooted wave reader) into consecutive slots 0..3 on
    // channel 0, so navigating the menu plays the recovered one-shot cues. Any
    // failure (no source root / no device / missing wave) leaves `menu_audio` as
    // None and navigation stays silent.
    let mut menu_audio = source_root.and_then(|root| {
        let audio_host = AudioHost::open(&sdl, root).ok()?;
        let shared = audio_host.mixer();
        {
            let mut mixer = shared.lock().ok()?;
            for file in MENU_WAVE_FILES {
                let name = file.trim_start_matches(['\\', '/']);
                mixer.load_wave(name, 22_050, 16, false).ok()?;
            }
        }
        let menu = MenuChannel {
            channel: ChannelId::new(0).ok()?,
            base_slot: WaveSlotId::from_ordinal(0),
        };
        Some((audio_host, menu))
    });

    while !host_state.exit_requested() {
        let mut pending = Vec::new();
        if let Some(event) = events.wait_event_timeout(INTERACTIVE_FRAME_INTERVAL) {
            pending.push(event);
            pending.extend(events.poll_iter());
        }
        let mut host_events = Vec::with_capacity(pending.len());
        for event in pending {
            if is_fullscreen_toggle(&event) {
                toggle_fullscreen(&mut canvas).map_err(host)?;
            }
            feed_input_event(&event, &mut mapper);
            let host_event = map_event(&event);
            host_events.push(host_event);
            match host_event {
                HostEvent::GamepadAdded(id) => {
                    if let Ok(handle) = gamepad.open(id.into()) {
                        open_gamepads.insert(id, handle);
                        host_state.apply(HostEvent::GamepadAdded(id));
                    }
                }
                HostEvent::GamepadRemoved(id) => {
                    open_gamepads.remove(&id);
                    host_state.apply(HostEvent::GamepadRemoved(id));
                }
                HostEvent::RendererReset => {
                    texture = texture_creator
                        .create_texture_streaming(
                            PixelFormat::RGBA32,
                            u32::from(extent.width),
                            u32::from(extent.height),
                        )
                        .map_err(|error| {
                            host(SdlHostError::sdl("streaming texture recreation", error))
                        })?;
                    texture.set_scale_mode(ScaleMode::Nearest);
                    upload_rgba_frame(&mut texture, &rgba).map_err(InteractiveRootError::Host)?;
                    redraw_present(&mut canvas, &texture, extent)
                        .map_err(InteractiveRootError::Host)?;
                }
                HostEvent::RedrawRequested => {
                    redraw_present(&mut canvas, &texture, extent)
                        .map_err(InteractiveRootError::Host)?;
                }
                other => host_state.apply(other),
            }
        }
        let decision = advance_root_lifecycle(&host_events, &mut lifecycle);
        if host_state.exit_requested() || decision.quit {
            break;
        }
        if decision.restore_surfaces {
            redraw_present(&mut canvas, &texture, extent).map_err(InteractiveRootError::Host)?;
        }
        if !decision.active {
            // Unfocused: pause ticking and rendering; block on the next event.
            continue;
        }

        let input = mapper.poll();
        // Play the recovered menu SFX for this tick's input edge (confirm /
        // cancel / vertical move) on the shared software mixer, before the
        // controller consumes the input.
        if let Some(sound) = menu_sound_for_input(input)
            && let Some((audio_host, menu)) = menu_audio.as_mut()
            && let Ok(mut mixer) = audio_host.mixer().lock()
        {
            let _ = apply_commands(&mut *mixer, &play_menu_sound(*menu, sound, 100));
        }
        let tick = interaction.tick(input, elapsed_ms(clock), save_allowed);
        if tick.exit_requested {
            break;
        }
        if let Some(render_input) = tick.render_input
            && render_input != current_input
        {
            current_input = render_input;
            rgba = rendered_rgba(render(&current_input).map_err(InteractiveRootError::Render)?)?;
            upload_rgba_frame(&mut texture, &rgba).map_err(InteractiveRootError::Host)?;
            redraw_present(&mut canvas, &texture, extent).map_err(InteractiveRootError::Host)?;
        }
    }

    Ok(())
}

/// The live input handed to the `step` callback of [`run_windowed_game_session`]
/// each tick: the held-key state always, and the gamepad driving state when at
/// least one gamepad is connected (so the caller can pick the recovered keyboard
/// or analog driver).
pub struct WindowedInput<'a> {
    /// The live held-VK keyboard state.
    pub keys: &'a KeyboardVkState,
    /// The live gamepad driving state, or `None` when no gamepad is connected.
    pub gamepad: Option<&'a GamepadDrivingState>,
    /// Edge-triggered menu input from the same keyboard/gamepad event stream.
    pub frontend: FrontendInput,
    /// The open software mixer, when the audio device opened, for game-side
    /// sound that must run inside the simulation tick (the engine sound).
    pub audio: Option<&'a SharedSoftwareMixer>,
}

/// One windowed game-session frame, returned by the `step` callback of
/// [`run_windowed_game_session`].
pub struct WindowedGameFrame {
    /// The software frame to present this tick (e.g. the stage composed from a
    /// follow camera), or `None` to clear to black when the active render route
    /// composes no world scene.
    pub present: Option<SoftwarePresentedFrame>,
    /// Whether the session has reached the terminal stopped state.
    pub stopped: bool,
    /// Whether the recovered front-end music should play this frame (true during
    /// the front-end/menu band; false while racing or stopped).
    pub music_active: bool,
    /// Co-driver note words submitted this frame (`FUN_00444380`), spoken by
    /// the recovered sequencer `FUN_00444450` on the speech channel.
    pub codriver_calls: Vec<u32>,
    /// The music volume byte (`live[7]`) and fade target byte (`img[0xf]`)
    /// the record holds, when the caller has the record; otherwise the
    /// harness bytes stay.
    pub music_config: Option<[u8; 2]>,
}

/// The documented harness music config byte for the windowed session's front-end
/// music, used for both the full-volume and fade-target config inputs. The
/// shipped game reads these from the runtime config (`DAT_00d3ca80[7]` and
/// `0x00d3ac60[0xf]`, both `& 0x7f`), which are not in the static image; `8`
/// scales (via the recovered `music_volume` = `cfg*400000/100`) to ~`0x7d00`,
/// near the mixer's full scale, reached over an `8*0x14 = 160`-frame fade-in
/// (the recovered `music_fade_volume` ramp lands exactly on that full volume).
const FRONTEND_MUSIC_CONFIG_BYTE: u8 = 8;

/// Runs the windowed game session: drives the recovered top-level FSM with live
/// keyboard input and presents the frame the recovered render route selected,
/// until the session stops or the window closes.
///
/// SDL owns the window, keyboard, and presentation; the recovered game logic is
/// entirely in the `step` callback, which is handed the live held-key state (the
/// [`KeyboardVkState`] the loop maintains from key events through
/// [`sdl_keycode_to_vk`]) and returns that tick's [`WindowedGameFrame`]: the
/// software frame to present (the caller composes it per the recovered render
/// route, e.g. the stage from a follow camera) or `None` to clear. `initial`
/// sizes the window and is presented first. `Escape` is left to the game (its
/// recovered pause binding, VK `0x1b`); the window close button and a quit event
/// exit. Focus loss pauses the stepping, as the recovered host loop does.
pub fn run_windowed_game_session<F>(
    initial: &SoftwarePresentedFrame,
    source_root: Option<&Path>,
    mut step: F,
) -> Result<(), SdlHostError>
where
    F: FnMut(WindowedInput<'_>) -> WindowedGameFrame,
{
    let rgba = initial
        .expand_rgba8()
        .map_err(SdlHostError::FrameConversion)?;
    let extent = rgba.extent();

    let sdl = sdl3::init().map_err(|error| SdlHostError::sdl("initialization", error))?;
    let video = sdl
        .video()
        .map_err(|error| SdlHostError::sdl("video initialization", error))?;
    let gamepad = sdl
        .gamepad()
        .map_err(|error| SdlHostError::sdl("gamepad initialization", error))?;
    let window = video
        .window(
            "Colin McRae Rally — reconstruction",
            u32::from(extent.width),
            u32::from(extent.height),
        )
        .position_centered()
        .resizable()
        .build()
        .map_err(|error| SdlHostError::sdl("window creation", error))?;
    let mut canvas = window.into_canvas();
    let texture_creator = canvas.texture_creator();
    let mut texture = texture_creator
        .create_texture_streaming(
            PixelFormat::RGBA32,
            u32::from(extent.width),
            u32::from(extent.height),
        )
        .map_err(|error| SdlHostError::sdl("streaming texture creation", error))?;
    texture.set_scale_mode(ScaleMode::Nearest);
    upload_rgba_frame(&mut texture, &rgba)?;
    redraw_present(&mut canvas, &texture, extent)?;

    let mut events = sdl
        .event_pump()
        .map_err(|error| SdlHostError::sdl("event pump creation", error))?;
    let mut keys = KeyboardVkState::new();
    let mut pad = GamepadDrivingState::new();
    let mut frontend = FrontendInputMapper::new();
    // Open any already-connected gamepads; hotplug is handled in the loop. The
    // handles must be kept alive for the device to keep sending events.
    let mut open_gamepads = BTreeMap::new();
    if let Ok(ids) = gamepad.gamepads() {
        for id in ids {
            if let Ok(handle) = gamepad.open(id) {
                open_gamepads.insert(id.raw(), handle);
            }
        }
    }
    // Drive focus/pause/restore/shutdown through the recovered host lifecycle
    // (B12), seeded focused at window creation, exactly as the interactive root
    // loop does.
    let mut lifecycle = HostLifecycle::new(WindowMode::Windowed);
    lifecycle.apply(PlatformLifecycleEvent::FocusGained);

    // Best-effort front-end music: open the audio device, register the recovered
    // front-end music wave (its shipped path is install-root-relative, so its
    // leading separator is stripped for the source-rooted wave reader), and build
    // the recovered music controller on channel 0. Any failure (no source root,
    // no device, missing wave) leaves `audio` as None and the session runs
    // silent.
    let host = source_root.and_then(|root| AudioHost::open(&sdl, root).ok());
    let mut music = host.as_ref().and_then(|host| {
        let name = FRONTEND_MUSIC.trim_start_matches(['\\', '/']);
        let slot = host
            .mixer()
            .lock()
            .ok()?
            .load_wave(name, 22_050, 16, false)
            .ok()?;
        Some(MusicController::new(
            ChannelId::new(0).ok()?,
            slot,
            FRONTEND_MUSIC_CONFIG_BYTE,
            FRONTEND_MUSIC_CONFIG_BYTE,
        ))
    });
    let mut speech = host.as_ref().and_then(SpeechBank::load);
    let game_mixer = host.as_ref().map(AudioHost::mixer);

    let pacing_clock = Instant::now();
    let mut pacing = FixedStepDeadline::new(Duration::ZERO, GAME_SESSION_TICK_INTERVAL);

    loop {
        let mut host_events = Vec::new();
        let now = pacing_clock.elapsed();
        let first_event = match pacing.wait_duration(now) {
            Some(wait) => events.wait_event_timeout(sdl_wait_duration(wait)),
            None => Some(events.wait_event()),
        };
        if let Some(event) = first_event {
            for event in std::iter::once(event).chain(events.poll_iter()) {
                feed_input_event(&event, &mut frontend);
                if is_fullscreen_toggle(&event) {
                    toggle_fullscreen(&mut canvas)?;
                }
                match event {
                    Event::KeyDown {
                        keycode: Some(code),
                        ..
                    } => keys.apply_key(code, true),
                    Event::KeyUp {
                        keycode: Some(code),
                        ..
                    } => keys.apply_key(code, false),
                    Event::GamepadAxisMotion { axis, value, .. } => {
                        pad.apply_axis(axis, value);
                        keys.apply_gamepad_axis(axis, value);
                    }
                    Event::GamepadButtonDown { button, .. } => {
                        pad.apply_button(button, true);
                        keys.apply_gamepad_button(button, true);
                    }
                    Event::GamepadButtonUp { button, .. } => {
                        pad.apply_button(button, false);
                        keys.apply_gamepad_button(button, false);
                    }
                    Event::GamepadAdded { which, .. } => {
                        if let Ok(handle) = gamepad.open(which) {
                            open_gamepads.insert(which.raw(), handle);
                        }
                    }
                    Event::GamepadRemoved { which, .. } => {
                        open_gamepads.remove(&which.raw());
                    }
                    _ => {}
                }
                host_events.push(map_event(&event));
            }
        }
        let decision = advance_root_lifecycle(&host_events, &mut lifecycle);
        if decision.quit {
            break;
        }
        if decision.restore_surfaces {
            redraw_present(&mut canvas, &texture, extent)?;
        }
        let now = pacing_clock.elapsed();
        pacing.set_active(decision.active, now);
        if !decision.active {
            // Unfocused: pause stepping and presenting; block on the next event.
            continue;
        }
        if !pacing.consume_due(now) {
            // Input and window events remain responsive, but an early wakeup
            // cannot advance the fixed-step game clock.
            continue;
        }

        let input = WindowedInput {
            keys: &keys,
            gamepad: (!open_gamepads.is_empty()).then_some(&pad),
            frontend: frontend.poll(),
            audio: game_mixer.as_ref(),
        };
        let frame = step(input);
        pacing.set_interval(if frame.music_active {
            FRONTEND_SESSION_TICK_INTERVAL
        } else {
            GAME_SESSION_TICK_INTERVAL
        });
        if frame.stopped {
            break;
        }
        // Drive the recovered front-end music: start/fade it in during the
        // front-end band (entering the frontend context gate), stop it otherwise,
        // applying the emitted mixer commands to the shared software mixer.
        if let Some(host) = host.as_ref()
            && let Some(speech) = speech.as_mut()
            && let Ok(mut mixer) = host.mixer().lock()
        {
            let commands = speech.frame(&*mixer, &frame.codriver_calls);
            let _ = apply_commands(&mut *mixer, &commands);
        }
        if let Some(host) = host.as_ref()
            && let Some(music) = music.as_mut()
            && let Ok(mut mixer) = host.mixer().lock()
        {
            if let Some([volume, fade]) = frame.music_config {
                music.set_config(volume, fade);
            }
            let commands = if frame.music_active {
                if music.is_active() {
                    let busy = mixer.is_channel_busy(music.channel());
                    music.keepalive(busy)
                } else {
                    music.on_context(FRONTEND_CONTEXT)
                }
            } else if music.is_active() {
                music.stop()
            } else {
                Vec::new()
            };
            let _ = apply_commands(&mut *mixer, &commands);
        }
        match frame.present {
            Some(presented) => {
                let rgba = presented
                    .expand_rgba8()
                    .map_err(SdlHostError::FrameConversion)?;
                upload_rgba_frame(&mut texture, &rgba)?;
                redraw_present(&mut canvas, &texture, extent)?;
            }
            None => {
                // A route that composes no world scene: clear to black until the
                // recovered front-end/overlay compositors are wired for this path.
                canvas.clear();
                if !canvas.present() {
                    return Err(SdlHostError::sdl("frame presentation", sdl3::get_error()));
                }
            }
        }
    }

    Ok(())
}

/// The co-driver speech channel (`FUN_00444160`'s allocation, 14 in the
/// native stage-load order: pause cues 0, engine 1..=8, prangs 9..=13).
pub const SPEECH_CHANNEL: u32 = 14;

/// The co-driver speech bank (`FUN_00444160`: the 40 pacenote waves at
/// 11025 Hz / 16-bit / mono, the speech channel at volume `0x7d00`) and the
/// recovered sequencer that drains the queued note words one sample per
/// frame. Harness choices: English speech (the language byte
/// `DAT_00d3ca80[0x18] & 7` is runtime config) and a normal (unmirrored)
/// stage; the channel follows the native allocation order ([`SPEECH_CHANNEL`]).
struct SpeechBank {
    channel: ChannelId,
    slots: Vec<Option<WaveSlotId>>,
    sequencer: cmr_core::pacenote_schedule::CodriverSequencer,
}

impl SpeechBank {
    fn load(host: &AudioHost) -> Option<Self> {
        use cmr_core::audio_pacenote::{
            PACENOTE_WAVE_BITS, PACENOTE_WAVE_RATE_HZ, PACENOTE_WAVES, codriver_source_path,
            select_speech_language,
        };
        let language = select_speech_language(0, false);
        let shared = host.mixer();
        let mut mixer = shared.lock().ok()?;
        let slots: Vec<Option<WaveSlotId>> = PACENOTE_WAVES
            .iter()
            .map(|file| {
                mixer
                    .load_wave(
                        &codriver_source_path(language, file),
                        PACENOTE_WAVE_RATE_HZ,
                        PACENOTE_WAVE_BITS,
                        false,
                    )
                    .ok()
            })
            .collect();
        if slots.iter().all(Option::is_none) {
            return None;
        }
        let channel = ChannelId::new(SPEECH_CHANNEL).ok()?;
        let _ = apply_commands(
            &mut *mixer,
            &[cmr_core::audio_mixer::MixerCommand::SetVolume {
                channel,
                volume: cmr_core::audio_pacenote::PACENOTE_CHANNEL_VOLUME,
            }],
        );
        Some(Self {
            channel,
            slots,
            sequencer: cmr_core::pacenote_schedule::CodriverSequencer::default(),
        })
    }

    /// One frame: the sequencer runs before this frame's calls are queued
    /// (`FUN_00444450` precedes the mechanics that submit them).
    fn frame<M: AudioMixer>(
        &mut self,
        mixer: &M,
        calls: &[u32],
    ) -> Vec<cmr_core::audio_mixer::MixerCommand> {
        use cmr_core::audio_mixer::MixerCommand;
        let mut commands = Vec::new();
        let busy = mixer.is_channel_busy(self.channel);
        if let Some(slot) = self.sequencer.tick(busy)
            && let Some(Some(wave)) = usize::try_from(slot).ok().and_then(|i| self.slots.get(i))
        {
            commands.push(MixerCommand::Play {
                slot: *wave,
                channel: self.channel,
                flags: cmr_core::audio_pacenote::PACENOTE_PLAY_FLAGS,
            });
            commands.push(MixerCommand::SetVolume {
                channel: self.channel,
                volume: cmr_core::audio_pacenote::PACENOTE_CHANNEL_VOLUME,
            });
        }
        for &word in calls {
            self.sequencer.submit(word, false);
        }
        commands
    }
}

/// What the interactive loop should do after a batch of host events.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RootLoopDecision {
    /// A shutdown was requested (window close / quit).
    pub quit: bool,
    /// The window is focused and the loop should tick and render.
    pub active: bool,
    /// A focus gain restored lost surfaces; the frame must be re-presented.
    pub restore_surfaces: bool,
}

/// Fold the lifecycle-relevant host events in a batch through the recovered
/// host lifecycle ([`cmr_core::host_lifecycle`]) and summarize what the
/// interactive native loop should do: quit, pause (when unfocused), or
/// re-present after a focus gain restored surfaces. Non-lifecycle events
/// (input, gamepad, redraw) are ignored here and handled by the loop directly.
pub fn advance_root_lifecycle(
    events: &[HostEvent],
    lifecycle: &mut HostLifecycle,
) -> RootLoopDecision {
    let mut restore_surfaces = false;
    for event in events {
        if let Some(lifecycle_event) = event.as_lifecycle() {
            let transition = lifecycle.apply(lifecycle_event);
            if transition
                .effects
                .iter()
                .any(|effect| matches!(effect, WindowEffect::RestoreLostSurfaces))
            {
                restore_surfaces = true;
            }
        }
    }
    RootLoopDecision {
        quit: lifecycle.is_shutting_down(),
        active: lifecycle.is_active(),
        restore_surfaces,
    }
}

fn feed_input_event(event: &Event, mapper: &mut FrontendInputMapper) {
    match event {
        Event::KeyDown {
            keycode: Some(keycode),
            ..
        } => mapper.apply_key(*keycode, true),
        Event::KeyUp {
            keycode: Some(keycode),
            ..
        } => mapper.apply_key(*keycode, false),
        Event::GamepadButtonDown { button, .. } => mapper.apply_button(*button, true),
        Event::GamepadButtonUp { button, .. } => mapper.apply_button(*button, false),
        Event::GamepadAxisMotion { axis, value, .. } => mapper.apply_axis(*axis, *value),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_speech_bank_speaks_a_queued_call_from_the_next_frame() {
        use cmr_core::audio_mixer::MixerCommand;
        use cmr_core::audio_render::{FileWaveSource, SoftwareMixer};
        let mixer = SoftwareMixer::with_source(FileWaveSource::new(std::env::temp_dir()));
        let mut bank = SpeechBank {
            channel: ChannelId::new(1).unwrap(),
            slots: (0..40)
                .map(|i| Some(WaveSlotId::from_ordinal(100 + i)))
                .collect(),
            sequencer: cmr_core::pacenote_schedule::CodriverSequencer::default(),
        };
        // "3": the number field at bits 3-6 speaks slot 2 + 3.
        assert!(bank.frame(&mixer, &[3 << 3]).is_empty());
        let commands = bank.frame(&mixer, &[]);
        assert_eq!(
            commands,
            [
                MixerCommand::Play {
                    slot: WaveSlotId::from_ordinal(105),
                    channel: bank.channel,
                    flags: cmr_core::audio_pacenote::PACENOTE_PLAY_FLAGS,
                },
                MixerCommand::SetVolume {
                    channel: bank.channel,
                    volume: cmr_core::audio_pacenote::PACENOTE_CHANNEL_VOLUME,
                },
            ]
        );
        // The spent entry advances with nothing behind it: silence.
        assert!(bank.frame(&mixer, &[]).is_empty());
    }

    #[test]
    fn fixed_step_deadline_ignores_early_event_wakeups() {
        let mut pacing = FixedStepDeadline::new(Duration::ZERO, GAME_SESSION_TICK_INTERVAL);

        assert_eq!(
            pacing.wait_duration(Duration::ZERO),
            Some(Duration::from_millis(40))
        );
        assert!(!pacing.consume_due(Duration::from_millis(7)));
        assert_eq!(
            pacing.wait_duration(Duration::from_millis(7)),
            Some(Duration::from_millis(33))
        );
        assert!(!pacing.consume_due(Duration::from_millis(39)));
    }

    #[test]
    fn fixed_step_deadline_ticks_once_at_the_exact_deadline() {
        let mut pacing = FixedStepDeadline::new(Duration::ZERO, GAME_SESSION_TICK_INTERVAL);

        assert!(pacing.consume_due(Duration::from_millis(40)));
        assert_eq!(
            pacing.wait_duration(Duration::from_millis(40)),
            Some(Duration::from_millis(40))
        );
        assert!(!pacing.consume_due(Duration::from_millis(40)));
        assert!(pacing.consume_due(Duration::from_millis(80)));
    }

    #[test]
    fn fixed_step_deadline_resynchronizes_an_overrun_without_catchup() {
        let mut pacing = FixedStepDeadline::new(Duration::ZERO, GAME_SESSION_TICK_INTERVAL);

        assert!(pacing.consume_due(Duration::from_millis(130)));
        assert_eq!(
            pacing.wait_duration(Duration::from_millis(130)),
            Some(Duration::from_millis(40))
        );
        assert!(!pacing.consume_due(Duration::from_millis(130)));
        assert!(!pacing.consume_due(Duration::from_millis(169)));
        assert!(pacing.consume_due(Duration::from_millis(170)));
    }

    #[test]
    fn fixed_step_deadline_pauses_and_restarts_after_focus_gain() {
        let mut pacing = FixedStepDeadline::new(Duration::ZERO, GAME_SESSION_TICK_INTERVAL);

        pacing.set_active(false, Duration::from_millis(15));
        assert_eq!(pacing.wait_duration(Duration::from_millis(100)), None);
        assert!(!pacing.consume_due(Duration::from_millis(100)));

        pacing.set_active(true, Duration::from_millis(100));
        assert_eq!(
            pacing.wait_duration(Duration::from_millis(100)),
            Some(Duration::from_millis(40))
        );
        assert!(!pacing.consume_due(Duration::from_millis(139)));
        assert!(pacing.consume_due(Duration::from_millis(140)));
    }

    #[test]
    fn repeated_event_checks_at_one_timestamp_never_add_ticks() {
        let mut pacing = FixedStepDeadline::new(Duration::ZERO, GAME_SESSION_TICK_INTERVAL);

        for _ in 0..100 {
            assert!(!pacing.consume_due(Duration::from_millis(20)));
        }
        assert!(pacing.consume_due(Duration::from_millis(40)));
        for _ in 0..100 {
            assert!(!pacing.consume_due(Duration::from_millis(40)));
        }
    }

    #[test]
    fn sdl_wait_rounds_positive_fractional_milliseconds_up() {
        assert_eq!(sdl_wait_duration(Duration::ZERO), Duration::ZERO);
        assert_eq!(
            sdl_wait_duration(Duration::from_nanos(1)),
            Duration::from_millis(1)
        );
        assert_eq!(
            sdl_wait_duration(Duration::from_millis(7)),
            Duration::from_millis(7)
        );
        assert_eq!(
            sdl_wait_duration(Duration::from_micros(7_001)),
            Duration::from_millis(8)
        );
    }

    #[test]
    fn interactive_lifecycle_pauses_on_focus_loss_and_resumes_on_gain() {
        let mut lifecycle = HostLifecycle::new(WindowMode::Windowed);
        lifecycle.apply(PlatformLifecycleEvent::FocusGained);

        // Non-lifecycle events leave the loop active and rendering.
        let decision = advance_root_lifecycle(
            &[HostEvent::RedrawRequested, HostEvent::GamepadAdded(1)],
            &mut lifecycle,
        );
        assert_eq!(
            decision,
            RootLoopDecision {
                quit: false,
                active: true,
                restore_surfaces: false,
            }
        );

        // Focus loss pauses the loop.
        let decision = advance_root_lifecycle(&[HostEvent::FocusLost], &mut lifecycle);
        assert!(!decision.active);
        assert!(!decision.quit);

        // Focus gain resumes and restores the surfaces owed by the loss.
        let decision = advance_root_lifecycle(&[HostEvent::FocusGained], &mut lifecycle);
        assert!(decision.active);
        assert!(decision.restore_surfaces);

        // A close request quits regardless of focus.
        let decision = advance_root_lifecycle(&[HostEvent::QuitRequested], &mut lifecycle);
        assert!(decision.quit);
    }

    #[test]
    fn host_events_bridge_to_backend_neutral_lifecycle_events() {
        assert_eq!(
            HostEvent::QuitRequested.as_lifecycle(),
            Some(PlatformLifecycleEvent::CloseRequested)
        );
        assert_eq!(
            HostEvent::FocusGained.as_lifecycle(),
            Some(PlatformLifecycleEvent::FocusGained)
        );
        assert_eq!(
            HostEvent::FocusLost.as_lifecycle(),
            Some(PlatformLifecycleEvent::FocusLost)
        );
        // Input and redraw events are not lifecycle events.
        for event in [
            HostEvent::EscapePressed,
            HostEvent::GamepadAdded(1),
            HostEvent::RedrawRequested,
            HostEvent::RendererReset,
            HostEvent::Ignored,
        ] {
            assert_eq!(event.as_lifecycle(), None);
        }
    }

    #[test]
    fn focus_events_track_window_focus_without_requesting_exit() {
        let mut state = HostEventState::default();
        assert!(state.focused());
        state.apply(HostEvent::FocusLost);
        assert!(!state.focused());
        assert!(!state.exit_requested());
        state.apply(HostEvent::FocusGained);
        assert!(state.focused());
        assert!(!state.exit_requested());
    }

    #[test]
    fn quit_and_escape_are_terminal_host_events() {
        for event in [HostEvent::QuitRequested, HostEvent::EscapePressed] {
            let mut state = HostEventState::default();
            state.apply(event);
            assert!(state.exit_requested());
        }
    }

    #[test]
    fn gamepad_hotplug_is_idempotent_and_preserves_other_devices() {
        let mut state = HostEventState::with_connected_gamepads([7, 9]);
        state.apply(HostEvent::GamepadAdded(9));
        state.apply(HostEvent::GamepadAdded(11));
        state.apply(HostEvent::GamepadRemoved(7));
        state.apply(HostEvent::GamepadRemoved(404));

        assert_eq!(state.connected_gamepads(), &BTreeSet::from([9, 11]));
        assert!(!state.exit_requested());
    }

    #[test]
    fn unrelated_events_do_not_change_host_state() {
        let mut state = HostEventState::with_connected_gamepads([3]);
        let before = state.clone();
        state.apply(HostEvent::Ignored);
        assert_eq!(state, before);
    }

    #[test]
    fn packed_rows_respect_an_sdl_owned_destination_pitch() {
        let source = [1, 2, 3, 4, 5, 6, 7, 8];
        let mut destination = [0xaa; 12];

        copy_packed_rows(&source, 4, &mut destination, 6, 2).unwrap();

        assert_eq!(
            destination,
            [1, 2, 3, 4, 0xaa, 0xaa, 5, 6, 7, 8, 0xaa, 0xaa]
        );
    }

    #[test]
    fn f11_is_the_fullscreen_toggle() {
        let event = Event::KeyDown {
            timestamp: 0,
            window_id: 1,
            keycode: Some(Keycode::F11),
            scancode: None,
            keymod: sdl3::keyboard::Mod::empty(),
            repeat: false,
            raw: 0,
            which: 0,
        };
        assert!(is_fullscreen_toggle(&event));
        let other = Event::KeyDown {
            timestamp: 0,
            window_id: 1,
            keycode: Some(Keycode::F10),
            scancode: None,
            keymod: sdl3::keyboard::Mod::empty(),
            repeat: false,
            raw: 0,
            which: 0,
        };
        assert!(!is_fullscreen_toggle(&other));
    }

    #[test]
    #[ignore = "requires an SDL video driver"]
    fn fullscreen_toggle_and_physical_size_work() {
        let sdl = sdl3::init().unwrap();
        let video = sdl.video().unwrap();
        let window = video
            .window("fullscreen contract", 32, 24)
            .resizable()
            .build()
            .unwrap();
        let mut canvas = window.into_canvas();
        assert_eq!(
            physical_drawable_size(&canvas),
            PhysicalSize {
                width: 32,
                height: 24,
            }
        );
        toggle_fullscreen(&mut canvas).unwrap();
        assert_ne!(
            canvas.window().fullscreen_state(),
            sdl3::video::FullscreenType::Off
        );
        toggle_fullscreen(&mut canvas).unwrap();
        assert_eq!(
            canvas.window().fullscreen_state(),
            sdl3::video::FullscreenType::Off
        );
    }

    #[test]
    fn packed_rows_reject_short_buffers_and_narrow_destination_pitch() {
        let mut destination = [0; 8];
        assert!(matches!(
            copy_packed_rows(&[1; 7], 4, &mut destination, 4, 2),
            Err(SdlHostError::InvalidUploadLayout { .. })
        ));
        assert!(matches!(
            copy_packed_rows(&[1; 8], 4, &mut destination, 3, 2),
            Err(SdlHostError::InvalidUploadLayout { .. })
        ));
    }
}
