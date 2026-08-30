import * as vscode from "vscode";

export type FlowSymbolKind = "variable" | "function" | "event" | "interface";

export interface FlowSymbol {
	readonly name: string;
	readonly kind: FlowSymbolKind;
	readonly detail: string;
	readonly selectionRange: vscode.Range;
	readonly fullRange: vscode.Range;
}

export type FlowCallKind = "bare" | "path" | "method";

export interface FlowCall {
	/** The member being called: the bare name, the last path segment or the method name. */
	readonly name: string;
	readonly kind: FlowCallKind;
	/** As written: `stringTrim`, `string::trim`, `s.trim`. */
	readonly display: string;
	/** Namespace segments before the member for `path` calls (`["ai", "ml"]`). */
	readonly path?: readonly string[];
	/** Receiver expression text for `method` calls (`s`, `a.b[0]`, `"lit"`). */
	readonly receiverText?: string;
	/** Range of the member name. */
	readonly range: vscode.Range;
	/** Range of the whole callee (`string::trim`). */
	readonly headRange: vscode.Range;
	/** Brace depth at the call's `(`. */
	readonly depth: number;
	/** Inside a template literal's `${ … }` expression. */
	readonly inTemplate: boolean;
}

/** A `const`/`let` declaration with its declared type and initializer node call. */
export interface FlowVariable {
	readonly name: string;
	readonly keyword: "const" | "let";
	/** Range of the declared name. */
	readonly range: vscode.Range;
	/** Explicit type annotation (`: Struct`), if present. */
	readonly typeText?: string;
	/** Callee of the initializer when the RHS is `node(...)`, `ns::node(...)` or `of node(...)`. */
	readonly initCall?: string;
	/** Method-call initializer (`s.trim()`): receiver text and member. */
	readonly initMethod?: { readonly receiverText: string; readonly member: string };
	/** Output/field selected from the initializer (`const { a: x } = call()` → `a`). */
	readonly initField?: string;
	/** Loop bindings (`for (const item of items)`): the iterated expression. */
	readonly iterates?: string;
	/** Raw initializer literal text when the RHS is not a node call. */
	readonly initLiteral?: string;
	/** Unescaped JSON Schema string from a preceding `@schema("…")` decorator. */
	readonly schemaText?: string;
}

export interface FlowInterfaceField {
	readonly name: string;
	readonly typeText: string;
	readonly optional: boolean;
	readonly range: vscode.Range;
}

export interface FlowInterface {
	readonly name: string;
	readonly range: vscode.Range;
	readonly fullRange: vscode.Range;
	readonly fields: FlowInterfaceField[];
}

export type UseDeclaration = {
	readonly path: readonly string[];
	readonly range: vscode.Range;
} & (
	| { readonly kind: "namespace" }
	| { readonly kind: "glob" }
	| { readonly kind: "members"; readonly members: readonly string[] }
	| { readonly kind: "alias"; readonly alias: string }
	| { readonly kind: "invalid"; readonly error: string }
);

export interface FlowDocumentModel {
	readonly symbols: FlowSymbol[];
	readonly calls: FlowCall[];
	/** Locally declared names: variables, functions, and event handlers. */
	readonly localNames: Set<string>;
	/** Every `const`/`let` declaration at any depth. */
	readonly variables: FlowVariable[];
	/** Top-level `interface Name { ... }` declarations. */
	readonly interfaces: Map<string, FlowInterface>;
	/** Top-level `use` declarations in document order. */
	readonly uses: UseDeclaration[];
	/** `function` declarations → method class of their first parameter (UFCS), `undefined` = any. */
	readonly functionReceivers: Map<string, string | undefined>;
}

interface Tok {
	readonly text: string;
	readonly line: number;
	readonly col: number;
	readonly offset: number;
	/** Inside a template literal's `${ … }` expression. */
	readonly inTemplate: boolean;
}

const KEYWORDS = new Set([
	"const",
	"let",
	"function",
	"interface",
	"if",
	"else",
	"for",
	"of",
	"in",
	"while",
	"break",
	"continue",
	"return",
	"true",
	"false",
	"null",
	"use",
	"as",
]);

