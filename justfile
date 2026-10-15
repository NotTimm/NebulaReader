name := 'cosmic-comic'
export APPID := 'com.tsingel.CosmicComic'

rootdir := ''
prefix := '/usr'
base-dir := absolute_path(clean(rootdir / prefix))

bin-src := 'target' / 'release' / name
bin-dst := base-dir / 'bin' / name

desktop-src := 'res' / (APPID + '.desktop')
desktop-dst := base-dir / 'share' / 'applications' / (APPID + '.desktop')

icon-src := 'res' / 'icons' / (APPID + '.svg')
icon-dst := base-dir / 'share' / 'icons' / 'hicolor' / 'scalable' / 'apps' / (APPID + '.svg')

metainfo-src := 'res' / (APPID + '.metainfo.xml')
metainfo-dst := base-dir / 'share' / 'metainfo' / (APPID + '.metainfo.xml')

# Default recipe: build in release mode
default: build-release

# Compiles with debug profile
build-debug *args:
    cargo build {{args}}

# Compiles with release profile
build-release *args: (build-debug '--release' args)

# Runs a clippy check
check *args:
    cargo clippy --all-features {{args}} -- -W clippy::pedantic

# Run the app locally without installing
run *args:
    cargo run --release {{args}}

# Installs the binary, desktop entry, and icon (defaults to /usr, override with `just prefix=/usr/local install`)
install:
    install -Dm0755 {{bin-src}} {{bin-dst}}
    install -Dm0644 {{desktop-src}} {{desktop-dst}}
    install -Dm0644 {{icon-src}} {{icon-dst}}
    install -Dm0644 {{metainfo-src}} {{metainfo-dst}}

# Build release and install for the current user (~/.local, no sudo). Same as ./install.sh minus the git pull.
install-local: build-release
    just prefix="$HOME/.local" install
    -update-desktop-database "$HOME/.local/share/applications" 2>/dev/null
    -gtk-update-icon-cache -f "$HOME/.local/share/icons/hicolor" 2>/dev/null
    @echo "Installed to $HOME/.local/bin/cosmic-comic"

# Remove a user-local install
uninstall-local:
    just prefix="$HOME/.local" uninstall

# Run the test suite
test *args:
    cargo test --release {{args}}

# Removes installed files
uninstall:
    rm -f {{bin-dst}} {{desktop-dst}} {{icon-dst}} {{metainfo-dst}}

# Build every package format this machine can (output in ./dist)
package:
    bash packaging/build-all.sh

# Build individual package formats
package-deb:
    bash packaging/build-deb.sh
package-rpm:
    bash packaging/build-rpm.sh
package-appimage:
    bash packaging/build-appimage.sh
package-flatpak:
    bash packaging/build-flatpak.sh

# Runs `cargo clean`
clean:
    cargo clean

# Removes built packages
clean-dist:
    rm -rf dist
