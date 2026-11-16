import type {
	CatalogQualityRating,
	ReportedQualityBenchmark,
} from "./catalog-quality";

type Category =
	| "ImageGeneration"
	| "VideoGeneration"
	| "Forecasting"
	| "SystemOne";
type Entry = readonly [
	id: string,
	name: string,
	score: number,
	low: number,
	high: number,
	rationale: string,
];

const CRITERIA: Record<Category, string> = {
	ImageGeneration:
		"Visual fidelity, prompt adherence, typography and editing consistency. Editorial 0–100 scale within image generation; price and speed are excluded.",
	VideoGeneration:
		"Motion coherence, visual fidelity and prompt adherence in short generated clips. Editorial 0–100 scale within video generation; price and speed are excluded.",
	Forecasting:
		"Zero-shot forecast accuracy and useful uncertainty estimates across time-series domains. Editorial 0–100 scale within forecasting; validate on the target series and horizon.",
	SystemOne:
		"Correct choices and useful probability estimates for routing, classification and rubric decisions. Editorial 0–100 scale within decision models; this is not an accuracy percentage.",
};

function publisherBenchmarks(id: string): ReportedQualityBenchmark[] {
	const forecastScores: Record<string, [number, number]> = {
		"Datadog/Toto-2.0-22m": [0.496, 0.719],
		"mistralai/Timestral-5M": [0.479, 0.705],
		"mistralai/Timestral-209M": [0.446, 0.658],
	};
	const forecast = forecastScores[id];
	if (forecast) {
		return ["CRPS", "MASE"].map((metric, index) => ({
			name: `GIFT-Eval ${metric}`,
			score: forecast[index],
			unit: `${metric} aggregate`,
			source: `https://huggingface.co/${id}`,
			scope: `Publisher-reported ${id} reference implementation on GIFT-Eval`,
			notes:
				"Dataset-aggregated result from the model card. Benchmark versions and evaluation protocols must match for comparison. Flow-Like's forecasting runtime was not evaluated in this run.",
			direction: "lower",
		}));
	}
	if (id === "ggml-org/Julia-1-GGUF") {
		return [
			{
				name: "Typed decisions accuracy",
				score: 73.15,
				unit: "% accuracy",
				source: "https://huggingface.co/SupersonicLabs/Julia-1",
				scope:
					"Publisher's original Julia-1 checkpoint, H200 BF16 with strict encoding, September 24, 2026",
				notes:
					"1,463 correct answers out of 2,000 typed decisions. This is an accuracy result for a particular suite, not a probability-calibration measurement or a benchmark of the GGUF conversion.",
				direction: "higher",
			},
		];
	}
	if (id === "ggml-org/OpenJev-GGUF") {
		return [
			{
				name: "Text decision accuracy",
				score: 84.0,
				unit: "% accuracy",
				source: "https://huggingface.co/openjev/openjev",
				scope:
					"Publisher's original OpenJev checkpoint on 10,000 text questions from 34 sources",
				notes:
					"The publisher reports 8,403 correct answers, rounded to 84.0%. Some questions had been used during development. This is not a probability-calibration result or a measurement of the local GGUF build.",
				direction: "higher",
			},
		];
	}
	return [];
}

function makeRating(
	category: Category,
	entry: Entry,
	sources: string[],
	creator: [string, string],
): CatalogQualityRating {
	const [id, name, score, low, high, rationale] = entry;
	const benchmarks = publisherBenchmarks(id);
	const evidenceSources =
		id === "perplexity/pplx-decider-flash-v1.2"
			? [...sources, "https://openrouter.ai/~perplexity/pplx-decider-latest"]
			: sources;
	const limitations =
		category === "SystemOne"
			? " No common published calibration benchmark covering all catalog decision models was verified. The range is deliberately broad; validate decision thresholds on your own labeled cases."
			: " This is an editorial estimate, not a measured aggregate benchmark or a statistical confidence interval.";
	return {
		slug: `flow-like-estimate-quality-${category.toLowerCase()}-${id
			.toLowerCase()
			.replace(/[^a-z0-9]+/g, "-")
			.replace(/^-|-$/g, "")}`,
		name,
		creator_name: creator[0],
		creator_slug: creator[1],
		model_ids: [id],
		...(benchmarks.length ? { reported_benchmarks: benchmarks } : {}),
		quality_estimate: {
			category,
			criteria: CRITERIA[category],
			score,
			low,
			high,
			confidence: "low",
			reasoning: `Editorial estimate. ${rationale}${limitations}`,
			sources: evidenceSources,
			updated_at: "2026-11-16",
		},
	};
}