const IDENT = "[A-Za-z_$][\\w$]*";
const IDENT_RE = new RegExp(`^${IDENT}$`);
const PATH_SPLIT_RE = /\s*::\s*/;

/**
 * Walk the code characters of a FlowScript document, skipping string contents, template
 * literal text and comments (the quotes themselves are visited so literals keep their shape).
 * `${ … }` bodies inside template literals are code. `visit` receives every code character
 * with its offset.
 */
export function scanCode(
	text: string,
	visit: (ch: string, offset: number, inTemplate: boolean) => void,
): void {
	type State =
		| { kind: "code"; template: boolean; depth: number }
		| { kind: "string"; quote: string }
		| { kind: "template" }
		| { kind: "comment" };
	const stack: State[] = [{ kind: "code", template: false, depth: 0 }];
	let templateDepth = 0;
	let i = 0;
	while (i < text.length) {
		const ch = text[i];
		const top = stack[stack.length - 1];
		const inTemplate = templateDepth > 0;
		switch (top.kind) {
			case "code":
				if (ch === '"' || ch === "'") {
					visit(ch, i, inTemplate);
					stack.push({ kind: "string", quote: ch });
				} else if (ch === "`") {
					visit(ch, i, inTemplate);
					stack.push({ kind: "template" });
				} else if (ch === "/" && text[i + 1] === "/") {
					stack.push({ kind: "comment" });
					i += 2;
					continue;
				} else if (top.template && ch === "{") {
					top.depth++;
					visit(ch, i, inTemplate);
				} else if (top.template && ch === "}") {
					if (top.depth === 0) {
						stack.pop();
						templateDepth--;
					} else {
						top.depth--;
						visit(ch, i, inTemplate);
					}
				} else {
					visit(ch, i, inTemplate);
				}
				break;
			case "string":
				if (ch === "\\") {
					i += 2;
					continue;
				}
				if (ch === top.quote || ch === "\n") {
					visit(ch, i, inTemplate);
					stack.pop();
				}
				break;
			case "template":
				if (ch === "\\") {
					i += 2;
					continue;
				}
				if (ch === "`") {
					visit(ch, i, inTemplate);
					stack.pop();
				} else if (ch === "$" && text[i + 1] === "{") {
					stack.push({ kind: "code", template: true, depth: 0 });
					templateDepth++;
					i += 2;
					continue;
				}
				break;
			case "comment":
				if (ch === "\n") {
					stack.pop();
				}
				break;
		}
		i++;
	}
}

/** Text with string contents, template text and comments blanked (offsets preserved). */
export function maskLiterals(text: string): string {
	const out = new Array<string>(text.length).fill(" ");
	for (let i = 0; i < text.length; i++) {
		if (text[i] === "\n") {
			out[i] = "\n";
		}
	}
	scanCode(text, (ch, offset) => {
		out[offset] = ch;
	});
	return out.join("");
}

/**
 * Scan a FlowScript document into a structural model used by the language
 * providers and linter. Strings and comments are skipped so identifiers inside
 * them never count as calls or declarations.
 */
