---
title: Deploy and operate services
description: Select app events, pin versions, configure device settings, and manage service updates and outages.
sidebar:
  order: 3
---

Deploy an app's events to an enrolled device when they should run without
Studio remaining open. A service combines the selected events with a fixed
app version, device-specific settings, and its runtime limits. The agent
restores services requested to run when it starts again.

Start from a device's page or the app's **Devices** settings. The deployment
wizard supports one or several devices and shows the plan for each before
applying it. The devices must be online, unlocked, and accessible with
deployment permission.

## Prepare the app

Define the entry points you want to run under [Events](/apps/events/). Devices
support the following workload types, subject to the installed agent's
advertised capabilities:

| Event or entry point | How it runs on a device |
| --- | --- |
| Page, chat, HTTP endpoint, or API event | The service's HTTP host serves the interface or route. |
| REST or MCP server | The Flow binds its own listener and reports readiness through its server node. |
| Daemon | The Flow initializes, calls **Service Ready**, and continues running. |
| Repeating or one-time schedule | The agent starts runs at the configured times. |
| Quick action or form | An authorized person starts the event on demand. |
| Telegram or Discord bot | The device maintains the provider connection using a provisioned bot token. |

The wizard lists ineligible events and the reason they cannot run on a
selected device. A daemon that merely starts a process has not demonstrated
readiness. Its Flow must call [Service Ready](/nodes/web/services/service-ready/)
after initialization, and then remain alive. A form that takes a file needs
a service Page through which the file can be submitted.

Expose values that differ between installations as
[runtime variables](/apps/runtime-variables/). For example, use a runtime
variable for a branch's internal API address and a secret variable for its
credential. Device deployments provision their own values; they do not
automatically inherit secrets saved in another user's Desktop setup.

## Select events and services

In **What**, select the app and choose **Whole app**, **Choose events**, or
**One event**. Whole app selects eligible events, but schedules and bots need
an explicit selection because deployment can move the place where they run.

Choose one service for the selected events when they should share settings,
cloud approval, and service access. Choose one service per event when they
need separate credentials, limits, or lifecycles. Events that disagree about
a shared variable may need separate services.

Each service has a permanent service ID. A device cannot reuse a retired
service ID, and the wizard adjusts a new ID if another service already uses
it. The CLI calls the same unit a placement. A deployment ID associates it
with cloud approvals and is also permanent.

In **Where**, select the target devices and review differences. An event may
be valid on one device and unsupported on another. Each target receives its
own installed service and settings; a multi-device deployment is not a single
shared process or shared local database.

## Understand the version being installed

The deployed version pins an exact version of each event and its referenced
Flow. Later edits in Studio do not silently change a running service.

When an event follows **Latest**, preparation resolves it to the Flow as it
exists at that moment. It can create a Flow version from current edits; that
version remains in the Flow's history while the event continues to follow
Latest in the authoring app. Review this when deploying work that is still
being edited.

For an update, choose the newest version or keep each service's installed
version and change only its settings or selected events. Keeping a version
can only select events already present in that installed version. The wizard
does not reconstruct arbitrary older versions for a new deployment.

An app version, a **settings version**, and a **start/stop request** describe
different changes. The app version identifies pinned executable content;
the settings version changes when its configuration changes; the request
number changes for start, stop, or scaling. Use the service's **Requested vs
actual** view to see whether the device has applied your latest request.

## Choose the data behavior

The app determines how it runs: local-only apps receive an offline copy;
online apps use approved cloud data. **How it runs** prepares the content
and reports the models, packages, files, and hashes involved.

### Local-only apps

Prepare an offline copy in Desktop on the computer that holds the app. A
browser cannot read a local-only app's files. Preparation checks pinned
definitions and dependencies before uploading the same verified copy to each
target.

The copy includes tables with their current rows, required local files,
and selected user data. It does not reproduce table history or search
indexes. Flows depending on that history or those indexes need adjustment
before export. Prepare a consistent copy while nothing else is writing to
the app. The transfer has limits of 8,192 files, 4 GiB per file, and 8 GiB in
total; the device's remaining artifact budget can impose a lower limit.

