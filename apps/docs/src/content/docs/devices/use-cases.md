---
title: Company edge and home automation
description: Design a company edge layer or home automation deployment around local workflows, existing equipment, scoped remote access, and explicit outage behavior.
---

Device deployment is useful when a workflow needs to run close to the things
it reads or controls. A company can place a Flow-Like runtime at each site. A
household can run one on an always-on computer beside its automation gateway.
In both cases, the workflow executes on that host while authorized people
manage it from Flow-Like Desktop or the web app.

The device is a supported Linux or macOS computer running the standalone
runtime. Sensors, switches, and controllers can remain on their existing
platforms. Flow-Like reaches them through the APIs or protocols exposed by that
equipment. A Linux ARM64 gateway can be a target when its operating system,
available release, and workload dependencies meet the
[setup requirements](/devices/setup/); a microcontroller is not a standalone
runtime target.

## Build an edge layer for a company

A site runtime gives centrally maintained workflows access to local systems
without requiring every local system to become an internet-facing endpoint.
An operator deploys pinned Event and Flow versions, supplies site-specific
configuration, and checks the resulting service from the device workspace.

| Site requirement | Example workflow | What stays at the site |
| --- | --- | --- |
| Equipment inspection | Read a local inspection API, evaluate results, and store exceptions | Raw measurements and any local processing you configure |
| Branch document handling | Process files near a scanner, extract fields, and send approved results upstream | Source files when the workflow uses local storage and local models |
| Internal operator tools | Serve an app page, form, or quick action on the site's private network | The service and its execution; online app data may still live in the hub |
| Periodic reconciliation | Run a schedule against a local system and update an approved cloud table | Local input and processing; queued writes require explicit configuration |

These are deployment patterns. Each depends on the equipment interface, nodes,
credentials, and runtime packages the workflow uses.

### Example: inspection at several workshops

Suppose each workshop has an inspection machine with a local HTTP API. The
company wants a shared validation workflow, a local operator view, and a
central record of exceptions.

1. Build and test the app with a sample machine response. Separate the machine
   address and credentials from the Flow definition using runtime configuration.
2. Enroll one device at each workshop under the appropriate owner. Give each
   device a name that identifies its site and purpose.
3. Deploy the released Event and Flow versions to a pilot device. Configure the
   local machine address, secrets, storage, and any model dependencies there.
4. Use a schedule to poll the machine, or an authenticated HTTP endpoint if the
   machine can send requests with the service's bearer token. Keep that listener
   on the required private interface.
5. If results go to an online table, approve only the needed resource access.
   Decide whether the workflow stops, continues with cached data, or buffers
   eligible writes during a connectivity outage.
6. Validate the actual operator result before deploying to the remaining sites.
   Updates can restart services, so choose a maintenance window appropriate to
   the process.

Give site support staff status, logs, or restart access within the required
scope. Reserve deployment changes and host-level operations for the people
responsible for those changes. Treat service tokens as credentials for the
underlying workflow, separate from management permissions.

The company can use a self-hosted hub alongside the devices. The hub remains
the coordination and authorized cloud-resource layer; site devices execute
their assigned workloads. Existing databases, identity systems, and equipment
remain integrations with their own access controls.

### Decide what an outage means

An offline app copy can run without fetching its definition from the hub once
its dependencies are present. That alone does not make an online database,
hosted model, or remote API available offline. Online placements may use
configured caches and supported queued writes, subject to their grants,
validity, and capacity limits.

Each offline service starts with its own data copy. Later app updates preserve
that service's data; changes do not automatically synchronize back to the
authoring computer or other devices. Choose an online deployment when the
workflow needs shared hub tables and files, and test its outage limits.

Test loss of internet access, loss of the local equipment connection, and a
device restart separately. Check what was actually processed before retrying
an operation with external side effects. Review the write queue and conflicts
after reconnection. [Deployment operations](/devices/deployments/) explains
these controls and configuration recovery.

## Run home automation on a local gateway

An always-on host can coordinate home services while a laptop used for editing
is asleep. Keep the existing home automation controller responsible for the
devices it already supports, and use Flow-Like for workflows that combine its
API with schedules, local data, or an operator page.

For example, a supported Linux host could call an existing home automation
gateway's authenticated HTTP API to read room state and request a lighting
scene. Store the gateway credential as a device secret. The Flow and the target
configuration decide which rooms and operations it can reach.

### Example: a household scene controller

1. Test a Flow that reads the gateway state and performs one limited action,
   such as selecting an existing lighting scene.
