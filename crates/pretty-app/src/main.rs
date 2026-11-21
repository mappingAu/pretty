//! Pretty — a hardware accelerated, kitty-compatible terminal emulator.

mod app;
mod config;
mod i18n;
mod menu;
mod platform;
mod settings;

// The Android entry point is `android_main` below, which needs winit's
// NativeActivity glue (`winit/android-native-activity`). Without it the app
// would build a desktop event loop that can never be attached to a surface.
#[cfg(all(target_os = "android", not(feature = "android")))]
compile_error!(
    "Android builds require `--features android` (enables winit/android-native-activity)"
);

#[cfg(not(target_os = "android"))]
use std::path::PathBuf;
#[cfg(not(target_os = "android"))]
use std::process::ExitCode;

#[cfg(not(target_os = "android"))]
use pretty_renderer::probe_adapters;
use pretty_renderer::RendererKind;
use winit::event_loop::EventLoop;

use crate::app::{AppOptions, PrettyApp, RendererNotice, UserEvent};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Startup notice for a rendering backend that cannot run as configured.
///
/// Returns `None` when nothing was substituted. A requested backend that was
/// rewritten by `Config::validate` (e.g. GLES on a Windows build) or GLES on a
/// Wayland session falling back to X11 both produce a modal instead of a
/// silent change.
fn renderer_notice_for(
    config: &crate::config::Config,
    gles_wayland_fallback: bool,
    display: platform::DisplayKind,
    wayland_session: bool,
) -> Option<RendererNotice> {
    let requested = config.requested_renderer.as_str();
    if requested.is_empty() || requested == "auto" {
        return None;
    }
    if requested != config.renderer {
        return Some(RendererNotice::RendererUnavailable {
            requested: requested.to_owned(),
        });
    }
    if gles_wayland_fallback && wayland_session {
        let actual = if display == platform::DisplayKind::Wayland {
            "Wayland"
        } else {
            "X11 (XWayland)"
        };
        return Some(RendererNotice::GlesNeedsX11 {
            session: "Wayland",
            actual,
        });
    }
    None
}

#[cfg(not(target_os = "android"))]
#[derive(Debug, Default)]
struct Cli {
    config: Option<PathBuf>,
    renderer: Option<String>,
    command: Option<String>,
    /// Arguments after `-e`/`-x`; executed directly as the pane command.
    argv: Option<Vec<String>>,
    print_info: bool,
    font_size: Option<f32>,
    opacity: Option<f32>,
    no_blur: bool,
    fullscreen: bool,
    title: Option<String>,
    benchmark: Option<u32>,
    allow_software: bool,
    import_kitty_conf: Option<PathBuf>,
    help: bool,
    version: bool,
}

#[cfg(not(target_os = "android"))]
fn parse_args() -> Result<Cli, String> {
    parse_args_from(std::env::args().skip(1))
}

