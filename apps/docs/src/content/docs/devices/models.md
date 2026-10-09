---
title: Host models on a device
description: Run chat, vision, embedding, and decision models on your own device and serve them to its services and your desktop workflows.
---

A device can host AI models and serve them to two kinds of callers: the
services deployed on it, and you and the people you share the device with.
The device's agent runs one model host that owns every model process on the
device. It downloads model files itself and answers requests through an
OpenAI-compatible gateway on the device's loopback interface. Remote requests
reach that gateway through the device's encrypted tunnel, so the hub never
sees prompts or answers.

Model hosting needs a device agent that supports it. If the agent is too old,
the **Models** tab says so; update the agent first. Services that are already
deployed keep running either way.

## What a device can host

| Kind | Model files | Engine | Runs on |
| --- | --- | --- | --- |
| Chat | GGUF | llama.cpp | The processor, or a GPU through a GPU runtime |
| Vision | GGUF plus its projector file | llama.cpp | The processor, or a GPU through a GPU runtime |
| Decisions (SystemOne) | Native decision-model GGUF, with a projector for image input | llama.cpp | The processor, or a GPU through a GPU runtime |
| Embedding | GGUF | llama.cpp | The processor, or a GPU through a GPU runtime |
| Embedding | ONNX with its tokenizer files | ONNX Runtime, built into the agent | The processor |
| Chat or vision | MLX folder: config, tokenizer files, and safetensors weights; a vision model also its processor config | MLX | The GPU of an Apple silicon Mac, through Metal |

MLX models run only on Apple silicon Macs. An MLX model answers one request
at a time, so **Parallel requests** doesn't apply to it. A reasoning model's
thinking text stays inside its answer, because MLX doesn't split it out the
way llama.cpp does. There is no CUDA runtime; NVIDIA GPUs on Linux use the
Vulkan runtime.

Decision models serve `/v1/systemone` through the device gateway. Connect
**Find Decision Model** or **Load Bit** to **SystemOne Choice**, **SystemOne Score**,
or **SystemOne Noul** to request a category, score, or yes/no probability. **Invoke SystemOne** asks
several named questions together. The gateway checks that the selected model
is a decision model and returns its typed answers without streaming.