const CREATORS: Record<string, [string, string]> = {
	"black-forest-labs": ["Black Forest Labs", "black-forest-labs"],
	"bytedance-seed": ["ByteDance Seed", "bytedance-seed"],
	recraft: ["Recraft", "recraft"],
	qwen: ["Qwen", "qwen"],
	krea: ["Krea", "krea"],
	sourceful: ["Sourceful", "sourceful"],
	"x-ai": ["xAI", "x-ai"],
	cloudflare: ["Cloudflare", "cloudflare"],
	microsoft: ["Microsoft", "microsoft"],
	upstage: ["Upstage", "upstage"],
	togethercomputer: ["Together AI", "together-ai"],
	"~typesafe": ["TypeSafe", "typesafe"],
	typesafe: ["TypeSafe", "typesafe"],
	respan: ["Respan", "respan"],
	liquid: ["Liquid AI", "liquid-ai"],
	openai: ["OpenAI", "openai"],
	"nace-ai": ["Nace.AI", "nace-ai"],
	"~perplexity": ["Perplexity", "perplexity"],
	perplexity: ["Perplexity", "perplexity"],
	scaledown: ["ScaleDown", "scaledown"],
	jaredpalmer: ["Jared Palmer", "jared-palmer"],
};

// Provider listings establish the specific variant and intended quality tier.
// These judgments are deliberately coarser than a benchmark-derived ranking.
const HOSTED_IMAGES: Entry[] = [
	[
		"black-forest-labs/flux-3-image",
		"FLUX 3 Image",
		92,
		84,
		97,
		"The provider's flagship supports detailed generation and editing with multiple references. Its broad editing controls support a high provisional rating; difficult spatial relationships and text still need review.",
	],
	[
		"black-forest-labs/flux-3-image-klein",
		"FLUX 3 Image Klein",
		84,
		74,
		92,
		"The compact open-weight variant supports generation and reference editing. Rate it below the flagship provisionally; the shared family name does not establish equal prompt adherence.",
	],
	[
		"black-forest-labs/flux.2-max",
		"FLUX.2 Max",
		89,
		80,
		95,
		"The provider positions Max as the quality-oriented FLUX.2 tier for prompt understanding and editing consistency. The rating reflects that positioning with uncertainty across styles and prompts.",
	],
	[
		"black-forest-labs/flux.2-pro",
		"FLUX.2 Pro",
		86,
		76,
		93,
		"The provider emphasizes detailed images, prompt adherence and consistent reference editing. This supports a strong provisional rating for general image work.",
	],
	[
		"black-forest-labs/flux.2-flex",
		"FLUX.2 Flex",
		85,
		75,
		93,
		"Typography and fine detail are the stated strengths, alongside reference editing. The estimate favors those tasks; other visual styles may rank differently.",
	],
	[
		"black-forest-labs/flux.2-klein-4b",
		"FLUX.2 Klein 4B",
		77,
		66,
		87,
		"The compact distilled model supports generation and multi-reference edits. Its own model card lists distorted text and prompt-following failures as limitations; size and speed do not establish flagship-level fidelity.",
	],
	[
		"qwen/qwen-image-3-pro",
		"Qwen Image 3 Pro",
		91,
		81,
		97,
		"The Pro generation and editing tier emphasizes small text and fine details. A high provisional score reflects that capability focus, with uncertainty for complex layouts and multilingual typography.",
	],
	[
		"qwen/qwen-image-3",
		"Qwen Image 3",
		88,
		78,
		95,
		"The provider describes improved text rendering and scene knowledge in a unified generator and editor. These strengths support detailed illustration and layout tasks; exact text still needs checking.",
	],
	[
		"qwen/qwen-image-2.1-pro",
		"Qwen Image 2.1 Pro",
		86,
		76,
		94,
		"The provider identifies Pro as the quality-oriented 2.1 tier. This estimate favors detailed generation and editing over the Turbo variant, without assuming uniform gains on every prompt.",
	],
	[
		"qwen/qwen-image-2.1-turbo",
		"Qwen Image 2.1 Turbo",
		78,
		66,
		88,
		"Turbo prioritizes throughput over peak fidelity according to the provider. It remains a useful drafting option, with a lower provisional quality rating than Pro.",
	],
	[
		"recraft/recraft-v4.1-pro",
		"Recraft V4.1 Pro",
		88,
		78,
		95,
		"The aesthetics-oriented Pro tier produces larger images than standard V4.1. The estimate favors polished visual design and fine detail while keeping uncertainty around strict compositional constraints.",
	],
	[
		"recraft/recraft-v4.1",
		"Recraft V4.1",
		84,
		74,
		92,
		"The provider reports more natural photorealism and more reliable short-prompt following than V4. This supports a strong design-oriented estimate; only one reference image is supported.",
	],
	[
		"recraft/recraft-v4.1-utility-pro",
		"Recraft V4.1 Utility Pro",
		86,
		76,
		94,
		"The general-purpose Pro tier offers larger output than Utility. The estimate rewards detail potential and general editing use without assuming aesthetics and utility variants rank identically.",
	],
	[
		"recraft/recraft-v4.1-utility",
		"Recraft V4.1 Utility",
		82,
		71,
		91,
		"The general-purpose V4.1 variant supports generation and image-conditioned work. This is a provisional broad-use rating with uncertainty for typography and complex layouts.",
	],
	[
		"recraft/recraft-v4.1-flash",
		"Recraft V4.1 Flash",
		76,
		63,
		87,
		"The Flash tier targets rapid text-to-image drafts. Its speed focus supports a lower provisional fidelity estimate than the standard and Pro tiers; speed itself is excluded from this score.",
	],
	[
		"recraft/recraft-v4-pro",
		"Recraft V4 Pro",
		84,
		73,
		93,
		"The Pro tier provides larger images within the V4 generation and editing family. The estimate favors design detail while allowing newer V4.1 variants a modest provisional advantage.",
	],
	[
		"recraft/recraft-v4",
		"Recraft V4",
		80,
		68,
		90,
		"V4 emphasizes composition and visual design in generation and editing. The estimate reflects useful general quality with less evidence for the refinements described for V4.1.",
	],
	[
		"recraft/recraft-v3",
		"Recraft V3",
		73,
		60,
		85,
		"The earlier Recraft generation remains useful for controlled visual assets. Its provisional rating sits below later generations with published composition and aesthetics improvements.",
	],
	[
		"sourceful/riverflow-v2.6-pro",
		"Riverflow V2.6 Pro",
		90,
		79,
		97,
		"The provider's flagship combines generation and editing with a planning stage. This supports a high provisional rating for controlled creative production; the planning stage is not itself evidence of correct output.",
	],
	[
		"sourceful/riverflow-v2.5-pro",
		"Riverflow V2.5 Pro",
		87,
		76,
		95,
		"The quality-oriented 2.5 tier targets precise control in generation and editing. Its estimate is above Fast, with broad overlap because no common local evaluation was run.",
	],
	[
		"sourceful/riverflow-v2.5-fast",
		"Riverflow V2.5 Fast",
		81,
		69,
		91,
		"The provider describes a speed-oriented 2.5 variant. The provisional score reflects a likely quality tradeoff relative to Pro while retaining the same generation and editing task family.",
	],
	[
		"sourceful/riverflow-v2-pro",
		"Riverflow V2 Pro",
		84,
		72,
		93,
		"The Pro variant emphasizes text rendering and generation control. The estimate treats these as strengths while allowing uncertainty for reference consistency and newer releases.",
	],
	[
		"sourceful/riverflow-v2-fast",
		"Riverflow V2 Fast",
		78,
		65,
		89,
		"The earlier speed-oriented Riverflow tier targets iterative production. Its provisional fidelity rating is below Pro; actual quality depends on prompt complexity and editing needs.",
	],
	[
		"bytedance-seed/seedream-5-0-pro",
		"Seedream 5.0 Pro",
		90,
		80,
		96,
		"The provider emphasizes lifelike scenes and precise editing for visual production. The estimate favors those strengths while retaining uncertainty for exact text and multilingual layouts.",
	],
	[
		"bytedance-seed/seedream-5-0-lite",
		"Seedream 5.0 Lite",
		85,
		74,
		94,
		"The provider highlights complex-prompt understanding and reference-informed creation. The estimate is strong but below Pro provisionally; external knowledge support does not guarantee visual accuracy.",
	],
	[
		"bytedance-seed/seedream-5-0-flash",
		"Seedream 5.0 Flash",
		81,
		68,
		91,
		"The fast 5.0 tier targets interactive generation and editing. Its quality estimate is conservative relative to Pro, with substantial overlap because workload-specific results can differ.",
	],
	[
		"bytedance-seed/seedream-4.5",
		"Seedream 4.5",
		83,
		72,
		92,
		"The provider reports improved editing consistency and subject preservation over 4.0. This supports a strong estimate for reference-based edits, below newer quality-focused releases provisionally.",
	],
	[
		"krea/krea-2-large",
		"Krea 2 Large",
		86,
		75,
		94,
		"The provider emphasizes textured realism and expressive styles in the larger checkpoint. The estimate favors photographic and artistic work; that aesthetic preference may differ from strict graphic-layout needs.",
	],
	[
		"krea/krea-2-medium",
		"Krea 2 Medium",
		82,
		71,
		91,
		"The provider positions Medium as a broadly useful model with consistent outputs. Its provisional score reflects general creative quality while leaving room for Large on demanding visual detail.",
	],
	[
		"krea/krea-2-medium-turbo",
		"Krea 2 Medium Turbo",
		76,
		63,
		87,
		"This distilled variant targets rapid design exploration. Its lower provisional fidelity estimate reflects the speed-focused distillation, rather than a measured universal quality gap.",
	],
	[
		"x-ai/grok-imagine-image-2.0",
		"Grok Imagine Image 2.0",
		86,
		74,
		95,
		"The generation and reference-editing endpoint supports general visual creation. The estimate reflects that broad capability with a wide range because no shared catalog benchmark was run.",
	],
];