#[cfg(not(target_os = "android"))]
fn parse_args_from<I>(args: I) -> Result<Cli, String>
where
    I: IntoIterator<Item = String>,
{
    let mut cli = Cli::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let (key, inline) = match arg.split_once('=') {
            Some((k, v)) => (k.to_owned(), Some(v.to_owned())),
            None => (arg, None),
        };
        // Terminal emulators are invoked as `TERMINAL -e COMMAND...`
        // (and historically `-x`). Everything after -e is the command, not
        // Pretty's own options: KDE's Terminal=true launcher runs
        // `/usr/bin/pretty -e <Exec...>`, and a single-argument -e used to
        // leave the command's own flags for Pretty to reject with exit 1.
        if key == "--command" {
            let command = if let Some(value) = inline {
                value
            } else {
                args.next().ok_or_else(|| "--command requires a value".to_owned())?
            };
            if is_self_command(&command) {
                log::info!("terminal launcher asked Pretty to run itself; ignoring nested launch");
                continue;
            }
            // --command keeps its original contract: one shell command string,
            // not a terminal-launcher argv.
            cli.command = Some(command);
            continue;
        }
        if matches!(key.as_str(), "-e" | "-x") {
            let first = if let Some(value) = inline {
                value
            } else {
                args.next().ok_or_else(|| "--command requires a value".to_owned())?
            };
            if is_self_command(&first) {
                // Terminal=true with Exec=pretty (Pretty is also the default
                // terminal) would otherwise recurse into a second Pretty
                // window. The outer window is the requested terminal; ignore
                // the self-exec and keep parsing the remaining arguments as
                // Pretty's own options.
                log::info!("terminal launcher asked Pretty to run itself; ignoring nested launch");
                continue;
            }
            let mut rest = vec![first];
            rest.extend(args.by_ref());
            if rest.iter().any(|arg| arg.contains('%')) && cfg!(windows) {
                return Err("command argument contains '%' (cmd.exe expansion)".to_owned());
            }
            cli.argv = Some(rest);
            break;
        }
        let mut next = |name: &str| -> Result<String, String> {
            if let Some(v) = inline.clone() {
                Ok(v)
            } else {
                args.next().ok_or_else(|| format!("{name} requires a value"))
            }
        };
        match key.as_str() {
            "--config" => cli.config = Some(PathBuf::from(next("--config")?)),
            "--renderer" => cli.renderer = Some(next("--renderer")?),
            "--font-size" => {
                cli.font_size = Some(next("--font-size")?.parse().map_err(|_| "invalid --font-size")?)
            }
            "--opacity" => {
                cli.opacity = Some(next("--opacity")?.parse().map_err(|_| "invalid --opacity")?)
            }
            "--title" => cli.title = Some(next("--title")?),
            "--import-kitty-conf" => {
                cli.import_kitty_conf = Some(PathBuf::from(next("--import-kitty-conf")?))
            }
            "--benchmark" => {
                cli.benchmark = Some(next("--benchmark")?.parse().map_err(|_| "invalid --benchmark")?)
            }
            "--print-info" | "--print-backends" => cli.print_info = true,
            "--allow-software" => cli.allow_software = true,
            "--no-blur" => cli.no_blur = true,
            "--fullscreen" => cli.fullscreen = true,
            "--help" | "-h" => cli.help = true,
            "--version" | "-V" => cli.version = true,
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    Ok(cli)
}

#[cfg(not(target_os = "android"))]
fn is_self_command(arg: &str) -> bool {
    let Some(exe) = std::env::current_exe().ok() else {
        return false;
    };
    let given = PathBuf::from(arg);
    let same_name = given.file_name().and_then(|name| name.to_str())
        == exe.file_name().and_then(|name| name.to_str());
    if !same_name {
        return false;
    }
    if !given.is_absolute() {
        return true;
    }
    match (given.canonicalize(), exe.canonicalize()) {
        (Ok(given), Ok(exe)) => given == exe,
        _ => false,
    }
}

#[cfg(not(target_os = "android"))]
fn usage() -> &'static str {
    "Pretty — hardware accelerated kitty-compatible terminal\n\
\n\
Usage: pretty [OPTIONS]\n\
\n\
Options:\n\
  --renderer <auto|vulkan|gles|metal|dx>  Select rendering backend\n\
                                           (metal: macOS builds, dx: Windows builds)\n\
  --config <PATH>                          Configuration file\n\
  --command <COMMAND>                      Run a command instead of the login shell\n\
  --font-size <PX>                         Override font size\n\
  --opacity <0.2..1.0>                     Override background opacity\n\
  --no-blur                                Disable compositor blur for this run\n\
  --fullscreen                             Start fullscreen\n\
  --title <TITLE>                          Initial window title\n\
  --print-info                             Print GPU/backend/protocol diagnostics and exit\n\
  --import-kitty-conf <PATH>                Merge a kitty.conf into the Pretty config\n\
  --benchmark <FRAMES>                     Continuous full repaint benchmark, then exit\n\
  --allow-software                         Testing only: permit a CPU/software adapter\n\
  --version                                Print version and exit\n\
  -h, --help                               Print this help\n\
\n\
Environment:\n\
  PRETTY_RENDERER=vulkan|gles|auto|metal|dx\n\
  PRETTY_CONFIG=/path/to/config.toml\n\
  RUST_LOG=info\n\
\n\
Shortcuts:\n\
  Ctrl+, settings        Ctrl+Shift+E split right    Ctrl+Shift+O split down\n\
  Ctrl+Shift+W close     Alt+Arrows move focus        Shift+PgUp/PgDn scroll\n\
  Ctrl+Shift+C/V copy/paste    Ctrl+Shift+Q quit\n"
}