A deployed service resolves model Bits pinned during deployment preparation.
A literal ID on a **Load Bit** node in an exported Flow or template includes
that Bit automatically. For IDs computed at runtime or candidates selected by
**Find Decision Model**, declare the Bits in the project's manifest dependencies.
Adding a model to a profile alone does not package it. Services using the same
model assets share the device's cached files. See the
[deployment prerequisites](/topics/genai/models/#decision-models).

## Open the Models tab

Open **Devices**, select the device, and open **Models**. The tab reads
models, downloads, and runtimes live from the device with your keys: unlock
the device and connect live. Locking the device clears what the tab showed.

The tab has these parts:

- A headline: how many models the device serves, and its tokens and requests
  in the last 24 hours.
- **Recommendations**: what the device could serve better, each with a fix
  where one exists.
- **Models**: each model's kind, engine, state, memory, busy slots, speed,
  requests in the last 24 hours, and who used it. **Load…**, **Unload…**,
  **Settings…**, **Remove…**, and **Use from my apps…** act on one model.
- **Usage and performance**: charts and a per-caller table, described in
  [Watch usage and performance](#watch-usage-and-performance).
- **Hardware and runtimes**: the processor, memory, GPUs, the model disk, and
  the installed and available runtimes.
- **Downloads**: the files the device is fetching, with their source, rate,
  time left, and the reason of a failure.

## Add a model

Select **Add model…**. The wizard has four steps:

1. **Model**: pick one from the **Flow-Like hub**, enter a **Hugging Face**
   repository such as `Qwen/Qwen3-8B-GGUF`, or pick one of **My Bits**.
2. **Version**: each quantization shows how it fits on this device: **Fits on
   the GPU**, **Partly on the GPU**, **Processor only**, or **Too large**. It
   also shows the memory it needs, an expected speed, and the download size.
   A vision model also needs a projector file; the first one offered is
   recommended.
3. **Settings**: the recommended settings for this device, each with its
   reason. See [Tune settings and residency](#tune-settings-and-residency).
4. **Review**: the files, their sources, the disk space and memory the model
   takes, and when it runs.

The fit check counts the model's weights, the context its slots hold, and
the runtime's overhead against the free GPU memory, unified memory, or RAM.
The device reports a GPU's memory only after a runtime for that GPU is
installed. Until then, the check counts the processor only.

The wizard refuses a version that the device can't take: more than 32 files,
a split model with missing parts, a vision model without a projector, an
engine the device can't run, or a download larger than the space left on the
model disk. A file without a fingerprint needs a download source so
preparation can read it and compute one. Files with neither are refused.

Once you add it, the device downloads the files in the background. The model
loads when you load it or when its first request arrives. Adding files that
another model on the device already uses creates a second model with its own
settings; the files are not downloaded twice.

## How model files reach the device

### The device downloads them itself

The device fetches every model file itself. It tries the file's sources in
order: first the model's own download link, such as the Flow-Like CDN for hub
models, then the file on Hugging Face at the exact commit the model was added
from. Each source gets three attempts with a growing pause between them. A
source that sends less than 1 MiB in five minutes, or less than 32 KiB a second
on average, is too slow: it gets no further attempt, and the next source
continues the download. The device downloads two files at a time and resumes an
interrupted download where it stopped.

Every file has a fingerprint that is fixed before the download starts: the
hub's blake3 hash, Hugging Face's sha256, or a sha256 the deploying computer
computes. The device checks the downloaded bytes against it and keeps only a
file that matches. Where the bytes came from never changes that check.

**Downloads** shows each file as **Queued**, **Downloading**, **Verifying**,
**Done**, **Failed**, or **Waiting for a computer to send it**. A failed
download names its reason:

| Reason | Meaning |
| --- | --- |
| The device couldn't reach any download source | Every source failed to connect or sent too slowly. The device's network may block outbound connections. |
| The download source answered with an error | A source answered with an HTTP error status. |
| The file didn't match its fingerprint | The device discarded the bytes. |
| The file had another size than expected | The device discarded the bytes. |
| The model disk has no room left for it | See [Model disk space](#model-disk-space). |
| The device couldn't write the file to its disk | Check the disk that holds the agent's state. |
| No download source is known for this file | Send the file from a computer that has it. |
| Cancelled | Someone cancelled the download. |

### Send a file from your computer

When the device can't download a file, select **Send from this computer**
under the failed download. Your computer then sends the file through the
encrypted tunnel, and the device checks it against its fingerprint as usual.

- The desktop app sends the file from the models stored on this computer. If
  the model is not stored there, the app downloads it first.
- The web app downloads the file from its source in the browser and streams
  it to the device as it arrives.

**Stop sending** keeps what the device received; sending again continues from
there. A file whose source your computer doesn't know can't be sent this way.
Deploy the app again from the desktop app instead.

### Model files in a deployment

When you deploy an app that uses a local model and every target device hosts
models, the deployment carries each model file's fingerprint, size, and
sources instead of the file itself. Files that preparation cannot describe
with a public source can travel inside the deployment artifact. Neither path
has a fixed byte limit.

After the upload, the rollout lists each file under **Model files**. The
device downloads what it doesn't have yet. For a file it can't download, your
computer sends it automatically, with its progress and **Stop sending** in
view. The update is applied only once every file is on the device.

The web app can download and fingerprint model files without a fixed byte
limit. Downloads and hashing still need browser storage and processing time.

If a target device's agent doesn't host models, the deployment carries the
model files inside the offline copy, with no fixed byte limit.

If a model file of a deployed service is missing when the service starts, for
example after a reset of the device, the device downloads it again first. If
that fails, the service fails to start with `model_asset_missing`.

### Model disk space

Each file is stored once per device, however many models and services use
it. Files that nothing uses any more are deleted after a day. They go sooner
when a new download needs their space, the oldest first, and within the hour
when less than a tenth of the disk is free. New model files may fill the disk
that holds the agent's state until a tenth of it is left free. A download
that would need more is refused before it starts. **Hardware and runtimes**
shows the space models use under **Model disk**.

To cap model storage, set `FLOW_LIKE_DEVICE_MODELS_MAX_BYTES` to a positive
number of bytes in the state directory's private `agent.env` and restart the
agent. For example, `FLOW_LIKE_DEVICE_MODELS_MAX_BYTES=68719476736` caps stored,
staged and reserved model files at 64 GiB. Lowering the limit keeps files
that models or deployments still use. Downloads that would exceed the limit
are refused.

## Runtimes and GPUs

A runtime is the engine program that runs llama.cpp or MLX models. Runtimes
are signed by Flow-Like and downloaded on demand from the same signed release
source as agent updates. The device checks the signature and every file
before it installs a runtime.

| Device | Runtimes |
| --- | --- |
| Linux x86-64 | CPU, Vulkan |
| Linux ARM64 | CPU |
| macOS Apple silicon | Metal, MLX |

When a llama.cpp model loads for the first time and no runtime is installed,
the device installs the recommended one: Metal on Apple silicon, Vulkan on a
Linux x86-64 device with a GPU, and CPU otherwise. The first MLX model to
load installs the MLX runtime the same way. Install or remove runtimes
under **Hardware and runtimes**. If a GPU sits unused because a model runs on
the processor, **Recommendations** offers to install a GPU runtime.

Linux runtimes need glibc 2.35 or later, for example Ubuntu 22.04 or Debian
12. ONNX embedding models need no runtime download; the agent runs them
itself.

### Docker

The generated Docker package doesn't give the container access to a GPU. To
use an NVIDIA GPU, give the container GPU access through the NVIDIA Container
Toolkit and include `graphics` in `NVIDIA_DRIVER_CAPABILITIES`, for example
`compute,utility,graphics`; the Vulkan runtime needs it. If the container asks
for an NVIDIA GPU but can't see one, **Recommendations** says so.

### Memory

Loaded models share a memory budget: the memory of discrete GPUs plus three
quarters of the RAM, or three quarters of the unified memory on Apple
silicon. Before a model loads, the device unloads models that load on demand
and answer no request at the moment, longest idle first, until the new model
fits. If it still doesn't fit, the model fails with "Not enough memory, even
after idle models were unloaded."

## Tune settings and residency

Select **Settings…** on a model. Each field shows its recommended value and
the reason for it; **Use recommended** applies them all.

| Setting | What it changes |
| --- | --- |
| Context per slot | How many tokens one request may hold: prompt, history, and answer |
| Parallel requests | How many requests the model answers at the same time; each keeps its own context. On a GPU, four answer at nearly the speed of one. |
| Context cache precision | Full precision, 8-bit, or 4-bit. Lower precision halves the memory the context takes, at a small cost in quality. |
| Layers on the GPU | As many as fit, all, none, or a number. Layers on the GPU answer much faster. |
| Processor threads | Threads for the parts that run on the processor. One per physical core is fastest. |
| Flash attention | A faster way to read long prompts that also takes less memory |

An MLX model uses only the context per slot and the context cache precision.

**When it runs** sets the residency:

- **Always loaded**: answers without a start-up wait and keeps its memory.
- **Loads on demand**: the first request loads it, and it unloads after a
  time without requests, 15 minutes by default.
- **Kept off**: never loads, not even for a request.

Changing the settings of a loaded model restarts it with the new settings.
Requests in flight finish first, and new requests wait until the model runs
again. If someone else changed the settings after you opened the sheet,
the sheet says so and keeps your change: load their version, then apply your
change again.

**Recommendations** names what the device could serve better, for example an
unused GPU, a model that only partly fits in GPU memory, waiting requests,
prompts near the context limit, low memory or disk space, a model kept loaded
but unused, slow first tokens, a thread count that doesn't match the cores, or
a newer runtime. Where a single change fixes it, the recommendation offers
that change.

## Watch usage and performance

**Usage and performance** charts tokens in and out, requests answered and
failed, time to first token (median and 95th percentile), generation speed,
and queue wait. Choose **24 h**, **7 d**, or **90 d**, and one model or all of
them. **Who called** breaks requests and tokens down by caller: the owner,
each person the device is shared with, and each service on the device.
The owner sees every caller. Other readers see their own usage; **Manage
models** also shows other people's usage. A service's usage needs permission
to read that service's status. The charts still show device totals.

The device keeps the details of each request for 48 hours, per-minute totals
for 7 days, and hourly totals for 90 days. Statistics hold counts and timings
only, never prompt or answer text.

**Try a model…** opens a playground for chat and embeddings. It shows the
time to the first token, the speed, and the token counts. Its requests travel
through the encrypted tunnel and count as yours in the statistics.

## Use device models from your apps

### Add the model to your models

Select **Use from my apps…** on a model, check its name and context length,
and select **Add to my models**. For an embedding model, the sheet asks for
the input length and the vector length instead, which the model card states.
The device must be unlocked.

This adds a model to your account that names the device and the model on it.
It holds no secret and copies no weights. The hub accepts it only for a
device you own or that is shared with you. Model pickers in the desktop app
then show it with a **Device** badge, in flows, chat, and FlowPilot alike.
The web app's pickers leave device models out, because its runs execute in
the cloud.

**Keep unlocked for model access** keeps the device's keys open on this
computer. In the desktop app your runs can call its models until you lock the
device or quit Flow-Like; in the web app, until you lock the device or close
the window. Without it, the device locks again after 30 minutes unused.

### When a run uses a device model

A run in the desktop app reaches the device through its encrypted tunnel. If
the device is locked on this computer, Flow-Like asks once with a dialog that
names the run, the model, and the device. Enter the device password and
select **Unlock**, or select **Not now**. **Keep unlocked until I quit** is
on by default; when it is off, the device locks again after 30 minutes
unused. No answer within 2 minutes counts as **Not now**, and so does a run
while the main Flow-Like window is hidden. After **Not now**, the run doesn't
ask again and goes on without device models it would need to unlock.

What happens next depends on how the flow picks its model.
[Find Model](/nodes/ai/generative/ai-generative-find-model/) treats an
unavailable device model as missing and takes the next model that matches. A
flow that names the device model fails with a sentence that names the model,
the device, and the cause:

| Situation | Find Model | A flow that names the model fails with |
| --- | --- | --- |
| The run executes in the cloud | Skips device models | "…which this host cannot reach; device models run from the desktop app." |
| You chose **Not now**, or didn't answer | Takes the next model | "…which stayed locked." |
| The device is offline | Takes the next model | "…which is offline." |
| The model was removed from the device | Takes the next model | "…which does not host this model." |
| The device was removed or is no longer shared with you | Takes the next model | "…which was removed or is no longer shared with you." |
| Your access lacks **Use models** | Takes the next model | "…which has not granted you model access." |

A cloud run is any run that doesn't execute in the desktop app or on a
device, such as a run started from the web app. Use Find Model when a flow
should keep working there.

To lock a device before you quit, lock it in **Devices**, or select
**Lock All Devices** in the tray menu. Either clears the device's keys on
this computer, including keys a run's dialog kept unlocked.

### Services on the device

Services deployed to the device use its model host directly. A local GGUF or
ONNX model in a deployed app runs on the model host instead of in the
service, and so does an MLX model on an Apple silicon Mac. If a model with
the same files, engine, and kind is already hosted, the service shares it,
settings included. Otherwise the device creates one with default settings,
and it appears on the **Models** tab. A device model of the same device
works from a deployed service; device models of other devices don't.

## Permissions

Two permissions cover models. Both apply to the whole device only.

| Permission | Allows | Consequence |
| --- | --- | --- |
| Use models (`model_use`) | Calling the device's models from apps and the playground, and reading the **Models** tab | Each call uses the device's memory and compute |
| Manage models (`model_manage`) | Adding, configuring, loading, unloading, and removing models and runtimes | Uses disk, memory, and GPU, and can unload models other people use |

The **Model user** preset gives **Use models**. **Device admin** includes both
permissions on a device whose agent hosts models. The access screen offers
the model permissions only for such devices, because an older agent rejects
an access policy that contains them.

A deployment's model downloads need **Deploy & configure** on the app's
project. That permission also lets the deploying computer send the files the
deployment asked for. Sending other model files needs **Manage models**.

The model gateway is a tunnel target of its own. **Use models** does not
include **Connect to services**, and **Connect to services** does not include
model access. See
[Service access and port forwarding](/devices/service-access/).

To use your device's models, another person needs **Use models**, the device
in their own **Devices** area, and its keys on their computer. Their calls
appear under **Who called**.

## Security

| Concern | How the device handles it |
| --- | --- |
| Swapped or corrupted weights | A fingerprint fixed before the download decides; the device stores a file only after it matches. A source, mirror, or computer that sends other bytes changes nothing. |
| A download source that points into your network | Downloads use HTTPS only, and redirects (at most five) must stay on HTTPS. The device resolves every address and refuses loopback, private, link-local, unique-local, and cloud metadata addresses. |
| A malicious model file | Engines parse model files. On Linux with bubblewrap, Landlock and a usable delegated cgroup, each engine has read-only system, runtime and model mounts, GPU access, a private `/tmp` and a private network namespace. It communicates through a private UNIX socket on verified `/dev/shm` tmpfs; Internet binds and connections are blocked. Finite limits bound CPU, RAM and process/thread counts. Memory and process allowances count against the shared workload budget; engine CPU shares the parent ceiling. With a compatible host policy, macOS and Linux hosts without these facilities run engines under the agent's account. A required isolation policy refuses to start the model host without them. |
| A tampered runtime | Runtimes are signed with the Flow-Like release key. The device checks the signed list and each file's sha256 before installing. |
| Someone else's identity or quota | The gateway listens on loopback only. Each service gets its own token. For tunnel requests, the agent sets the caller from the grant the tunnel authenticated and removes any identity headers the client sent. |
| One caller crowding out the others | Each caller gets 8 requests at a time. Further requests wait; the gateway answers `429 Too Many Requests` when 32 are already waiting or a request has waited 60 seconds. |
| Prompts and answers in transit | Requests and answers stay inside the device's end-to-end encrypted tunnel, directly over WebRTC or through the encrypted relay. The hub relays only encrypted frames. |
| Usage statistics | They hold counts and timings, never text, and stay on the device. The device's encrypted metrics snapshot carries only totals: models, loaded and failed models, requests, tokens, errors, and disk use. |
| Kept-unlocked keys | The desktop app holds them in its memory only and clears them when you lock the device, lock all devices, sign out, or quit. They are never written to disk. |
| A full disk | The device checks the space before a download starts and refuses downloads beyond the model disk's limit. |

Read [Device security and networking](/devices/security/) for the device's
identity, access policies, and host isolation.

## Troubleshooting

| Observation | Check |
| --- | --- |
| "The agent on … can't host models yet" | Update the device agent. |
| A download fails because the device couldn't reach any source | The device needs outbound HTTPS access to the source. Otherwise, use **Send from this computer**. |
| A model fails with "No runtime for its engine is installed" | Install a runtime under **Hardware and runtimes**. Downloading it needs outbound access to the release source. |
| A model fails with "Not enough memory, even after idle models were unloaded" | Choose a smaller quantization, a shorter context per slot, an 8-bit cache, or fewer parallel requests, or unload another model. |
| A model runs on the processor although the device has a GPU | Install the GPU runtime. In Docker, give the container GPU access. |
| The playground or an app reports too many waiting requests | The caller reached its limit. More parallel requests on the model, or fewer concurrent callers, help. |
| A desktop run fails with "…which stayed locked" | Unlock the device when the run asks, or keep it unlocked for model access. |
| A run in the web app fails with "…which this host cannot reach" | Device models run from the desktop app. Use Find Model so that cloud runs take another model. |
| A deployment stops at **Model files** | Check the reason shown for the file. A device without internet access needs the file sent from a computer that has it. |
