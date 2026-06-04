import * as vscode from "vscode";

export type FlowSymbolKind = "variable" | "function" | "event";

export interface FlowSymbol {
	readonly name: string;
	readonly kind: FlowSymbolKind;
	readonly detail: string;
	readonly selectionRange: vscode.Range;
	readonly fullRange: vscode.Range;
}

export interface FlowCall {
	readonly name: string;
	readonly range: vscode.Range;
	/** Brace depth at the call's `(`. */
	readonly depth: number;
}

/** A `const`/`let` declaration with its declared type and initializer node call. */
export interface FlowVariable {
	readonly name: string;
	readonly keyword: "const" | "let";
	/** Range of the declared name. */
	readonly range: vscode.Range;
	/** Explicit type annotation (`: Struct`), if present. */
	readonly typeText?: string;
	/** Callee of the initializer when the RHS is `node(...)` or `of node(...)`. */
	readonly initCall?: string;
	/** Raw initializer literal text when the RHS is not a node call. */
	readonly initLiteral?: string;
	/** Unescaped JSON Schema string from a preceding `@schema("…")` decorator. */
	readonly schemaText?: string;
}

export interface FlowDocumentModel {
	readonly symbols: FlowSymbol[];
	readonly calls: FlowCall[];
	/** Locally declared names: variables, functions, and event handlers. */
	readonly localNames: Set<string>;
	/** Every `const`/`let` declaration at any depth. */
	readonly variables: FlowVariable[];
}

interface Tok {
	readonly text: string;
	readonly line: number;
	readonly col: number;
	readonly offset: number;
}

const KEYWORDS = new Set([
	"const",
	"let",
	"function",
	"if",
	"else",
	"for",
	"of",
	"return",
	"true",
	"false",
	"null",
]);

/**
 * Scan a FlowScript document into a structural model used by the language
 * providers and linter. Strings and comments are skipped so identifiers inside
 * them never count as calls or declarations.
 */
export function analyzeFlowDocument(
	document: vscode.TextDocument,
): FlowDocumentModel {
	const text = document.getText();
	const idents = tokenizeIdentifiers(text);
	const symbols: FlowSymbol[] = [];
	const calls: FlowCall[] = [];
	const localNames = new Set<string>();
	const variables: FlowVariable[] = [];

	for (let i = 0; i < idents.length; i++) {
		const tok = idents[i];
		const next = nextNonWsChar(text, tok.offset + tok.text.length);
		const depth = braceDepthAt(text, tok.offset);

		if (tok.text === "const" || tok.text === "let") {
			const name = idents[i + 1];
			if (name) {
				const decl = parseVarDecl(document, name);
				variables.push({
					name: name.text,
					keyword: tok.text,
					range: tokRange(document, name),
					typeText: decl.typeText,
					initCall: decl.initCall,
					initLiteral: decl.initLiteral,
					schemaText: schemaDecoratorAbove(document, tok.line),
				});
				if (depth === 0) {
					symbols.push(
						makeSymbol(
							document,
							name,
							"variable",
							decl.typeText ?? tok.text,
							text,
							tok.offset,
						),
					);
					localNames.add(name.text);
				}
			}
			continue;
		}

		if (tok.text === "function") {
			const name = idents[i + 1];
			if (name) {
				symbols.push(
					makeSymbol(document, name, "function", "function", text, tok.offset),
				);
				localNames.add(name.text);
			}
			continue;
		}

		if (KEYWORDS.has(tok.text)) {
			continue;
		}

		// An identifier directly followed by `(`.
		if (next === "(") {
			const range = tokRange(document, tok);
			if (depth === 0) {
				// Top-level `name(...) {` is an event-handler block declaration.
				symbols.push(
					makeSymbol(document, tok, "event", "event", text, tok.offset),
				);
				localNames.add(tok.text);
			} else {
				calls.push({ name: tok.text, range, depth });
			}
		}
	}

	return { symbols, calls, localNames, variables };
}

