//! Cross-platform user, configuration, font and terminfo paths.
//!
//! Pretty used to rely on Linux-only conventions (`/usr/share/fonts`,
//! `~/.config`, `/usr/share/terminfo`). This module centralises that knowledge
//! behind one small API:
//!
//! * Linux/BSD honours the [XDG Base Directory specification]: `XDG_CONFIG_HOME`,
//!   `XDG_DATA_HOME`, `XDG_CACHE_HOME`, `XDG_RUNTIME_DIR` and `XDG_DATA_DIRS`;
//! * Windows uses the roaming/local AppData known folders;
//! * macOS uses `~/Library/Application Support` and `~/Library/Caches`;
//! * Android reads the system font directories; the per-app data directory is
//!   supplied by `android-activity` at runtime (see `main.rs`).
//!
//! Every rule is evaluated by [`Paths::resolve`] with an injected environment
//! lookup, so unit tests exercise Linux, Windows, macOS and Android layouts on
//! any host without reading the real home directory. [`Paths::host`] is the
//! only function that touches the process environment.
//!
//! [XDG Base Directory specification]: https://specifications.freedesktop.org/basedir-spec/latest/

#![allow(dead_code)] // Call sites in config.rs/app.rs are migrated separately.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Directory name used for Pretty's own configuration/cache/data on Linux.
pub const APP_DIR: &str = "pretty";
/// Bundle-style identifier used by macOS (`ProjectDirs::from("org", "pretty", "pretty")`).
pub const MACOS_BUNDLE_ID: &str = "org.pretty.pretty";

/// The platform whose path conventions should be applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Linux,
    MacOs,
    Windows,
    Android,
}

/// Host platform, using the XDG rules for every other Unix.
pub const HOST: Platform = if cfg!(target_os = "windows") {
    Platform::Windows
} else if cfg!(target_os = "macos") {
    Platform::MacOs
} else if cfg!(target_os = "android") {
    Platform::Android
} else {
    Platform::Linux
};

/// Resolved per-user directories plus the font/terminfo search paths.
///
/// `config_dir`, `data_dir` and `cache_dir` are Pretty-specific (they already
/// contain [`APP_DIR`]); `config_home` is the platform's base user-config
/// directory, used for tools that follow their own subdirectory (kitty).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    pub platform: Platform,
    pub config_home: Option<PathBuf>,
    pub config_dir: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
    pub runtime_dir: Option<PathBuf>,
    pub font_dirs: Vec<PathBuf>,
    pub terminfo_dirs: Vec<PathBuf>,
    pub cjk_font_candidates: Vec<PathBuf>,
}

impl Paths {
    /// Resolve the paths for the running process.
    pub fn host() -> Self {
        Self::resolve(HOST, &|key| std::env::var_os(key))
    }

    /// Resolve the paths for `platform` using `env` as the environment.
    ///
    /// The lookup closure is the only source of environment data, which keeps
    /// tests hermetic and makes the Windows/macOS rules testable on Linux.
    pub fn resolve(platform: Platform, env: &dyn Fn(&str) -> Option<OsString>) -> Self {
        let mut paths = match platform {
            Platform::Linux => linux_paths(env),
            Platform::MacOs => macos_paths(env),
            Platform::Windows => windows_paths(env),
            Platform::Android => android_paths(env),
        };
        paths.platform = platform;
        dedup(&mut paths.font_dirs);
        dedup(&mut paths.terminfo_dirs);
        dedup(&mut paths.cjk_font_candidates);
        paths
    }

    /// Pretty's configuration file (`<config_dir>/config.toml`).
    pub fn config_file(&self) -> Option<PathBuf> {
        self.config_dir
            .as_ref()
            .map(|dir| self.join(dir, &["config.toml"]))
    }

