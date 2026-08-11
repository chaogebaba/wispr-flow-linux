# Wispr Flow for Linux (unofficial)

[![CI](https://github.com/chaogebaba/wispr-flow-linux/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/chaogebaba/wispr-flow-linux/actions/workflows/ci.yml?query=branch%3Amain)
[![License: Unlicense](https://img.shields.io/badge/license-Unlicense-blue.svg)](UNLICENSE)

This project provides build scripts to run the proprietary **Wispr Flow**
voice-dictation app natively on Linux. It repackages the Windows installer and
pairs it with a **clean-room Rust helper**, producing `.deb` packages
(Debian/Ubuntu), `.rpm` packages (Fedora/RHEL), and distribution-agnostic
AppImages for amd64 and arm64, plus a Nix flake. The helper reimplements the one
native capability Wispr Flow ships only for macOS and Windows: injecting
transcribed text into your focused application.

**This is an unofficial port.** I'm not affiliated with Wispr. For the official
app and support, see [wisprflow.ai](https://wisprflow.ai). If you hit a
build-script or Linux issue,
[open an issue](https://github.com/wispr-flow-linux/wispr-flow-linux/issues) here.

**Documentation:** full docs at [`docs/index.md`](docs/index.md). Build details
in [`docs/building.md`](docs/building.md). Release history in
[`CHANGELOG.md`](CHANGELOG.md). Contributing: [`CONTRIBUTING.md`](CONTRIBUTING.md).
Security: [`SECURITY.md`](SECURITY.md).

## Installation

This fork is local-build-only: it publishes no package repository, signing key,
or automatic-update channel. Build the package you need and install it manually:

```bash
# Fedora / RHEL
./build.sh --build rpm
sudo dnf install --nogpgcheck build-linux/rpm/rpmbuild/RPMS/*/wispr-flow-*.rpm

# Debian / Ubuntu
./build.sh --build deb
sudo apt install build-linux/deb/wispr-flow_*.deb
```

The helper source lives in [`helper/`](helper/) and is compiled for the package
architecture during staging. Set `HELPER_BIN` only when intentionally supplying
a matching prebuilt helper.

> [!NOTE]
> These packages bundle the proprietary Wispr Flow app, downloaded from Wispr's
> official endpoint at build time. Wispr Flow is a trademark of its owners; this
> is an unofficial community port.

## Building

By default `build.sh` downloads the Wispr Flow installer from Wispr's official
endpoint at build time; the repository never bundles or commits it. Build a
package with:

```bash
# Build an .rpm (downloads the installer automatically)
./build.sh --build rpm

# ...or point it at an installer you already have
./build.sh --build rpm --exe ~/Downloads/"Wispr Flow Setup-v1.6.7.exe"
```

`--exe` is optional: without it, `build.sh` fetches the latest installer and
verifies it matches the pinned version; with it, the build uses your local `.exe`
and never fetches the proprietary app.

Here are the common options (`./build.sh --help` lists all):

- `-b, --build <deb|rpm|appimage|nix>` — package format (default: auto-detected)
- `--arch <amd64|arm64>` — target architecture (default: host)
- `-e, --exe <path>` — installer .exe to use (optional; default: fetch latest)
- `-c, --clean <yes|no>` — remove intermediate build files when done

I cover prerequisites, the Linux Electron download, the native sqlite rebuild, and
the mandatory launcher rename in [`docs/building.md`](docs/building.md).

## Configuration

I documented the environment variables, state locations, the uinput udev rule,
clipboard dependencies, the GNOME extension, and AT-SPI in
[`docs/configuration.md`](docs/configuration.md).

## Troubleshooting

Run `wispr-flow --doctor` first. It's the built-in diagnostic, and it checks the
display server / session, `/dev/uinput` access, clipboard tooling, the GNOME
extension, AT-SPI, push-to-talk input access, and the launcher rename. When
something breaks, I keep symptom-keyed fixes in
[`docs/troubleshooting.md`](docs/troubleshooting.md).

## License

Build scripts and the Rust helper in this repository are released into the public
domain under the [Unlicense](UNLICENSE). The Wispr Flow application itself is
proprietary and subject to its own terms.
