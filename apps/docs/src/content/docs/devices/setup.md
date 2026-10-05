---
title: Set up a device
description: Create a signed setup package, register the standalone agent, and configure persistent startup.
sidebar:
  order: 2
---

Set up the agent on the computer that will run your services. You create its
setup package in Flow-Like, copy that package to the computer, and start it
there. The first start registers the device; later starts restore its saved
services and requested running state.

## Before you start

Sign in to the intended Flow-Like account and profile, then open **Devices**.
Devices, keys, and access belong to that account and hub. If the area is
unavailable, **Hub status** identifies the missing device support, release
configuration, or infrastructure checks.

The target needs a writable persistent directory owned by the account running
the agent. Keep the package and its `state` directory in a permanent location.
The state holds the device identity, service configuration, and local data;
replacing it with a fresh directory creates a different installation.

The signed release determines which packages the setup wizard offers:

| Target | Native program | Docker package |
| --- | --- | --- |
| Linux x86-64 | Supported | Supported when the release includes the image |
| Linux ARM64 | Supported | Supported when the release includes the image |
| macOS Intel | Supported | Not offered |
| macOS Apple silicon | Supported | Not offered |

A Raspberry Pi needs a compatible 64-bit Linux installation for the ARM64
package. Check the target rather than the computer on which you create the
package:

```sh
uname -s -m
```

For example, `Linux aarch64` selects Linux ARM64, and `Darwin arm64` selects
macOS Apple silicon. Docker mode requires Docker with Compose on the target.
The agent also needs outbound access to the configured hub, an accurate system
clock, and any release or container registry it must download from. A package
that defers its binary download needs `curl` and `shasum` on first start.

## Create the package

Open the device setup wizard and follow its steps:

1. **Check** verifies hub readiness, your available device slots, and the
   signed agent release. Resolve a rejected or expired release at the hub
   before continuing.
2. **Name** identifies the device. Choose a name that describes its location
   or job, such as `factory-line-4` or `home-gateway`.
3. **Platform** selects the target architecture and native program, Docker,
   or both when the release supports those choices.
4. **Password** protects the owner keys on the computer creating the package.
   This is a device password, separate from your account password. Keep an
   encrypted account backup enabled when you want to restore the keys on
   another computer.
5. **Create** makes the owner management keys and one-time setup package.
6. **Save** downloads the package and encrypted key backup file. Save the
   backup somewhere other than the browser or computer holding the active keys.

The package can only be downloaded from the window that created it. If you
lose it before starting the target, cancel the entry in **Pending setups** and
create another package. Use the package before the deadline shown in the
wizard. The target generates its permanent device keys during first
enrollment. Its one-time enrollment secret is removed from the unpacked
package after successful enrollment.

:::caution[Keep the two artifacts separate]
The setup package authorizes the first enrollment. Treat it as a credential
until it has been used. The encrypted key backup restores your ability to
manage the device and requires the device password. The setup package does
not contain that password or your owner keys.
:::

## Install and start the agent

Copy the ZIP to the target through a method you already trust, such as an
authenticated file transfer or removable storage. Unpack it into its permanent
directory and open that directory in a terminal. The wizard supplies commands
with the actual download name; this example uses `device-setup.zip`:

```sh
unzip device-setup.zip -d flow-like-device
cd flow-like-device
```

### Native program

Start the supplied script:

```sh
sh start.sh
```

The script downloads and checks the agent if necessary, enrolls it on first
start, and runs it in the foreground. Leave it running until **Waiting** shows
the first check-in. Stop the foreground process with Ctrl+C before installing
autostart:

```sh
./flow-like-standalone --state-dir ./state install-service
./flow-like-standalone --state-dir ./state service-status
```

On Linux, this installs and starts a **systemd user service**. Installation
enables user lingering so the service can start at boot without an interactive
login. If system policy prevents this, the command names the
`loginctl enable-linger` action an administrator must perform; retry
installation after that action succeeds.

On macOS, this installs a **LaunchAgent** for the current user. It starts after
that user logs in to a graphical session, including the first login after a
reboot. It does not provide unattended startup before login. Use a Linux host
when the device must return to service without anyone signing in.