    /// A file inside Pretty's per-user data directory.
    pub fn data_file(&self, relative: impl AsRef<Path>) -> Option<PathBuf> {
        self.data_dir
            .as_ref()
            .map(|dir| self.join(dir, &[relative.as_ref().to_string_lossy().as_ref()]))
    }

    /// Ordered CJK fallback font files for this platform. Callers should use
    /// the first entry that exists and fall back to `fontdb` when none does.
    pub fn cjk_font_candidates(&self) -> &[PathBuf] {
        &self.cjk_font_candidates
    }

    /// kitty's configuration file, used by `--import-kitty-conf` defaults.
    pub fn kitty_config_file(&self) -> Option<PathBuf> {
        self.config_home
            .as_ref()
            .map(|dir| self.join(dir, &["kitty", "kitty.conf"]))
    }

    /// Join path parts with the separator of the resolved platform, so the
    /// Windows layout can be asserted on a Linux test host.
    fn join(&self, base: &Path, parts: &[&str]) -> PathBuf {
        if self.platform == Platform::Windows {
            win_join(base, parts)
        } else {
            join(base, parts)
        }
    }

    /// Locate a compiled terminfo entry, preferring the first configured root.
    ///
    /// Terminfo entries live in a one-character directory (`p/pretty`), which
    /// keeps directory scans small on every platform.
    pub fn find_terminfo(&self, name: &str) -> Option<PathBuf> {
        let prefix = name.chars().next()?;
        self.terminfo_dirs
            .iter()
            .map(|root| root.join(prefix.to_string()).join(name))
            .find(|candidate| candidate.is_file())
    }
}