export function analyzeFlowDocument(
	document: vscode.TextDocument,
): FlowDocumentModel {
	const text = document.getText();
	const masked = maskLiterals(text);
	const idents = tokenizeIdentifiers(text);
	const symbols: FlowSymbol[] = [];
	const calls: FlowCall[] = [];
	const localNames = new Set<string>();
	const variables: FlowVariable[] = [];
	const interfaces = new Map<string, FlowInterface>();
	const functionReceivers = new Map<string, string | undefined>();
	const uses = parseUseDeclarations(document, masked);
	const depthAt = braceDepthIndex(masked);

	for (let i = 0; i < idents.length; i++) {
		const tok = idents[i];
		const prevChar = previousNonWsChar(masked, tok.offset);
		// Decorator names such as `cache` are followed by `(` but are metadata, not top-level
		// event declarations or workflow calls. Keep them out of symbols/local names as well as
		// unknown-function diagnostics.
		if (prevChar === "@") {
			continue;
		}
		const next = nextNonWsChar(masked, tok.offset + tok.text.length);
		const depth = depthAt(tok.offset);

		if (tok.text === "const" || tok.text === "let") {
			const declared = parseBindings(document, masked, tok, idents, i);
			for (const decl of declared) {
				variables.push(decl);
				if (depth === 0) {
					symbols.push({
						name: decl.name,
						kind: "variable",
						detail: decl.typeText ?? tok.text,
						selectionRange: decl.range,
						fullRange: lineRange(document, decl.range.start.line),
					});
					localNames.add(decl.name);
				}
			}
			continue;
		}

		if (tok.text === "interface") {
			const name = idents[i + 1];
			if (name && depth === 0) {
				const fullRange = blockRange(document, masked, tok.offset);
				const range = tokRange(document, name);
				const iface: FlowInterface = {
					name: name.text,
					range,
					fullRange,
					fields: parseInterfaceFields(document, fullRange),
				};
				interfaces.set(name.text, iface);
				symbols.push({
					name: name.text,
					kind: "interface",
					detail: "interface",
					selectionRange: range,
					fullRange,
				});
				localNames.add(name.text);
			}
			continue;
		}

		if (tok.text === "function") {
			const name = idents[i + 1];
			if (name) {
				symbols.push(
					makeSymbol(document, name, "function", "function", masked, tok.offset),
				);
				localNames.add(name.text);
				functionReceivers.set(name.text, firstParamClass(masked, name));
			}
			continue;
		}

		if (KEYWORDS.has(tok.text)) {
			continue;
		}

		// A namespace segment (`string::`) is never a call or a declaration by itself.
		if (next === ":" && masked.slice(tok.offset + tok.text.length).trimStart().startsWith("::")) {
			continue;
		}

		// An identifier directly followed by `(`.
		if (next === "(") {
			const range = tokRange(document, tok);
			const head = callHead(document, masked, tok);
			if (depth === 0 && head.kind === "bare") {
				// Top-level `name(...) {` is an event-handler block declaration.
				symbols.push(
					makeSymbol(document, tok, "event", "event", masked, tok.offset),
				);
				localNames.add(tok.text);
			} else {
				calls.push({
					...head,
					name: tok.text,
					range,
					depth,
					inTemplate: tok.inTemplate,
				});
			}
		}
	}

	return { symbols, calls, localNames, variables, interfaces, uses, functionReceivers };
}

/** Classify the callee ending at `tok` (`a::b::tok`, `expr.tok` or bare). */
function callHead(
	document: vscode.TextDocument,
	masked: string,
	tok: Tok,
): Omit<FlowCall, "name" | "range" | "depth" | "inTemplate"> {
	const before = masked.slice(0, tok.offset).replace(/\s+$/, "");
	if (before.endsWith("::")) {
		const segments: string[] = [];
		let cursor = before;
		let headStart = tok.offset;
		while (cursor.endsWith("::")) {
			cursor = cursor.slice(0, -2).replace(/\s+$/, "");
			const m = new RegExp(`(${IDENT})$`).exec(cursor);
			if (!m) {
				break;
			}
			segments.unshift(m[1]);
			headStart = cursor.length - m[1].length;
			cursor = cursor.slice(0, headStart).replace(/\s+$/, "");
		}
		return {
			kind: "path",
			display: `${segments.join("::")}::${tok.text}`,
			path: segments,
			headRange: new vscode.Range(
				document.positionAt(headStart),
				document.positionAt(tok.offset + tok.text.length),
			),
		};
	}
	if (before.endsWith(".")) {
		const receiverText = trailingExpression(before.slice(0, -1));
		return {
			kind: "method",
			display: `${receiverText}.${tok.text}`,
			receiverText,
			headRange: new vscode.Range(
				document.positionAt(tok.offset),
				document.positionAt(tok.offset + tok.text.length),
			),
		};
	}
	return {
		kind: "bare",
		display: tok.text,
		headRange: new vscode.Range(
			document.positionAt(tok.offset),
			document.positionAt(tok.offset + tok.text.length),
		),
	};
}

