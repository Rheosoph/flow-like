import type { Monaco } from "@monaco-editor/react";
import type { INode } from "../../../lib/schema/flow/node";
import { IPinType, IVariableType } from "../../../lib/schema/flow/pin";

export const FLOWSCRIPT_LANGUAGE_ID = "flowscript";

const KEYWORDS = [
	"const",
	"let",
	"interface",
	"function",
	"if",
	"else",
	"for",
	"of",
	"return",
];

const TYPE_KEYWORDS = [
	"string",
	"int",
	"float",
	"bool",
	"void",
	"Date",
	"Generic",
	"Byte",
	"PathBuf",
	"Struct",
	"Map",
	"Set",
	"any",
];

const CONSTANTS = ["true", "false", "null"];

/** Mirrors `to_camel_case` in packages/ast/src/text.rs so completions match rendered node names. */
export function toFlowScriptIdentifier(input: string): string {
	let out = "";
	let upcomingUpper = false;
	let first = true;
	for (const ch of input) {
		if (/[a-zA-Z0-9]/.test(ch)) {
			if (first) {
				out += ch.toLowerCase();
				first = false;
			} else if (upcomingUpper) {
				out += ch.toUpperCase();
			} else {
				out += ch;
			}
			upcomingUpper = false;
		} else if (!first) {
			upcomingUpper = true;
		}
	}
	if (out.length === 0) return "node";
	return /^\d/.test(out) ? `_${out}` : out;
}

export function registerFlowScriptLanguage(monaco: Monaco): void {
	if (
		monaco.languages
			.getLanguages()
			.some((lang) => lang.id === FLOWSCRIPT_LANGUAGE_ID)
	) {
		return;
	}

	monaco.languages.register({ id: FLOWSCRIPT_LANGUAGE_ID });

	monaco.languages.setLanguageConfiguration(FLOWSCRIPT_LANGUAGE_ID, {
		comments: { lineComment: "//" },
		brackets: [
			["{", "}"],
			["[", "]"],
			["(", ")"],
		],
		autoClosingPairs: [
			{ open: "{", close: "}" },
			{ open: "[", close: "]" },
			{ open: "(", close: ")" },
			{ open: '"', close: '"', notIn: ["string"] },
		],
		surroundingPairs: [
			{ open: "{", close: "}" },
			{ open: "[", close: "]" },
			{ open: "(", close: ")" },
			{ open: '"', close: '"' },
		],
		folding: {
			markers: {
				start: /^\s*\/\/\s*#?region\b/,
				end: /^\s*\/\/\s*#?endregion\b/,
			},
		},
	});

	monaco.languages.setMonarchTokensProvider(FLOWSCRIPT_LANGUAGE_ID, {
		defaultToken: "",
		keywords: KEYWORDS,
		typeKeywords: TYPE_KEYWORDS,
		constants: CONSTANTS,
		operators: [
			"===",
			"!==",
			"==",
			"!=",
			">=",
			"<=",
			">",
			"<",
			"&&",
			"||",
			"!",
			"+",
			"-",
			"*",
			"/",
			"%",
			"=",
			"?",
		],
		symbols: /[=><!~?:&|+\-*/^%]+/,
		escapes: /\\(?:["\\/nrt]|u[0-9A-Fa-f]{4})/,
		tokenizer: {
			root: [
				// Anchor comments carry round-trip identity — highlight distinctly.
				[/\/\/@[nvl]:[^\n]*/, "comment.anchor"],
				[/\/\/.*$/, "comment"],
				[/@(secret|readonly|runtime|category|description|schema)\b/, "tag"],
				[
					/\b(interface|function)\b(\s+)([A-Za-z_$][\w$]*)/,
					["keyword", "", "type.identifier"],
				],
				[
					/\b(const|let)\b(\s+)([A-Za-z_$][\w$]*)/,
					["keyword", "", "variable.name"],
				],
				[
					/[a-zA-Z_$][\w$]*/,
					{
						cases: {
							"@keywords": "keyword",
							"@typeKeywords": "type",
							"@constants": "constant",
							"@default": "identifier",
						},
					},
				],
				[/[A-Z][\w$]*/, "type.identifier"],
				{ include: "@whitespace" },
				[/[{}()[\]]/, "@brackets"],
				[/-?\d+\.\d+([eE][+-]?\d+)?/, "number.float"],
				[/-?\d+/, "number"],
				[
					/@symbols/,
					{
						cases: {
							"@operators": "operator",
							"@default": "",
						},
					},
				],
				[/"([^"\\]|\\.)*$/, "string.invalid"],
				[/"/, { token: "string.quote", bracket: "@open", next: "@string" }],
				[/[;,.]/, "delimiter"],
			],
			string: [
				[/[^\\"]+/, "string"],
				[/@escapes/, "string.escape"],
				[/\\./, "string.escape.invalid"],
				[/"/, { token: "string.quote", bracket: "@close", next: "@pop" }],
			],
			whitespace: [[/[ \t\r\n]+/, ""]],
		},
	});
}

/**
 * Completion items for catalog node calls. Registered per-editor; dispose the
 * returned disposable on unmount to avoid stacking providers.
 */
export function registerFlowScriptCompletions(
	monaco: Monaco,
	getCatalogNodes: () => INode[] | undefined,
): { dispose: () => void } {
	return monaco.languages.registerCompletionItemProvider(
		FLOWSCRIPT_LANGUAGE_ID,
		{
			provideCompletionItems: (model, position) => {
				const word = model.getWordUntilPosition(position);
				const range = {
					startLineNumber: position.lineNumber,
					endLineNumber: position.lineNumber,
					startColumn: word.startColumn,
					endColumn: word.endColumn,
				};

				const nodes = getCatalogNodes() ?? [];
				const seen = new Set<string>();
				const suggestions = [];
				for (const node of nodes) {
					const identifier = toFlowScriptIdentifier(node.name);
					if (seen.has(identifier)) continue;
					seen.add(identifier);

					const args = Object.values(node.pins)
						.filter(
							(pin) =>
								pin.pin_type === IPinType.Input &&
								pin.data_type !== IVariableType.Execution,
						)
						.sort((a, b) => a.index - b.index)
						.map(
							(pin, argIndex) =>
								`${toFlowScriptIdentifier(pin.name)}: $${argIndex + 1}`,
						);

					suggestions.push({
						label: identifier,
						kind: monaco.languages.CompletionItemKind.Function,
						detail: node.friendly_name,
						documentation: node.description,
						insertText:
							args.length > 0
								? `${identifier}({ ${args.join(", ")} })`
								: `${identifier}()`,
						insertTextRules:
							monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
						range,
					});
				}
				return { suggestions };
			},
		},
	);
}