/** Extract the declared type and initializer call of a `const`/`let` from its line. */
function parseVarDecl(
	document: vscode.TextDocument,
	nameTok: Tok,
): { typeText?: string; initCall?: string; initLiteral?: string } {
	const lineText = document.lineAt(nameTok.line).text;
	const rest = lineText.slice(nameTok.col + nameTok.text.length);
	let typeText: string | undefined;
	let initCall: string | undefined;
	let initLiteral: string | undefined;

	const eqIdx = rest.indexOf("=");
	const beforeEq = eqIdx === -1 ? rest : rest.slice(0, eqIdx);
	const colonMatch = /^\s*:\s*(.+?)\s*$/.exec(beforeEq);
	if (colonMatch) {
		typeText = colonMatch[1].trim();
	}

	if (eqIdx !== -1) {
		const rhs = rest.slice(eqIdx + 1).trim();
		const callMatch = /^([A-Za-z_$][\w$]*)\s*\(/.exec(rhs);
		if (callMatch) {
			initCall = callMatch[1];
		} else if (rhs.length > 0) {
			initLiteral = rhs;
		}
	} else {
		// Loop binding: `const handle of node(...)`.
		const ofMatch = /^\s*of\s+([A-Za-z_$][\w$]*)\s*\(/.exec(rest);
		if (ofMatch) {
			initCall = ofMatch[1];
		}
	}

	return { typeText, initCall, initLiteral };
}

/** Read the unescaped argument of a `@schema("…")` decorator immediately preceding the
 * declaration on line `declLine`. Scans upward across stacked decorator lines. Returns the
 * raw JSON Schema string (decorator quoting removed) or `undefined` when none is present. */
function schemaDecoratorAbove(
	document: vscode.TextDocument,
	declLine: number,
): string | undefined {
	for (let line = declLine - 1; line >= 0; line--) {
		const text = document.lineAt(line).text.trim();
		if (text.length === 0) {
			continue;
		}
		if (!text.startsWith("@")) {
			break;
		}
		const match = /^@schema\(\s*("(?:[^"\\]|\\.)*")\s*\)$/.exec(text);
		if (match) {
			try {
				return JSON.parse(match[1]) as string;
			} catch {
				return undefined;
			}
		}
	}
	return undefined;
}

/**
 * All ranges where `name` appears as a standalone identifier (strings and
 * comments excluded). Used by references, rename and document highlight.
 */
export function identifierOccurrences(
	document: vscode.TextDocument,
	name: string,
): vscode.Range[] {
	const out: vscode.Range[] = [];
	for (const tok of tokenizeIdentifiers(document.getText())) {
		if (tok.text === name) {
			out.push(tokRange(document, tok));
		}
	}
	return out;
}

function makeSymbol(
	document: vscode.TextDocument,
	nameTok: Tok,
	kind: FlowSymbolKind,
	detail: string,
	text: string,
	declStart: number,
): FlowSymbol {
	const selectionRange = tokRange(document, nameTok);
	const fullRange =
		kind === "variable"
			? lineRange(document, nameTok.line)
			: blockRange(document, text, declStart);
	return {
		name: nameTok.text,
		kind,
		detail,
		selectionRange,
		fullRange,
	};
}

function tokRange(document: vscode.TextDocument, tok: Tok): vscode.Range {
	const start = new vscode.Position(tok.line, tok.col);
	const end = new vscode.Position(tok.line, tok.col + tok.text.length);
	return new vscode.Range(start, end);
}

function lineRange(document: vscode.TextDocument, line: number): vscode.Range {
	return document.lineAt(line).range;
}

/** Range from a declaration start to the matching closing brace of its block. */
function blockRange(
	document: vscode.TextDocument,
	text: string,
	declStart: number,
): vscode.Range {
	const braceOpen = text.indexOf("{", declStart);
	if (braceOpen === -1) {
		return lineRange(document, document.positionAt(declStart).line);
	}
	let depth = 0;
	let inString = false;
	let inComment = false;
	for (let i = braceOpen; i < text.length; i++) {
		const ch = text[i];
		if (inComment) {
			if (ch === "\n") {
				inComment = false;
			}
			continue;
		}
		if (inString) {
			if (ch === "\\") {
				i++;
			} else if (ch === '"') {
				inString = false;
			}
			continue;
		}
		if (ch === '"') {
			inString = true;
		} else if (ch === "/" && text[i + 1] === "/") {
			inComment = true;
		} else if (ch === "{") {
			depth++;
		} else if (ch === "}") {
			depth--;
			if (depth === 0) {
				return new vscode.Range(
					document.positionAt(declStart),
					document.positionAt(i + 1),
				);
			}
		}
	}
	return new vscode.Range(
		document.positionAt(declStart),
		document.positionAt(text.length),
	);
}

/** Tokenize identifiers only, tracking line/col and skipping strings/comments. */
function tokenizeIdentifiers(text: string): Tok[] {
	const toks: Tok[] = [];
	let line = 0;
	let col = 0;
	let i = 0;
	const isIdentStart = (c: string) => /[A-Za-z_$]/.test(c);
	const isIdentPart = (c: string) => /[A-Za-z0-9_$]/.test(c);

	while (i < text.length) {
		const ch = text[i];
		if (ch === "\n") {
			line++;
			col = 0;
			i++;
			continue;
		}
		if (ch === '"') {
			i++;
			col++;
			while (i < text.length && text[i] !== '"') {
				if (text[i] === "\\") {
					i++;
					col++;
				}
				if (text[i] === "\n") {
					line++;
					col = 0;
				} else {
					col++;
				}
				i++;
			}
			i++;
			col++;
			continue;
		}
		if (ch === "/" && text[i + 1] === "/") {
			while (i < text.length && text[i] !== "\n") {
				i++;
			}
			continue;
		}
		if (isIdentStart(ch)) {
			const startCol = col;
			const startOffset = i;
			let name = "";
			while (i < text.length && isIdentPart(text[i])) {
				name += text[i];
				i++;
				col++;
			}
			toks.push({ text: name, line, col: startCol, offset: startOffset });
			continue;
		}
		i++;
		col++;
	}
	return toks;
}

function nextNonWsChar(text: string, from: number): string | undefined {
	for (let i = from; i < text.length; i++) {
		const ch = text[i];
		if (ch !== " " && ch !== "\t" && ch !== "\r" && ch !== "\n") {
			return ch;
		}
	}
	return undefined;
}

/** Brace nesting depth at the given offset (strings/comments ignored). */
function braceDepthAt(text: string, offset: number): number {
	let depth = 0;
	let inString = false;
	let inComment = false;
	for (let i = 0; i < offset; i++) {
		const ch = text[i];
		if (inComment) {
			if (ch === "\n") {
				inComment = false;
			}
			continue;
		}
		if (inString) {
			if (ch === "\\") {
				i++;
			} else if (ch === '"') {
				inString = false;
			}
			continue;
		}
		if (ch === '"') {
			inString = true;
		} else if (ch === "/" && text[i + 1] === "/") {
			inComment = true;
		} else if (ch === "{") {
			depth++;
		} else if (ch === "}") {
			depth--;
		}
	}
	return depth;
}