fn linux_paths(env: &dyn Fn(&str) -> Option<OsString>) -> Paths {
    let home = home_dir(env);
    let config_home = xdg_dir(env, "XDG_CONFIG_HOME", home.as_deref(), &[".config"]);
    let data_home = xdg_dir(env, "XDG_DATA_HOME", home.as_deref(), &[".local", "share"]);
    let cache_home = xdg_dir(env, "XDG_CACHE_HOME", home.as_deref(), &[".cache"]);
    let runtime_dir = env_abs_path(env, "XDG_RUNTIME_DIR");

    let mut font_dirs = Vec::new();
    if let Some(dir) = &data_home {
        font_dirs.push(dir.join("fonts"));
    }
    if let Some(home) = &home {
        font_dirs.push(home.join(".fonts"));
    }
    let data_dirs = match env("XDG_DATA_DIRS").filter(|value| !value.is_empty()) {
        Some(value) => std::env::split_paths(&value).collect::<Vec<_>>(),
        None => vec![PathBuf::from("/usr/local/share"), PathBuf::from("/usr/share")],
    };
    font_dirs.extend(data_dirs.into_iter().map(|dir| dir.join("fonts")));

    let mut terminfo_dirs = Vec::new();
    push_terminfo_root(&mut terminfo_dirs, env, "TERMINFO");
    if let Some(data) = &data_home {
        terminfo_dirs.push(data.join("terminfo"));
    }
    if let Some(home) = &home {
        terminfo_dirs.push(home.join(".terminfo"));
    }
    terminfo_dirs.extend(
        [
            "/usr/share/terminfo",
            "/usr/local/share/terminfo",
            "/usr/lib/terminfo",
            "/lib/terminfo",
            "/etc/terminfo",
        ]
        .into_iter()
        .map(PathBuf::from),
    );

    Paths {
        platform: Platform::Linux,
        config_home: config_home.clone(),
        config_dir: config_home.map(|dir| dir.join(APP_DIR)),
        data_dir: data_home.as_ref().map(|dir| dir.join(APP_DIR)),
        cache_dir: cache_home.as_ref().map(|dir| dir.join(APP_DIR)),
        runtime_dir,
        font_dirs,
        terminfo_dirs,
        cjk_font_candidates: vec![
            "/usr/share/fonts/google-noto-sans-cjk-vf-fonts/NotoSansCJK-VF.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
            "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect(),
    }
}

fn macos_paths(env: &dyn Fn(&str) -> Option<OsString>) -> Paths {
    let home = home_dir(env);
    let support = home
        .as_ref()
        .map(|home| home.join("Library").join("Application Support"));
    let caches = home.as_ref().map(|home| home.join("Library").join("Caches"));
    // directories::ProjectDirs uses the bundle identifier
    // `org.pretty.pretty` for both config and data on macOS.
    let config_home = support.clone();
    let config_dir = support
        .as_ref()
        .map(|dir| dir.join(MACOS_BUNDLE_ID));

    let mut font_dirs = Vec::new();
    if let Some(home) = &home {
        font_dirs.push(home.join("Library").join("Fonts"));
    }
    font_dirs.push(PathBuf::from("/Library/Fonts"));
    font_dirs.push(PathBuf::from("/System/Library/Fonts"));

    let mut terminfo_dirs = Vec::new();
    push_terminfo_root(&mut terminfo_dirs, env, "TERMINFO");
    if let Some(home) = &home {
        terminfo_dirs.push(home.join(".terminfo"));
    }
    terminfo_dirs.extend(
        [
            "/usr/share/terminfo",
            "/usr/local/share/terminfo",
            "/opt/homebrew/share/terminfo",
        ]
        .into_iter()
        .map(PathBuf::from),
    );

    Paths {
        platform: Platform::MacOs,
        config_home,
        config_dir,
        data_dir: support
            .as_ref()
            .map(|dir| dir.join(MACOS_BUNDLE_ID)),
        cache_dir: caches.map(|dir| dir.join(MACOS_BUNDLE_ID)),
        runtime_dir: None,
        font_dirs,
        terminfo_dirs,
        cjk_font_candidates: vec![
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/STHeiti Light.ttc",
            "/Library/Fonts/Arial Unicode.ttf",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect(),
    }
}

fn windows_paths(env: &dyn Fn(&str) -> Option<OsString>) -> Paths {
    let home = home_dir(env);
    let appdata = env_path(env, "APPDATA")
        .or_else(|| home.as_ref().map(|home| win_join(home, &["AppData", "Roaming"])));
    let localappdata = env_path(env, "LOCALAPPDATA")
        .or_else(|| home.as_ref().map(|home| win_join(home, &["AppData", "Local"])));

    // Keep the layout documented for the 0.2.0 Windows zip and produced by
    // directories::ProjectDirs: %APPDATA%\pretty\pretty\config\config.toml.
    let config_home = appdata.clone();
    let config_dir = appdata
        .as_ref()
        .map(|dir| win_join(dir, &[APP_DIR, APP_DIR, "config"]));
    let data_dir = appdata
        .as_ref()
        .map(|dir| win_join(dir, &[APP_DIR, APP_DIR, "data"]));
    let cache_dir = localappdata
        .as_ref()
        .map(|dir| win_join(dir, &[APP_DIR, APP_DIR, "cache"]));

    let windir = env_path(env, "WINDIR")
        .or_else(|| env_path(env, "SystemRoot"))
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let mut font_dirs = vec![win_join(&windir, &["Fonts"])];
    if let Some(local) = &localappdata {
        font_dirs.push(win_join(local, &["Microsoft", "Windows", "Fonts"]));
    }

    let mut terminfo_dirs = Vec::new();
    push_terminfo_root(&mut terminfo_dirs, env, "TERMINFO");
    if let Some(data) = &data_dir {
        terminfo_dirs.push(win_join(data, &["terminfo"]));
    }

    let cjk_font_candidates = vec![
        win_join(&windir, &["Fonts", "msyh.ttc"]),
        win_join(&windir, &["Fonts", "msyhbd.ttc"]),
        win_join(&windir, &["Fonts", "simsun.ttc"]),
        win_join(&windir, &["Fonts", "msjh.ttc"]),
    ];

    Paths {
        platform: Platform::Windows,
        config_home,
        config_dir,
        data_dir,
        cache_dir,
        runtime_dir: None,
        font_dirs,
        terminfo_dirs,
        cjk_font_candidates,
    }
}

fn android_paths(env: &dyn Fn(&str) -> Option<OsString>) -> Paths {
    // Android has no process-wide home directory. The NativeActivity glue
    // passes the per-app internal data directory to `android_main`, which
    // exports PRETTY_CONFIG before loading the configuration. Fonts, however,
    // live in fixed system directories.
    let mut terminfo_dirs = Vec::new();
    push_terminfo_root(&mut terminfo_dirs, env, "TERMINFO");
    terminfo_dirs.push(PathBuf::from("/system/etc/terminfo"));

    Paths {
        platform: Platform::Android,
        config_home: None,
        config_dir: None,
        data_dir: None,
        cache_dir: None,
        runtime_dir: None,
        font_dirs: vec![
            PathBuf::from("/system/fonts"),
            PathBuf::from("/system/font"),
            PathBuf::from("/data/fonts"),
        ],
        terminfo_dirs,
        cjk_font_candidates: vec![
            "/system/fonts/NotoSansCJK-Regular.ttc",
            "/system/fonts/NotoSansCJK-VF.otf.ttc",
            "/system/fonts/NotoSansSC-Regular.otf",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect(),
    }
}

fn home_dir(env: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    env_path(env, "HOME").or_else(|| env_path(env, "USERPROFILE"))
}

fn env_path(env: &dyn Fn(&str) -> Option<OsString>, key: &str) -> Option<PathBuf> {
    env(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// XDG variables must be absolute; a relative value is ignored and the
/// fallback (`$HOME/<parts>`) is used instead, as the specification requires.
fn env_abs_path(env: &dyn Fn(&str) -> Option<OsString>, key: &str) -> Option<PathBuf> {
    env_path(env, key).filter(|path| path.is_absolute())
}

fn xdg_dir(
    env: &dyn Fn(&str) -> Option<OsString>,
    key: &str,
    home: Option<&Path>,
    fallback_parts: &[&str],
) -> Option<PathBuf> {
    env_abs_path(env, key).or_else(|| home.map(|home| join(home, fallback_parts)))
}

fn join(base: &Path, parts: &[&str]) -> PathBuf {
    parts.iter().fold(base.to_path_buf(), |path, part| path.join(part))
}

/// Build a Windows-style path regardless of the host, so the Windows rules can
/// be asserted on Linux.
fn win_join(base: &Path, parts: &[&str]) -> PathBuf {
    let mut value = base.as_os_str().to_os_string();
    for part in parts {
        value.push("\\");
        value.push(part);
    }
    PathBuf::from(value)
}

fn push_terminfo_root(roots: &mut Vec<PathBuf>, env: &dyn Fn(&str) -> Option<OsString>, key: &str) {
    if let Some(dir) = env_path(env, key) {
        roots.push(dir);
    }
}

fn dedup(values: &mut Vec<PathBuf>) {
    let mut unique = Vec::with_capacity(values.len());
    for value in values.drain(..) {
        if !unique.contains(&value) {
            unique.push(value);
        }
    }
    *values = unique;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn fake_env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, OsString> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), OsString::from(value)))
            .collect();
        move |key| map.get(key).cloned()
    }

    // These assert the Linux/XDG layout with '/' separators. Windows PathBuf
    // normalises separators, so the platform-parametrised resolver is covered
    // there by the windows_* tests instead.
    #[cfg(unix)]
    #[test]
    fn xdg_overrides_are_honoured() {
        let env = fake_env(&[
            ("HOME", "/home/tester"),
            ("XDG_CONFIG_HOME", "/xdg/config"),
            ("XDG_DATA_HOME", "/xdg/data"),
            ("XDG_CACHE_HOME", "/xdg/cache"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ]);
        let paths = Paths::resolve(Platform::Linux, &env);

        assert_eq!(paths.config_dir, Some(PathBuf::from("/xdg/config/pretty")));
        assert_eq!(
            paths.config_file(),
            Some(PathBuf::from("/xdg/config/pretty/config.toml"))
        );
        assert_eq!(paths.data_dir, Some(PathBuf::from("/xdg/data/pretty")));
        assert_eq!(paths.cache_dir, Some(PathBuf::from("/xdg/cache/pretty")));
        assert_eq!(paths.runtime_dir, Some(PathBuf::from("/run/user/1000")));
        assert!(paths
            .font_dirs
            .contains(&PathBuf::from("/xdg/data/fonts")));
    }

    #[test]
    fn linux_falls_back_to_home_directories() {
        let env = fake_env(&[("HOME", "/home/tester")]);
        let paths = Paths::resolve(Platform::Linux, &env);

        assert_eq!(
            paths.config_dir,
            Some(PathBuf::from("/home/tester/.config/pretty"))
        );
        assert_eq!(
            paths.data_dir,
            Some(PathBuf::from("/home/tester/.local/share/pretty"))
        );
        assert_eq!(
            paths.cache_dir,
            Some(PathBuf::from("/home/tester/.cache/pretty"))
        );
        assert_eq!(paths.runtime_dir, None);
        assert_eq!(
            paths.kitty_config_file(),
            Some(PathBuf::from("/home/tester/.config/kitty/kitty.conf"))
        );
        assert!(paths
            .font_dirs
            .contains(&PathBuf::from("/home/tester/.local/share/fonts")));
        assert!(paths.font_dirs.contains(&PathBuf::from("/usr/share/fonts")));
        assert!(paths
            .terminfo_dirs
            .contains(&PathBuf::from("/usr/share/terminfo")));
    }

    #[test]
    fn relative_xdg_values_are_ignored() {
        let env = fake_env(&[
            ("HOME", "/home/tester"),
            ("XDG_CONFIG_HOME", "relative/config"),
            ("XDG_DATA_HOME", "relative/data"),
            ("XDG_CACHE_HOME", "relative/cache"),
            ("XDG_RUNTIME_DIR", "relative/run"),
        ]);
        let paths = Paths::resolve(Platform::Linux, &env);

        assert_eq!(
            paths.config_dir,
            Some(PathBuf::from("/home/tester/.config/pretty"))
        );
        assert_eq!(
            paths.data_dir,
            Some(PathBuf::from("/home/tester/.local/share/pretty"))
        );
        assert_eq!(
            paths.cache_dir,
            Some(PathBuf::from("/home/tester/.cache/pretty"))
        );
        assert_eq!(paths.runtime_dir, None);
    }

    #[test]
    fn missing_home_yields_no_user_directories() {
        let env = fake_env(&[]);
        let paths = Paths::resolve(Platform::Linux, &env);

        assert_eq!(paths.config_dir, None);
        assert_eq!(paths.data_dir, None);
        assert_eq!(paths.cache_dir, None);
        assert_eq!(paths.config_file(), None);
        // System font/terminfo roots are still present.
        assert!(paths.font_dirs.contains(&PathBuf::from("/usr/share/fonts")));
    }

    #[cfg(unix)]
    #[test]
    fn xdg_data_dirs_control_font_search_order() {
        let env = fake_env(&[
            ("HOME", "/home/tester"),
            ("XDG_DATA_DIRS", "/opt/share:/usr/share"),
        ]);
        let paths = Paths::resolve(Platform::Linux, &env);
        assert!(paths.font_dirs.contains(&PathBuf::from("/opt/share/fonts")));
        assert!(paths.font_dirs.contains(&PathBuf::from("/usr/share/fonts")));
    }

    #[test]
    fn windows_uses_known_folders() {
        let env = fake_env(&[
            ("USERPROFILE", r"C:\Users\tester"),
            ("APPDATA", r"C:\Users\tester\AppData\Roaming"),
            ("LOCALAPPDATA", r"C:\Users\tester\AppData\Local"),
            ("WINDIR", r"C:\Windows"),
        ]);
        let paths = Paths::resolve(Platform::Windows, &env);

        assert_eq!(
            paths.config_file(),
            Some(PathBuf::from(
                r"C:\Users\tester\AppData\Roaming\pretty\pretty\config\config.toml"
            ))
        );
        assert_eq!(
            paths.data_dir,
            Some(PathBuf::from(
                r"C:\Users\tester\AppData\Roaming\pretty\pretty\data"
            ))
        );
        assert_eq!(
            paths.cache_dir,
            Some(PathBuf::from(
                r"C:\Users\tester\AppData\Local\pretty\pretty\cache"
            ))
        );
        assert!(paths
            .font_dirs
            .contains(&PathBuf::from(r"C:\Windows\Fonts")));
        assert_eq!(
            paths.cjk_font_candidates.first(),
            Some(&PathBuf::from(r"C:\Windows\Fonts\msyh.ttc"))
        );
        assert_eq!(paths.kitty_config_file(), Some(PathBuf::from(
            r"C:\Users\tester\AppData\Roaming\kitty\kitty.conf"
        )));
    }

    #[test]
    fn windows_falls_back_to_the_user_profile() {
        let env = fake_env(&[("USERPROFILE", r"C:\Users\tester")]);
        let paths = Paths::resolve(Platform::Windows, &env);

        assert_eq!(
            paths.config_dir,
            Some(PathBuf::from(
                r"C:\Users\tester\AppData\Roaming\pretty\pretty\config"
            ))
        );
        assert_eq!(
            paths.cache_dir,
            Some(PathBuf::from(
                r"C:\Users\tester\AppData\Local\pretty\pretty\cache"
            ))
        );
        assert!(paths
            .cjk_font_candidates
            .contains(&PathBuf::from(r"C:\Windows\Fonts\msyh.ttc")));
    }

    #[test]
    fn macos_uses_application_support_and_caches() {
        let env = fake_env(&[("HOME", "/Users/tester")]);
        let paths = Paths::resolve(Platform::MacOs, &env);

        assert_eq!(
            paths.config_dir,
            Some(PathBuf::from(
                "/Users/tester/Library/Application Support/org.pretty.pretty"
            ))
        );
        assert_eq!(
            paths.data_dir,
            Some(PathBuf::from(
                "/Users/tester/Library/Application Support/org.pretty.pretty"
            ))
        );
        assert_eq!(
            paths.cache_dir,
            Some(PathBuf::from(
                "/Users/tester/Library/Caches/org.pretty.pretty"
            ))
        );
        assert!(paths
            .font_dirs
            .contains(&PathBuf::from("/System/Library/Fonts")));
        assert_eq!(
            paths.cjk_font_candidates.first(),
            Some(&PathBuf::from("/System/Library/Fonts/PingFang.ttc"))
        );
    }

    #[test]
    fn android_has_system_fonts_and_no_user_home() {
        let env = fake_env(&[("HOME", "/home/tester")]);
        let paths = Paths::resolve(Platform::Android, &env);

        assert_eq!(paths.config_dir, None);
        assert_eq!(paths.data_dir, None);
        assert_eq!(paths.cache_dir, None);
        assert!(paths.font_dirs.contains(&PathBuf::from("/system/fonts")));
        assert!(paths
            .terminfo_dirs
            .contains(&PathBuf::from("/system/etc/terminfo")));
    }

    #[test]
    fn terminfo_env_override_comes_first() {
        let env = fake_env(&[("HOME", "/home/tester"), ("TERMINFO", "/opt/terminfo")]);
        let paths = Paths::resolve(Platform::Linux, &env);
        assert_eq!(paths.terminfo_dirs.first(), Some(&PathBuf::from("/opt/terminfo")));
    }
}