const HOSTED_DECISIONS: Entry[] = [
	[
		"cloudflare/clef",
		"Clef Decisions",
		82,
		65,
		93,
		"The publisher's Decision Index results support strong routing and classification, with variable reasoning performance. The hosted endpoint currently reads roughly the first 2K text tokens, which limits long-state reliability.",
	],
	[
		"cloudflare/clef-flash",
		"Clef Flash Decisions",
		77,
		59,
		90,
		"The smaller 9B variant has publisher results showing task-dependent gains and losses versus Clef. Hosted text is currently truncated near 2K tokens, so the nominal context does not establish long-document coverage.",
	],
	[
		"cloudflare/clef-omni",
		"Clef Omni",
		78,
		56,
		93,
		"The multimodal mixture-of-experts member supports decisions across several media types. This estimate is provisional because modality breadth does not establish calibration or accuracy for a particular task.",
	],
	[
		"microsoft/microsoft-decision-1",
		"Microsoft Decisions",
		77,
		56,
		91,
		"The provider describes a Qwen3.5-9B model post-trained for single-pass scoring. It fits classification and verification; continually updated weights mean the same endpoint may change behavior.",
	],
	[
		"upstage/solar-decide",
		"Solar Decisions",
		77,
		55,
		92,
		"The Solar Mini 4 decision endpoint is designed for routing and policy checks, with Korean-language support. Its large advertised context is a capability limit rather than measured decision accuracy.",
	],
	[
		"upstage/solar-decide-flash",
		"Solar Decide Flash",
		73,
		51,
		90,
		"The fast Solar Mini 4 variant retains the same decision interface and Korean focus. The provisional rating is slightly lower than the standard variant, with broad overlap and no measured quality penalty asserted.",
	],
	[
		"togethercomputer/tev1-4b-experimental",
		"Tev Decisions Experimental",
		60,
		35,
		82,
		"The experimental 4B model is trained to select labeled options. Its provider page documents ordinary chat completions rather than a dedicated decision-head runtime, so this rating does not verify the catalog's SystemOne integration.",
	],
	[
		"~typesafe/jev-latest",
		"Jev Latest",
		80,
		57,
		94,
		"The alias follows the current Jev release. Jev targets typed decisions, but alias changes can alter accuracy and calibration without a catalog version change; validate the resolved model.",
	],
	[
		"typesafe/jev-1.13",
		"Jev Decisions 1.13",
		78,
		57,
		92,
		"The pinned Jev release targets structured routing and classification. Publisher descriptions and related comparisons support a capable estimate, but do not reproduce this exact endpoint on a common calibration suite.",
	],
	[
		"respan/span-02",
		"Span Decisions",
		76,
		54,
		91,
		"The successor to Span-01 targets text and image evaluation, guardrails and routing. Its broad judgment use cases support a provisional rating rather than an established cross-domain accuracy score.",
	],
	[
		"liquid/d1-ultra",
		"Liquid D1 Ultra",
		78,
		55,
		93,
		"Liquid's largest decision endpoint handles text and image states. The estimate is modestly above d1 based on its stated capability tier, with wide uncertainty because size alone does not establish calibration.",
	],
	[
		"liquid/d1",
		"Liquid Decisions",
		71,
		48,
		88,
		"The structured endpoint supports classification and policy decisions. This provisional rating excludes availability; its provider page announces retirement on November 20, 2026.",
	],
	[
		"openai/gpt-6-luna-decisions",
		"GPT-6 Luna Decisions",
		84,
		62,
		96,
		"The Luna decision endpoint supports typed probabilities and rubric scores over text and images. This high provisional rating reflects its general model foundation; dedicated calibration and the catalog runtime were not independently evaluated.",
	],
	[
		"nace-ai/drex-v2.0",
		"Drex Decisions",
		75,
		52,
		91,
		"The provider describes a larger mixture-of-experts successor trained for typed decision probabilities. It appears suitable for routing and policy checks, while public capability claims do not establish consistent calibration.",
	],
	[
		"~perplexity/pplx-decider-latest",
		"Perplexity Decisions",
		78,
		55,
		93,
		"The alias follows the latest Decider release. Its provisional quality estimate must be revisited when the target changes; a stable alias is not a fixed evaluated checkpoint.",
	],
	[
		"perplexity/pplx-decider-flash-v1.2",
		"Perplexity Decider Flash",
		70,
		43,
		89,
		"Provisional estimate for the catalog's versioned Flash decision endpoint. Available family documentation supports the task type, but exact-version comparative accuracy and calibration were not verified.",
	],
	[
		"scaledown/classify-1",
		"ScaleDown Classify",
		72,
		48,
		89,
		"The provider documents typed choices and scores for moderation, intent detection and domain tagging. This supports task fit; broad probability calibration remains unverified.",
	],
	[
		"jaredpalmer/kev-4b",
		"Kev Decisions",
		68,
		45,
		85,
		"The open 4B model uses a tuned adapter and decision head, and is the publisher's recommended Kev checkpoint. The compact backbone supports routing experiments, with uncertainty for reasoning-heavy and unfamiliar domains.",
	],
];

