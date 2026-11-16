import type { CatalogQualityRating } from "./catalog-quality";

const HF = "https://huggingface.co/";
const OPENAI = "https://developers.openai.com/api/docs/guides/embeddings";
const COHERE = "https://docs.cohere.com/docs/cohere-embed";
const VOYAGE = "https://docs.voyageai.com/docs/embeddings";
const GEMINI = "https://ai.google.dev/gemini-api/docs/embeddings";
const TITAN =
	"https://docs.aws.amazon.com/bedrock/latest/userguide/titan-embedding-models.html";

interface EmbeddingRatingInput {
	name: string;
	ids: string[];
	creator: [string, string];
	score: number;
	range: [number, number];
	reasoning: string;
	sources: string[];
}

function embedding(input: EmbeddingRatingInput): CatalogQualityRating {
	return {
		slug: `flow-like-estimate-quality-${input.ids[0]
			.toLowerCase()
			.replace(/[^a-z0-9]+/g, "-")}`,
		name: input.name,
		creator_name: input.creator[0],
		creator_slug: input.creator[1],
		model_ids: input.ids,
		repository_urls: input.ids
			.map((id) => `${HF}${id}`)
			.filter((url) =>
				input.sources.some(
					(source) => source.toLowerCase() === url.toLowerCase(),
				),
			),
		quality_estimate: {
			category: "Embedding",
			score: input.score,
			low: input.range[0],
			high: input.range[1],
			confidence: "low",
			criteria:
				"Retrieval relevance and semantic representation in the stated text domain. Editorial 0–100 scale within text embeddings; excludes price and latency. Multimodal capabilities do not imply a higher text-retrieval score.",
			reasoning: `Editorial estimate. ${input.reasoning} Serving parity with the cited model is inferred; the catalog endpoint or quantized artifact has not been independently benchmarked.`,
			sources: input.sources,
			updated_at: "2026-11-16",
		},
	};
}