/** The trailing primary expression of masked text (identifier / literal / call / member chain). */
export function trailingExpression(masked: string): string {
	let i = masked.length;
	let depth = 0;
	while (i > 0) {
		const c = masked[i - 1];
		if (c === ")" || c === "]" || c === "}") {
			depth++;
			i--;
		} else if (c === "(" || c === "[" || c === "{") {
			if (depth === 0) {
				break;
			}
			depth--;
			i--;
		} else if (depth > 0) {
			i--;
		} else if (c === '"' || c === "'" || c === "`") {
			const open = masked.lastIndexOf(c, i - 2);
			i = open < 0 ? i - 1 : open;
			break;
		} else if (/[\w$.:]/.test(c)) {
			i--;
		} else {
			break;
		}
	}
	return masked.slice(i);
}

/** Parse `const`/`let` bindings: plain, destructured (`{ a, b: c }`) and loop (`[i, item]`). */
function parseBindings(
	document: vscode.TextDocument,
	masked: string,
	keyword: Tok,
	idents: readonly Tok[],
	index: number,
): FlowVariable[] {
	const kw = keyword.text as "const" | "let";
	const after = masked.slice(keyword.offset + keyword.text.length);
	const leading = after.length - after.trimStart().length;
	const open = after[leading];
	const schemaText = schemaDecoratorAbove(document, keyword.line);

	if (open === "{" || open === "[") {
		const closeIdx = after.indexOf(open === "{" ? "}" : "]", leading);
		if (closeIdx < 0) {
			return [];
		}
		const body = after.slice(leading + 1, closeIdx);
		const rest = after.slice(closeIdx + 1);
		const bodyOffset = keyword.offset + keyword.text.length + leading + 1;
		const out: FlowVariable[] = [];
		const rhs = initializerOf(rest);
		if (open === "{") {
			for (const part of body.split(",")) {
				const m = new RegExp(`^(\\s*)(${IDENT})\\s*(?::\\s*(${IDENT}))?\\s*$`).exec(part);
				if (!m) {
					continue;
				}
				const local = m[3] ?? m[2];
				const field = m[2];
				const localOffset =
					bodyOffset + body.indexOf(part) + part.lastIndexOf(local);
				out.push({
					name: local,
					keyword: kw,
					range: new vscode.Range(
						document.positionAt(localOffset),
						document.positionAt(localOffset + local.length),
					),
					initCall: rhs.initCall,
					initMethod: rhs.initMethod,
					initField: field,
					schemaText,
				});
			}
		} else {
			const names = body.split(",").map((p) => p.trim()).filter((p) => IDENT_RE.test(p));
			const iterated = /^\s*of\s+([\s\S]*)$/.exec(rest);
			names.forEach((name, position) => {
				const localOffset = bodyOffset + body.indexOf(name);
				out.push({
					name,
					keyword: kw,
					range: new vscode.Range(
						document.positionAt(localOffset),
						document.positionAt(localOffset + name.length),
					),
					typeText: position === 0 && names.length === 2 ? "int" : undefined,
					iterates:
						position === 0 && names.length === 2
							? undefined
							: iterated
								? expressionText(iterated[1])
								: undefined,
					schemaText,
				});
			});
		}
		return out;
	}

	const name = idents[index + 1];
	if (!name) {
		return [];
	}
	const decl = parseVarDecl(masked, name);
	return [
		{
			name: name.text,
			keyword: kw,
			range: tokRange(document, name),
			typeText: decl.typeText,
			initCall: decl.initCall,
			initMethod: decl.initMethod,
			iterates: decl.iterates,
			initLiteral: decl.initLiteral,
			schemaText,
		},
	];
}

/** Callee of an initializer expression: `node(`, `ns::node(` or `expr.method(`. */
function initializerOf(
	rhsText: string,
): { initCall?: string; initMethod?: FlowVariable["initMethod"] } {
	const eq = rhsText.indexOf("=");
	if (eq < 0) {
		return {};
	}
	return calleeOf(expressionText(rhsText.slice(eq + 1)));
}

/** The expression up to the end of its statement (balanced brackets, first newline or `;`). */
function expressionText(text: string): string {
	let depth = 0;
	let i = 0;
	const trimmed = text.replace(/^\s+/, "");
	for (; i < trimmed.length; i++) {
		const c = trimmed[i];
		if (c === "(" || c === "[" || c === "{") {
			depth++;
		} else if (c === ")" || c === "]" || c === "}") {
			if (depth === 0) {
				break;
			}
			depth--;
		} else if (depth === 0 && (c === "\n" || c === ";")) {
			break;
		}
	}
	return trimmed.slice(0, i).trim();
}

