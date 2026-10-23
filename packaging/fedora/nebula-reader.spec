# Fedora / COPR spec: builds from the release's source tarball plus its
# vendor tarball (packaging/fedora/make-vendor.sh, attached to every
# release), offline, with Fedora's Rust macros. The GitHub .rpm comes from
# packaging/build-rpm.sh instead. Keep Version in step with Cargo.toml
# (the release workflow checks).

# Tests read and write archives in temporary folders only; no network.
%bcond check 1

Name:           nebula-reader
Version:        0.1.13
Release:        1%{?dist}
Summary:        Comic, manga and ebook reader with Jellyfin support

SourceLicense:  GPL-3.0-only
# The app (GPL-3.0-only) plus the crates built into it, from
# %%cargo_license_summary in the build log; refresh when dependencies change:
# (Apache-2.0 OR MIT) AND BSD-3-Clause
# (MIT OR Apache-2.0) AND Unicode-3.0
# 0BSD OR MIT OR Apache-2.0
# Apache-2.0
# Apache-2.0 AND MIT
# Apache-2.0 OR BSL-1.0
# Apache-2.0 OR GPL-2.0-only
# Apache-2.0 OR MIT
# Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT
# BSD-2-Clause
# BSD-2-Clause OR Apache-2.0 OR MIT
# BSD-3-Clause
# BSD-3-Clause OR Apache-2.0
# BSD-3-Clause OR MIT OR Apache-2.0
# BSL-1.0
# CC0-1.0
# CC0-1.0 OR MIT-0 OR Apache-2.0
# GPL-3.0-only
# ISC
# MIT
# MIT OR Apache-2.0
# MIT OR Apache-2.0 OR CC0-1.0
# MIT OR Apache-2.0 OR LGPL-2.1-or-later
# MIT OR Apache-2.0 OR Zlib
# MIT OR LGPL-3.0-or-later
# MIT OR Zlib OR Apache-2.0
# MPL-2.0
# Unicode-3.0
# Unlicense OR MIT
# Zlib
# Zlib OR Apache-2.0 OR MIT
License:        (Apache-2.0 OR MIT) AND BSD-3-Clause AND (MIT OR Apache-2.0) AND Unicode-3.0 AND (0BSD OR MIT OR Apache-2.0) AND Apache-2.0 AND Apache-2.0 AND MIT AND (Apache-2.0 OR BSL-1.0) AND (Apache-2.0 OR GPL-2.0-only) AND (Apache-2.0 OR MIT) AND (Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT) AND BSD-2-Clause AND (BSD-2-Clause OR Apache-2.0 OR MIT) AND BSD-3-Clause AND (BSD-3-Clause OR Apache-2.0) AND (BSD-3-Clause OR MIT OR Apache-2.0) AND BSL-1.0 AND CC0-1.0 AND (CC0-1.0 OR MIT-0 OR Apache-2.0) AND GPL-3.0-only AND ISC AND MIT AND (MIT OR Apache-2.0) AND (MIT OR Apache-2.0 OR CC0-1.0) AND (MIT OR Apache-2.0 OR LGPL-2.1-or-later) AND (MIT OR Apache-2.0 OR Zlib) AND (MIT OR LGPL-3.0-or-later) AND (MIT OR Zlib OR Apache-2.0) AND MPL-2.0 AND Unicode-3.0 AND (Unlicense OR MIT) AND Zlib AND (Zlib OR Apache-2.0 OR MIT)
URL:            https://github.com/NotTimm/NebulaReader
Source0:        %{url}/archive/v%{version}/NebulaReader-%{version}.tar.gz
Source1:        %{url}/releases/download/v%{version}/%{name}-%{version}-vendor.tar.xz

%global app_id io.github.NotTimm.NebulaReader

BuildRequires:  cargo-rpm-macros >= 26
BuildRequires:  gcc
BuildRequires:  gcc-c++
BuildRequires:  pkgconfig(libarchive)
BuildRequires:  pkgconfig(xkbcommon)
BuildRequires:  pkgconfig(wayland-client)
BuildRequires:  pkgconfig(openssl)
BuildRequires:  desktop-file-utils
BuildRequires:  libappstream-glib

# dlopen()ed by winit on X11 sessions, so the automatic requires miss it.
Recommends:     libxkbcommon-x11

%description
Nebula Reader reads comic book archives (CBZ and CBR), manga and comic
series folders, and EPUB books, from local folders and Jellyfin servers.
It keeps a library with covers and reading progress, shows one or two
pages at a time with zoom, theater mode and full screen, and can look up
metadata on AniList and ComicVine. It follows the COSMIC, KDE or GNOME
look.

%prep
%autosetup -n NebulaReader-%{version} -p1 -a1
%cargo_prep -v vendor
# %%cargo_prep only redirects crates.io; libcosmic and its git
# dependencies are vendored too.
cat cargo-vendor-git.toml >> .cargo/config.toml

%build
# Updates come from the repositories: no GitHub check, and About says so.
export NEBULA_DISTRO="Fedora package"
%cargo_build
%{cargo_license_summary}
%{cargo_license} > LICENSE.dependencies
%{cargo_vendor_manifest}

%install
install -Dpm0755 target/rpm/%{name} %{buildroot}%{_bindir}/%{name}
install -Dpm0644 res/%{app_id}.desktop %{buildroot}%{_datadir}/applications/%{app_id}.desktop
install -Dpm0644 res/%{app_id}.metainfo.xml %{buildroot}%{_metainfodir}/%{app_id}.metainfo.xml
for png in res/icons/hicolor/*/apps/%{app_id}.png; do
    install -Dpm0644 "$png" "%{buildroot}%{_datadir}/icons/${png#res/icons/}"
done

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/%{app_id}.desktop
appstream-util validate-relax --nonet %{buildroot}%{_metainfodir}/%{app_id}.metainfo.xml
%if %{with check}
export NEBULA_DISTRO="Fedora package"
%cargo_test
%endif

%files
%license LICENSE LICENSE.dependencies cargo-vendor.txt
%doc README.md
%{_bindir}/%{name}
%{_datadir}/applications/%{app_id}.desktop
%{_metainfodir}/%{app_id}.metainfo.xml
%{_datadir}/icons/hicolor/*/apps/%{app_id}.png

%changelog
* Fri Oct 23 2026 Tim Singel <gamer.tdogtim@gmail.com> - 0.1.13-1
- Ctrl + mouse wheel scrolls a zoomed comic page (Ctrl + Shift: sideways)

* Wed Oct 21 2026 Tim Singel <gamer.tdogtim@gmail.com> - 0.1.12-1
- Fit to Width (Space) for comics
- Faster Jellyfin opening and page turns, less memory per page
- Dark mode no longer turns light when a check fails

* Tue Oct 20 2026 Tim Singel <gamer.tdogtim@gmail.com> - 0.1.11-1
- First COPR build: no GitHub update check, updates come through dnf

* Tue Oct 20 2026 Tim Singel <gamer.tdogtim@gmail.com> - 0.1.10-1
- First Fedora spec
