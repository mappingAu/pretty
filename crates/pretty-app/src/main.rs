//! Pretty — a hardware accelerated, kitty-compatible terminal emulator.

mod app;
mod config;
mod i18n;
mod menu;
mod platform;
mod settings;

use std::path::PathBuf;
use std::process::ExitCode;

use pretty_renderer::{probe_adapters, RendererKind, RESERVED_BACKENDS};
use winit::event_loop::EventLoop;

use crate::app::{AppOptions, PrettyApp, UserEvent};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Default)]
struct Cli {
    config: Option<PathBuf>,
    renderer: Option<String>,
    command: Option<String>,
    print_info: bool,
    font_size: Option<f32>,
    opacity: Option<f32>,
    no_blur: bool,
    fullscreen: bool,
    title: Option<String>,
    benchmark: Option<u32>,
    allow_software: bool,
    help: bool,
    version: bool,
}

fn parse_args() -> Result<Cli, String> {
    let mut cli = Cli::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let (key, inline) = match arg.split_once('=') {
            Some((k, v)) => (k.to_owned(), Some(v.to_owned())),
            None => (arg, None),
        };
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
            "--command" | "-e" => cli.command = Some(next("--command")?),
            "--font-size" => {
                cli.font_size = Some(next("--font-size")?.parse().map_err(|_| "invalid --font-size")?)
            }
            "--opacity" => {
                cli.opacity = Some(next("--opacity")?.parse().map_err(|_| "invalid --opacity")?)
            }
            "--title" => cli.title = Some(next("--title")?),
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

fn usage() -> &'static str {
    "Pretty — hardware accelerated kitty-compatible terminal\n\
\n\
Usage: pretty [OPTIONS]\n\
\n\
Options:\n\
  --renderer <auto|vulkan|gles|metal|dx>  Select rendering backend\n\
  --config <PATH>                          Configuration file\n\
  --command <COMMAND>                      Run a command instead of the login shell\n\
  --font-size <PX>                         Override font size\n\
  --opacity <0.2..1.0>                     Override background opacity\n\
  --no-blur                                Disable compositor blur for this run\n\
  --fullscreen                             Start fullscreen\n\
  --title <TITLE>                          Initial window title\n\
  --print-info                             Print GPU/backend/protocol diagnostics and exit\n\
  --benchmark <FRAMES>                     Continuous full repaint benchmark, then exit\n\
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
    if std::env::var_os("XMODIFIERS").is_some() || std::env::var_os("DISPLAY").is_none() {
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

fn print_info(config: &config::Config) {
    #[cfg(target_os = "linux")]
    std::env::set_var("PRETTY_GL_FORCE_GLES", "1");
    std::env::set_var("PRETTY_GL_ALLOW_NON_NATIVE", "1");
    println!("Pretty {VERSION}");
    println!("os: {} {}", std::env::consts::OS, std::env::consts::ARCH);
    println!("kernel: {}", std::env::consts::FAMILY);
    println!("config.renderer: {}", config.renderer);
    println!(
        "compiled_backends: vulkan={} gles={} metal={} dx={} (metal/dx reserved)",
        cfg!(feature = "vulkan"),
        cfg!(feature = "gles"),
        cfg!(feature = "metal"),
        cfg!(feature = "dx")
    );
    for (name, reason) in RESERVED_BACKENDS {
        println!("reserved_backend: {name}: {reason}");
    }
    for kind in [RendererKind::Vulkan, RendererKind::Gles] {
        println!("-- backend {} --", kind);
        let adapters = probe_adapters(kind);
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

    if cli.print_info {
        print_info(&config);
        return ExitCode::SUCCESS;
    }

    let renderer = cli
        .renderer
        .clone()
        .or_else(|| std::env::var("PRETTY_RENDERER").ok())
        .unwrap_or_else(|| config.renderer.clone());
    let renderer = match renderer.parse::<RendererKind>() {
        Ok(kind) => kind,
        Err(e) => {
            log::error!("{e}");
            RendererKind::Auto
        }
    };

    // KWin's blur-behind property is only available to X11 windows. winit 0.30
    // selects Wayland whenever WAYLAND_DISPLAY is set, so temporarily clear it
    // to select XWayland when blur is requested. This is explicit in config and
    // documented; native Wayland remains available with blur disabled.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        let gles_on_x11 = renderer == RendererKind::Gles;
        let blur_needs_x11 = config.blur && config.prefer_x11_for_blur;
        if (gles_on_x11 || blur_needs_x11)
            && std::env::var_os("DISPLAY").is_some()
            && std::env::var_os("WAYLAND_DISPLAY").is_some()
        {
            std::env::remove_var("WAYLAND_DISPLAY");
            if gles_on_x11 {
                log::info!("GLES backend selected; using X11/XWayland window surface (native Wayland EGL presentation is not supported by this driver)");
            } else {
                log::info!("selecting winit X11 backend for compositor blur-behind");
            }
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
    let options = AppOptions {
        renderer,
        command: cli.command,
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