const LOCAL_DECISIONS: readonly [Entry, string, [string, string]][] = [
	[
		[
			"ggml-org/Clef-GGUF",
			"Clef Decisions Local",
			81,
			63,
			93,
			"The original Clef card reports strong classification results on its Decision Index suite. This estimate covers the local GGUF conversion; quantization and runtime parity were not independently evaluated.",
		],
		"Cloudflare/clef",
		["Cloudflare", "cloudflare"],
	],
	[
		[
			"ggml-org/Clef-Flash-GGUF",
			"Clef Flash Decisions Local",
			76,
			57,
			90,
			"The original smaller Clef variant has task-dependent publisher results. The local conversion retains uncertainty about probability shifts from quantization; hosted text-truncation behavior is not assumed for local execution.",
		],
		"Cloudflare/clef-flash",
		["Cloudflare", "cloudflare"],
	],
	[
		[
			"ggml-org/OpenJev-GGUF",
			"OpenJEV Decisions Local",
			80,
			63,
			92,
			"The original publisher reports 84.0% on its 10,000-question text decision comparison, with some cases previously used during development. This is evidence for task fit, not a calibrated aggregate score for the GGUF conversion.",
		],
		"openjev/openjev",
		["OpenJev", "openjev"],
	],
	[
		[
			"ggml-org/Bespoke-Nimble-9B-v3-GGUF",
			"Nimble Decisions Local",
			71,
			49,
			88,
			"The pack identifies a decision-tuned Qwen3.5-9B checkpoint. Related publisher comparisons suggest useful classification but do not verify this exact GGUF v3 checkpoint across catalog workloads.",
		],
		"bespokelabs/Bespoke-Nimble-9B-v3",
		["Bespoke Labs", "bespoke-labs"],
	],
	[
		[
			"ggml-org/Laya-GGUF",
			"Laya Decisions Local",
			48,
			28,
			71,
			"The original Laya documentation distinguishes the base English checkpoint from a substantially stronger task-specific fine-tune. This catalog pack is not that tuned checkpoint, so its general decision-quality estimate is conservative.",
		],
		"convaiinnovations/laya",
		["ConvAI Innovations", "convaiinnovations"],
	],
	[
		[
			"Atlas3D/JEV-27B-VL-GGUF",
			"JEV Decisions Local",
			74,
			51,
			90,
			"The pack's own 309-decision parity test reports lower agreement at Q4_K_M than Q8_0. Agreement with a reference implementation measures conversion fidelity, not correctness; this estimate keeps uncertainty for the installed quantization.",
		],
		"autotrust/JEV-27B-VL",
		["AutoTrust", "autotrust"],
	],
	[
		[
			"ggml-org/lev-GGUF",
			"LEV Decisions Local",
			65,
			42,
			84,
			"The published pack adapts an Interfaze decision model based on Qwen3.5-4B. This supports compact routing and classification use; quality of this converted checkpoint remains provisionally assessed.",
		],
		"interfaze-ai/lev",
		["Interfaze", "interfaze"],
	],
	[
		[
			"ggml-org/Kev-4B-GGUF",
			"KEV Decisions Local",
			66,
			43,
			84,
			"The converted Kev 4B checkpoint provides a native typed-decision interface. Its rating follows the original compact decision model with additional uncertainty for quantized probability outputs.",
		],
		"jaredpalmer/kev-4b",
		["Jared Palmer", "jared-palmer"],
	],
	[
		[
			"ggml-org/Julia-1-GGUF",
			"Julia-1 Decisions Local",
			62,
			42,
			82,
			"The original 144M checkpoint reports 73.15% on typed decisions and uneven classification pilot results. Missing knowledge, long option lists and multi-step reasoning are stated limitations; the GGUF build was not benchmarked here.",
		],
		"SupersonicLabs/Julia-1",
		["Supersonic Labs", "supersonic-labs"],
	],
];