/// Make a relocatable tarball work even before `install.sh` has run: if
/// `../share/terminfo` exists next to the binary, point children at it.
#[cfg(not(target_os = "android"))]
fn ensure_terminfo() {
    if std::env::var_os("TERMINFO").is_some() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(root) = exe.parent().and_then(|p| p.parent()) else { return };
    let terminfo = root.join("share").join("terminfo");
    if terminfo.join("p").join("pretty").exists() {
        std::env::set_var("TERMINFO", &terminfo);
        log::info!("using bundled terminfo at {}", terminfo.display());
    }
}

/// XWayland applications need XMODIFIERS to reach Fcitx's XIM server. Plasma
/// Wayland sessions commonly start Fcitx without exporting it to X11 clients,
/// which makes IME input silently impossible. Only fill the gap when an X11
/// display exists and Fcitx is actually running.
#[cfg(target_os = "linux")]
fn prepare_x11_ime() {
    if std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var_os("XMODIFIERS").is_some()
        || std::env::var_os("DISPLAY").is_none()
    {
        return;
    }
    let running = std::process::Command::new("fcitx5-remote")
        .arg("--check")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if running {
        std::env::set_var("XMODIFIERS", "@im=fcitx");
        log::info!("XMODIFIERS unset; using running Fcitx5 XIM server");
    }
}

#[cfg(not(target_os = "android"))]
fn print_info(config: &config::Config) {
    #[cfg(target_os = "linux")]
    std::env::set_var("PRETTY_GL_FORCE_GLES", "1");
    std::env::set_var("PRETTY_GL_ALLOW_NON_NATIVE", "1");
    println!("Pretty {VERSION}");
    println!("os: {} {}", std::env::consts::OS, std::env::consts::ARCH);
    println!("kernel: {}", std::env::consts::FAMILY);
    println!("config.renderer: {}", config.renderer);
    println!(
        "compiled_backends: vulkan={} gles={} metal={} dx={}",
        cfg!(feature = "vulkan"),
        cfg!(feature = "gles"),
        cfg!(feature = "metal"),
        cfg!(feature = "dx")
    );
    let auto_order = RendererKind::Auto.candidates();
    let auto_names: Vec<&str> = auto_order.iter().map(|kind| kind.as_str()).collect();
    println!("auto_order: {}", auto_names.join(","));
    for kind in &auto_order {
        println!(
            "backend {}: compiled={} platform_supported={}",
            kind,
            kind.compile_enabled(),
            kind.platform_supported()
        );
    }
    for kind in &auto_order {
        println!("-- backend {} --", kind);
        let adapters = probe_adapters(*kind);
        if adapters.is_empty() {
            println!("  no adapters detected");
        }
        for info in adapters {
            println!(
                "  adapter={:?} backend={:?} type={:?} vendor=0x{:04x} device=0x{:04x} driver={} {}",
                info.name, info.backend, info.device_type, info.vendor, info.device, info.driver, info.driver_info
            );
        }
    }
    println!(
        "kitty_protocol: graphics=yes(partial) keyboard=yes hyperlinks=yes notifications=yes clipboard=yes"
    );
}

