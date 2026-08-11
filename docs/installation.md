[< Back to docs index](index.md)

# Installing Wispr Flow for Linux

Build and install standalone packages manually; this fork configures no package
repository, signing key, or automatic-update channel.

```bash
./build.sh --build rpm
sudo dnf install --nogpgcheck build-linux/rpm/rpmbuild/RPMS/*/wispr-flow-*.rpm
```

> [!NOTE]
> The packages bundle the proprietary Wispr Flow app, downloaded from Wispr's
> official endpoint at build time. Wispr Flow is a trademark of its owners; this
> is an unofficial community port. To supply the installer yourself, pass
> `--exe /path/to/Wispr\ Flow\ Setup.exe`.

## RPM on Fedora / RHEL

Build the RPM from the repository root:

```bash
./build.sh --build rpm
```

The build downloads the pinned helper release, verifies its SHA-256 digest, and
embeds it in the RPM. The package is intentionally unsigned, so install it as a
local artifact:

```bash
sudo dnf install --nogpgcheck build-linux/rpm/rpmbuild/RPMS/*/wispr-flow-*.x86_64.rpm
```

For arm64, use the matching architecture flag and artifact:

```bash
./build.sh --build rpm --arch arm64
sudo dnf install --nogpgcheck build-linux/rpm/rpmbuild/RPMS/*/wispr-flow-*.aarch64.rpm
```

The RPM owns the application files, desktop entry, udev rule, launcher, and
helper. It does not create anything under `/etc/yum.repos.d` or import an RPM
signing key.

## Updating

Pull the source changes, rebuild, and install the new local RPM. DNF performs a
normal package replacement and removes files no longer owned by the new build.

```bash
git pull
./build.sh --build rpm
sudo dnf install --nogpgcheck build-linux/rpm/rpmbuild/RPMS/*/wispr-flow-*.rpm
```

## Other standalone formats

```bash
# Debian / Ubuntu
./build.sh --build deb
sudo apt install build-linux/deb/wispr-flow_*.deb

# AppImage
./build.sh --build appimage
chmod +x build-linux/appimage/wispr-flow-*.AppImage
build-linux/appimage/wispr-flow-*.AppImage
```

The `.deb` and `.rpm` post-install scripts install the `/dev/uinput` udev rule.
The AppImage cannot run a root post-install script, so install the rule once:

```bash
build-linux/appimage/wispr-flow-*.AppImage --install-udev-rules
```

## After installing

Run the built-in diagnostic first:

```bash
wispr-flow --doctor
```

It checks the display server, `/dev/uinput`, clipboard tooling, GNOME extension,
AT-SPI, push-to-talk input access, and launcher setup. Configuration paths and
permissions are documented in [configuration.md](configuration.md); fixes are
organized by symptom in [troubleshooting.md](troubleshooting.md).

## Uninstalling

```bash
sudo dnf remove wispr-flow      # Fedora / RHEL
sudo apt remove wispr-flow      # Debian / Ubuntu
```

User state under the paths documented in
[configuration.md](configuration.md) is intentionally preserved. Remove it
separately only when you want a full profile reset.

## See also

- [building.md](building.md) — build flags, dependencies, and supplying your own
  installer
- [compatibility.md](compatibility.md) — validated compositors and backend
  requirements
- [troubleshooting.md](troubleshooting.md) — diagnostics and symptom-keyed fixes