const FORECASTS: readonly [Entry, [string, string]][] = [
	[
		[
			"Datadog/Toto-2.0-22m",
			"Datadog Toto 2 22M",
			83,
			72,
			92,
			"The exact 22M card reports GIFT-Eval CRPS 0.496 and MASE 0.719, plus observability-focused results. Larger Toto family members score better, so their leaderboard claims are not transferred to this compact checkpoint.",
		],
		["Datadog", "datadog"],
	],
	[
		[
			"google/timesfm-2.5-200m-pytorch",
			"Google TimesFM 2.5 200M",
			81,
			68,
			91,
			"Google's checkpoint provides pretrained point and quantile forecasting with a documented reference implementation. This estimate is below newer broad-task leaders provisionally; the card does not establish accuracy on the user's target series.",
		],
		["Google", "google"],
	],
	[
		[
			"mistralai/Timestral-5M",
			"Mistral Timestral 5M",
			85,
			75,
			93,
			"The publisher's family comparison reports GIFT-Eval CRPS 0.479 and MASE 0.705 for the 5M model, weaker than 209M but close on several tasks. Domain and forecast horizon can change that ordering.",
		],
		["Mistral", "mistral"],
	],
	[
		[
			"mistralai/Timestral-209M",
			"Mistral Timestral 209M",
			92,
			83,
			97,
			"The publisher reports GIFT-Eval CRPS 0.446 and MASE 0.658, with strong TIME and covariate-task results. These support a high estimate for broad forecasting; they do not reproduce Flow-Like's runtime or every domain.",
		],
		["Mistral", "mistral"],
	],
	[
		[
			"ibm-granite/granite-timeseries-ttm-r2",
			"IBM Granite TinyTimeMixer R2",
			74,
			60,
			86,
			"IBM's compact multivariate model supports pretrained forecasting and adaptation. Quality depends on choosing the appropriate context, horizon and frequency variant; the estimate avoids transferring results from fine-tuned configurations to every zero-shot use.",
		],
		["IBM", "ibm"],
	],
	[
		[
			"amazon/chronos-2",
			"Amazon Chronos 2",
			87,
			77,
			95,
			"Amazon documents univariate, multivariate and covariate-informed forecasting with quantile outputs and several public evaluations. Broad task support and benchmark evidence justify a strong estimate; validate probability coverage on the target data.",
		],
		["Amazon", "amazon"],
	],
];

