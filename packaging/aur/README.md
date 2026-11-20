# Arch Linux (AUR)

`PKGBUILD` builds `nebula-reader` from the tagged GitHub source tarball against
Arch's system `libarchive`, `openssl`, `libxkbcommon`, and `wayland`, with
`NEBULA_DISTRO=1` (so updates are managed by the AUR helper rather than the
in-app GitHub banner).

## Publishing to the AUR

1. Create an AUR account at <https://aur.archlinux.org> and add your SSH public key.
2. Refresh the version and real SHA-256 for the latest release:
   ```bash
   bash packaging/aur/update-aur.sh 0.2.2
   ```
3. Clone the (initially empty) AUR package repository and push:
   ```bash
   git clone ssh://aur@aur.archlinux.org/nebula-reader.git /tmp/aur-nebula-reader
   cp packaging/aur/PKGBUILD packaging/aur/.SRCINFO /tmp/aur-nebula-reader/
   cd /tmp/aur-nebula-reader
   git add PKGBUILD .SRCINFO
   git commit -m "Update to 0.2.2"
   git push -u origin master
   ```

Arch / CachyOS / EndeavourOS users can then install with:

```bash
yay -S nebula-reader
# or: paru -S nebula-reader
```

Note: on an Arch machine you can also regenerate `.SRCINFO` directly with
`makepkg --printsrcinfo > .SRCINFO` and verify the build with `makepkg -si`.