/** Classify the head call of an expression (`node(...)`, `a::b(...)`, `recv.m(...)`). */
export function calleeOf(
	expr: string,
): { initCall?: string; initMethod?: FlowVariable["initMethod"] } {
	const pathCall = new RegExp(`^((?:${IDENT}\\s*::\\s*)*${IDENT})\\s*\\(`).exec(expr);
	if (pathCall) {
		return { initCall: pathCall[1].replace(/\s*::\s*/g, "::") };
	}
	const methodCall = new RegExp(`^([\\s\\S]*?)\\.\\s*(${IDENT})\\s*\\(`).exec(expr);
	if (methodCall) {
		const receiverText = trailingExpression(methodCall[1]);
		if (receiverText === methodCall[1].trim()) {
			return { initMethod: { receiverText, member: methodCall[2] } };
		}
	}
	return {};
}

function parseInterfaceFields(
	document: vscode.TextDocument,
	range: vscode.Range,
): FlowInterfaceField[] {
	const text = document.getText(range);
	const open = text.indexOf("{");
	const close = text.lastIndexOf("}");
	if (open === -1 || close === -1 || close <= open) {
		return [];
	}

	const baseOffset = document.offsetAt(range.start);
	const body = text.slice(open + 1, close);
	const fields: FlowInterfaceField[] = [];
	const fieldRe =
		/(^|[;\n,])\s*([A-Za-z_$][\w$]*)\s*(\?)?\s*:\s*([^=;,\n]+)(?:\s*=\s*(?:"(?:[^"\\]|\\.)*"|[^;,\n]+))?\s*(?=;|,|\n|$)/g;
	let match: RegExpExecArray | null;
	while ((match = fieldRe.exec(body))) {
		const name = match[2];
		const nameInMatch = match[0].indexOf(name);
		const nameOffset = baseOffset + open + 1 + match.index + nameInMatch;
		fields.push({
			name,
			typeText: match[4].trim(),
			optional: match[3] === "?",
			range: new vscode.Range(
				document.positionAt(nameOffset),
				document.positionAt(nameOffset + name.length),
			),
		});
	}
	return fields;
}

/** Extract the declared type and initializer of a `const`/`let` from the masked text after its name. */
function parseVarDecl(
	masked: string,
	nameTok: Tok,
): {
	typeText?: string;
	initCall?: string;
	initMethod?: FlowVariable["initMethod"];
	iterates?: string;
	initLiteral?: string;
} {
	const lineEnd = masked.indexOf("\n", nameTok.offset);
	const rest = masked.slice(
		nameTok.offset + nameTok.text.length,
		lineEnd < 0 ? masked.length : lineEnd,
	);
	let typeText: string | undefined;

	const eqIdx = rest.indexOf("=");
	const beforeEq = eqIdx === -1 ? rest : rest.slice(0, eqIdx);
	const colonMatch = /^\s*:\s*(.+?)\s*$/.exec(beforeEq);
	if (colonMatch) {
		typeText = colonMatch[1].trim();
	}

	if (eqIdx !== -1) {
		const rhs = expressionText(masked.slice(nameTok.offset + nameTok.text.length + eqIdx + 1));
		const callee = calleeOf(rhs);
		if (callee.initCall || callee.initMethod) {
			return { typeText, ...callee };
		}
		return { typeText, initLiteral: rhs.length > 0 ? rhs : undefined };
	}
	// Loop binding: `const item of items` / `const handle of node(...)`.
	const ofMatch = /^\s*of\s+([\s\S]*)$/.exec(rest);
	if (ofMatch) {
		const iterated = expressionText(ofMatch[1]);
		const callee = calleeOf(iterated);
		return { typeText, iterates: iterated, ...callee };
	}
	return { typeText };
}

/** Method class of a `function name(first: Type, …)` first parameter, from masked text. */
function firstParamClass(masked: string, nameTok: Tok): string | undefined {
	const after = masked.slice(nameTok.offset + nameTok.text.length);
	const m = new RegExp(`^\\s*\\(\\s*${IDENT}\\s*:\\s*([^,)]+)`).exec(after);
	return m ? classOfAnnotation(m[1].trim()) : undefined;
}