const LOCAL_GENERATION: readonly [Category, Entry, string, [string, string]][] =
	[
		[
			"ImageGeneration",
			[
				"qwen-image-2.1-q4-k-m",
				"Qwen Image 2.1 Q4_K_M",
				82,
				69,
				92,
				"The original 2.1 model emphasizes typography, realistic textures and editing. This score covers the catalog's quantized local generation pack; reference features and full-precision results are not assumed to transfer unchanged.",
			],
			"Qwen/Qwen-Image-2.1",
			["Qwen", "qwen"],
		],
		[
			"ImageGeneration",
			[
				"qwen-image-2512-q4-k-m",
				"Qwen Image 2512 Q4_K_M",
				79,
				66,
				90,
				"The publisher reports improved realism, natural detail and text rendering over the original Qwen Image. The estimate allows uncertainty for local quantization and the newer image generations now in the catalog.",
			],
			"Qwen/Qwen-Image-2512",
			["Qwen", "qwen"],
		],
		[
			"ImageGeneration",
			[
				"flux2-klein-4b-q4-k-m",
				"FLUX.2 Klein 4B Q4_K_M",
				75,
				62,
				87,
				"The compact distilled checkpoint supports useful local image creation, but the publisher lists text distortion and prompt mismatches as limitations. Local Q4 conversion adds uncertainty beyond the original model's demonstrations.",
			],
			"black-forest-labs/FLUX.2-klein-4B",
			["Black Forest Labs", "black-forest-labs"],
		],
		[
			"ImageGeneration",
			[
				"z-image-turbo-q4-k",
				"Z-Image-Turbo Q4_K",
				76,
				63,
				88,
				"The publisher emphasizes photorealism and English/Chinese text rendering in an eight-step distilled model. This estimate is for the local quantized Turbo pack and does not assume the editing capabilities of other Z-Image variants.",
			],
			"Tongyi-MAI/Z-Image-Turbo",
			["Tongyi-MAI", "tongyi-mai"],
		],
		[
			"VideoGeneration",
			[
				"wan22-ti2v-5b-q4-k-m",
				"Wan2.2 TI2V 5B Q4_K_M",
				75,
				61,
				87,
				"The newer 5B checkpoint supports text- and image-conditioned video with greater detail potential than the compact Wan2.1 model. The estimate includes uncertainty for motion, long sequences and the catalog's quantized runtime.",
			],
			"Wan-AI/Wan2.2-TI2V-5B",
			["Wan AI", "wan-ai"],
		],
		[
			"VideoGeneration",
			[
				"wan21-t2v-13b-fp16",
				"Wan2.1 T2V 1.3B FP16",
				63,
				48,
				78,
				"The compact 1.3B text-to-video checkpoint is useful for short drafts. Its capacity and generation scope suggest a lower provisional fidelity and motion estimate than Wan2.2 5B; the identifier refers to 1.3B, not 13B.",
			],
			"Wan-AI/Wan2.1-T2V-1.3B",
			["Wan AI", "wan-ai"],
		],
	];

export const SPECIALIZED_QUALITY_RATINGS: CatalogQualityRating[] = [
	...HOSTED_IMAGES.map((entry) =>
		makeRating(
			"ImageGeneration",
			entry,
			[`https://openrouter.ai/${entry[0]}`],
			CREATORS[entry[0].split("/")[0]],
		),
	),
	...HOSTED_DECISIONS.map((entry) =>
		makeRating(
			"SystemOne",
			entry,
			[`https://openrouter.ai/${entry[0]}`],
			CREATORS[entry[0].split("/")[0]],
		),
	),
	...LOCAL_DECISIONS.map(([entry, original, creator]) =>
		makeRating(
			"SystemOne",
			entry,
			[
				`https://huggingface.co/${entry[0]}`,
				`https://huggingface.co/${original}`,
			],
			creator,
		),
	),
	...FORECASTS.map(([entry, creator]) =>
		makeRating(
			"Forecasting",
			entry,
			[`https://huggingface.co/${entry[0]}`],
			creator,
		),
	),
	...LOCAL_GENERATION.map(([category, entry, original, creator]) =>
		makeRating(
			category,
			entry,
			[`https://huggingface.co/${original}`],
			creator,
		),
	),
];
