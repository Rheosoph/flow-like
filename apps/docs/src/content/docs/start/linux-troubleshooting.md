---
title: Linux Troubleshooting
description: Fixes for blank windows, EGL errors and AppImage mount failures on Linux
sidebar:
  order: 25
---

Most Linux problems with Flow-Like Desktop come from the AppImage interacting
with the host graphics stack. This page covers the known symptoms and their
workarounds.

If none of these match, collect the diagnostics at the bottom of this page and
[open a GitHub issue](https://github.com/Rheosoph/flow-like/issues).

## The window is blank and the terminal says `EGL_BAD_PARAMETER`

**Symptom.** The application window opens but stays completely empty — no menu,
no controls, no reaction to keyboard shortcuts. Started from a terminal, it
prints:

```
Could not create default EGL display: EGL_BAD_PARAMETER. Aborting...
```

**Affects.** AppImage builds up to and including 0.1.7, on distributions that
ship a recent Mesa — Arch, Manjaro, EndeavourOS, CachyOS and derivatives. It
happens on both X11 and Wayland sessions.

**Cause.** Those AppImages bundle their own `libwayland-client.so.0`, and
`AppRun` puts the bundled library directory ahead of the system one. Your Mesa
EGL driver needs a symbol that only exists in newer Wayland versions, fails to
load against the older bundled copy, and the EGL setup collapses. The browser
engine treats that as fatal and terminates its rendering process, which leaves
the empty window behind.

:::caution[This is a packaging bug, not a problem with your system]
Nothing is wrong with your drivers. The workaround below makes the app use your
distribution's own Wayland libraries instead of the ones we shipped.
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
`LIBGL_ALWAYS_SOFTWARE` does **not** help with this particular failure — the
AppImage already forces an X11 backend internally, and the software renderer
lives inside the same library that fails to load.

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
bundle no graphics libraries at all, so they avoid the whole class of problem
above. On Debian and Ubuntu derivatives, install the `.deb` normally.

On distributions without a matching package manager you can still run the
payload directly. On Arch and Manjaro:

```bash
sudo pacman -S --needed webkit2gtk-4.1 gtk3 libayatana-appindicator
mkdir -p /tmp/flow-like && cd /tmp/flow-like
bsdtar -xf ~/Downloads/Flow.Like_0.1.7_amd64.deb
bsdtar -xf data.tar.* -C /tmp/flow-like
./usr/bin/flow-like-desktop
```

This is not a managed installation — there is no package database entry and no
automatic updates — and the binary must be started from inside the extracted
tree, because it locates its bundled AI runtime libraries relative to itself.

## System requirements

The Linux builds are compiled on Ubuntu 24.04. They require glibc 2.39 or newer,
which means Ubuntu 24.04+, Debian 13+, Fedora 40+, or any current rolling
release. Older distributions are not supported by the prebuilt artifacts; build
[from source](/dev/build/) instead.

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

If `eglinfo -B` fails on its own — without Flow-Like involved — the problem is in
the host graphics stack rather than in our packaging. A partial system upgrade or
a driver update without a reboot are the usual causes.
