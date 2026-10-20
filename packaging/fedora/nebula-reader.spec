# Fedora / COPR spec: builds from the release's source tarball plus its
# vendor tarball (packaging/fedora/make-vendor.sh, attached to every
# release), offline, with Fedora's Rust macros. The GitHub .rpm comes from
# packaging/build-rpm.sh instead. Keep Version in step with Cargo.toml
# (the release workflow checks).

# Tests read and write archives in temporary folders only; no network.
%bcond check 1

Name:           nebula-reader
Version:        0.1.10
Release:        1%{?dist}
Summary:        Comic, manga and ebook reader with Jellyfin support

SourceLicense:  GPL-3.0-only
# The app, plus the crates built into it (%%cargo_license_summary).
License:        @LICENSE@
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
* Tue Oct 20 2026 Tim Singel <gamer.tdogtim@gmail.com> - 0.1.10-1
- First Fedora spec
