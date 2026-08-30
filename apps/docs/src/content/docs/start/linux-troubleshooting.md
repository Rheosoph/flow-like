---
title: Linux Troubleshooting
description: Fixes for glibc errors, blank windows, EGL errors and AppImage mount failures on Linux
sidebar:
  order: 25
---

Linux startup failures usually come from a system-library baseline mismatch or
from the AppImage interacting with the host graphics stack. This page covers
the known symptoms and their workarounds.

If none of these match, collect the diagnostics at the bottom of this page and
[open a GitHub issue](https://github.com/Rheosoph/flow-like/issues).

## The window is blank and the terminal says `EGL_BAD_PARAMETER`

**Symptom.** The application window opens but stays completely empty: no menu,
no controls, no reaction to keyboard shortcuts. Started from a terminal, it
prints:

```
Could not create default EGL display: EGL_BAD_PARAMETER. Aborting...
```

**Affects.** AppImage builds up to and including 0.1.7, on distributions that
ship a recent Mesa, including Arch, Manjaro, EndeavourOS, CachyOS and their
derivatives. It happens on both X11 and Wayland sessions.

**Cause.** Those AppImages bundle their own `libwayland-client.so.0`, and
`AppRun` puts the bundled library directory ahead of the system one. Your Mesa
EGL driver needs a symbol that only exists in newer Wayland versions, fails to
load against the older bundled copy, and the EGL setup collapses. The browser
engine treats that as fatal and terminates its rendering process, which leaves
the empty window behind.

:::caution[The package caused this failure]
Your drivers are working. The workaround below makes the app use your
distribution's own Wayland libraries instead of the copies we shipped.
:::

**Fix.** Extract the AppImage and remove the bundled Wayland libraries:

```bash
APP=~/Downloads/Flow.Like_0.1.7_amd64.AppImage   # adjust to your download
chmod +x "$APP"
"$APP" --appimage-extract
cd squashfs-root
mkdir -p disabled && mv usr/lib/libwayland-*.so* disabled/
./AppRun
```

Run `./AppRun` from the extracted directory from now on, or create a desktop
entry pointing at it. If it fails with an "undefined symbol" error instead, undo
with `mv disabled/* usr/lib/` and use the `.deb` payload described below.

**Alternative without extracting.** Preload your system's copy over the bundled
one. This is load-order dependent, so it does not work everywhere, and it can
cost noticeably more CPU:

```bash
LD_PRELOAD=$(ldconfig -p | awk '/libwayland-client\.so\.0/{print $NF; exit}') \
  ~/Downloads/Flow.Like_0.1.7_amd64.AppImage
```

Setting `GDK_BACKEND`, `WEBKIT_FORCE_COMPOSITING_MODE` or
`LIBGL_ALWAYS_SOFTWARE` does **not** help with this particular failure. The
AppImage already forces an X11 backend internally, and the software renderer
lives inside the same library that fails to load.

## The terminal reports `GLIBC_2.38` or `GLIBC_2.39` not found

**Symptom.** The AppImage exits before opening a window and prints one or more
`version 'GLIBC_2.38' not found` or `version 'GLIBC_2.39' not found` messages.
The `.deb` package installs, but its application does not start either.

**Affects.** Prebuilt Linux artifacts up to and including version 0.1.8 on
distributions with glibc older than 2.39. This includes Ubuntu and Pop!_OS
22.04 with glibc 2.35, and Debian 12 with glibc 2.36.

**Cause.** The affected Linux artifacts were built using Ubuntu 24.04 userland.
The desktop executable and libraries copied into its AppImage require newer
glibc symbols than older distributions provide. AppImage uses the host's glibc,
so extracting the AppImage or installing the `.deb` cannot resolve this
mismatch.

**Fix.** Install Flow-Like 0.1.9 or later. These releases use a glibc 2.35
baseline. If that release is not available yet, use the web application or
build [from source](/dev/build/) on the target distribution.

Do not replace the system glibc manually. Ubuntu treats it as a core system
component, and replacing it can leave the operating system unusable.

## The AppImage will not start at all

**Symptom.** Nothing opens. Depending on the distribution you see a mount
failure, a message about `libfuse.so.2`, or the process exits immediately.

**Affects.** The 0.1.7 `amd64` AppImage specifically. It was packaged with an
older AppImage runtime that requires FUSE 2 at runtime, which is not installed
by default on Arch, Fedora, Debian 13 or Ubuntu 24.04.

**Fix.** Either run it without mounting at all:

```bash
APPIMAGE_EXTRACT_AND_RUN=1 ~/Downloads/Flow.Like_0.1.7_amd64.AppImage
```

Or extract it once and run `./AppRun`, as in the previous section. Installing
your distribution's FUSE 2 compatibility package (`fuse2` on Arch and Manjaro)
also works.

## Using the `.deb` or `.rpm` instead

Every release publishes `.deb` and `.rpm` packages next to the AppImage. These
link against your distribution's own web engine and graphics libraries and
bundle no graphics libraries, so they avoid AppImage-specific graphics library
conflicts. They contain the same desktop executable and keep the release's
glibc requirement. In particular, the 0.1.8 `.deb` does not resolve the
`GLIBC_2.38` or `GLIBC_2.39` error. On Debian and Ubuntu derivatives, install a
compatible release's `.deb` normally.

On distributions without a matching package manager you can still run the
payload directly. On Arch and Manjaro:

```bash
sudo pacman -S --needed webkit2gtk-4.1 gtk3 libayatana-appindicator
mkdir -p /tmp/flow-like && cd /tmp/flow-like
bsdtar -xf ~/Downloads/Flow.Like_0.1.7_amd64.deb
bsdtar -xf data.tar.* -C /tmp/flow-like
./usr/bin/flow-like-desktop
```

This is an unmanaged installation. There is no package database entry or
automatic update support. Start the binary from inside the extracted
tree, because it locates its bundled AI runtime libraries relative to itself.

## System requirements

Linux artifacts from version 0.1.9 onward are built on Ubuntu 22.04 and require
glibc 2.35 or newer. This includes Ubuntu and Pop!_OS 22.04+, Debian 12+, and
newer releases of those distributions. Prebuilt Linux artifacts up to and
including version 0.1.8 require glibc 2.39.

## Collecting diagnostics for a bug report

If the workarounds do not help, this is the information that makes a report
actionable:

```bash
{
  inxi -Gxxx 2>/dev/null || lspci -nnk | grep -A3 -Ei 'vga|3d|display'
  echo "session=$XDG_SESSION_TYPE"
  ldd --version | head -1
  eglinfo -B 2>&1 | head -20
} > /tmp/flow-like-sysinfo.txt 2>&1
```

`eglinfo` comes from `mesa-utils` or `mesa-demos`. Attach
`/tmp/flow-like-sysinfo.txt` together with the complete terminal output of the
failed launch.

If `eglinfo -B` fails on its own, without Flow-Like involved, the problem is in
the host graphics stack. A partial system upgrade or a driver update without a
reboot are the usual causes.
