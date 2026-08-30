/** Shared declaration fixtures: legacy flat lines plus the phase-5 namespaced form. */
export const DECLARATIONS = `// Legacy flat declarations (today's generator output).
/**
 * Trims whitespace
 * @param string — Input
 * @returns trimmed — Output
 */
declare function stringTrim({ string: string }): string;

/**
 * Hashes a string
 * @param input — Input
 * @returns hash — Digest
 */
declare function utilsHashMd5({ input: string }): string;

/** @impure */
declare function logInfo({ message: string }): void;

/** @impure */
declare function eventsSimple(): void;

/** Splits an array
 * @returns chunks — Batches
 * @returns chunkCount — Count
 */
declare function arrayChunk({ arrayIn: any[], size?: int }): { chunks: any[], chunkCount: int };

// Namespaced declarations (phase-5 generator output).
declare namespace string {
	/**
	 * Checks whether a string contains a substring
	 * @node string_contains @receiver string @alias stringContains
	 * @param substring — Needle
	 */
	function contains(this: string, { substring: string, ignoreCase?: bool }): bool;

	/** @node string_length @receiver string @alias stringLength */
	function length(this: string): int;
}

declare namespace int {
	/** @node int_abs @receiver integer @alias intAbs */
	function abs(this: int): int;
}

declare namespace http {
	/** @node http_fetch @alias httpFetch @impure */
	function fetch({ url: string }): HttpResponse;

	/** @node http_response_to_text @receiver response @alias httpResponseToText */
	function responseToText(this: HttpResponse): string;
}

declare namespace ai {
	namespace ml {
		/** @node ai_ml_model_read @alias aiMlModelRead */
		function read({ path: string }): Struct;
	}
}
`;

export const NAMES = JSON.stringify({
	string_trim: {
		qualified: "string::trim",
		namespace: "string",
		alias: "trim",
		flat: "stringTrim",
		receiver: "string",
		class: "string",
		category: "Utils/String",
	},
	utils_hash_md5: {
		qualified: "hash::md5",
		namespace: "hash",
		alias: "md5",
		flat: "utilsHashMd5",
		receiver: null,
		class: null,
		category: "Utils/Hash",
	},
	log_info: {
		qualified: "log::info",
		namespace: "log",
		alias: "info",
		flat: "logInfo",
		receiver: null,
		class: null,
		category: "Logging",
	},
	array_chunk: {
		qualified: "array::chunk",
		namespace: "array",
		alias: "chunk",
		flat: "arrayChunk",
		receiver: "array_in",
		class: "array",
		category: "Utils/Array",
	},
});

export const SCHEMAS = JSON.stringify({
	http_fetch: {
		outputs: {
			response: JSON.stringify({
				title: "HttpResponse",
				type: "object",
				properties: { status: { type: "integer" }, body: { type: "string" } },
			}),
		},
	},
});
