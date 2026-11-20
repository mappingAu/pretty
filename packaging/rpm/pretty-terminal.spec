Name:           pretty-terminal
Version:        0.2.4
Release:        1
Summary:        Hardware accelerated, kitty-compatible terminal emulator
License:        GPL-3.0-or-later
URL:            https://github.com/mappingAu/pretty
Source0:        pretty-stage.tar.gz
Requires:       glibc >= 2.35
Requires:       libgcc
Requires:       fontconfig
Requires:       freetype
Requires:       libxkbcommon
Requires:       libwayland-client
Requires:       libX11
Requires:       libxcb
Requires:       mesa-libEGL
Requires:       vulkan-loader
Recommends:     jetbrains-mono-fonts
Recommends:     google-noto-sans-mono-cjk-vf-fonts
Recommends:     google-noto-emoji-fonts
BuildArch:      x86_64
%description
Pretty is a GPU accelerated terminal emulator using Vulkan or OpenGL ES 3.0+
through wgpu. It implements the kitty graphics protocol, the kitty keyboard
protocol, OSC 8 hyperlinks, OSC 52 clipboard, desktop notifications, true color,
ligatures, scrollback, tiling panes and KDE/X11 compositor blur. Metal and
DirectX are reserved interfaces and are not implemented.
%prep
%build
%install
rm -rf %{buildroot}
mkdir -p %{buildroot}
tar -xzf %{SOURCE0} -C %{buildroot} --strip-components=1
%files
# Distribution-neutral payload paths: %{_docdir}/%{_datadir}/%{_mandir}
# depend on the building distribution's macros (e.g. Fedora expands
# %{_docdir} to /usr/share/doc/pretty-terminal and uses usrmove).
/usr/bin/pretty
/usr/share/applications/pretty.desktop
/usr/share/icons/hicolor/scalable/apps/pretty.svg
/usr/share/icons/hicolor/16x16/apps/pretty.png
/usr/share/icons/hicolor/24x24/apps/pretty.png
/usr/share/icons/hicolor/32x32/apps/pretty.png
/usr/share/icons/hicolor/48x48/apps/pretty.png
/usr/share/icons/hicolor/64x64/apps/pretty.png
/usr/share/icons/hicolor/128x128/apps/pretty.png
/usr/share/icons/hicolor/256x256/apps/pretty.png
/usr/share/icons/hicolor/512x512/apps/pretty.png
/usr/share/terminfo/p/pretty
/usr/share/man/man1/pretty.1.gz
/usr/share/doc/pretty/README.md
/usr/share/doc/pretty/KITTY-COMPAT.md
/usr/share/doc/pretty/TEST-REPORT.md
/usr/share/doc/pretty/RENDERER.md
/usr/share/doc/pretty/ARCHITECTURE.md
/usr/share/doc/pretty/KNOWN-ISSUES.md
/usr/share/doc/pretty/PACKAGING.md
/usr/share/doc/pretty/CONFIG.md
/usr/share/doc/pretty/CROSS-PLATFORM.md
/usr/share/doc/pretty/CROSS-COMPILE.md
/usr/share/doc/pretty/TEST-REPORT-V0.2.0.md
/usr/share/doc/pretty/TEST-REPORT-V0.2.1.md
/usr/share/doc/pretty/TEST-REPORT-V0.2.2.md
/usr/share/doc/pretty/TEST-REPORT-V0.2.3.md
/usr/share/doc/pretty/TEST-REPORT-V0.2.4.md
/usr/share/doc/pretty/AUDIT-020.md
/usr/share/doc/pretty/LICENSE
/usr/share/doc/pretty/copyright
/usr/share/doc/pretty/licenses
%changelog
* Fri Nov 20 2026 Pretty Authors <dev@pretty.local> - 0.2.4-1
- 0.2.4: handle Terminal=true terminal-launcher -e command lines
* Fri Nov 20 2026 Pretty Authors <dev@pretty.local> - 0.2.3-1
- 0.2.3: native Wayland file drops
* Sat Nov 14 2026 Pretty Authors <dev@pretty.local> - 0.2.2-1
- 0.2.2: disable GLES on Windows, compile Vulkan on every platform
* Sat Nov 14 2026 Pretty Authors <dev@pretty.local> - 0.2.1-1
- 0.2.1: clean window close, aligned title bar/radii, stable fullscreen chrome
* Sat Nov 07 2026 Pretty Authors <dev@pretty.local> - 0.2.0-1
- 0.2.0: native Metal/DX12, GUI customization, Kitty config import, platform artifacts