/** Method class of a FlowScript type annotation (`string`, `int[]`, `Mail`). */
export function classOfAnnotation(typeText: string): string | undefined {
	const text = typeText.trim();
	if (text.endsWith("[]")) {
		return "array";
	}
	if (/^Map\s*</.test(text)) {
		return "map";
	}
	if (/^Set\s*</.test(text)) {
		return "set";
	}
	switch (text) {
		case "string":
			return "string";
		case "int":
			return "int";
		case "float":
			return "float";
		case "bool":
			return "bool";
		case "Struct":
			return "struct";
		case "bytes":
		case "Byte":
			return "bytes";
		case "Path":
		case "PathBuf":
			return "path";
		case "Date":
			return "datetime";
		default:
			return /^[A-Z][\w$]*$/.test(text) ? text : undefined;
	}
}

const USE_TREE_RE = new RegExp(
	`^(${IDENT}(?:\\s*::\\s*${IDENT})*)(?:\\s*::\\s*(?:(\\*)|\\{([^}]*)\\}))?(?:\\s+as\\s+(${IDENT}))?$`,
);

/**
 * Parse every top-level `use` declaration (Rust use-tree subset): `use a::b`, `use a::b::*`,
 * `use a::{ x, y }`, `use a::b as x` and comma lists. Malformed trees come back as `invalid`.
 */
export function parseUseDeclarations(
	document: vscode.TextDocument,
	masked: string = maskLiterals(document.getText()),
): UseDeclaration[] {
	const out: UseDeclaration[] = [];
	const stmtRe = /(^|[\n;])[ \t]*use\b/g;
	let m: RegExpExecArray | null;
	while ((m = stmtRe.exec(masked)) !== null) {
		const useStart = m.index + m[0].length - 3;
		if (braceDepthAt(masked, useStart) !== 0) {
			continue;
		}
		const tailStart = useStart + 3;
		if (masked[tailStart] === "\n" || masked[tailStart] === ";") {
			continue;
		}
		let end = tailStart;
		let depth = 0;
		while (end < masked.length) {
			const c = masked[end];
			if (c === "{") {
				depth++;
			} else if (c === "}") {
				depth--;
			} else if (depth <= 0 && (c === "\n" || c === ";")) {
				break;
			}
			end++;
		}
		let pieceStart = tailStart;
		let pieceDepth = 0;
		for (let i = tailStart; i <= end; i++) {
			const c = masked[i];
			if (c === "{") {
				pieceDepth++;
			} else if (c === "}") {
				pieceDepth--;
			}
			if (i === end || (c === "," && pieceDepth === 0)) {
				const decl = parseUseTree(document, masked.slice(pieceStart, i), pieceStart);
				if (decl) {
					out.push(decl);
				}
				pieceStart = i + 1;
			}
		}
		stmtRe.lastIndex = Math.max(stmtRe.lastIndex, end);
	}
	return out;
}

function parseUseTree(
	document: vscode.TextDocument,
	raw: string,
	start: number,
): UseDeclaration | undefined {
	const leading = raw.length - raw.trimStart().length;
	const tree = raw.trim();
	if (!tree) {
		return undefined;
	}
	const range = new vscode.Range(
		document.positionAt(start + leading),
		document.positionAt(start + leading + tree.length),
	);
	const m = USE_TREE_RE.exec(tree);
	if (!m) {
		return {
			path: [],
			range,
			kind: "invalid",
			error: `Malformed use declaration '${tree}'. Expected \`use a::b\`, \`use a::b::*\`, \`use a::{ x, y }\` or \`use a::b as x\`.`,
		};
	}
	const path = m[1].split(PATH_SPLIT_RE);
	if (m[2]) {
		return m[4]
			? { path, range, kind: "invalid", error: "`as` cannot rename a glob import." }
			: { path, range, kind: "glob" };
	}
	if (m[3] !== undefined) {
		if (m[4]) {
			return { path, range, kind: "invalid", error: "`as` cannot rename a member list." };
		}
		const members = m[3]
			.split(",")
			.map((member) => member.trim())
			.filter(Boolean);
		if (members.length === 0 || members.some((member) => !IDENT_RE.test(member))) {
			return {
				path,
				range,
				kind: "invalid",
				error: "`use` member list must name at least one identifier.",
			};
		}
		return { path, range, kind: "members", members };
	}
	if (m[4]) {
		return { path, range, kind: "alias", alias: m[4] };
	}
	return { path, range, kind: "namespace" };
}