The service records the executable and state paths. Keep those paths stable.
To inspect the generated service definition without installing it:

```sh
./flow-like-standalone --state-dir ./state service-unit
```

### Docker

For a package containing Docker support, use its wrapper:

```sh
sh start-docker.sh
```

The wrapper supplies the current user's UID and GID and starts the Compose
service in the background. The image is pinned by digest from the signed
release. The package directory is mounted into the container, so `state`
persists outside the container.

The generated Compose service uses `restart: unless-stopped`. Docker itself
must start at boot for that policy to restore the agent after a reboot. If the
package contains both modes, choose one agent process for that state directory.

Docker's published service port defaults to `127.0.0.1:8080` on the host.
`FLOW_LIKE_PUBLISH_HOST` and `FLOW_LIKE_SERVICE_PORT` in `.env` control that
Compose port mapping. A deployed service's own listener must match the port
and be reachable inside the container for the mapping to work. Native
listeners are configured per service in the deployment wizard.

## Confirm that management works

A first check-in confirms that the hub has heard from the device. It does not
prove that a live management connection or a deployed service is healthy.
Unlock the device in **Devices**, connect live, and inspect its reported
platform and capabilities before deploying an app.

For a native installation, inspect local state from the package directory:

```sh
./flow-like-standalone --state-dir ./state status
```

The JSON output includes enrollment state, agent process state, and services'
requested and observed state, without dumping configured variable values or
credential fields. See [Deploy and operate services](/devices/deployments/)
for reading readiness, errors, and revision information.

## Configure the host

The package contains `state/agent.env` for durable operator settings. The
agent reads it in both native and Docker modes. Restart the agent after
changing it.

The default isolation policy is `compatible`: a service may run with the
agent account's access, or with the supported Linux sandbox when configured
for that service. A host that must refuse services without a sandbox can use:

```dotenv
FLOW_LIKE_DEVICE_ISOLATION_POLICY=required
```

Setting this value does not install the required kernel and filesystem
facilities. Linux isolation also needs the configured cgroups and disk quota
support. macOS cannot provide this sandbox. Read
[Device security](/devices/security/) before selecting a policy for code
written by other people.

The generated file also sets artifact admission budgets. Defaults are 64 GiB,
262,144 filesystem entries, and 1,024 revisions per device, with 16 GiB,
65,536 entries, and 128 revisions per app. These budgets cover stored app
artifacts and uploads; they are separate from each service's runtime memory,
CPU, and disk limits. Lowering a budget does not delete old revisions. It can
block further uploads until usage fits.

CLI examples use `--state-dir ./state` explicitly. Without an override, the CLI
reads `FLOW_LIKE_STANDALONE_STATE_DIR` from `.env` in the current directory;
without either, it defaults to `.flow-like-standalone`. Running a command from
another directory without the intended state path can inspect or create the
wrong local state.

## Troubleshooting and maintenance

| Observation | Next step |
| --- | --- |
| Package expired before enrollment | Cancel the unused setup and create a new package. |
| Agent cannot download its binary or image | Check access to the release host or registry shown in setup, and the tools required by the package. |
| Registered, with no check-in yet | Leave the agent running and allow about a minute, then inspect `status` and outbound hub connectivity. |
| Enrollment is denied | Correct the clock or access problem, then run `./flow-like-standalone --state-dir ./state recover-enrollment` to recheck with the permanent device key. |
| Device checks in, but cannot be unlocked | Restore the correct keys from **Keys & recovery** and verify that the account, hub, and profile match. |
| Service installation fails | Read the reported service-manager or permissions error. `service-status` inspects the exact managed service without creating a new device identity. |

**Check for agent update** compares the running agent with the hub's verified
release. Automatic agent replacement currently requires the exact managed
Linux user service and a compatible state schema. It restarts the agent and
interrupts its services, with a watchdog that can restore the previous
verified binary if startup fails. This is separate from updating an app's
service version. Do not assume this update path applies to Docker or macOS.

To stop and remove the native autostart integration while preserving device
and project data:

```sh
./flow-like-standalone --state-dir ./state uninstall-service
```

Removing autostart does not revoke the device's registration. Use the device's
revoke action when retiring its account access, and retain the required local
data and key backups before disposing of the machine.