2. Add a quick action or form for manual use. Add a schedule only if the action
   should run unattended.
3. Deploy the app to the always-on host. Supply the gateway address and a
   credential limited to the required operations.
4. For a household browser page, enable the service endpoint on the appropriate
   private interface, configure TLS for network access, and protect it with its
   service token. Open the bundled `/ui/` page from the home network.
5. Test with the editing laptop closed. Then disconnect the internet while
   retaining the home network and check which actions still work.

Keep local credentials, model files, and required data on the host for actions
that must work during an internet outage. A notification sent through Telegram,
Discord, or another cloud provider still depends on that provider and an
internet connection. Local schedules and their missed-run behavior also need
testing after a restart. Device Cron schedules run at most once per minute,
use the saved Event time zone or UTC, and skip missed or overlapping runs.
Use a long-running listener for reactions that need to happen between ticks.

Use remote device management to inspect status, update configuration, and run
supported quick actions or forms when away from home. Encrypted service
tunneling provides on-demand access to a deployed listener. See
[Service access and port forwarding](/devices/service-access/) for the Studio
and web app access available in your client.

### React to MQTT messages

MQTT is a messaging protocol commonly used by sensors and automation gateways.
The standalone runtime includes MQTT workflow nodes, so a Daemon Event can
subscribe to an existing broker without a dedicated MQTT Event type. The same
pattern can consume equipment messages at a company site.

1. Create a Flow starting with a Simple Event node, then configure a
   **Daemon** Event for that entry point.
2. Connect it to [MQTT Connect](/nodes/web/mqtt/mqtt-connect/) with the broker
   address, a client ID unique to this service, and the required credentials
   and TLS configuration. Keep credentials in secret runtime variables.
3. Pass **Session** and **Done** into
   [MQTT Subscribe](/nodes/web/mqtt/mqtt-subscribe/). Choose a topic filter such
   as `home/rooms/+/temperature` and set **Timeout (s)** to `0` to keep listening.
4. Reference a handler function from Subscribe to process each message. Give
   its entry node `topic` and `payload` outputs for the incoming topic and
   message, then connect its work to validation and the intended local action.
5. Connect Subscribe's **On Subscribed** output to
   [Service Ready](/nodes/web/services/service-ready/). Subscribe remains active
   while that readiness branch returns. Connecting Service Ready after
   **On Close** would leave startup waiting until the subscription ends.
6. Handle **Error** and **On Close**, and test recovery with the broker stopped.
   When the whole Daemon Flow returns, its service stops. Configure restart
   behavior deliberately instead of assuming the subscription always reconnects.

The current **On Subscribed** signal means that the client queued its
subscription request; it does not wait for the broker's acknowledgement.
Service Ready therefore reports that the listener workflow has started.
Verify a real message and the handler's output before treating the equipment
path as healthy.

Use [MQTT Publish](/nodes/web/mqtt/mqtt-publish/) for commands to topics that
your equipment already supports. Restrict the broker account to the topics
the workflow needs. MQTT delivery settings do not establish exactly-once
physical actions: validate payloads and make repeated commands safe for the
target equipment.

### Work with the hardware's existing interface

MQTT and HTTP support do not provide a driver for every radio or fieldbus.
Use the existing controller or a compatible bridge when equipment needs
Zigbee, Matter, or another protocol that your workflow cannot speak directly.

USB radios, GPIO, serial devices, and other host resources also need a supported
integration and operating-system permissions. A container requires explicit
access to any hardware it uses. Check these dependencies on the target before
moving an automation out of its original controller.

Keep independent controls for actions where failure could cause harm. Heating,
locks, alarms, and similar equipment should retain their own limits and manual
operation. A general workflow runtime does not establish real-time control or
a safety certification.

## Choose access to fit the job

| Need | Access approach |
| --- | --- |
| Maintain the device remotely | Use authenticated device management; the device initiates outbound connectivity |
| Run a supported quick action or form remotely | Use the device action in Desktop or the web app, subject to its input and permission limits |
| Use a dashboard on the same private network | Open the deployed service's `/ui/` page with its service token |
| Reach a private service through an on-demand tunnel | Use scoped service access; see [Service access and port forwarding](/devices/service-access/) for client support |
| Receive calls from an external provider | Provide a deliberately reachable authenticated endpoint, or use an integration that polls outward |

For either environment, start with one device and one workflow. Verify the
network path, service credentials, output, restart behavior, and update recovery
before expanding the deployment. The [device security guide](/devices/security/)
explains why avoiding public management ports reduces exposure and which
responsibilities remain on the operator.
