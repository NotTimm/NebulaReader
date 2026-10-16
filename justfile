name := 'nebula-reader'
export APPID := 'io.github.NotTimm.NebulaReader'

rootdir := ''
prefix := '/usr'
base-dir := absolute_path(clean(rootdir / prefix))

bin-src := 'target' / 'release' / name
bin-dst := base-dir / 'bin' / name

desktop-src := 'res' / (APPID + '.desktop')
desktop-dst := base-dir / 'share' / 'applications' / (APPID + '.desktop')

icon-sizes := '16 24 32 48 64 128 256 512'
icons-dst := base-dir / 'share' / 'icons' / 'hicolor'

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
    for s in {{icon-sizes}}; do install -Dm0644 "res/icons/hicolor/${s}x${s}/apps/{{APPID}}.png" "{{icons-dst}}/${s}x${s}/apps/{{APPID}}.png"; done
    install -Dm0644 {{metainfo-src}} {{metainfo-dst}}

# Build release and install for the current user (~/.local, no sudo). Same as ./install.sh minus the git pull.
install-local: build-release
    just prefix="$HOME/.local" install
    -update-desktop-database "$HOME/.local/share/applications" 2>/dev/null
    -gtk-update-icon-cache -f "$HOME/.local/share/icons/hicolor" 2>/dev/null
    @echo "Installed to $HOME/.local/bin/nebula-reader"

# Remove a user-local install
uninstall-local:
    just prefix="$HOME/.local" uninstall

# Run the test suite
test *args:
    cargo test --release {{args}}

# Removes installed files
uninstall:
    rm -f {{bin-dst}} {{desktop-dst}} {{metainfo-dst}}
    for s in {{icon-sizes}}; do rm -f "{{icons-dst}}/${s}x${s}/apps/{{APPID}}.png"; done
    rm -f "{{icons-dst}}/scalable/apps/{{APPID}}.svg"  # from older versions

# Build every package format this machine can (output in ./dist)
package:
    bash packaging/build-all.sh

# Build individual package formats
package-deb:
    bash packaging/build-deb.sh
package-rpm:
    bash packaging/build-rpm.sh

# Build an .rpm from the current checkout into ./dist. The snapshot release
# (timestamp + git hash) makes every rebuild an upgrade for dnf.
rpm:
    RPM_RELEASE="1.$(date +%Y%m%d%H%M).git$(git rev-parse --short HEAD)" bash packaging/build-rpm.sh

# Build an .rpm and install it system-wide (asks for sudo)
install-rpm: rpm
    sudo dnf install -y "$(ls -t dist/*.rpm | head -1)"
package-appimage:
    bash packaging/build-appimage.sh
package-flatpak:
    bash packaging/build-flatpak.sh

# Regenerate all icon sizes from res/icons/source/ (see the script)
icons:
    bash packaging/make-icons.sh

# Runs `cargo clean`
clean:
    cargo clean

# Removes built packages
clean-dist:
    rm -rf dist
