---
title: Local image and video generation
description: Generate images and video with stable-diffusion.cpp through the existing media nodes
sidebar:
  order: 3
---

Use **stable-diffusion.cpp Image Model** or **stable-diffusion.cpp Video Model** to run a diffusion model on the machine executing your workflow. Connect the resulting provider Bit to **Generate Image** or **Generate Video**. A Bit is the model configuration passed between nodes; the generation node writes the result to a FlowPath, Flow-Like's file reference.

The existing generation nodes also support cloud providers. Image generation includes OpenAI and Azure OpenAI, Gemini, Vertex AI, Bedrock, xAI, Together, Hugging Face, OpenRouter, and Mistral. Video generation includes OpenAI Sora, Vertex Veo, Runway, fal, and Replicate. Choose the matching provider and options nodes for each service.

## Choose where the model runs

The diffusion model builders accept a **Configuration** object. For a checkpoint containing the model components, set its path on the execution machine:

```json
{
  "model_path": "/models/sd-v1-5.safetensors",
  "offload_to_cpu": true,
  "startup_timeout_seconds": 300,
  "request_timeout_seconds": 1800
}
```

For models distributed as separate components, use `diffusion_model_path` and the companion paths required by that model. Available fields are `vae_path`, `clip_l_path`, `clip_g_path`, `t5xxl_path`, and `llm_path`. For example, a Flux model can require a diffusion model, VAE, CLIP-L, and T5 encoder. All files must already exist. The builder validates the configuration; generation checks the files before starting the runtime.

Flow-Like starts the bundled `sd-server` on a loopback port, waits for it to load, submits the job, and stops the process when the request finishes. Managed diffusion requests run one at a time within the execution process. Each request reloads its model and releases its GPU allocation afterward. `diffusion_flash_attention` is available as an optional configuration setting and defaults to `false`.

To keep a model loaded across requests, run your own compatible server and provide its base URL:

```json
{
  "endpoint": "http://127.0.0.1:1234",
  "request_timeout_seconds": 1800
}
```

With an endpoint, Flow-Like uses the model already loaded by that server. Model paths in the configuration do not select or reload a remote model. This integration uses the native `/sdcpp/v1` API; an OpenAI-compatible image endpoint alone is insufficient. Endpoint configuration does not provide authentication headers.

## Generate an image

1. Add **stable-diffusion.cpp Image Model** and set its Configuration.
2. Connect its **Model** output to **Generate Image → Provider**.
3. Supply History with a final user message containing the image prompt.
4. Connect **stable-diffusion.cpp Image Options → Options** to **Provider Options** to set dimensions, sampling steps, CFG scale, negative prompt, sampler, scheduler, or seed.
5. Set Output Path and run the flow.

The result is a PNG file. Defaults are 512 × 512 pixels, 20 steps, and CFG scale 7. Dimensions must be positive multiples of 8 and fit the loaded server's limits. A seed of `-1` selects a random seed; zero is a valid fixed seed. The `auto` sampler and scheduler keep the model's defaults. These are starting values; follow the model's sampling guidance when choosing settings.

## Generate a video

1. Add **stable-diffusion.cpp Video Model** with a configuration for a video model.
2. Connect its **Provider** output to **Generate Video → Provider** and set Prompt.
3. Connect **stable-diffusion.cpp Video Options → Options** to **Provider Options**. Set dimensions, frame count, FPS, and sampling controls.
4. Optionally connect a First Frame image for a model that supports image-to-video generation.
5. Set Output Path and run the flow.

Defaults are 832 × 480 pixels, 33 frames, 16 FPS, and 28 sampling steps. Frame counts must follow `4n + 1`, such as 33 or 81. AVI is the default container. Animated WebP and WebM are available when the server reports support. MP4 is not a native output format for this integration; use a video transcode node when MP4 is required. The returned path uses the actual output format's extension.

This integration does not accept Last Frame, Input Video, or audio generation. Those inputs produce an error instead of being discarded. The server's capabilities determine whether its loaded model supports image or video generation and the requested output format.

## Runtime installation and troubleshooting

Desktop platform builds prepare the pinned runtime through `apps/desktop/scripts/prepare-stablediffusion.ts`. To prepare it directly, run this from `apps/desktop`:

```sh
bun run prepare:stablediffusion
```

The script builds the pinned source on macOS for Flow-Like's macOS 14 deployment target. Install Git, CMake, and the Xcode command line tools first. Windows x64 uses a checksum-verified Vulkan release archive. Linux x64 builds from the pinned source with Vulkan enabled; release builds use Ubuntu 22.04 to preserve the glibc 2.35 minimum. Linux builds need a C++ compiler, CMake, Vulkan development files, `glslc`, and SPIR-V headers. Windows and Linux GPU drivers must provide Vulkan. Windows ARM and mobile targets do not bundle this runtime. They can connect to an existing server, and desktop hosts can use a compatible source build through `FLOW_LIKE_SD_SERVER`.

For a standalone executor, set `FLOW_LIKE_SD_SERVER` to the absolute path of a compatible `sd-server` executable before starting Flow-Like. Keep the runtime's libraries beside that executable. The executable path is host configuration, not a workflow input.

Runtime files live in their own directory, separate from llama.cpp libraries. A prepared runtime is reused while its version and file checksums match. The source revision is `6b3edaaf32cc19e5bb2d819c788bd557eddc8eba`, release `master-841-6b3edaa`. See the [pinned native API documentation](https://github.com/leejet/stable-diffusion.cpp/blob/6b3edaaf32cc19e5bb2d819c788bd557eddc8eba/examples/server/api.md) for its request and result formats.

If the runtime is missing, prepare it or set `FLOW_LIKE_SD_SERVER`. If startup fails, check the reported model path and the runtime's bounded diagnostic log. Increase `startup_timeout_seconds` for a model that takes longer to load. A generation timeout requests job cancellation; managed execution also stops its server. Cancellation of an externally managed job is best effort if the server cannot be reached.