On first start, each service initializes its own mutable local data from the
copy. Later app updates replace executable content and keep the service's
existing data. Uploading a changed table in a new copy does not reset an
existing service's table. Create a new service when you deliberately need a
new data initialization, and treat schema changes as migrations of the data
already on the device.

### Online apps

The controller approves and pins the executable definitions sent to the
device. Project data remains in cloud storage. Each service on each device
needs its own cloud approval; merely owning or being able to deploy to the
device does not grant access to an app's cloud data.

In **Access & cost**, select the approved resources and expiration. New online
services require an app Admin or Owner with **Execute boards** to approve
cloud access. Access to project files requires the app owner's approval.
Choose read-only access when the Flow does not need to change data.

Hosted models require a spending limit. The limit belongs to one approval,
so deploying the same amount to several devices creates several budgets.
Review the total before deployment. Existing service updates retain their
cloud approval unless you change it separately.

The runtime receives cloud credentials in short leases, normally ten minutes
at a time. The service's **Cloud access** view shows its approval separately
from process health. Valid credentials do not prove the service is ready.

## Set values, listeners, and limits

In **Settings**, supply the exposed values and secret inputs for the selected
events. Values may differ per device. Events in one service share a value for
the same variable; divide the events into separate services if that is not
the desired behavior.

Secret updates travel through the encrypted management connection and are
encrypted while queued for installation. Installed values are private files
on the device, readable by the runtime account, and cannot be read back
through the configuration UI. Protect the host and its backups accordingly.
Use **Change secret value** to replace an existing secret. A secret intended
for a newly staged version can be supplied with that update so the currently
running version keeps its existing value until the switch.

For hosted services, **Endpoint & limits** configures the listener, service
access token, timeout, and request limits. Tokens are generated by default.
Choose **No token** to let anyone who can reach the listener use the service's
endpoints, Pages, chats, and actions without a token. This setting applies to
the whole service. When updating, **Keep current** preserves whether each
service requires a token.

All hosted interfaces in a service share its access settings. An endpoint's
separate token from Events is not used on a device. Keep the listener private and
use [service access and forwarding](/devices/service-access/) unless LAN or
public access is part of the intended deployment.

Select the service's isolation mode and resource limits according to the
device's reported policy. Linux can provide the supported sandbox and
resource budgets; macOS services run with the agent account's access. See
[Device security](/devices/security/) for the host requirements and network
boundary.

The configured maximum is between 1 and 32 instances. Multiple instances
require the native HTTP service host. REST/MCP servers, daemons, schedules,
bots, and write-buffering services are limited to one instance. A standalone
quick action or form also needs one instance unless the service includes a
hosted interface. Increasing the maximum alone does not request that many
running instances; use the service's instance controls to scale it.

## Apply and verify

Review each device's events, version, settings, access, listener, and expected
running state. For several targets, choose all at once, one device at a time,
or a first device followed by the rest. Select whether other targets wait
after a failure. The rollout tracks each target separately, so a partial
success means some devices already have the new version.

Finish any required copy/upload, apply the deployment, and follow its rollout.
An interrupted upload can be resumed while its reservation remains valid.
Uploading content alone does not switch an existing service.

After the operation finishes, open the service's **Status** view and confirm
the installed version, requested state, and ready instance count. Use
**Activity & logs** for reported errors, **Metrics** for resource use, and
**Endpoint** to test the intended access path. Test an actual request or
automation outcome after readiness succeeds.

## Start, stop, and update

**Start** requests that the service run and clears its crash-loop retry
limit. **Stop** requests a graceful shutdown while preserving configuration
and data. The agent's default restart policy retries consecutive failures
five times, with backoff from two to sixty seconds. After that, investigate
the reported error before starting it again.

An update can replace the app version or change the events and settings. A
stopped service stays stopped when updated. Updating a running service uses
the available strategy shown in Review:

- **Quick update** applies the configuration and restarts the service.
- **Safe update** stages and validates the candidate, then switches to it
  and waits for readiness and a stabilization period. If it fails after the
  switch, the agent attempts to restore the previous configuration and checks
  that version's health too.

