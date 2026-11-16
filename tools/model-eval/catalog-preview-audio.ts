import type { CatalogPreviewEntry } from "./catalog-preview-types";

// Verified against the provider catalog, benchmark page and publisher samples on 2026-11-16.
// Audio previews illustrate particular voices and prompts; they do not establish a quality rank.
export const AUDIO_MODEL_PREVIEWS: CatalogPreviewEntry[] = [
	{
		bit_ids: ["or0024b06cd282b54e9ff14d"],
		model_id: "qwen/qwen-audio-3.0-tts-plus",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/qwen/qwen-audio-3.0-tts-plus-20260723/b2bf48d1-48a8-4292-bb8c-0750bf71bc93/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for Qwen: Qwen-Audio-3.0-TTS Plus, voice longanlingxin.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/qwen/qwen-audio-3.0-tts-plus-20260723/fe48c11e-ca52-477a-98c4-a4e235593c47/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for Qwen: Qwen-Audio-3.0-TTS Plus, voice longanlingxin.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/qwen/qwen-audio-3.0-tts-plus-20260723/4f2e8014-d114-49f8-8b08-1036e7fe06b4/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for Qwen: Qwen-Audio-3.0-TTS Plus, voice longanlingxin.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["or0bcf4008a2eb0f4b194502"],
		model_id: "deepgram/flux-tts",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/deepgram/flux-tts-20260812/dda2fc54-a4bd-4ce9-a96f-3a68cd370a3d/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for Deepgram: Flux TTS, voice flux-alexis-en.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/deepgram/flux-tts-20260812/b57f0e53-c31f-4893-86b8-de0c576e9dce/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for Deepgram: Flux TTS, voice flux-alexis-en.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/deepgram/flux-tts-20260812/a7ad6013-9c57-42dd-9fad-ca94277af847/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for Deepgram: Flux TTS, voice flux-alexis-en.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["or0be5d6e7b0d348a3015707"],
		model_id: "mistralai/voxtral-mini-tts-3-0",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/mistralai/voxtral-mini-tts-3-0-20261020/15490386-ff5b-4886-87e5-eeac4237ec08/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for Mistral: Voxtral TTS 3, voice voice_3150. This sample uses a different voice from the Bit's default (en_paul_neutral).",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/mistralai/voxtral-mini-tts-3-0-20261020/0bb2f663-36ed-4a26-8a1c-0648f1d2f31a/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for Mistral: Voxtral TTS 3, voice voice_3150. This sample uses a different voice from the Bit's default (en_paul_neutral).",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/mistralai/voxtral-mini-tts-3-0-20261020/10d913db-d5b2-4f12-a90d-074f7b8b5dcb/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for Mistral: Voxtral TTS 3, voice voice_3150. This sample uses a different voice from the Bit's default (en_paul_neutral).",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["or0cddd18c31e276889a2cbd"],
		model_id: "canopylabs/orpheus-3b-0.1-ft",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/canopylabs/orpheus-3b-0.1-ft/1962784b-c655-4691-bff8-eae8a0926252/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for Canopy Labs: Orpheus 3B, voice tara.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/canopylabs/orpheus-3b-0.1-ft/d2344f3a-65e8-45ae-ac1c-a05ad1a5991d/original-0.mp3",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for Canopy Labs: Orpheus 3B, voice tara.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/canopylabs/orpheus-3b-0.1-ft/f49d61b6-113a-4198-98c4-49f1a8b3b479/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for Canopy Labs: Orpheus 3B, voice tara.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["or12da8833ff023052bdd476"],
		model_id: "fish-audio/s1",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/fish-audio/s1-20260729/fdb9025f-76e0-437b-a125-15909d6fef6d/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for Fish Audio: S1.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/fish-audio/s1-20260729/f140ea68-05c6-48ef-8289-9a05bded427a/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for Fish Audio: S1.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/fish-audio/s1-20260729/18a76047-82b7-4f5e-9d6c-523d408d9b0b/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for Fish Audio: S1.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["or13e612dce1e72048aaba3f"],
		model_id: "elevenlabs/eleven-turbo-v2.5",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-turbo-v2.5-20260930/918f55ad-fb1a-4834-bf49-b368bd812b49/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for ElevenLabs: Eleven Turbo v2.5, voice george.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-turbo-v2.5-20260930/966d8a32-ea62-4b15-9c82-0bb6e266ac4a/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for ElevenLabs: Eleven Turbo v2.5, voice george.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-turbo-v2.5-20260930/e96f7727-32db-4f82-8e91-f1837ccac54e/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for ElevenLabs: Eleven Turbo v2.5, voice george.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["or1a6fb450ba8c3d5ac9e652"],
		model_id: "fish-audio/s2.1-pro-free:free",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/fish-audio/s2.1-pro-20260729/fab63068-0355-4699-b023-9e86f5f534fe/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for Fish Audio: S2.1 Pro. This is the paid S2.1 Pro model's sample. OpenRouter describes S2.1 Pro Free as using the same synthesis capabilities, with different availability and latency; the free route was not sampled separately.",
				scope: "base_model_output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/fish-audio/s2.1-pro-20260729/d25d0d87-5a99-4ecb-b0b3-dfc0c9fba5f7/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for Fish Audio: S2.1 Pro. The paid S2.1 Pro model supplies this example; the free route was not sampled separately.",
				scope: "base_model_output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/fish-audio/s2.1-pro-20260729/588d9643-e82d-4da1-8fee-0121d39763d8/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for Fish Audio: S2.1 Pro. The paid S2.1 Pro model supplies this example; the free route was not sampled separately.",
				scope: "base_model_output",
			},
		],
	},
	{
		bit_ids: ["or1cddc44178076b49fb126e"],
		model_id: "elevenlabs/eleven-v3-conversational",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-v3-conversational-20260930/2daffca0-32ee-4473-ba97-0b8bb26827ed/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for ElevenLabs: Eleven v3 Conversational, voice george.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-v3-conversational-20260930/0dd4ee35-8294-4263-b8bf-94e8dd6be867/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for ElevenLabs: Eleven v3 Conversational, voice george.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-v3-conversational-20260930/2da88951-0560-43b8-98c8-11192a620c85/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for ElevenLabs: Eleven v3 Conversational, voice george.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["or2277c3587c369d7b633ebf"],
		model_id: "elevenlabs/eleven-turbo-v2",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-turbo-v2-20260930/cddcea34-48bf-4eab-8447-50a10d0bc1d1/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for ElevenLabs: Eleven Turbo v2, voice george.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-turbo-v2-20260930/931daab2-33a7-4a00-9346-5260e0f41eb3/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for ElevenLabs: Eleven Turbo v2, voice george.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-turbo-v2-20260930/9d21e715-d87e-4d7f-95f8-efe3fd93b2b2/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for ElevenLabs: Eleven Turbo v2, voice george.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["or330fb82e5aa410a77f5690"],
		model_id: "fish-audio/s2.1-pro",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/fish-audio/s2.1-pro-20260729/fab63068-0355-4699-b023-9e86f5f534fe/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for Fish Audio: S2.1 Pro.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/fish-audio/s2.1-pro-20260729/d25d0d87-5a99-4ecb-b0b3-dfc0c9fba5f7/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for Fish Audio: S2.1 Pro.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/fish-audio/s2.1-pro-20260729/588d9643-e82d-4da1-8fee-0121d39763d8/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for Fish Audio: S2.1 Pro.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["or46fbc99aac9b19dcabb02a"],
		model_id: "assemblyai/assemblyai-tts-1",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/assemblyai/assemblyai-tts-1-20261028/4728b2d3-ca27-4598-9874-042941bcd1ae/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for AssemblyAI: TTS 1.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/assemblyai/assemblyai-tts-1-20261028/116b4d00-c0bf-4154-b2a6-77992bc57ab9/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for AssemblyAI: TTS 1.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/assemblyai/assemblyai-tts-1-20261028/b6b30c8a-2574-424b-9c1b-88fbd749b124/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for AssemblyAI: TTS 1.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["or523b87193ecc4596d05b5b"],
		model_id: "elevenlabs/eleven-flash-v2",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-flash-v2-20260930/eef81dab-1d46-4afa-8803-2a040c2a48f0/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for ElevenLabs: Eleven Flash v2, voice george.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-flash-v2-20260930/0e0dec98-9367-40a9-a435-3c47e69a9118/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for ElevenLabs: Eleven Flash v2, voice george.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-flash-v2-20260930/6947eae4-08ad-46d2-a096-8732eef9be89/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for ElevenLabs: Eleven Flash v2, voice george.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["or643fe556ed4e577f66e444"],
		model_id: "x-ai/grok-voice-tts-1.0",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/x-ai/grok-voice-tts-1.0/6d358e6a-c35b-426e-bca3-e628a0203101/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for SpaceXAI: Grok Voice TTS 1.0, voice eve.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/x-ai/grok-voice-tts-1.0/168faac3-5f4b-485f-bd3b-2866fc1fd10d/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for SpaceXAI: Grok Voice TTS 1.0, voice eve.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/x-ai/grok-voice-tts-1.0/52725c34-d19d-4f8a-a4d2-d224748ebd68/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for SpaceXAI: Grok Voice TTS 1.0, voice eve.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["or64f43a84af6b3299a73615"],
		model_id: "microsoft/mai-voice-2-flash",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/microsoft/mai-voice-2-flash-20260723/bac7ef5d-0b27-4e50-acea-90203a897b1b/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for Microsoft AI: MAI-Voice-2-Flash, voice en-US-Harper:MAI-Voice-2.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/microsoft/mai-voice-2-flash-20260723/39ca7a6d-775f-49fa-8f2c-b0479a1736f0/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for Microsoft: MAI-Voice-2-Flash, voice en-US-Harper:MAI-Voice-2.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/microsoft/mai-voice-2-flash-20260723/f812dafe-eccd-4777-b0fd-f03698886240/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for Microsoft: MAI-Voice-2-Flash, voice en-US-Harper:MAI-Voice-2.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["or6ae53ed8841d36b2f05b89"],
		model_id: "hexgrad/kokoro-82m",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/hexgrad/kokoro-82m/d7893cef-2c8c-4b8c-9eca-bdc7b50612af/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for hexgrad: Kokoro 82M, voice af_alloy. This sample uses a different voice from the Bit's default (af_heart).",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/hexgrad/kokoro-82m/e20be780-451f-4e30-8edc-a6fd0593c749/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for hexgrad: Kokoro 82M, voice af_alloy. This sample uses a different voice from the Bit's default (af_heart).",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/hexgrad/kokoro-82m/0a891358-fc2f-4e2f-8e47-08b29052a459/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for hexgrad: Kokoro 82M, voice af_alloy. This sample uses a different voice from the Bit's default (af_heart).",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["or74227bdd5b77555f986809"],
		model_id: "fish-audio/s2-pro",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/fish-audio/s2-pro-20260729/b343810e-51e1-4e3d-8a71-859d252ed5d3/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for Fish Audio: S2 Pro.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/fish-audio/s2-pro-20260729/62d887c1-1a7c-448f-ae49-86b5a8d8e4f2/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for Fish Audio: S2 Pro.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/fish-audio/s2-pro-20260729/deee5316-17cc-45e3-9500-11f85e1ab772/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for Fish Audio: S2 Pro.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["or7c64e07f669db08de3fa34"],
		model_id: "deepgram/aura-2",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/deepgram/aura-2-20260716/8841c461-64cd-4a1d-b5b0-5ff553c31db4/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for Deepgram: Aura-2, voice aura-2-thalia-en.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/deepgram/aura-2-20260716/43d1e92f-1f8c-48b8-9dd9-17fa936cdee0/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for Deepgram: Aura-2, voice aura-2-thalia-en.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/deepgram/aura-2-20260716/6f7057c7-bc31-44e1-aa8d-46aa9bdb25ee/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for Deepgram: Aura-2, voice aura-2-thalia-en.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["or81e5f39716ab88a6dbe4fa"],
		model_id: "mistralai/voxtral-mini-tts-2603",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/mistralai/voxtral-mini-tts-2603/521cd7d6-d458-487d-8189-50a284885f5e/original-0.mp3",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for Mistral: Voxtral Mini TTS, voice en_paul_sad. This sample uses a different voice from the Bit's default (en_paul_neutral).",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/mistralai/voxtral-mini-tts-2603/0468d027-b43f-4b8d-b21a-0f1886b38463/original-0.mp3",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for Mistral: Voxtral Mini TTS, voice en_paul_sad. This sample uses a different voice from the Bit's default (en_paul_neutral).",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/mistralai/voxtral-mini-tts-2603/c3c0208a-ba8a-4306-a935-154b54fadb62/original-0.mp3",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for Mistral: Voxtral Mini TTS, voice en_paul_sad. This sample uses a different voice from the Bit's default (en_paul_neutral).",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["or913b3f4b494919137db407"],
		model_id: "elevenlabs/eleven-v4-turbo",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-v4-turbo-20260930/6cfbf644-a3c7-4004-bb8d-666923ee4185/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for ElevenLabs: Eleven v4 Turbo, voice george.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-v4-turbo-20260930/5fc39d0e-e947-41af-8194-a68313d17909/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for ElevenLabs: Eleven v4 Turbo, voice george.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-v4-turbo-20260930/6903d91d-d0b2-427f-bfef-229a7cb2c562/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for ElevenLabs: Eleven v4 Turbo, voice george.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["or95f5de65dbca3a07ffc435"],
		model_id: "microsoft/mai-voice-2",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/microsoft/mai-voice-2/da671fec-0bf5-4da1-88bf-0b4471ccc410/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for Microsoft AI: MAI-Voice-2, voice en-US-Harper:MAI-Voice-2.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/microsoft/mai-voice-2/9875919d-376b-4562-89e7-0be0aefa667b/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for Microsoft: MAI-Voice-2, voice en-US-Harper:MAI-Voice-2.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/microsoft/mai-voice-2/57924969-6a99-4c55-b7c6-2e6aef4c9a6f/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for Microsoft: MAI-Voice-2, voice en-US-Harper:MAI-Voice-2.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["or961e4f27bf2cdf0acafba7"],
		model_id: "elevenlabs/eleven-v4",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-v4-20260930/9b697e4e-ee06-4ff1-9a01-5c6cb701e6fa/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for ElevenLabs: Eleven v4, voice george.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-v4-20260930/3cf89aac-d5ec-493d-9883-ffaba4f7e72b/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for ElevenLabs: Eleven v4, voice george.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-v4-20260930/5c5f877e-8024-4dee-ba27-16f4e40f2da2/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for ElevenLabs: Eleven v4, voice george.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["or9794eca610a95e1bea284d"],
		model_id: "elevenlabs/eleven-v3",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-v3-20260930/552b27a5-86fa-427a-be59-08990af822e8/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for ElevenLabs: Eleven v3, voice george.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-v3-20260930/c1994595-cb9b-413e-90a7-ad72a477f27c/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for ElevenLabs: Eleven v3, voice george.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-v3-20260930/b7cfb19c-1d83-4b9a-9b01-d3732eb14545/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for ElevenLabs: Eleven v3, voice george.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["or9b43c2138a97891db9b410"],
		model_id: "minimax/speech-2.8-hd",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/minimax/speech-2.8-hd-20260716/e4d426bf-ca41-4b82-b03a-a6aa55ec62ba/original-0.mp3",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for MiniMax: Speech 2.8 HD, voice English_expressive_narrator.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/minimax/speech-2.8-hd-20260716/58b88d8b-b9a1-4b15-928c-efc80bbf9eb7/original-0.mp3",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for MiniMax: Speech 2.8 HD, voice English_expressive_narrator.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/minimax/speech-2.8-hd-20260716/bcc92e90-dd36-4f49-9d66-338bba9c27fd/original-0.mp3",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for MiniMax: Speech 2.8 HD, voice English_expressive_narrator.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["ora56dead39c2cd8dca48c3c"],
		model_id: "fish-audio/drama-3",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/fish-audio/drama-3-20261013/2ec2094b-5f93-4c7c-a5db-3021af2e8b04/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for Fish Audio: Drama 3.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/fish-audio/drama-3-20261013/014c6e1a-69ff-4e4a-a7e6-0e6fe8fed07a/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for Fish Audio: Drama 3.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/fish-audio/drama-3-20261013/3877fac7-1848-47a6-9969-f2b97c89194b/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for Fish Audio: Drama 3.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["ora8ba3ea4797a858c8ea0fa"],
		model_id: "minimax/speech-2.8-turbo",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/minimax/speech-2.8-turbo-20260716/50c8e130-d651-461a-b62c-d9101fa806d0/original-0.mp3",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for MiniMax: Speech 2.8 Turbo, voice English_expressive_narrator.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/minimax/speech-2.8-turbo-20260716/641983fc-3682-428f-8d7d-70553b90535e/original-0.mp3",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for MiniMax: Speech 2.8 Turbo, voice English_expressive_narrator.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/minimax/speech-2.8-turbo-20260716/7cdd5197-d622-4129-a058-c7be1c1ab9ef/original-0.mp3",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for MiniMax: Speech 2.8 Turbo, voice English_expressive_narrator.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["orb03befda9fb6f1f21a9bff"],
		model_id: "microsoft/mai-voice-2.1",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/microsoft/mai-voice-2.1-20261001/2e250988-a7db-4965-b359-0aaa9fc2a27d/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for Microsoft AI: MAI-Voice-2.1, voice cs-CZ-Grant:MAI-Voice-2.1. This sample uses a different voice from the Bit's default (en-US-Harper:MAI-Voice-2.1).",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/microsoft/mai-voice-2.1-20261001/8b4cdca0-b53c-4545-99ee-95d35ad12aab/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for Microsoft AI: MAI-Voice-2.1, voice cs-CZ-Grant:MAI-Voice-2.1. This sample uses a different voice from the Bit's default (en-US-Harper:MAI-Voice-2.1).",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/microsoft/mai-voice-2.1-20261001/60ecfc9c-59fc-4a1b-a635-996d2bb12f92/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for Microsoft AI: MAI-Voice-2.1, voice cs-CZ-Grant:MAI-Voice-2.1. This sample uses a different voice from the Bit's default (en-US-Harper:MAI-Voice-2.1).",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["orba4c3b7129651577bcabf1"],
		model_id: "elevenlabs/eleven-multilingual-v2",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-multilingual-v2-20260930/d6fa8f0d-f654-48bf-a2bc-b82be9da9ce3/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for ElevenLabs: Eleven Multilingual v2, voice george.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-multilingual-v2-20260930/5774c582-21bc-434a-9d93-8d4dd27ce97e/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for ElevenLabs: Eleven Multilingual v2, voice george.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-multilingual-v2-20260930/7471aee7-248f-4565-aa9d-aa5887ef8684/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for ElevenLabs: Eleven Multilingual v2, voice george.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["orc05353b9684ca8b5c72640"],
		model_id: "sesame/csm-1b",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/sesame/csm-1b/1e6294af-18ed-41d3-ac95-8276d96e5322/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for Sesame: CSM 1B, voice conversational_a.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/sesame/csm-1b/4b6c467b-30e0-4377-b01b-a331151b3497/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for Sesame: CSM 1B, voice conversational_a.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["orcc00b12d95da6b0e3b76d9"],
		model_id: "qwen/qwen-audio-3.0-tts-flash",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/qwen/qwen-audio-3.0-tts-flash-20260723/b7b9b349-dcb5-4904-ba19-8d7ba1edc291/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for Qwen: Qwen-Audio-3.0-TTS Flash, voice loongjohn.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/qwen/qwen-audio-3.0-tts-flash-20260723/e44a7c23-b7fa-4d90-8725-de262f55d958/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for Qwen: Qwen-Audio-3.0-TTS Flash, voice loongjohn.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/qwen/qwen-audio-3.0-tts-flash-20260723/fd06193c-13e8-4f43-b02d-ce706d694005/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for Qwen: Qwen-Audio-3.0-TTS Flash, voice loongjohn.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["ord01e5ccee6e9bf390576df"],
		model_id: "microsoft/mai-voice-2.1-flash",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/microsoft/mai-voice-2.1-flash-20261001/ac77ea94-a00b-44be-818b-3d031542636d/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for Microsoft AI: MAI-Voice-2.1-Flash, voice cs-CZ-Grant:MAI-Voice-2.1-Flash. This sample uses a different voice from the Bit's default (en-US-Harper:MAI-Voice-2.1-Flash).",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/microsoft/mai-voice-2.1-flash-20261001/bb0d9854-d85c-46f1-b7c3-076553d812fb/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for Microsoft AI: MAI-Voice-2.1-Flash, voice cs-CZ-Grant:MAI-Voice-2.1-Flash. This sample uses a different voice from the Bit's default (en-US-Harper:MAI-Voice-2.1-Flash).",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/microsoft/mai-voice-2.1-flash-20261001/4e4672cb-c738-40ec-99d9-6e3e56fff43e/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for Microsoft AI: MAI-Voice-2.1-Flash, voice cs-CZ-Grant:MAI-Voice-2.1-Flash. This sample uses a different voice from the Bit's default (en-US-Harper:MAI-Voice-2.1-Flash).",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["orf328a75d323aab2dac878a"],
		model_id: "elevenlabs/eleven-flash-v2.5",
		media: [
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-flash-v2.5-20260930/78d81533-5683-4e84-b79e-edca7a5a8fc5/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/plain-sentence",
				description:
					"OpenRouter's plain-sentence output sample for ElevenLabs: Eleven Flash v2.5, voice george.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-flash-v2.5-20260930/5680a225-6d42-4ce2-a6c6-0ba74616e23e/original-0.wav",
				kind: "audio",
				source_url:
					"https://openrouter.ai/benchmarks/media/speech/full-paragraph",
				description:
					"OpenRouter long-paragraph narration example for ElevenLabs: Eleven Flash v2.5, voice george.",
				scope: "output",
			},
			{
				url: "https://model-assets.openrouter.ai/model-examples/elevenlabs/eleven-flash-v2.5-20260930/2d9f4e54-b9b6-4f20-ad52-d03cf9bbe4f8/original-0.wav",
				kind: "audio",
				source_url: "https://openrouter.ai/benchmarks/media/speech/punctuation",
				description:
					"OpenRouter punctuation and phrasing example for ElevenLabs: Eleven Flash v2.5, voice george.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["sfa53g81356j8jlntbsm6536"],
		model_id: "Qwen/Qwen3-TTS-12Hz-1.7B-CustomVoice",
		media: [
			{
				url: "https://qianwen-res.oss-cn-beijing.aliyuncs.com/Qwen3-TTS-0115/F05.wav",
				kind: "audio",
				source_url: "https://qwen.ai/blog?id=qwen3tts-0115",
				description:
					"Qwen's published Chinese speech sample for Qwen3-TTS-12Hz-1.7B-CustomVoice, using the Serena voice. This publisher example is not a recording of the local Flow-Like runtime.",
				scope: "output",
			},
			{
				url: "https://qianwen-res.oss-cn-beijing.aliyuncs.com/Qwen3-TTS-0115/m11.wav",
				kind: "audio",
				source_url: "https://qwen.ai/blog?id=qwen3tts-0115",
				description:
					"Official Qwen3-TTS CustomVoice example: English narration in the Aiden voice. This publisher example was not generated with the local Bit runtime.",
				scope: "output",
			},
			{
				url: "https://qianwen-res.oss-cn-beijing.aliyuncs.com/Qwen3-TTS-0115/jp@f3001.wav",
				kind: "audio",
				source_url: "https://qwen.ai/blog?id=qwen3tts-0115",
				description:
					"Official Qwen3-TTS CustomVoice example: Japanese speech in the Ono Anna voice. This publisher example was not generated with the local Bit runtime.",
				scope: "output",
			},
		],
	},
	{
		bit_ids: ["sy42sai6ejy55mmf10vvevtz"],
		model_id: "hexgrad/Kokoro-82M",
		media: [
			{
				url: "https://huggingface.co/hexgrad/Kokoro-82M/resolve/main/samples/af_heart_0.wav",
				kind: "audio",
				source_url:
					"https://huggingface.co/hexgrad/Kokoro-82M/blob/main/SAMPLES.md",
				description:
					"The Kokoro-82M publisher's English sample using af_heart, the voice bundled with this Bit. This publisher example is not a recording of the local Flow-Like runtime.",
				scope: "output",
			},
			{
				url: "https://huggingface.co/hexgrad/Kokoro-82M/resolve/main/samples/af_heart_2.wav",
				kind: "audio",
				source_url:
					"https://huggingface.co/hexgrad/Kokoro-82M/blob/main/SAMPLES.md",
				description:
					"Official Kokoro-82M example in the bundled af_heart voice: short fictional narration. This publisher example was not generated with the local Bit runtime.",
				scope: "output",
			},
			{
				url: "https://huggingface.co/hexgrad/Kokoro-82M/resolve/main/samples/af_heart_3.wav",
				kind: "audio",
				source_url:
					"https://huggingface.co/hexgrad/Kokoro-82M/blob/main/SAMPLES.md",
				description:
					"Official Kokoro-82M example in the bundled af_heart voice: a longer historical passage. This publisher example was not generated with the local Bit runtime.",
				scope: "output",
			},
		],
	},
];
