export type GenerationModelKind = "image" | "video";

export type GenerationAssetRole =
	| "model"
	| "diffusion_model"
	| "vae"
	| "clip_l"
	| "clip_g"
	| "t5xxl"
	| "llm";

export interface GenerationModelPreset {
	id: string;
	kind: GenerationModelKind;
	label: string;
	description: string;
	notes: string;
	license: string;
	repository: string;
	authors: string[];
	assets: {
		role: GenerationAssetRole;
		repo: string;
		revision: string;
		path: string;
		size: number;
		license?: string;
	}[];
	defaults: {
		width: number;
		height: number;
		steps: number;
		cfg_scale: number;
		sampler?: string;
		scheduler?: string;
		video_frames?: number;
		fps?: number;
		output_format?: string;
	};
	config?: {
		offload_to_cpu?: boolean;
		diffusion_flash_attention?: boolean;
		startup_timeout_seconds?: number;
		request_timeout_seconds?: number;
	};
}

// Files, full revisions and byte sizes were checked against the Hugging Face
// model API on 2026-09-05. Compatibility follows the packaged native revision:
// https://github.com/leejet/stable-diffusion.cpp/tree/6b3edaaf32cc19e5bb2d819c788bd557eddc8eba/docs
// Each bundle includes its text encoder and VAE. File size is download size;
// inference also needs memory for activations and decoded frames.
export const GENERATION_MODEL_PRESETS: GenerationModelPreset[] = [
	{
		id: "flux2-klein-4b-q4-k-m",
		kind: "image",
		label: "FLUX.2 klein 4B · Q4_K_M",
		description:
			"Generate images in four sampling steps with the distilled FLUX.2 klein 4B model.",
		notes:
			"Includes the Q4_K_M diffusion model, Qwen3 4B text encoder and FLUX.2 decoder. Starts at 1024 × 1024 with guidance 1, as recommended for the distilled model. Download size does not include the additional memory needed during generation.",
		license: "apache-2.0",
		repository: "https://huggingface.co/black-forest-labs/FLUX.2-klein-4B",
		authors: [
			"https://huggingface.co/black-forest-labs",
			"https://huggingface.co/unsloth",
			"https://huggingface.co/Comfy-Org",
		],
		// The klein model uses base Qwen3-4B, rather than the Z-Image encoder.
		// https://github.com/leejet/stable-diffusion.cpp/blob/6b3edaaf32cc19e5bb2d819c788bd557eddc8eba/docs/flux2.md
		assets: [
			{
				role: "diffusion_model",
				repo: "unsloth/FLUX.2-klein-4B-GGUF",
				revision: "0084d1df98e2e2137fe776d55170bc4792ec1d66",
				path: "flux-2-klein-4b-Q4_K_M.gguf",
				size: 2_604_311_104,
			},
			{
				role: "llm",
				repo: "unsloth/Qwen3-4B-GGUF",
				revision: "22c9fc8a8c7700b76a1789366280a6a5a1ad1120",
				path: "Qwen3-4B-Q4_K_M.gguf",
				size: 2_497_281_312,
			},
			{
				role: "vae",
				repo: "Comfy-Org/vae-text-encorder-for-flux-klein-4b",
				revision: "5f526678002e43af5551dadb73ce2e8c91b43afe",
				path: "split_files/vae/flux2-vae.safetensors",
				size: 336_211_292,
			},
		],
		defaults: {
			width: 1024,
			height: 1024,
			steps: 4,
			cfg_scale: 1,
			sampler: "euler",
			output_format: "png",
		},
		config: { offload_to_cpu: true, diffusion_flash_attention: true },
	},
	{
		id: "z-image-turbo-q4-k",
		kind: "image",
		label: "Z-Image-Turbo · Q4_K",
		description:
			"Generate photos, illustrations and text in images with the eight-step Z-Image-Turbo model.",
		notes:
			"Includes the Q4_K diffusion model, Qwen3 4B Instruct 2507 text encoder and image decoder. Uses eight sampling steps and guidance 1. The text encoder is a required download, even when another Qwen model is already installed.",
		license: "apache-2.0",
		repository: "https://huggingface.co/Tongyi-MAI/Z-Image-Turbo",
		authors: [
			"https://huggingface.co/Tongyi-MAI",
			"https://huggingface.co/leejet",
			"https://huggingface.co/unsloth",
			"https://huggingface.co/Comfy-Org",
		],
		// https://github.com/leejet/stable-diffusion.cpp/blob/6b3edaaf32cc19e5bb2d819c788bd557eddc8eba/docs/z_image.md
		assets: [
			{
				role: "diffusion_model",
				repo: "leejet/Z-Image-Turbo-GGUF",
				revision: "c61c0e422dc8b541b7548cf33a4ef8302b0f8085",
				path: "z_image_turbo-Q4_K.gguf",
				size: 3_864_250_304,
			},
			{
				role: "llm",
				repo: "unsloth/Qwen3-4B-Instruct-2507-GGUF",
				revision: "a06e946bb6b655725eafa393f4a9745d460374c9",
				path: "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
				size: 2_497_281_120,
			},
			{
				role: "vae",
				repo: "Comfy-Org/z_image_turbo",
				revision: "08d04455279082882deaabc8d0d09fc914c071e1",
				path: "split_files/vae/ae.safetensors",
				size: 335_304_388,
			},
		],
		defaults: {
			width: 1024,
			height: 1024,
			steps: 8,
			cfg_scale: 1,
			sampler: "euler",
			output_format: "png",
		},
		config: { offload_to_cpu: true, diffusion_flash_attention: true },
	},
	{
		id: "qwen-image-2512-q4-k-m",
		kind: "image",
		label: "Qwen-Image-2512 · Q4_K_M",
		description:
			"Generate detailed images and typography with the larger Qwen-Image December 2025 model.",
		notes:
			"Includes the Q4_K_M diffusion model, Qwen2.5-VL 7B text encoder and Qwen image decoder. This is the largest image bundle here and uses 50 sampling steps. Allow additional system and graphics memory beyond its download size.",
		license: "apache-2.0",
		repository: "https://huggingface.co/Qwen/Qwen-Image-2512",
		authors: [
			"https://huggingface.co/Qwen",
			"https://huggingface.co/unsloth",
			"https://huggingface.co/mradermacher",
			"https://huggingface.co/Comfy-Org",
		],
		// Encoder/VAE mapping: https://github.com/leejet/stable-diffusion.cpp/blob/6b3edaaf32cc19e5bb2d819c788bd557eddc8eba/docs/qwen_image.md
		// The 2512 publisher documents stable-diffusion.cpp support:
		// https://huggingface.co/unsloth/Qwen-Image-2512-GGUF
		assets: [
			{
				role: "diffusion_model",
				repo: "unsloth/Qwen-Image-2512-GGUF",
				revision: "1626d7531f84b4d2ea1cd6d2e69f41ec027dd354",
				path: "qwen-image-2512-Q4_K_M.gguf",
				size: 13_244_758_560,
			},
			{
				role: "llm",
				repo: "mradermacher/Qwen2.5-VL-7B-Instruct-GGUF",
				revision: "cfa2baa09946b211c107e6e104948987a64dd2c1",
				path: "Qwen2.5-VL-7B-Instruct.Q4_K_M.gguf",
				size: 4_683_072_512,
				// The quantizer omits license metadata; the original is Apache 2.0.
				// https://huggingface.co/Qwen/Qwen2.5-VL-7B-Instruct
				license: "apache-2.0",
			},
			{
				role: "vae",
				repo: "Comfy-Org/Qwen-Image_ComfyUI",
				revision: "7beb7b647f04469fbe64ba8adc2bb0d7e5e9f73f",
				path: "split_files/vae/qwen_image_vae.safetensors",
				size: 253_806_246,
			},
		],
		// Steps and guidance follow https://huggingface.co/Qwen/Qwen-Image-2512;
		// resolution and Euler sampling follow the native integration guide.
		defaults: {
			width: 1024,
			height: 1024,
			steps: 50,
			cfg_scale: 4,
			sampler: "euler",
			output_format: "png",
		},
		config: {
			offload_to_cpu: true,
			diffusion_flash_attention: true,
			startup_timeout_seconds: 600,
			request_timeout_seconds: 3600,
		},
	},
	{
		id: "wan21-t2v-13b-fp16",
		kind: "video",
		label: "Wan2.1 T2V 1.3B · FP16",
		description:
			"Generate short silent videos from text with the smaller Wan2.1 1.3B diffusion model.",
		notes:
			"Includes the FP16 diffusion model, Q4_K_M UMT5 text encoder and Wan2.1 video decoder. Starts with 33 frames at 832 × 480 and 16 fps. This preset accepts text prompts; use Wan2.2 TI2V for an input image. Video decoding can require substantially more memory than the downloaded weights.",
		license: "apache-2.0",
		repository: "https://huggingface.co/Wan-AI/Wan2.1-T2V-1.3B",
		authors: [
			"https://huggingface.co/Wan-AI",
			"https://huggingface.co/Comfy-Org",
			"https://huggingface.co/city96",
		],
		// https://github.com/leejet/stable-diffusion.cpp/blob/6b3edaaf32cc19e5bb2d819c788bd557eddc8eba/docs/wan.md
		// The guide omits steps (native default 20). Flow shift uses the engine
		// default of 5; the CLI guide's override of 3 is not exposed by the node.
		assets: [
			{
				role: "diffusion_model",
				repo: "Comfy-Org/Wan_2.1_ComfyUI_repackaged",
				revision: "617a7633e636506f850e043bc4605f290a466a8e",
				path: "split_files/diffusion_models/wan2.1_t2v_1.3B_fp16.safetensors",
				size: 2_838_303_560,
			},
			{
				role: "t5xxl",
				repo: "city96/umt5-xxl-encoder-gguf",
				revision: "b535255bee98c2b0a59ea7c0ae2dcd0c6657b3b7",
				path: "umt5-xxl-encoder-Q4_K_M.gguf",
				size: 3_655_145_312,
			},
			{
				role: "vae",
				repo: "Comfy-Org/Wan_2.1_ComfyUI_repackaged",
				revision: "617a7633e636506f850e043bc4605f290a466a8e",
				path: "split_files/vae/wan_2.1_vae.safetensors",
				size: 253_815_318,
			},
		],
		defaults: {
			width: 832,
			height: 480,
			steps: 20,
			cfg_scale: 6,
			sampler: "euler",
			video_frames: 33,
			fps: 16,
			output_format: "webm",
		},
		config: {
			offload_to_cpu: true,
			diffusion_flash_attention: true,
			startup_timeout_seconds: 600,
			request_timeout_seconds: 3600,
		},
	},
	{
		id: "wan22-ti2v-5b-q4-k-m",
		kind: "video",
		label: "Wan2.2 TI2V 5B · Q4_K_M",
		description:
			"Generate silent videos from text or a first-frame image with Wan2.2 TI2V 5B.",
		notes:
			"Includes the Q4_K_M diffusion model, UMT5 text encoder and the required Wan2.2 decoder. Starts with a short 33-frame preview at 832 × 480 and 24 fps. Increase resolution and frame count after checking available memory. The model also supports 1280 × 704; larger outputs take more time and memory.",
		license: "apache-2.0",
		repository: "https://huggingface.co/Wan-AI/Wan2.2-TI2V-5B",
		authors: [
			"https://huggingface.co/Wan-AI",
			"https://huggingface.co/QuantStack",
			"https://huggingface.co/Comfy-Org",
			"https://huggingface.co/city96",
		],
		// https://github.com/leejet/stable-diffusion.cpp/blob/6b3edaaf32cc19e5bb2d819c788bd557eddc8eba/docs/wan.md
		// TI2V 5B uses one diffusion model and the Wan2.2 VAE, unlike A14B.
		assets: [
			{
				role: "diffusion_model",
				repo: "QuantStack/Wan2.2-TI2V-5B-GGUF",
				revision: "57437632ddd08bdcbd1508c866aa22e126ed51d2",
				path: "Wan2.2-TI2V-5B-Q4_K_M.gguf",
				size: 3_433_116_000,
			},
			{
				role: "t5xxl",
				repo: "city96/umt5-xxl-encoder-gguf",
				revision: "b535255bee98c2b0a59ea7c0ae2dcd0c6657b3b7",
				path: "umt5-xxl-encoder-Q4_K_M.gguf",
				size: 3_655_145_312,
			},
			{
				role: "vae",
				repo: "Comfy-Org/Wan_2.2_ComfyUI_Repackaged",
				revision: "c4f60d30c55a624e35427060fdd217579a6c1d77",
				path: "split_files/vae/wan2.2_vae.safetensors",
				size: 1_409_400_960,
			},
		],
		// Official steps, guidance and fps; native flow shift already defaults to 5.
		// https://github.com/Wan-Video/Wan2.2/blob/main/wan/configs/wan_ti2v_5B.py
		defaults: {
			width: 832,
			height: 480,
			steps: 50,
			cfg_scale: 5,
			sampler: "euler",
			video_frames: 33,
			fps: 24,
			output_format: "webm",
		},
		config: {
			offload_to_cpu: true,
			diffusion_flash_attention: true,
			startup_timeout_seconds: 600,
			request_timeout_seconds: 3600,
		},
	},
];
