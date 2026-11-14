Name:           pretty-terminal
Version:        0.2.1
Release:        1%{?dist}
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
%{_bindir}/pretty
%{_datadir}/applications/pretty.desktop
%{_datadir}/icons/hicolor/scalable/apps/pretty.svg
%{_datadir}/icons/hicolor/16x16/apps/pretty.png
%{_datadir}/icons/hicolor/24x24/apps/pretty.png
%{_datadir}/icons/hicolor/32x32/apps/pretty.png
%{_datadir}/icons/hicolor/48x48/apps/pretty.png
%{_datadir}/icons/hicolor/64x64/apps/pretty.png
%{_datadir}/icons/hicolor/128x128/apps/pretty.png
%{_datadir}/icons/hicolor/256x256/apps/pretty.png
%{_datadir}/icons/hicolor/512x512/apps/pretty.png
%{_datadir}/terminfo/p/pretty
%{_mandir}/man1/pretty.1.gz
%{_docdir}/pretty/README.md
%{_docdir}/pretty/KITTY-COMPAT.md
%{_docdir}/pretty/TEST-REPORT.md
%{_docdir}/pretty/RENDERER.md
%{_docdir}/pretty/ARCHITECTURE.md
%{_docdir}/pretty/KNOWN-ISSUES.md
%{_docdir}/pretty/PACKAGING.md
%{_docdir}/pretty/CONFIG.md
%{_docdir}/pretty/CROSS-PLATFORM.md
%{_docdir}/pretty/CROSS-COMPILE.md
%{_docdir}/pretty/TEST-REPORT-V0.2.0.md
%{_docdir}/pretty/TEST-REPORT-V0.2.1.md
%{_docdir}/pretty/AUDIT-020.md
%{_docdir}/pretty/LICENSE
%{_docdir}/pretty/copyright
%{_docdir}/pretty/licenses
%changelog
* Sat Nov 14 2026 Pretty Authors <dev@pretty.local> - 0.2.1-1
- 0.2.1: clean window close, aligned title bar/radii, stable fullscreen chrome
* Sat Nov 07 2026 Pretty Authors <dev@pretty.local> - 0.2.0-1
- 0.2.0: native Metal/DX12, GUI customization, Kitty config import, platform artifacts