// Ratings follow the same category scale as catalog-quality.ts. Intervals express
// editorial uncertainty, and are not confidence intervals from a shared test set.
export const ADDITIONAL_EMBEDDING_RATINGS: CatalogQualityRating[] = [
	embedding({
		name: "EmbeddingGemma 300M",
		ids: ["google/embeddinggemma-300m", "@cf/google/embeddinggemma-300m"],
		creator: ["Google", "google"],
		score: 82,
		range: [75, 89],
		reasoning:
			"The publisher reports multilingual, English and code retrieval evaluations for this compact model. Its short context and the quality loss from reducing vector dimensions limit some document-search workloads. This rating assumes the full 768-dimensional output.",
		sources: [`${HF}google/embeddinggemma-300m`],
	}),
	embedding({
		name: "EmbeddingGemma 2",
		ids: [
			"google/embeddinggemma-2",
			"onnx-community/embeddinggemma-2-ONNX",
			"@cf/google/embeddinggemma-2",
		],
		creator: ["Google", "google"],
		score: 86,
		range: [76, 93],
		reasoning:
			"A newer encoder supporting text and media in a common embedding space. This provisional rating covers text retrieval with the prescribed task prefixes. The catalog's Q4 conversion and reduced dimensions may change results; multimodal breadth alone does not establish text-retrieval superiority.",
		sources: [
			"https://huggingface.co/docs/transformers/model_doc/embedding_gemma2",
			`${HF}onnx-community/embeddinggemma-2-ONNX`,
		],
	}),
	embedding({
		name: "Granite Embedding 311M Multilingual R2",
		ids: ["ibm-granite/granite-embedding-311m-multilingual-r2"],
		creator: ["IBM", "ibm"],
		score: 84,
		range: [77, 91],
		reasoning:
			"IBM reports improved multilingual, code and long-document retrieval over its previous generation. The 311M model outperforms the 97M variant in the publisher's retrieval comparisons. Explicit retrieval training covers fewer languages than the tokenizer's full language inventory.",
		sources: [`${HF}ibm-granite/granite-embedding-311m-multilingual-r2`],
	}),
	embedding({
		name: "Granite Embedding 97M Multilingual R2",
		ids: ["ibm-granite/granite-embedding-97m-multilingual-r2"],
		creator: ["IBM", "ibm"],
		score: 79,
		range: [71, 87],
		reasoning:
			"The smaller R2 encoder improves retrieval over the older Granite family, with lower publisher-reported aggregate retrieval results than the 311M variant. Suitable for multilingual passage search; compactness is not counted as quality in this score.",
		sources: [
			`${HF}ibm-granite/granite-embedding-97m-multilingual-r2`,
			`${HF}ibm-granite/granite-embedding-311m-multilingual-r2`,
		],
	}),
	embedding({
		name: "Harrier OSS v1 0.6B",
		ids: ["microsoft/harrier-oss-v1-0.6b"],
		creator: ["Microsoft", "microsoft"],
		score: 88,
		range: [81, 94],
		reasoning:
			"Microsoft reports strong multilingual MTEB v2 performance for the 0.6B variant. The model is instruction-aware and needs the matching query prompt, last-token pooling and normalization. Domain and language coverage should be checked on the actual corpus.",
		sources: [`${HF}microsoft/harrier-oss-v1-0.6b`],
	}),
	...(
		[
			["0.6B", 86, 79, 93],
			["4B", 91, 84, 96],
			["8B", 93, 87, 97],
		] as const
	).map(([size, score, low, high]) =>
		embedding({
			name: `Qwen3 Embedding ${size}`,
			ids: [
				`Qwen/Qwen3-Embedding-${size}`,
				`qwen/qwen3-embedding-${size.toLowerCase()}`,
				...(size === "0.6B" ? ["@cf/qwen/qwen3-embedding-0.6b"] : []),
			],
			creator: ["Qwen", "qwen"],
			score,
			range: [low, high],
			reasoning: `The ${size} member of Qwen's instruction-aware embedding family supports multilingual and code retrieval. Publisher evaluations support differentiated quality across the family; larger variants receive higher estimates. Query instructions, output dimensions and domain can change the ordering.`,
			sources: [`${HF}Qwen/Qwen3-Embedding-${size}`],
		}),
	),
	embedding({
		name: "Perplexity Embed v1 0.6B",
		ids: ["perplexity-ai/pplx-embed-v1-0.6b", "perplexity/pplx-embed-v1-0.6b"],
		creator: ["Perplexity", "perplexity"],
		score: 86,
		range: [78, 93],
		reasoning:
			"Designed for independent queries and documents in web-scale retrieval. The smaller v1 model targets a compact representation; it is distinct from the context model for chunks with surrounding text. Local and hosted entries share this estimate, assuming equivalent model processing and cosine similarity.",
		sources: [`${HF}perplexity-ai/pplx-embed-v1-0.6b`],
	}),
	embedding({
		name: "Perplexity Embed v1 4B",
		ids: ["perplexity/pplx-embed-v1-4b", "perplexity-ai/pplx-embed-v1-4b"],
		creator: ["Perplexity", "perplexity"],
		score: 92,
		range: [84, 97],
		reasoning:
			"The larger v1 encoder targets retrieval quality for web-scale search. It embeds independent text, unlike the contextual variant. The estimate assumes the native vector handling and cosine similarity described by the publisher; specialized enterprise corpora may produce a different ordering.",
		sources: [
			`${HF}perplexity-ai/pplx-embed-v1-4b`,
			"https://openrouter.ai/perplexity/pplx-embed-v1-4b",
		],
	}),
	embedding({
		name: "Llama Nemotron Embed VL 1B V2",
		ids: ["nvidia/llama-nemotron-embed-vl-1b-v2:free"],
		creator: ["NVIDIA", "nvidia"],
		score: 86,
		range: [76, 93],
		reasoning:
			"Designed for question-to-document retrieval, including document pages with visual structure. This estimate covers its text-search use in the catalog. Publisher visual-document evaluations provide useful context but do not establish a universal text-search rank.",
		sources: [`${HF}nvidia/llama-nemotron-embed-vl-1b-v2`],
	}),
	embedding({
		name: "Nemotron 3 Embed 1B",
		ids: ["nvidia/nemotron-3-embed-1b:free"],
		creator: ["NVIDIA", "nvidia"],
		score: 87,
		range: [77, 94],
		reasoning:
			"The model is positioned for enterprise, code and agentic text retrieval. This provisional quality estimate reflects its retrieval specialization; throughput and the free endpoint are excluded. The serving description does not supply a directly comparable catalog-wide evaluation.",
		sources: ["https://openrouter.ai/nvidia/nemotron-3-embed-1b:free"],
	}),
	embedding({
		name: "LFM2.5 Embedding 350M",
		ids: [
			"liquid/lfm-2.5-embedding-350m:free",
			"LiquidAI/LFM2.5-Embedding-350M",
		],
		creator: ["Liquid AI", "liquid-ai"],
		score: 83,
		range: [74, 91],
		reasoning:
			"A dense encoder for multilingual and cross-language retrieval across eleven target languages. The separate ColBERT model uses a different retrieval method and is not covered by this rating. Small size and free hosting do not increase the quality score.",
		sources: [`${HF}LiquidAI/LFM2.5-Embedding-350M`],
	}),
	embedding({
		name: "Cohere Embed 5 Pro",
		ids: ["cohere/embed-5-pro"],
		creator: ["Cohere", "cohere"],
		score: 95,
		range: [87, 99],
		reasoning:
			"Cohere identifies Pro as its quality-focused Embed 5 tier for retrieval and indexing. This rating covers text in its supported languages. Dimension reduction and quantization may lower quality; visual-input support is outside this score.",
		sources: [COHERE],
	}),
	embedding({
		name: "Cohere Embed 5 Fast",
		ids: ["cohere/embed-5-fast"],
		creator: ["Cohere", "cohere"],
		score: 90,
		range: [81, 96],
		reasoning:
			"The lighter Embed 5 tier shares Pro's vector space but trades some retrieval quality for latency. This score reflects that documented family tradeoff, not a measurement against every other catalog model. It assumes appropriate query and document input types.",
		sources: [COHERE],
	}),
	...(
		[
			[
				"voyage-4-large",
				95,
				88,
				99,
				"Voyage's quality-focused general and multilingual retrieval tier.",
			],
			[
				"voyage-4",
				91,
				83,
				97,
				"The general and multilingual retrieval member of the Voyage 4 family.",
			],
			[
				"voyage-4-lite",
				85,
				76,
				93,
				"The lighter Voyage 4 model prioritizes latency and cost; those advantages are excluded from this quality score.",
			],
			[
				"voyage-code-4",
				95,
				86,
				99,
				"A code-retrieval specialist for repository search and coding agents. Its rating is scoped to code, and does not establish the same rank for general prose.",
			],
		] as const
	).map(([model, score, low, high, reasoning]) =>
		embedding({
			name: model,
			ids: [`mongodb/${model}`],
			creator: ["Voyage AI", "voyage-ai"],
			score,
			range: [low, high],
			reasoning: `${reasoning} Publisher positioning informs this estimate; use the correct query/document input type and evaluate vector truncation on the target corpus.`,
			sources: [VOYAGE],
		}),
	),
	embedding({
		name: "Voyage Multimodal 3.5",
		ids: ["mongodb/voyage-multimodal-3.5"],
		creator: ["Voyage AI", "voyage-ai"],
		score: 91,
		range: [81, 97],
		reasoning:
			"A retrieval model for text and interleaved media. This provisional rating covers the catalog's text-embedding use. Image and video retrieval have different failure modes, and its multimodal scope does not establish parity with Voyage's text-only specialist models.",
		sources: ["https://openrouter.ai/mongodb/voyage-multimodal-3.5"],
	}),
	...(
		[
			[
				"gemini-embedding-001",
				92,
				84,
				97,
				"A text-only model with broad language and domain coverage; select the task type appropriate to retrieval.",
			],
			[
				"gemini-embedding-2",
				94,
				85,
				98,
				"The current multimodal model also supports text retrieval across many languages; use the recommended query and document prefixes.",
			],
			[
				"gemini-embedding-2-preview",
				91,
				79,
				97,
				"A preview release of the multimodal family. The estimate has wider uncertainty because preview and stable endpoints can differ.",
			],
		] as const
	).map(([model, score, low, high, reasoning]) =>
		embedding({
			name: model,
			ids: [`google/${model}`],
			creator: ["Google", "google"],
			score,
			range: [low, high],
			reasoning: `${reasoning} This score concerns text relevance, not media understanding or index compatibility with other Gemini versions.`,
			sources: [GEMINI, `https://openrouter.ai/google/${model}`],
		}),
	),
	...(
		[
			[
				"text-embedding-3-large",
				86,
				79,
				93,
				"The larger third-generation encoder has stronger published English and multilingual results than the small variant.",
			],
			[
				"text-embedding-3-small",
				79,
				71,
				87,
				"The compact third-generation encoder improves on the legacy ada model, with lower published retrieval quality than the large variant.",
			],
			[
				"text-embedding-ada-002",
				69,
				60,
				79,
				"The legacy encoder remains useful for established indexes but has weaker published retrieval results than the third-generation family.",
			],
		] as const
	).map(([model, score, low, high, reasoning]) =>
		embedding({
			name: model,
			ids: [`openai/${model}`],
			creator: ["OpenAI", "openai"],
			score,
			range: [low, high],
			reasoning: `${reasoning} The estimate assumes full-size embeddings; shorter vectors, language and domain can alter results.`,
			sources: [OPENAI],
		}),
	),
	embedding({
		name: "Mistral Embed 2312",
		ids: ["mistralai/mistral-embed-2312"],
		creator: ["Mistral AI", "mistral"],
		score: 78,
		range: [69, 87],
		reasoning:
			"A general text encoder for semantic search and retrieval-augmented generation. This estimate favors ordinary document relevance; it does not transfer the code-specific strengths of Codestral Embed to this earlier model.",
		sources: ["https://docs.mistral.ai/inference/embeddings"],
	}),
	embedding({
		name: "Codestral Embed 2505",
		ids: ["mistralai/codestral-embed-2505"],
		creator: ["Mistral AI", "mistral"],
		score: 90,
		range: [81, 96],
		reasoning:
			"Specialized for repository and code retrieval, which warrants a higher estimate in that domain than general text encoders. This score should not be read as the same rank for prose, multilingual documents or every programming language.",
		sources: [
			"https://docs.mistral.ai/inference/embeddings",
			"https://openrouter.ai/mistralai/codestral-embed-2505",
		],
	}),
	...(
		[
			[
				"titan-embed-text-v1",
				66,
				56,
				77,
				"A first-generation text encoder with a fixed vector size. It is a baseline option for established Titan indexes.",
			],
			[
				"titan-embed-text-v2",
				76,
				66,
				86,
				"An updated encoder optimized for English text retrieval. The publisher warns that cross-language queries can produce suboptimal results.",
			],
		] as const
	).map(([model, score, low, high, reasoning]) =>
		embedding({
			name: model,
			ids: [`amazon/${model}`],
			creator: ["Amazon", "amazon"],
			score,
			range: [low, high],
			reasoning: `${reasoning} Long-document support does not remove the need for relevant passage chunking.`,
			sources: [TITAN, `https://openrouter.ai/amazon/${model}`],
		}),
	),
	embedding({
		name: "Titan Multimodal Embeddings G1",
		ids: ["amazon/titan-embed-image-v1"],
		creator: ["Amazon", "amazon"],
		score: 68,
		range: [56, 80],
		reasoning:
			"A shared image-text encoder for multimodal search and recommendation. The catalog places it under text embeddings, so this estimate concerns short text queries and descriptions; it is poorly matched to long-document text retrieval. Image retrieval needs its own evaluation.",
		sources: ["https://openrouter.ai/amazon/titan-embed-image-v1"],
	}),
	...(
		[
			[
				"bge-base-en-v1.5",
				78,
				70,
				85,
				"An English passage encoder with a strong retrieval focus; the base model has less capacity than the large variant.",
			],
			[
				"bge-large-en-v1.5",
				81,
				73,
				88,
				"The larger English passage encoder offers stronger family-level retrieval results, with a short input limit requiring document chunking.",
			],
			[
				"bge-m3",
				84,
				76,
				91,
				"A multilingual model with long-document support. This rating covers dense embeddings only; sparse and multi-vector combinations can yield different results.",
			],
		] as const
	).map(([model, score, low, high, reasoning]) =>
		embedding({
			name: model,
			ids: [`baai/${model}`, `BAAI/${model}`],
			creator: ["BAAI", "baai"],
			score,
			range: [low, high],
			reasoning,
			sources: [`${HF}BAAI/${model}`, `${HF}BAAI/bge-m3`],
		}),
	),
	...(
		[
			[
				"e5-base-v2",
				75,
				67,
				83,
				"An English query-passage encoder with established retrieval performance and a short input limit.",
			],
			[
				"e5-large-v2",
				78,
				70,
				86,
				"The larger English E5 v2 variant improves aggregate quality over the base variant, with domain-dependent gains.",
			],
			[
				"multilingual-e5-large",
				83,
				75,
				90,
				"A multilingual retrieval encoder covering many languages. This is the original large variant, whose behavior differs from large-instruct.",
			],
		] as const
	).map(([model, score, low, high, reasoning]) =>
		embedding({
			name: model,
			ids: [`intfloat/${model}`],
			creator: ["Microsoft", "microsoft"],
			score,
			range: [low, high],
			reasoning: `${reasoning} Use the required query and passage prefixes; truncating long inputs can discard relevant evidence.`,
			sources: [`${HF}intfloat/${model}`],
		}),
	),
	...(
		[
			["gte-base", 77, 69, 85],
			["gte-large", 80, 72, 88],
		] as const
	).map(([model, score, low, high]) =>
		embedding({
			name: model,
			ids: [`thenlper/${model}`],
			creator: ["Alibaba", "alibaba"],
			score,
			range: [low, high],
			reasoning:
				"The original English GTE family performs well on passage similarity and retrieval, with a modest publisher-reported advantage for large over base. Its 512-token input limit makes passage chunking necessary. This is distinct from later multilingual GTE models.",
			sources: [`${HF}thenlper/${model}`],
		}),
	),
	...(
		[
			[
				"all-mpnet-base-v2",
				"all-mpnet-base-v2",
				71,
				63,
				80,
				"A general English sentence encoder suited to semantic similarity and clustering as well as search. Short-input training limits whole-document representations.",
			],
			[
				"all-minilm-l6-v2",
				"all-MiniLM-L6-v2",
				64,
				55,
				74,
				"A compact English encoder for sentences and short paragraphs. Useful for broad semantic matching, with less capacity for difficult retrieval distinctions.",
			],
			[
				"all-minilm-l12-v2",
				"all-MiniLM-L12-v2",
				67,
				58,
				77,
				"The deeper MiniLM variant has a modest quality allowance over L6 for general semantic matching. It remains a short-text encoder.",
			],
			[
				"paraphrase-minilm-l6-v2",
				"paraphrase-MiniLM-L6-v2",
				59,
				48,
				71,
				"A paraphrase and sentence-similarity specialist. Its score is conservative for query-to-document retrieval because symmetric similarity training serves a different task.",
			],
			[
				"multi-qa-mpnet-base-dot-v1",
				"multi-qa-mpnet-base-dot-v1",
				73,
				64,
				82,
				"Trained for question-and-answer retrieval, giving it a more direct fit for query-passage search. Preserve the documented dot-product scoring behavior.",
			],
		] as const
	).map(([id, model, score, low, high, reasoning]) =>
		embedding({
			name: model,
			ids: [`sentence-transformers/${id}`, `sentence-transformers/${model}`],
			creator: ["Sentence Transformers", "sentence-transformers"],
			score,
			range: [low, high],
			reasoning,
			sources: [`${HF}sentence-transformers/${model}`],
		}),
	),
];