/** Expand the first segment of a path through `use a::b` / `use a as x` imports. */
export function expandUsePath(
	path: readonly string[],
	uses: readonly UseDeclaration[],
): string[] {
	if (path.length === 0) {
		return [];
	}
	for (const use of uses) {
		if (use.kind === "namespace" && use.path[use.path.length - 1] === path[0]) {
			return [...use.path, ...path.slice(1)];
		}
		if (use.kind === "alias" && use.alias === path[0]) {
			return [...use.path, ...path.slice(1)];
		}
	}
	return [...path];
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
	masked: string,
	declStart: number,
): FlowSymbol {
	const selectionRange = tokRange(document, nameTok);
	const fullRange =
		kind === "variable"
			? lineRange(document, nameTok.line)
			: blockRange(document, masked, declStart);
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

/** Range from a declaration start to the matching closing brace of its block (masked text). */
function blockRange(
	document: vscode.TextDocument,
	masked: string,
	declStart: number,
): vscode.Range {
	const braceOpen = masked.indexOf("{", declStart);
	if (braceOpen === -1) {
		return lineRange(document, document.positionAt(declStart).line);
	}
	let depth = 0;
	for (let i = braceOpen; i < masked.length; i++) {
		const ch = masked[i];
		if (ch === "{") {
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
		document.positionAt(masked.length),
	);
}

/** Tokenize identifiers only, tracking line/col and skipping strings/comments. */
function tokenizeIdentifiers(text: string): Tok[] {
	const lineStarts = [0];
	for (let i = 0; i < text.length; i++) {
		if (text.charCodeAt(i) === 10) {
			lineStarts.push(i + 1);
		}
	}
	const lineOf = (offset: number): number => {
		let lo = 0;
		let hi = lineStarts.length - 1;
		while (lo < hi) {
			const mid = (lo + hi + 1) >> 1;
			if (lineStarts[mid] <= offset) {
				lo = mid;
			} else {
				hi = mid - 1;
			}
		}
		return lo;
	};
	const toks: Tok[] = [];
	let current: { text: string; offset: number; inTemplate: boolean } | undefined;
	let lastOffset = -1;
	const flush = () => {
		if (current) {
			const line = lineOf(current.offset);
			toks.push({
				text: current.text,
				line,
				col: current.offset - lineStarts[line],
				offset: current.offset,
				inTemplate: current.inTemplate,
			});
			current = undefined;
		}
	};
	scanCode(text, (ch, offset, inTemplate) => {
		if (current && offset === lastOffset + 1 && /[A-Za-z0-9_$]/.test(ch)) {
			current.text += ch;
		} else {
			flush();
			if (/[A-Za-z_$]/.test(ch)) {
				current = { text: ch, offset, inTemplate };
			}
		}
		lastOffset = offset;
	});
	flush();
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

function previousNonWsChar(text: string, from: number): string | undefined {
	for (let i = from - 1; i >= 0; i--) {
		const ch = text[i];
		if (ch !== " " && ch !== "\t" && ch !== "\r" && ch !== "\n") {
			return ch;
		}
	}
	return undefined;
}

/** Brace nesting depth at the given offset of masked text. */
export function braceDepthAt(masked: string, offset: number): number {
	return braceDepthIndex(masked)(offset);
}

/** Precomputed brace depth lookup for masked text (depth before the character at `offset`). */
function braceDepthIndex(masked: string): (offset: number) => number {
	const depths = new Int32Array(masked.length + 1);
	let depth = 0;
	for (let i = 0; i < masked.length; i++) {
		depths[i] = depth;
		if (masked[i] === "{") {
			depth++;
		} else if (masked[i] === "}") {
			depth--;
		}
	}
	depths[masked.length] = depth;
	return (offset) => depths[Math.max(0, Math.min(offset, masked.length))];
}