/// Android NativeActivity entry point.
///
/// `android-activity` loads the app as a shared library and calls this symbol
/// from its Java/Kotlin glue; there is no `main` process. `cargo check` and
/// `cargo build` still need a `main` for the bin target, so the desktop `main`
/// is replaced on Android by the small stub at the bottom of this file.
///
/// This is initialization only: the APK packaging step (Gradle + NDK, cdylib
/// target) is intentionally not part of this repository. See
/// `docs/CROSS-PLATFORM.md` for the exact compile-check status.
#[cfg(all(target_os = "android", feature = "android"))]
mod android_entry {
    use super::*;
    use winit::platform::android::activity::AndroidApp;
    use winit::platform::android::EventLoopBuilderExtAndroid;

    #[no_mangle]
    pub fn android_main(app: AndroidApp) {
        env_logger::Builder::from_env(
            env_logger::Env::default().default_filter_or(
                "info,wgpu_core=warn,wgpu_hal=warn,naga=warn,egui_wgpu=warn",
            ),
        )
        .format_timestamp_millis()
        .init();

        // Android has no HOME: the NativeActivity glue supplies the per-app
        // internal data directory instead. Export it before `config::config_path`
        // so the normal config loader (and `--config` on desktop) stay unified.
        if std::env::var_os("PRETTY_CONFIG").is_none() {
            if let Some(dir) = app.internal_data_path() {
                let config_path = dir.join("config.toml");
                std::env::set_var("PRETTY_CONFIG", &config_path);
                log::info!("Android config path: {}", config_path.display());
            }
        }
        let path = config::config_path(None);
        let mut config = config::load(&path);
        config.validate();

        let renderer = std::env::var("PRETTY_RENDERER")
            .ok()
            .and_then(|value| value.parse::<RendererKind>().ok())
            .or_else(|| config.renderer.parse::<RendererKind>().ok())
            .unwrap_or(RendererKind::Auto);
        log::info!(
            "Pretty {VERSION} starting on Android: requested_renderer={renderer} config={}",
            path.display()
        );

        let mut builder = EventLoop::<UserEvent>::with_user_event();
        builder.with_android_app(app);
        let event_loop = match builder.build() {
            Ok(event_loop) => event_loop,
            Err(e) => {
                log::error!("failed to create Android event loop: {e}");
                return;
            }
        };
        event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
        let options = AppOptions {
            renderer,
            command: None,
            argv: Vec::new(),
            startup_notice: None,
            fullscreen: false,
            title: None,
            config_path: path,
            benchmark_frames: None,
            allow_software: false,
        };
        let mut pretty_app = PrettyApp::new(config, options, event_loop.create_proxy());
        if let Err(e) = event_loop.run_app(&mut pretty_app) {
            log::error!("Android event loop failed: {e}");
        }
    }
}

/// `cargo check`/`cargo build` of this bin target need a Rust `main`, but an
/// APK never calls it: `android-activity` invokes [`android_entry::android_main`].
#[cfg(target_os = "android")]
fn main() {
    eprintln!(
        "pretty: this is the Android bin-target stub; the NativeActivity entry point is android_main"
    );
}

