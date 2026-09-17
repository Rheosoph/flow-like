// Verbatim copies of packages/schema/llm/*.json, generated from the Rust model-provider types by
// apps/schema-gen. tests/llm.test.ts fails when they drift; copy the files again after regenerating.
import type { JsonSchema } from "./contract";

export const LLM_SCHEMAS: Record<
	"History" | "Response" | "ResponseChunk",
	JsonSchema
> = {
	History: {
		$schema: "https://json-schema.org/draft/2020-12/schema",
		title: "History",
		type: "object",
		properties: {
			model: {
				type: "string",
			},
			messages: {
				type: "array",
				items: {
					$ref: "#/$defs/HistoryMessage",
				},
			},
			preset: {
				type: ["string", "null"],
			},
			stream: {
				type: ["boolean", "null"],
			},
			stream_options: {
				anyOf: [
					{
						$ref: "#/$defs/StreamOptions",
					},
					{
						type: "null",
					},
				],
			},
			max_completion_tokens: {
				type: ["integer", "null"],
				format: "uint32",
				minimum: 0,
			},
			top_p: {
				type: ["number", "null"],
				format: "float",
			},
			temperature: {
				type: ["number", "null"],
				format: "float",
			},
			thinking: {
				anyOf: [
					{
						$ref: "#/$defs/HistoryThinking",
					},
					{
						type: "null",
					},
				],
			},
			seed: {
				type: ["integer", "null"],
				format: "uint32",
				minimum: 0,
			},
			presence_penalty: {
				type: ["number", "null"],
				format: "float",
			},
			frequency_penalty: {
				type: ["number", "null"],
				format: "float",
			},
			user: {
				type: ["string", "null"],
			},
			stop: {
				type: ["array", "null"],
				items: {
					type: "string",
				},
			},
			response_format: {
				anyOf: [
					{
						$ref: "#/$defs/ResponseFormat",
					},
					{
						type: "null",
					},
				],
			},
			n: {
				type: ["integer", "null"],
				format: "uint32",
				minimum: 0,
			},
			tools: {
				type: ["array", "null"],
				items: {
					$ref: "#/$defs/Tool",
				},
			},
			tool_choice: {
				anyOf: [
					{
						$ref: "#/$defs/ToolChoice",
					},
					{
						type: "null",
					},
				],
			},
			usage: {
				anyOf: [
					{
						$ref: "#/$defs/Usage",
					},
					{
						type: "null",
					},
				],
			},
		},
		required: ["model", "messages"],
		$defs: {
			HistoryMessage: {
				type: "object",
				properties: {
					role: {
						$ref: "#/$defs/Role",
					},
					content: {
						$ref: "#/$defs/MessageContent",
					},
					name: {
						type: ["string", "null"],
					},
					tool_calls: {
						type: ["array", "null"],
						items: {
							$ref: "#/$defs/ToolCall",
						},
					},
					tool_call_id: {
						type: ["string", "null"],
					},
					annotations: {
						type: ["array", "null"],
						items: {
							$ref: "#/$defs/Annotation",
						},
					},
				},
				required: ["role", "content"],
			},
			Role: {
				type: "string",
				enum: ["system", "user", "assistant", "function", "tool"],
			},
			MessageContent: {
				anyOf: [
					{
						type: "string",
					},
					{
						type: "array",
						items: {
							$ref: "#/$defs/Content",
						},
					},
				],
			},
			Content: {
				anyOf: [
					{
						type: "object",
						properties: {
							type: {
								$ref: "#/$defs/ContentType",
							},
							text: {
								type: "string",
							},
						},
						required: ["type", "text"],
					},
					{
						type: "object",
						properties: {
							type: {
								$ref: "#/$defs/ContentType",
							},
							image_url: {
								$ref: "#/$defs/ImageUrl",
							},
						},
						required: ["type", "image_url"],
					},
					{
						type: "object",
						properties: {
							type: {
								$ref: "#/$defs/ContentType",
							},
							audio_url: {
								type: "string",
							},
							media_type: {
								type: ["string", "null"],
							},
							additional_params: true,
						},
						required: ["type", "audio_url"],
					},
					{
						type: "object",
						properties: {
							type: {
								$ref: "#/$defs/ContentType",
							},
							video_url: {
								type: "string",
							},
							media_type: {
								type: ["string", "null"],
							},
							additional_params: true,
						},
						required: ["type", "video_url"],
					},
					{
						type: "object",
						properties: {
							type: {
								$ref: "#/$defs/ContentType",
							},
							document_url: {
								type: "string",
							},
							media_type: {
								type: ["string", "null"],
							},
							additional_params: true,
						},
						required: ["type", "document_url"],
					},
				],
			},
			ContentType: {
				type: "string",
				enum: ["text", "image_url", "audio_url", "video_url", "document_url"],
			},
			ImageUrl: {
				type: "object",
				properties: {
					url: {
						type: "string",
					},
					detail: {
						type: ["string", "null"],
					},
					media_type: {
						type: ["string", "null"],
					},
					additional_params: true,
				},
				required: ["url"],
			},
			ToolCall: {
				type: "object",
				properties: {
					id: {
						type: "string",
					},
					type: {
						type: "string",
					},
					function: {
						$ref: "#/$defs/ToolCallFunction",
					},
				},
				required: ["id", "type", "function"],
			},
			ToolCallFunction: {
				type: "object",
				properties: {
					name: {
						type: "string",
					},
					arguments: {
						type: "string",
					},
				},
				required: ["name", "arguments"],
			},
			Annotation: {
				type: "object",
				properties: {
					type: {
						type: "string",
					},
					url_citation: {
						anyOf: [
							{
								$ref: "#/$defs/UrlCitation",
							},
							{
								type: "null",
							},
						],
					},
				},
				required: ["type"],
			},
			UrlCitation: {
				type: "object",
				properties: {
					end_index: {
						type: "integer",
						format: "uint32",
						minimum: 0,
					},
					start_index: {
						type: "integer",
						format: "uint32",
						minimum: 0,
					},
					title: {
						type: "string",
					},
					url: {
						type: "string",
					},
					content: {
						type: ["string", "null"],
					},
				},
				required: ["end_index", "start_index", "title", "url"],
			},
			StreamOptions: {
				type: "object",
				properties: {
					include_usage: {
						type: "boolean",
					},
				},
				required: ["include_usage"],
			},
			HistoryThinking: {
				type: "string",
				enum: ["off", "low", "mid", "high"],
			},
			ResponseFormat: {
				anyOf: [
					{
						type: "string",
					},
					true,
				],
			},
			Tool: {
				type: "object",
				properties: {
					type: {
						$ref: "#/$defs/ToolType",
					},
					function: {
						$ref: "#/$defs/HistoryFunction",
					},
				},
				required: ["type", "function"],
			},
			ToolType: {
				type: "string",
				enum: ["function"],
			},
			HistoryFunction: {
				type: "object",
				properties: {
					name: {
						type: "string",
					},
					description: {
						type: ["string", "null"],
					},
					parameters: {
						$ref: "#/$defs/HistoryFunctionParameters",
					},
				},
				required: ["name", "parameters"],
			},
			HistoryFunctionParameters: {
				type: "object",
				properties: {
					type: {
						$ref: "#/$defs/HistoryJSONSchemaType",
					},
					properties: {
						type: ["object", "null"],
						additionalProperties: {
							$ref: "#/$defs/HistoryJSONSchemaDefine",
						},
					},
					required: {
						type: ["array", "null"],
						items: {
							type: "string",
						},
					},
				},
				required: ["type"],
			},
			HistoryJSONSchemaType: {
				type: "string",
				enum: ["object", "number", "string", "array", "null", "boolean"],
			},
			HistoryJSONSchemaDefine: {
				type: "object",
				properties: {
					type: {
						anyOf: [
							{
								$ref: "#/$defs/HistoryJSONSchemaType",
							},
							{
								type: "null",
							},
						],
					},
					description: {
						type: ["string", "null"],
					},
					default: true,
					enum: {
						type: ["array", "null"],
						items: {
							type: "string",
						},
					},
					properties: {
						type: ["object", "null"],
						additionalProperties: {
							$ref: "#/$defs/HistoryJSONSchemaDefine",
						},
					},
					required: {
						type: ["array", "null"],
						items: {
							type: "string",
						},
					},
					items: {
						anyOf: [
							{
								$ref: "#/$defs/HistoryJSONSchemaDefine",
							},
							{
								type: "null",
							},
						],
					},
				},
			},
			ToolChoice: {
				anyOf: [
					{
						type: "null",
					},
					{
						type: "null",
					},
					{
						type: "null",
					},
					{
						type: "object",
						properties: {
							type: {
								$ref: "#/$defs/ToolType",
							},
							function: {
								$ref: "#/$defs/HistoryFunction",
							},
						},
						required: ["type", "function"],
					},
				],
			},
			Usage: {
				type: "object",
				properties: {
					include: {
						type: "boolean",
					},
				},
				required: ["include"],
			},
		},
	},
	Response: {
		$schema: "https://json-schema.org/draft/2020-12/schema",
		title: "Response",
		type: "object",
		properties: {
			id: {
				type: ["string", "null"],
			},
			choices: {
				type: "array",
				items: {
					$ref: "#/$defs/Choice",
				},
			},
			created: {
				type: ["integer", "null"],
				format: "uint64",
				minimum: 0,
			},
			model: {
				type: ["string", "null"],
			},
			service_tier: {
				type: ["string", "null"],
			},
			system_fingerprint: {
				type: ["string", "null"],
			},
			object: {
				type: ["string", "null"],
			},
			usage: {
				$ref: "#/$defs/Usage",
			},
		},
		required: ["choices", "usage"],
		$defs: {
			Choice: {
				type: "object",
				properties: {
					index: {
						type: "integer",
						format: "int32",
					},
					finish_reason: {
						type: "string",
					},
					message: {
						$ref: "#/$defs/ResponseMessage",
					},
					logprobs: {
						anyOf: [
							{
								$ref: "#/$defs/LogProbs",
							},
							{
								type: "null",
							},
						],
					},
				},
				required: ["index", "finish_reason", "message"],
			},
			ResponseMessage: {
				type: "object",
				properties: {
					role: {
						type: "string",
					},
					content: {
						type: ["string", "null"],
					},
					content_parts: {
						description:
							"Ordered structured content returned by Rig when the assistant response contains media.\nText-only responses continue to use `content` alone for OpenAI compatibility.",
						type: "array",
						items: {
							$ref: "#/$defs/Content",
						},
					},
					refusal: {
						type: ["string", "null"],
					},
					annotations: {
						type: ["array", "null"],
						items: {
							$ref: "#/$defs/Annotation",
						},
					},
					audio: {
						anyOf: [
							{
								$ref: "#/$defs/Audio",
							},
							{
								type: "null",
							},
						],
					},
					reasoning: {
						type: ["string", "null"],
					},
					tool_calls: {
						type: "array",
						items: {
							$ref: "#/$defs/FunctionCall",
						},
					},
				},
				required: ["role", "tool_calls"],
			},
			Content: {
				anyOf: [
					{
						type: "object",
						properties: {
							type: {
								$ref: "#/$defs/ContentType",
							},
							text: {
								type: "string",
							},
						},
						required: ["type", "text"],
					},
					{
						type: "object",
						properties: {
							type: {
								$ref: "#/$defs/ContentType",
							},
							image_url: {
								$ref: "#/$defs/ImageUrl",
							},
						},
						required: ["type", "image_url"],
					},
					{
						type: "object",
						properties: {
							type: {
								$ref: "#/$defs/ContentType",
							},
							audio_url: {
								type: "string",
							},
							media_type: {
								type: ["string", "null"],
							},
							additional_params: true,
						},
						required: ["type", "audio_url"],
					},
					{
						type: "object",
						properties: {
							type: {
								$ref: "#/$defs/ContentType",
							},
							video_url: {
								type: "string",
							},
							media_type: {
								type: ["string", "null"],
							},
							additional_params: true,
						},
						required: ["type", "video_url"],
					},
					{
						type: "object",
						properties: {
							type: {
								$ref: "#/$defs/ContentType",
							},
							document_url: {
								type: "string",
							},
							media_type: {
								type: ["string", "null"],
							},
							additional_params: true,
						},
						required: ["type", "document_url"],
					},
				],
			},
			ContentType: {
				type: "string",
				enum: ["text", "image_url", "audio_url", "video_url", "document_url"],
			},
			ImageUrl: {
				type: "object",
				properties: {
					url: {
						type: "string",
					},
					detail: {
						type: ["string", "null"],
					},
					media_type: {
						type: ["string", "null"],
					},
					additional_params: true,
				},
				required: ["url"],
			},
			Annotation: {
				type: "object",
				properties: {
					type: {
						type: "string",
					},
					url_citation: {
						anyOf: [
							{
								$ref: "#/$defs/UrlCitation",
							},
							{
								type: "null",
							},
						],
					},
				},
				required: ["type"],
			},
			UrlCitation: {
				type: "object",
				properties: {
					end_index: {
						type: "integer",
						format: "uint32",
						minimum: 0,
					},
					start_index: {
						type: "integer",
						format: "uint32",
						minimum: 0,
					},
					title: {
						type: "string",
					},
					url: {
						type: "string",
					},
					content: {
						type: ["string", "null"],
					},
				},
				required: ["end_index", "start_index", "title", "url"],
			},
			Audio: {
				type: "object",
				properties: {
					data: {
						type: "string",
					},
					expires_at: {
						type: ["integer", "null"],
						format: "uint64",
						minimum: 0,
					},
					id: {
						type: "string",
					},
					transcript: {
						type: ["string", "null"],
					},
				},
				required: ["data", "id"],
			},
			FunctionCall: {
				type: "object",
				properties: {
					index: {
						type: ["integer", "null"],
						format: "int32",
					},
					id: {
						type: "string",
					},
					type: {
						type: ["string", "null"],
					},
					function: {
						$ref: "#/$defs/ResponseFunction",
					},
				},
				required: ["id", "function"],
			},
			ResponseFunction: {
				type: "object",
				properties: {
					name: {
						type: "string",
					},
					arguments: {
						type: "string",
					},
				},
				required: ["name", "arguments"],
			},
			LogProbs: {
				type: "object",
				properties: {
					content: {
						type: ["array", "null"],
						items: {
							$ref: "#/$defs/TokenLogProbs",
						},
					},
					refusal: {
						type: ["array", "null"],
						items: {
							$ref: "#/$defs/TokenLogProbs",
						},
					},
				},
			},
			TokenLogProbs: {
				type: "object",
				properties: {
					token: {
						type: "string",
					},
					logprob: {
						type: "number",
						format: "double",
					},
					bytes: {
						type: ["array", "null"],
						items: {
							type: "integer",
							format: "uint8",
							minimum: 0,
							maximum: 255,
						},
					},
					top_logprobs: {
						type: ["array", "null"],
						items: {
							$ref: "#/$defs/TopLogProbs",
						},
					},
				},
				required: ["token", "logprob"],
			},
			TopLogProbs: {
				type: "object",
				properties: {
					token: {
						type: "string",
					},
					logprob: {
						type: "number",
						format: "double",
					},
					bytes: {
						type: ["array", "null"],
						items: {
							type: "integer",
							format: "uint8",
							minimum: 0,
							maximum: 255,
						},
					},
				},
				required: ["token", "logprob"],
			},
			Usage: {
				type: "object",
				properties: {
					completion_tokens: {
						type: "integer",
						format: "uint32",
						minimum: 0,
					},
					prompt_tokens: {
						type: "integer",
						format: "uint32",
						minimum: 0,
					},
					total_tokens: {
						type: "integer",
						format: "uint32",
						minimum: 0,
					},
					cost: {
						type: ["number", "null"],
						format: "double",
					},
					prompt_tokens_details: {
						anyOf: [
							{
								$ref: "#/$defs/PromptTokenDetails",
							},
							{
								type: "null",
							},
						],
					},
					completion_tokens_details: {
						anyOf: [
							{
								$ref: "#/$defs/CompletionTokenDetails",
							},
							{
								type: "null",
							},
						],
					},
					upstream_inference_cost: {
						anyOf: [
							{
								$ref: "#/$defs/CostDetails",
							},
							{
								type: "null",
							},
						],
					},
				},
				required: ["completion_tokens", "prompt_tokens", "total_tokens"],
			},
			PromptTokenDetails: {
				type: "object",
				properties: {
					cached_tokens: {
						type: ["integer", "null"],
						format: "uint32",
						minimum: 0,
					},
					audio_tokens: {
						type: ["integer", "null"],
						format: "uint32",
						minimum: 0,
					},
				},
			},
			CompletionTokenDetails: {
				type: "object",
				properties: {
					accepted_prediction_tokens: {
						type: ["integer", "null"],
						format: "uint32",
						minimum: 0,
					},
					audio_tokens: {
						type: ["integer", "null"],
						format: "uint32",
						minimum: 0,
					},
					reasoning_tokens: {
						type: ["integer", "null"],
						format: "uint32",
						minimum: 0,
					},
					rejected_prediction_tokens: {
						type: ["integer", "null"],
						format: "uint32",
						minimum: 0,
					},
				},
			},
			CostDetails: {
				type: "object",
				properties: {
					upstream_inference_cost: {
						type: ["integer", "null"],
						format: "uint32",
						minimum: 0,
					},
				},
			},
		},
	},
	ResponseChunk: {
		$schema: "https://json-schema.org/draft/2020-12/schema",
		title: "ResponseChunk",
		type: "object",
		properties: {
			id: {
				type: "string",
			},
			choices: {
				type: "array",
				items: {
					$ref: "#/$defs/ResponseChunkChoice",
				},
			},
			created: {
				type: ["integer", "null"],
				format: "uint64",
				minimum: 0,
			},
			model: {
				type: ["string", "null"],
			},
			service_tier: {
				type: ["string", "null"],
			},
			system_fingerprint: {
				type: ["string", "null"],
			},
			usage: {
				anyOf: [
					{
						$ref: "#/$defs/Usage",
					},
					{
						type: "null",
					},
				],
			},
			x_prefill_progress: {
				type: ["number", "null"],
				format: "float",
			},
		},
		required: ["id", "choices"],
		$defs: {
			ResponseChunkChoice: {
				type: "object",
				properties: {
					index: {
						type: "integer",
						format: "int32",
					},
					delta: {
						anyOf: [
							{
								$ref: "#/$defs/Delta",
							},
							{
								type: "null",
							},
						],
					},
					finish_reason: {
						type: ["string", "null"],
					},
					logprobs: {
						anyOf: [
							{
								$ref: "#/$defs/LogProbs",
							},
							{
								type: "null",
							},
						],
					},
				},
				required: ["index"],
			},
			Delta: {
				type: "object",
				properties: {
					role: {
						type: ["string", "null"],
					},
					content: {
						type: ["string", "null"],
					},
					content_parts: {
						type: ["array", "null"],
						items: {
							$ref: "#/$defs/Content",
						},
					},
					tool_calls: {
						type: ["array", "null"],
						items: {
							$ref: "#/$defs/DeltaFunctionCall",
						},
					},
					refusal: {
						type: ["string", "null"],
					},
					reasoning: {
						type: ["string", "null"],
					},
				},
			},
			Content: {
				anyOf: [
					{
						type: "object",
						properties: {
							type: {
								$ref: "#/$defs/ContentType",
							},
							text: {
								type: "string",
							},
						},
						required: ["type", "text"],
					},
					{
						type: "object",
						properties: {
							type: {
								$ref: "#/$defs/ContentType",
							},
							image_url: {
								$ref: "#/$defs/ImageUrl",
							},
						},
						required: ["type", "image_url"],
					},
					{
						type: "object",
						properties: {
							type: {
								$ref: "#/$defs/ContentType",
							},
							audio_url: {
								type: "string",
							},
							media_type: {
								type: ["string", "null"],
							},
							additional_params: true,
						},
						required: ["type", "audio_url"],
					},
					{
						type: "object",
						properties: {
							type: {
								$ref: "#/$defs/ContentType",
							},
							video_url: {
								type: "string",
							},
							media_type: {
								type: ["string", "null"],
							},
							additional_params: true,
						},
						required: ["type", "video_url"],
					},
					{
						type: "object",
						properties: {
							type: {
								$ref: "#/$defs/ContentType",
							},
							document_url: {
								type: "string",
							},
							media_type: {
								type: ["string", "null"],
							},
							additional_params: true,
						},
						required: ["type", "document_url"],
					},
				],
			},
			ContentType: {
				type: "string",
				enum: ["text", "image_url", "audio_url", "video_url", "document_url"],
			},
			ImageUrl: {
				type: "object",
				properties: {
					url: {
						type: "string",
					},
					detail: {
						type: ["string", "null"],
					},
					media_type: {
						type: ["string", "null"],
					},
					additional_params: true,
				},
				required: ["url"],
			},
			DeltaFunctionCall: {
				type: "object",
				properties: {
					index: {
						type: ["integer", "null"],
						format: "int32",
					},
					id: {
						type: ["string", "null"],
					},
					type: {
						type: ["string", "null"],
					},
					function: {
						$ref: "#/$defs/DeltaResponseFunction",
					},
				},
				required: ["function"],
			},
			DeltaResponseFunction: {
				type: "object",
				properties: {
					name: {
						type: ["string", "null"],
					},
					arguments: {
						type: ["string", "null"],
					},
				},
			},
			LogProbs: {
				type: "object",
				properties: {
					content: {
						type: ["array", "null"],
						items: {
							$ref: "#/$defs/TokenLogProbs",
						},
					},
					refusal: {
						type: ["array", "null"],
						items: {
							$ref: "#/$defs/TokenLogProbs",
						},
					},
				},
			},
			TokenLogProbs: {
				type: "object",
				properties: {
					token: {
						type: "string",
					},
					logprob: {
						type: "number",
						format: "double",
					},
					bytes: {
						type: ["array", "null"],
						items: {
							type: "integer",
							format: "uint8",
							minimum: 0,
							maximum: 255,
						},
					},
					top_logprobs: {
						type: ["array", "null"],
						items: {
							$ref: "#/$defs/TopLogProbs",
						},
					},
				},
				required: ["token", "logprob"],
			},
			TopLogProbs: {
				type: "object",
				properties: {
					token: {
						type: "string",
					},
					logprob: {
						type: "number",
						format: "double",
					},
					bytes: {
						type: ["array", "null"],
						items: {
							type: "integer",
							format: "uint8",
							minimum: 0,
							maximum: 255,
						},
					},
				},
				required: ["token", "logprob"],
			},
			Usage: {
				type: "object",
				properties: {
					completion_tokens: {
						type: "integer",
						format: "uint32",
						minimum: 0,
					},
					prompt_tokens: {
						type: "integer",
						format: "uint32",
						minimum: 0,
					},
					total_tokens: {
						type: "integer",
						format: "uint32",
						minimum: 0,
					},
					cost: {
						type: ["number", "null"],
						format: "double",
					},
					prompt_tokens_details: {
						anyOf: [
							{
								$ref: "#/$defs/PromptTokenDetails",
							},
							{
								type: "null",
							},
						],
					},
					completion_tokens_details: {
						anyOf: [
							{
								$ref: "#/$defs/CompletionTokenDetails",
							},
							{
								type: "null",
							},
						],
					},
					upstream_inference_cost: {
						anyOf: [
							{
								$ref: "#/$defs/CostDetails",
							},
							{
								type: "null",
							},
						],
					},
				},
				required: ["completion_tokens", "prompt_tokens", "total_tokens"],
			},
			PromptTokenDetails: {
				type: "object",
				properties: {
					cached_tokens: {
						type: ["integer", "null"],
						format: "uint32",
						minimum: 0,
					},
					audio_tokens: {
						type: ["integer", "null"],
						format: "uint32",
						minimum: 0,
					},
				},
			},
			CompletionTokenDetails: {
				type: "object",
				properties: {
					accepted_prediction_tokens: {
						type: ["integer", "null"],
						format: "uint32",
						minimum: 0,
					},
					audio_tokens: {
						type: ["integer", "null"],
						format: "uint32",
						minimum: 0,
					},
					reasoning_tokens: {
						type: ["integer", "null"],
						format: "uint32",
						minimum: 0,
					},
					rejected_prediction_tokens: {
						type: ["integer", "null"],
						format: "uint32",
						minimum: 0,
					},
				},
			},
			CostDetails: {
				type: "object",
				properties: {
					upstream_inference_cost: {
						type: ["integer", "null"],
						format: "uint32",
						minimum: 0,
					},
				},
			},
		},
	},
};
