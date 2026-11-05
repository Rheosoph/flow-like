# Bot playgrounds on Linux devices

A bot's computer and browser run on the device in a private virtual display (Xvfb, a small window manager, headful
Chromium). The owner can watch the display from the app. The agent starts all of this on demand; you only install the
pieces. Without Xvfb, browser use still works headless and computer use reports "install xvfb".

## Container

Use the `-playground` tag next to the normal image, for example `ghcr.io/<owner>/flow-like-standalone:sequence-<n>-playground`.
It contains Xvfb, xauth, openbox, Chromium, emoji/CJK/Latin fonts and runs as user 65532 with the same volume
(`/var/lib/flow-like`) and entrypoint as the base image.

Chromium keeps its sandbox on and never runs with `--no-sandbox`. Inside a container that needs two things:

- A seccomp profile that allows `clone`, `clone3` and `unshare` for user namespaces. Docker's and Kubernetes'
  default profile blocks them ("No usable sandbox!"). Use `apps/standalone/packaging/chrome-seccomp.json`, which is
  Docker's default profile with those three calls allowed. No added capabilities are needed.
- A larger `/dev/shm`: `--shm-size=1g` (Docker's default is 64 MB).

```
docker run -d --name flow-like \
  --security-opt seccomp=chrome-seccomp.json \
  --shm-size=1g \
  -v flow-like-state:/var/lib/flow-like \
  ghcr.io/<owner>/flow-like-standalone:sequence-<n>-playground
```

Kubernetes: set `securityContext.seccompProfile` to `Localhost` with the file installed on the node, and mount an
`emptyDir` with `medium: Memory` at `/dev/shm`.

Do not use `--cap-add SYS_ADMIN` or `--privileged` as a shortcut; both widen what code in the container can do to the host.
The setuid `chrome-sandbox` helper does not help in containers and is not used.

## Bare-metal Linux

Install a virtual display stack and a browser:

| | Debian / Ubuntu | Fedora |
|---|---|---|
| Virtual display | `xvfb xauth` | `xorg-x11-server-Xvfb xorg-x11-xauth` |
| Window manager | `openbox` | `openbox` |
| Fonts | `fonts-liberation fonts-noto-color-emoji fonts-wqy-microhei` | `liberation-fonts google-noto-color-emoji-fonts wqy-microhei-fonts` |
| Browser | `chromium` (Debian) or Google Chrome | `chromium` or Google Chrome |
| Runtime library | `ocl-icd-libopencl1` | `ocl-icd` |

Ubuntu ships Chromium only as a snap, which the agent uses only as a last resort. Install Google Chrome or let the agent
download Chrome for Testing. Ubuntu 23.10 and later restrict unprivileged user namespaces with AppArmor; if the browser
reports "No usable sandbox", run `sudo sysctl kernel.apparmor_restrict_unprivileged_userns=0` or use Google Chrome, which has an AppArmor profile.

The agent runs as the `systemd --user` unit it installs. A headless machine needs no display variables: with no `DISPLAY`
and Xvfb installed the agent starts a private `Xvfb` per bot. If the user manager has imported a real `DISPLAY` (a desktop
login), bots use that screen one at a time instead; to keep them on private virtual displays, add a drop-in with
`systemctl --user edit <unit>`:

```
[Service]
Environment=FLOW_LIKE_PLAYGROUND=virtual
```

Use `FLOW_LIKE_PLAYGROUND=off` the same way to disable computer use on a device. On a headless machine without a login
session, keep the user manager running across logouts and reboots:

```
sudo loginctl enable-linger "$USER"
```