#[cfg(not(target_os = "android"))]
fn main() -> ExitCode {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(
            "info,wgpu_core=warn,wgpu_hal=warn,naga=warn,egui_wgpu=warn",
        ),
    )
    .format_timestamp_millis()
    .init();

    let cli = match parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("pretty: {e}\n\n{}", usage());
            return ExitCode::FAILURE;
        }
    };
    if cli.help {
        print!("{}", usage());
        return ExitCode::SUCCESS;
    }
    if cli.version {
        println!("pretty {VERSION}");
        return ExitCode::SUCCESS;
    }

    let path = config::config_path(cli.config.as_deref());
    if let Some(source) = &cli.import_kitty_conf {
        match config::import_kitty_conf_to_file(source, &path) {
            Ok(report) => {
                println!("{}", report.summary());
                for warning in &report.warnings {
                    eprintln!("warning: {warning}");
                }
                for key in &report.unsupported {
                    eprintln!("skipped: {key}");
                }
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                eprintln!("pretty: failed to import {}: {e}", source.display());
                return ExitCode::FAILURE;
            }
        }
    }
    let mut config = config::load(&path);
    if let Some(size) = cli.font_size {
        config.font_size = size;
    }
    if let Some(opacity) = cli.opacity {
        config.opacity = opacity;
    }
    if cli.no_blur {
        config.blur = false;
    }
    config.validate();

    // An explicitly requested backend that does not parse must fail loudly:
    // silently running with `auto` hides a typo and can start a different GPU
    // API than the one the user asked for.
    let renderer = cli
        .renderer
        .clone()
        .or_else(|| std::env::var("PRETTY_RENDERER").ok())
        .unwrap_or_else(|| config.renderer.clone());
    let renderer = match renderer.parse::<RendererKind>() {
        Ok(kind) => kind,
        Err(e) => {
            eprintln!("pretty: {e}");
            eprintln!(
                "pretty: valid renderers are auto, vulkan, gles, metal (macOS builds) and dx (Windows builds)"
            );
            return ExitCode::FAILURE;
        }
    };

    if cli.print_info {
        print_info(&config);
        return ExitCode::SUCCESS;
    }

    // A config value is normalised by `Config::validate`; this catches an
    // explicit --renderer/PRETTY_RENDERER request. In particular GLES is
    // disabled on Windows until the crash in issue #1 is fixed.
    if !renderer.platform_supported() || !renderer.compile_enabled() {
        eprintln!(
            "pretty: the {} backend is not available in this build on {}",
            renderer,
            std::env::consts::OS
        );
        if cfg!(target_os = "windows") {
            eprintln!(
                "pretty: use --renderer auto, vulkan or dx (GLES is disabled on Windows; see issue #1)"
            );
        } else {
            eprintln!("pretty: use --renderer auto or vulkan");
        }
        return ExitCode::FAILURE;
    }

    // GLES still uses XWayland on this driver. Vulkan stays on native Wayland
    // so winit/IME use text-input-v3 (Fcitx's normal candidate popup) and KWin
    // can blur through the org_kde_kwin_blur protocol. The former
    // prefer_x11_for_blur switch is retained only for config compatibility.
    //
    // The fallback is recorded and the window shows a one-time notice; the
    // settings dialog also warns before the user selects GLES.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    let mut wayland_display_for_notice: Option<std::ffi::OsString> = None;
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    let gles_wayland_fallback = {
        let gles_on_x11 = renderer == RendererKind::Gles;
        let fallback = gles_on_x11
            && std::env::var_os("DISPLAY").is_some()
            && std::env::var_os("WAYLAND_DISPLAY").is_some();
        if fallback {
            // Only winit's backend choice cares about the variable: it reads
            // the environment once while building the event loop, so the value
            // is restored right after that (before the app starts).
            wayland_display_for_notice = std::env::var_os("WAYLAND_DISPLAY");
            std::env::remove_var("WAYLAND_DISPLAY");
            log::info!("GLES backend selected; using X11/XWayland window surface (native Wayland EGL presentation is not supported by this driver)");
        }
        fallback
    };
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    let gles_wayland_fallback = false;

    // Explain a silent fallback: the configured backend could not start, or
    // GLES had to leave native Wayland for X11/XWayland.
    let renderer_notice = renderer_notice_for(
        &config,
        gles_wayland_fallback,
        platform::current_display_kind(),
        platform::is_wayland_session(),
    );
    if let Some(kind) = &renderer_notice {
        match kind {
            RendererNotice::GlesNeedsX11 { .. } => {
                log::info!("GLES/Wayland notice queued (window runs through XWayland)")
            }
            RendererNotice::RendererUnavailable { requested } => log::warn!(
                "requested backend '{requested}' cannot start here; showing fallback notice"
            ),
        }
    }

    ensure_terminfo();
    log::info!(
        "Pretty {VERSION} starting: requested_renderer={renderer} config={} TERM=pretty",
        path.display()
    );
    log::info!(
        "protocol level: kitty graphics=partial keyboard=full hyperlinks=yes \
         notifications=yes osc52=yes mouse=all scrollback={}",
        config.scrollback
    );

    // wgpu's GL backend prefers desktop OpenGL when EGL advertises it. Pretty
    // selects the GL backend as "GLES", so force EGL_OPENGL_ES_API through the
    // local wgpu-hal patch. Vulkan is unaffected.
    #[cfg(target_os = "linux")]
    std::env::set_var("PRETTY_GL_FORCE_GLES", "1");
    std::env::set_var("PRETTY_GL_ALLOW_NON_NATIVE", "1");

    #[cfg(target_os = "linux")]
    prepare_x11_ime();

    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(e) => e,
        Err(e) => {
            log::error!("failed to create event loop: {e}");
            return ExitCode::FAILURE;
        }
    };
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
    // winit has already chosen its backend (X11 when GLES cleared
    // WAYLAND_DISPLAY). Restore the Wayland display so the app can tell a
    // Wayland session that fell back to XWayland from a real X11 session.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    if gles_wayland_fallback {
        if let Some(display) = wayland_display_for_notice.take() {
            std::env::set_var("WAYLAND_DISPLAY", display);
        }
    }
    let options = AppOptions {
        renderer,
        command: cli.command,
        argv: cli.argv.unwrap_or_default(),
        startup_notice: renderer_notice,
        fullscreen: cli.fullscreen,
        title: cli.title,
        config_path: path,
        benchmark_frames: cli.benchmark,
        allow_software: cli.allow_software,
    };
    let mut app = PrettyApp::new(config, options, event_loop.create_proxy());
    match event_loop.run_app(&mut app) {
        Ok(()) if !app.failed => ExitCode::SUCCESS,
        Ok(()) => ExitCode::FAILURE,
        Err(e) => {
            log::error!("event loop failed: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(all(test, not(target_os = "android"), not(windows)))]
mod cli_tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, String> {
        parse_args_from(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn terminal_e_consumes_the_whole_argv_verbatim() {
        let cli = parse(&["-e", "sh", "-c", "echo hi"]).unwrap();
        assert_eq!(
            cli.argv.as_deref(),
            Some(&["sh".to_owned(), "-c".to_owned(), "echo hi".to_owned()][..])
        );
        // -e must not let the command's flags be parsed as Pretty options.
        let cli = parse(&["-e", "foo", "--bar"]).unwrap();
        assert_eq!(cli.argv.as_deref(), Some(&["foo".to_owned(), "--bar".to_owned()][..]));
    }

    #[test]
    fn terminal_x_is_an_alias_for_e() {
        let cli = parse(&["-x", "htop"]).unwrap();
        assert_eq!(cli.argv.as_deref(), Some(&["htop".to_owned()][..]));
    }

    #[test]
    fn one_command_argument_is_kept_as_a_shell_string() {
        let cli = parse(&["--command", "bash -lc 'echo hi'"]).unwrap();
        assert_eq!(cli.command.as_deref(), Some("bash -lc 'echo hi'"));
    }

    #[test]
    fn terminal_e_requires_a_command() {
        assert!(parse(&["-e"]).is_err());
    }

    #[test]
    fn self_launch_is_detected_and_skipped() {
        let exe = std::env::current_exe().unwrap();
        let exe = exe.to_string_lossy().into_owned();
        let cli = parse(&["--title", "SelfTest", "-e", &exe, "--title", "Ignored"]).unwrap();
        assert_eq!(cli.command, None);
        assert!(cli.argv.is_none());
        assert_eq!(cli.title.as_deref(), Some("Ignored"));
        assert!(is_self_command(&exe));
        assert!(!is_self_command("/bin/sh"));
    }
}