The old version keeps running during preparation and validation. The switch
can interrupt requests; safe update is not a zero-downtime guarantee. A
failure to restore the previous version leaves the service stopped and is
reported in Status.

A staged update can be activated or discarded before switching begins and
expires after 24 hours if it is not activated. The device retains the last
32 finished update records. Rollback restores configuration and executable
version, not the mutable data already written by either version. Design data
migrations so the prior version can still read the resulting data if you
rely on automatic rollback.

To remove a service, stop it first and use its remove action. Removal retires
the service ID and preserves its data. Cloud approvals and spending limits
have a separate lifecycle; review or revoke them under **Cloud access** when
they are no longer needed.

## Operate through network outages

An installed offline service with local dependencies can continue without hub
connectivity. You cannot deploy changes or open a new remote management
session while the device is unreachable. External APIs and hosted models
still need their respective connections.

Online services maintain a read cache, with a default budget of 512 MiB per
placement. Cached reads cover content already fetched and validated. A cache
miss can still fail during an outage, and expired or explicitly denied
authorization prevents continued access. A restarted service also needs
valid retained outage state; a device that has never fetched the required
content cannot recover it from an empty cache.

Optional **Write buffering** is a defined queue for selected tables and file
folders. It requires read/write cloud access and one service instance.
Choose the table key columns, folder scopes, and budgets for change count,
size, and age. Once a budget is exceeded, the device refuses further buffered
changes and the Flow receives an error.

When connectivity returns, the agent reconciles eligible queued changes.
The service's **Write buffering** view identifies paused queues, conflicts,
and uncertain outcomes that need a decision. Replacing an app version does
not make those decisions for you. An offline app's local data needs no such
queue because its writes are already local.

Schedules also have timing limits. Repeating schedules do not make up missed
runs. A one-time schedule can run up to 15 minutes after its configured time;
after that it is missed. A schedule or bot tied to an online app also depends
on the hub's execution claim, so check its reported held or active state
during connectivity problems. Review multi-device schedules carefully:
separate local copies can each fire the same job.

## Local CLI operations

For recovery on a native installation, use the agent from its package
directory and point every command at the same state:

```sh
./flow-like-standalone --state-dir ./state status
./flow-like-standalone --state-dir ./state stop branch-api
./flow-like-standalone --state-dir ./state start branch-api
```

Replace `branch-api` with the service ID. Start and stop persist the requested
state; the running agent carries out the request. `status` reports requested
versus observed state, applied configuration, processes, replica readiness,
and the last error without printing configuration values. When
`observations_current` is false, the agent is not running and the saved
observations are not evidence that a process is still healthy.

Operators can also apply a prepared **offline placement manifest** locally.
This requires an existing project object-store root with
`apps/<project_id>/manifest.app`, exact event and Flow archives, and all
required files and dependencies. It is an advanced path for an already
prepared snapshot, not an export command for a live app.

For example, `placement.json` can describe a daemon with these pins:

```json
{
  "id": "local-worker",
  "project_id": "my-project",
  "deployment_id": "worker",
  "revision": "release-1",
  "source": "offline",
  "project_path": "./project-store",
  "events": [
    {
      "event_id": "daemon-event",
      "event_version": [1, 0, 0],
      "board_version": [1, 0, 0]
    }
  ],
  "variables": {}
}
```

Replace the example IDs and pins with those in the snapshot. `project_path`
resolves relative to the manifest. Export a private environment template for
its exposed runtime variables:

```sh
./flow-like-standalone --state-dir ./state export-env placement.json --output variables.env
```

Edit the generated template, preserving its keys and single-quoted JSON
values. Secret values are omitted until you supply them. Keep the file
private, then validate and apply it:

```sh
./flow-like-standalone --state-dir ./state apply placement.json --variables-env variables.env
```

The import validates values against the pinned variable schemas and installs
secret inputs into the device's secret store. The agent must be running to
start the placement. Use `apply --help` for the manifest requirements;
hosted interfaces and additional pinned assets need their corresponding
configuration. Reapplying an existing placement preserves its requested
running or stopped state.
