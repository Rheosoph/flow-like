import type { Monaco } from "@monaco-editor/react";
import type { INode } from "../../../lib/schema/flow/node";
import { IValueType, IVariableType } from "../../../lib/schema/flow/pin";
import {
	type CallArgs,
	type CallHead,
	type CallResolution,
	FLOWSCRIPT_LANGUAGE_ID,
	type FlowScriptArg,
	type FlowScriptIndex,
	KEYWORD_SET,
	RESERVED_WORDS,
	type Span,
	type TypeEnv,
	type UseDeclaration,
	type ValueType,
	analyzeCacheDecoratorContext,
	analyzeContext,
	buildCallSnippet,
	callHeadBefore,
	displayName,
	evaluateExpr,
	expandPath,
	getFlowScriptEnvDoc,
	getFlowScriptIndex,
	matchBracket,
	methodParams,
	namespaceKey,
	nodeHoverMarkdown,
	readCallArgs,
	receiverIsBound,
	renderSignature,
	resolveCallHead,
	skipWs,
	splitTopLevel,
	stripTrailingWord,
	toFlowScriptIdentifier,
	trimTrailingSpacesTabs,
} from "./flowscript-language";
import {
	type CancellationTokenLike,
	requestFlowScriptWorkerDocumentSymbols,
	requestFlowScriptWorkerEnvDoc,
	requestFlowScriptWorkerFolding,
	requestFlowScriptWorkerInlayHints,
	requestFlowScriptWorkerSemanticTokens,
} from "./flowscript-worker-client";

type GetCatalogNodes = () => INode[] | undefined;

const IDENT = "[A-Za-z_$][\\w$]*";
const IDENT_ONLY_RE = new RegExp(`^${IDENT}$`);
const PATH_ONLY_RE = new RegExp(`^${IDENT}(?:::${IDENT})*$`);
const PATH_CHAR_RE = /[\w$:]/;
const WS_RE = /\s/;

// ---------------------------------------------------------------------------
// Shared per-document analysis
// ---------------------------------------------------------------------------

export type FlowScriptBindingKind = "variable" | "local" | "param" | "loop";

export interface FlowScriptBinding {
	name: string;
	kind: FlowScriptBindingKind;
	/** Span of the declaring identifier itself. */
	nameSpan: Span;
	/** Offset from which references resolve to this binding (shadowing tie-breaker). */
	declStart: number;
	/** Region in which references resolve to this binding. */
	scope: Span;
	category?: string;
	/** Declared type annotation text, when present. */
	typeText?: string;
	/** Initializer expression (masked text), absent for params, loops and destructures. */
	rhs?: { text: string; start: number };
	destructured: boolean;
}

export type FlowScriptDeclarationKind =
	| "interface"
	| "variable"
	| "function"
	| "event"
	| "handler";

export interface FlowScriptDeclaration {
	kind: FlowScriptDeclarationKind;
	name: string;
	/** Leading node-type identifier of `eventsSimple onLoad() { … }` headers. */
	eventType?: string;
	category?: string;
	detail?: string;
	nameSpan: Span;
	span: Span;
	bodySpan?: Span;
	params: { name: string; typeText?: string; span: Span }[];
	children: FlowScriptDeclaration[];
}

export interface FlowScriptCallSite {
	head: CallHead;
	/** Span of the callee text (`ns::member`, `member` or the method name after `.`). */
	headSpan: Span;
	parenIndex: number;
	closeIndex: number;
	args: CallArgs | null;
	/** Span of the trailing named-args object, `{` to `}` inclusive, when present. */
	namedSpan?: Span;
	resolution: CallResolution;
	inTemplateExpr: boolean;
	/** The call sits inside another call's argument list (expression position). */
	enclosed: boolean;
}

export type FlowScriptOccurrenceKind =
	| "namespace"
	| "catalog"
	| "function"
	| "event"
	| "interface"
	| "variable"
	| "local"
	| "param"
	| "loop"
	| "member"
	| "argKey"
	| "unknown";

export interface FlowScriptOccurrence {
	name: string;
	span: Span;
	kind: FlowScriptOccurrenceKind;
	binding?: FlowScriptBinding;
	isDeclaration: boolean;
	isCall: boolean;
	/** A path head that is a `use`-introduced namespace alias. */
	aliasHead?: boolean;
}

interface BracketPair {
	open: number;
	close: number;
	ch: string;
}

/**
 * The document-environment subset of {@link FlowScriptAnalysis} that the
 * completion-family providers need. A full analysis satisfies it; the worker
 * client hydrates one from a serialized snapshot without occurrence/call data.
 */
export interface FlowScriptEnvContext {
	text: string;
	masked: string;
	lineStarts: number[];
	templateExprs: Span[];
	templates: Span[];
	index: FlowScriptIndex;
	env: TypeEnv;
	uses: UseDeclaration[];
}

export interface FlowScriptAnalysis {
	text: string;
	masked: string;
	lineStarts: number[];
	/** Full template literal spans (backtick to backtick). */
	templates: Span[];
	templateExprs: Span[];
	index: FlowScriptIndex;
	env: TypeEnv;
	uses: UseDeclaration[];
	/** The leading `use` block: start of the first use line to the end of the last tree. */
	useBlock: Span | null;
	/** Namespace alias name → span of the introducing segment in its `use` line. */
	useAliasDefs: Map<string, Span>;
	declarations: FlowScriptDeclaration[];
	bindings: FlowScriptBinding[];
	calls: FlowScriptCallSite[];
	occurrences: FlowScriptOccurrence[];
	brackets: BracketPair[];
}

const ANALYSIS_CACHE_LIMIT = 4;
const analysisCache = new Map<string, FlowScriptAnalysis>();

/**
 * One memoized structural pass over a FlowScript document: declarations, scope-aware
 * bindings, `use` lines, call sites with spans, and classified identifier occurrences.
 * Every feature provider below reads this instead of re-scanning the text itself.
 */
export function analyzeFlowScriptDocument(
	text: string,
	index: FlowScriptIndex,
): FlowScriptAnalysis {
	const cached = analysisCache.get(text);
	if (cached && cached.index === index) return cached;
	const analysis = computeAnalysis(text, index);
	if (analysisCache.size >= ANALYSIS_CACHE_LIMIT) {
		const oldest = analysisCache.keys().next().value;
		if (oldest !== undefined) analysisCache.delete(oldest);
	}
	analysisCache.set(text, analysis);
	return analysis;
}

function computeLineStarts(text: string): number[] {
	const starts = [0];
	for (let i = 0; i < text.length; i++) {
		if (text[i] === "\n") starts.push(i + 1);
	}
	return starts;
}

interface Pos {
	lineNumber: number;
	column: number;
}

interface RangeLike {
	startLineNumber: number;
	startColumn: number;
	endLineNumber: number;
	endColumn: number;
}

function positionOf(analysis: FlowScriptEnvContext, offset: number): Pos {
	const starts = analysis.lineStarts;
	let lo = 0;
	let hi = starts.length - 1;
	while (lo < hi) {
		const mid = (lo + hi + 1) >> 1;
		if (starts[mid] <= offset) lo = mid;
		else hi = mid - 1;
	}
	return { lineNumber: lo + 1, column: offset - starts[lo] + 1 };
}

function offsetOf(analysis: FlowScriptEnvContext, position: Pos): number {
	const starts = analysis.lineStarts;
	const line = Math.min(Math.max(position.lineNumber, 1), starts.length);
	return Math.min(
		starts[line - 1] + Math.max(position.column - 1, 0),
		analysis.text.length,
	);
}

function rangeOfSpan(analysis: FlowScriptEnvContext, span: Span): RangeLike {
	const start = positionOf(analysis, span.start);
	const end = positionOf(analysis, span.end);
	return {
		startLineNumber: start.lineNumber,
		startColumn: start.column,
		endLineNumber: end.lineNumber,
		endColumn: end.column,
	};
}

function scanBrackets(masked: string): {
	pairs: BracketPair[];
	braceDepth: Uint16Array;
	pairByOpen: Map<number, BracketPair>;
} {
	const pairs: BracketPair[] = [];
	const stack: { ch: string; open: number }[] = [];
	const braceDepth = new Uint16Array(masked.length + 1);
	let depth = 0;
	for (let i = 0; i < masked.length; i++) {
		braceDepth[i] = depth;
		const c = masked[i];
		if (c === "(" || c === "[" || c === "{") {
			stack.push({ ch: c, open: i });
			if (c === "{") depth++;
		} else if (c === ")" || c === "]" || c === "}") {
			if (c === "}" && depth > 0) depth--;
			const top = stack.pop();
			if (top) pairs.push({ open: top.open, close: i, ch: top.ch });
		}
	}
	braceDepth[masked.length] = depth;
	pairs.sort((a, b) => a.open - b.open);
	const pairByOpen = new Map<number, BracketPair>();
	for (const pair of pairs) pairByOpen.set(pair.open, pair);
	return { pairs, braceDepth, pairByOpen };
}

function enclosingBrace(
	pairs: BracketPair[],
	offset: number,
): BracketPair | undefined {
	let best: BracketPair | undefined;
	for (const pair of pairs) {
		if (pair.ch !== "{" || pair.open >= offset || pair.close < offset) continue;
		if (!best || pair.open > best.open) best = pair;
	}
	return best;
}

/** Depth-aware scan of an initializer: stops at a top-level newline or `;`. */
function scanRhsEnd(masked: string, from: number): number {
	let i = from;
	let depth = 0;
	while (i < masked.length) {
		const c = masked[i];
		if (c === "(" || c === "[" || c === "{") depth++;
		else if (c === ")" || c === "]" || c === "}") {
			if (depth === 0) break;
			depth--;
		} else if (depth === 0 && (c === "\n" || c === ";")) break;
		i++;
	}
	return i;
}

function prevNonWs(
	masked: string,
	from: number,
): { ch: string | undefined; index: number } {
	let i = from - 1;
	while (i >= 0 && WS_RE.test(masked[i])) i--;
	return { ch: i >= 0 ? masked[i] : undefined, index: i };
}

function identAt(masked: string, offset: number): Span | undefined {
	const m = new RegExp(`^${IDENT}`).exec(masked.slice(offset));
	if (!m) return undefined;
	return { start: offset, end: offset + m[0].length };
}

function parseParams(
	masked: string,
	parenOpen: number,
	parenClose: number,
): { name: string; typeText?: string; span: Span }[] {
	const params: { name: string; typeText?: string; span: Span }[] = [];
	const inner = masked.slice(parenOpen + 1, parenClose);
	if (!inner.trim()) return params;
	for (const piece of splitTopLevel(inner, parenOpen + 1)) {
		const m = new RegExp(
			`^(\\s*)(${IDENT})\\s*(?::\\s*([\\s\\S]+?))?\\s*$`,
		).exec(piece.text);
		if (!m) continue;
		const start = piece.start + m[1].length;
		params.push({
			name: m[2],
			typeText: m[3]?.trim(),
			span: { start, end: start + m[2].length },
		});
	}
	return params;
}

/** Parses `a, b: c` destructure members with the span of each declared local. */
function destructureLocals(
	masked: string,
	braceOpen: number,
	braceClose: number,
): { local: string; field: string; span: Span }[] {
	const out: { local: string; field: string; span: Span }[] = [];
	const inner = masked.slice(braceOpen + 1, braceClose);
	for (const piece of splitTopLevel(inner, braceOpen + 1)) {
		const m = new RegExp(`^\\s*(${IDENT})\\s*(?::\\s*(${IDENT}))?\\s*$`).exec(
			piece.text,
		);
		if (!m) continue;
		const local = m[2] ?? m[1];
		const at = m[2]
			? piece.start + piece.text.lastIndexOf(m[2])
			: piece.start + piece.text.indexOf(m[1]);
		out.push({
			local,
			field: m[1],
			span: { start: at, end: at + local.length },
		});
	}
	return out;
}

function categoryBefore(
	text: string,
	masked: string,
	declStart: number,
): { category?: string; decoratedStart: number } {
	let lineStart = masked.lastIndexOf("\n", declStart - 1) + 1;
	let category: string | undefined;
	let decoratedStart = lineStart;
	while (lineStart > 0) {
		const prevEnd = lineStart - 1;
		const prevStart = masked.lastIndexOf("\n", prevEnd - 1) + 1;
		const line = text.slice(prevStart, prevEnd);
		if (!/^\s*@/.test(line)) break;
		const m = /@category\(\s*"([^"]*)"\s*\)/.exec(line);
		if (m) category = m[1];
		decoratedStart = prevStart;
		if (prevStart === 0) break;
		lineStart = prevStart;
	}
	return { category, decoratedStart };
}

function computeAnalysis(
	text: string,
	index: FlowScriptIndex,
): FlowScriptAnalysis {
	const { masked, templateExprs, templates, env } = getFlowScriptEnvDoc(
		text,
		index,
	);
	const lineStarts = computeLineStarts(text);
	const { pairs, braceDepth, pairByOpen } = scanBrackets(masked);
	const uses = env.symbols.uses;

	let useBlock: Span | null = null;
	if (uses.length > 0) {
		const first = uses.reduce((a, b) => (a.start < b.start ? a : b));
		const last = uses.reduce((a, b) => (a.end > b.end ? a : b));
		useBlock = {
			start: masked.lastIndexOf("\n", first.start - 1) + 1,
			end: last.end,
		};
	}
	const useAliasDefs = new Map<string, Span>();
	for (const use of uses) {
		const raw = masked.slice(use.start, use.end);
		if (use.kind === "alias") {
			const m = new RegExp(`\\bas\\s+(${IDENT})\\s*$`).exec(raw);
			if (m) {
				const at = use.start + m.index + m[0].lastIndexOf(m[1]);
				useAliasDefs.set(m[1], { start: at, end: at + m[1].length });
			}
		} else if (use.kind === "namespace") {
			let lastIdent: RegExpExecArray | null = null;
			const identRe = new RegExp(IDENT, "g");
			for (let m = identRe.exec(raw); m; m = identRe.exec(raw)) lastIdent = m;
			if (lastIdent) {
				const at = use.start + lastIdent.index;
				useAliasDefs.set(lastIdent[0], {
					start: at,
					end: at + lastIdent[0].length,
				});
			}
		}
	}

	const bindings: FlowScriptBinding[] = [];
	const declarations: FlowScriptDeclaration[] = [];

	const scopeFor = (offset: number, nameStart: number): Span => {
		const brace = enclosingBrace(pairs, offset);
		return brace
			? { start: nameStart, end: brace.close }
			: { start: 0, end: masked.length };
	};

	// const / let declarations (top-level variables, locals, destructures, loop bindings).
	const declKeywordRe = /\b(const|let)\b/g;
	for (let m = declKeywordRe.exec(masked); m; m = declKeywordRe.exec(masked)) {
		const kwStart = m.index;
		const prev = prevNonWs(masked, kwStart);
		let isLoop = false;
		if (prev.ch === "(") {
			const word = prevNonWs(masked, prev.index);
			let q = word.index;
			while (q >= 0 && /[\w$]/.test(masked[q])) q--;
			if (masked.slice(q + 1, word.index + 1) === "for") isLoop = true;
		}
		const depth0 = braceDepth[kwStart] === 0;
		const i = skipWs(masked, kwStart + m[1].length);

		if (isLoop) {
			const forParen = pairByOpen.get(prev.index);
			const bodyOpen = forParen
				? skipWs(masked, forParen.close + 1)
				: undefined;
			const bodyClose =
				bodyOpen !== undefined && masked[bodyOpen] === "{"
					? matchBracket(masked, bodyOpen)
					: -1;
			const scopeEnd =
				bodyClose >= 0
					? bodyClose + 1
					: forParen
						? scanRhsEnd(masked, forParen.close + 1)
						: scanRhsEnd(masked, i);
			const loopScopeStart = forParen ? forParen.open : kwStart;
			const names: Span[] = [];
			if (masked[i] === "[") {
				const close = matchBracket(masked, i);
				if (close > 0) {
					for (const piece of splitTopLevel(
						masked.slice(i + 1, close),
						i + 1,
					)) {
						const ws = piece.text.length - piece.text.trimStart().length;
						const span = identAt(masked, piece.start + ws);
						if (span) names.push(span);
					}
				}
			} else {
				const span = identAt(masked, i);
				if (span) names.push(span);
			}
			let rhs: { text: string; start: number } | undefined;
			if (forParen) {
				const ofMatch = /\s(of|in)\s/.exec(
					masked.slice(kwStart, forParen.close),
				);
				if (ofMatch) {
					const rhsStart = skipWs(
						masked,
						kwStart + ofMatch.index + ofMatch[0].length,
					);
					rhs = {
						text: masked.slice(rhsStart, forParen.close).trim(),
						start: rhsStart,
					};
				}
			}
			for (const span of names) {
				bindings.push({
					name: masked.slice(span.start, span.end),
					kind: "loop",
					nameSpan: span,
					declStart: span.start,
					scope: { start: loopScopeStart, end: scopeEnd },
					rhs,
					destructured: false,
				});
			}
			continue;
		}

		if (masked[i] === "{") {
			const close = matchBracket(masked, i);
			if (close < 0) continue;
			const j = skipWs(masked, close + 1);
			let rhs: { text: string; start: number } | undefined;
			if (masked[j] === "=" && masked[j + 1] !== "=") {
				const rhsStart = skipWs(masked, j + 1);
				rhs = {
					text: masked.slice(rhsStart, scanRhsEnd(masked, rhsStart)).trim(),
					start: rhsStart,
				};
			}
			for (const member of destructureLocals(masked, i, close)) {
				bindings.push({
					name: member.local,
					kind: depth0 ? "variable" : "local",
					nameSpan: member.span,
					declStart: member.span.start,
					scope: depth0
						? { start: 0, end: masked.length }
						: scopeFor(kwStart, member.span.start),
					destructured: true,
				});
			}
			continue;
		}

		const nameSpan = identAt(masked, i);
		if (!nameSpan) continue;
		const name = masked.slice(nameSpan.start, nameSpan.end);
		if (RESERVED_WORDS.has(name)) continue;
		let j = skipWs(masked, nameSpan.end);
		let typeText: string | undefined;
		if (masked[j] === ":" && masked[j + 1] !== ":") {
			const annStart = j + 1;
			let k = annStart;
			let depth = 0;
			while (k < masked.length) {
				const c = masked[k];
				if (c === "(" || c === "[" || c === "{" || c === "<") depth++;
				else if (c === ")" || c === "]" || c === "}" || c === ">") depth--;
				else if (depth <= 0 && (c === "=" || c === "\n" || c === ";")) break;
				k++;
			}
			typeText = masked.slice(annStart, k).trim();
			j = k;
		}
		let rhs: { text: string; start: number } | undefined;
		let declEnd = nameSpan.end;
		if (masked[j] === "=" && masked[j + 1] !== "=") {
			const rhsStart = skipWs(masked, j + 1);
			const rhsEnd = scanRhsEnd(masked, rhsStart);
			rhs = { text: masked.slice(rhsStart, rhsEnd).trim(), start: rhsStart };
			declEnd = rhsEnd;
		}
		const binding: FlowScriptBinding = {
			name,
			kind: depth0 ? "variable" : "local",
			nameSpan,
			declStart: nameSpan.start,
			scope: depth0
				? { start: 0, end: masked.length }
				: scopeFor(kwStart, nameSpan.start),
			typeText,
			rhs,
			destructured: false,
		};
		if (depth0) {
			const { category, decoratedStart } = categoryBefore(
				text,
				masked,
				kwStart,
			);
			binding.category = category;
			declarations.push({
				kind: "variable",
				name,
				category,
				detail: typeText,
				nameSpan,
				span: { start: decoratedStart, end: declEnd },
				params: [],
				children: [],
			});
		}
		bindings.push(binding);
	}

	// interface / struct declarations.
	const ifaceRe = new RegExp(
		`(^|[\\n;])[ \\t]*(?:interface|struct)\\s+(${IDENT})`,
		"g",
	);
	for (let m = ifaceRe.exec(masked); m; m = ifaceRe.exec(masked)) {
		const declStart = m.index + m[1].length;
		if (braceDepth[declStart] !== 0) continue;
		const nameStart = m.index + m[0].length - m[2].length;
		const nameSpan = { start: nameStart, end: nameStart + m[2].length };
		let end = nameSpan.end;
		let bodySpan: Span | undefined;
		const bodyOpen = skipWs(masked, nameSpan.end);
		if (masked[bodyOpen] === "{") {
			const close = matchBracket(masked, bodyOpen);
			if (close > 0) {
				bodySpan = { start: bodyOpen, end: close + 1 };
				end = close + 1;
			}
		}
		declarations.push({
			kind: "interface",
			name: m[2],
			nameSpan,
			span: { start: declStart, end },
			bodySpan,
			params: [],
			children: [],
		});
	}

	// function / event keyword declarations, plus `<eventType> <name>(…) {` headers.
	const callables: FlowScriptDeclaration[] = [];
	const pushCallable = (decl: FlowScriptDeclaration) => {
		if (decl.bodySpan) {
			for (const param of decl.params) {
				bindings.push({
					name: param.name,
					kind: "param",
					nameSpan: param.span,
					declStart: param.span.start,
					scope: { start: param.span.start, end: decl.bodySpan.end },
					typeText: param.typeText,
					destructured: false,
				});
			}
		}
		callables.push(decl);
	};

	const fnRe = new RegExp(
		`(^|[\\n;])[ \\t]*(function|event)\\s+(${IDENT})\\s*\\(`,
		"g",
	);
	for (let m = fnRe.exec(masked); m; m = fnRe.exec(masked)) {
		const declStart = m.index + m[1].length;
		const parenOpen = m.index + m[0].length - 1;
		let nameEnd = parenOpen;
		while (nameEnd > 0 && WS_RE.test(masked[nameEnd - 1])) nameEnd--;
		const nameSpan = { start: nameEnd - m[3].length, end: nameEnd };
		const parenClose = matchBracket(masked, parenOpen);
		if (parenClose < 0) continue;
		const params = parseParams(masked, parenOpen, parenClose);
		let k = skipWs(masked, parenClose + 1);
		let returns = "";
		if (masked[k] === ":") {
			k = skipWs(masked, k + 1);
			if (masked[k] === "(") {
				const rClose = matchBracket(masked, k);
				if (rClose > 0) {
					returns = `: ${masked.slice(k, rClose + 1).trim()}`;
					k = skipWs(masked, rClose + 1);
				}
			}
		}
		let bodySpan: Span | undefined;
		let end = parenClose + 1;
		if (masked[k] === "{") {
			const close = matchBracket(masked, k);
			if (close > 0) {
				bodySpan = { start: k, end: close + 1 };
				end = close + 1;
			}
		}
		pushCallable({
			kind: m[2] === "function" ? "function" : "event",
			name: m[3],
			detail: `(${params
				.map((p) => (p.typeText ? `${p.name}: ${p.typeText}` : p.name))
				.join(", ")})${returns}`,
			nameSpan,
			span: { start: declStart, end },
			bodySpan,
			params,
			children: [],
		});
	}

	const eventRe = new RegExp(
		`(^|\\n)([\\t ]*)(${IDENT})(?:[\\t ]+(${IDENT}))?[\\t ]*\\(`,
		"g",
	);
	for (let m = eventRe.exec(masked); m; m = eventRe.exec(masked)) {
		if (KEYWORD_SET.has(m[3])) continue;
		if (m[4] && KEYWORD_SET.has(m[4])) continue;
		const declStart = m.index + m[1].length + m[2].length;
		const parenOpen = m.index + m[0].length - 1;
		const parenClose = matchBracket(masked, parenOpen);
		if (parenClose < 0) continue;
		const bodyOpen = skipWs(masked, parenClose + 1);
		if (masked[bodyOpen] !== "{") continue;
		const bodyClose = matchBracket(masked, bodyOpen);
		if (bodyClose < 0) continue;
		const name = m[4] ?? m[3];
		let nameEnd = parenOpen;
		while (nameEnd > 0 && WS_RE.test(masked[nameEnd - 1])) nameEnd--;
		const nameSpan = m[4]
			? { start: nameEnd - m[4].length, end: nameEnd }
			: { start: declStart, end: declStart + m[3].length };
		const params = parseParams(masked, parenOpen, parenClose);
		pushCallable({
			kind: braceDepth[declStart] === 0 ? "event" : "handler",
			name,
			eventType: m[4] ? m[3] : undefined,
			detail: `(${params
				.map((p) => (p.typeText ? `${p.name}: ${p.typeText}` : p.name))
				.join(", ")})`,
			nameSpan,
			span: { start: declStart, end: bodyClose + 1 },
			bodySpan: { start: bodyOpen, end: bodyClose + 1 },
			params,
			children: [],
		});
	}

	// Attach nested callables (handlers) to their enclosing declaration.
	callables.sort((a, b) => a.span.start - b.span.start);
	const roots: FlowScriptDeclaration[] = [];
	const nesting: FlowScriptDeclaration[] = [];
	for (const decl of callables) {
		while (nesting.length > 0) {
			const top = nesting[nesting.length - 1];
			if (top.bodySpan && decl.span.start < top.bodySpan.end) break;
			nesting.pop();
		}
		if (nesting.length > 0) nesting[nesting.length - 1].children.push(decl);
		else roots.push(decl);
		nesting.push(decl);
	}
	declarations.push(...roots);
	declarations.sort((a, b) => a.span.start - b.span.start);

	// Call sites.
	const calls: FlowScriptCallSite[] = [];
	const callRe = new RegExp(`((?:${IDENT}\\s*::\\s*)*${IDENT})\\s*\\(`, "g");
	for (let m = callRe.exec(masked); m; m = callRe.exec(masked)) {
		const headStart = m.index;
		const parenIndex = m.index + m[0].length - 1;
		const head = callHeadBefore(masked, parenIndex);
		if (!head) continue;
		const closeIndex = matchBracket(masked, parenIndex);
		if (closeIndex < 0) continue;
		const args = readCallArgs(masked, parenIndex);
		const resolution = resolveCallHead(
			head,
			env,
			args?.named?.map((arg) => arg.name) ?? [],
		);
		let namedSpan: Span | undefined;
		if (args?.named) {
			const pieces = splitTopLevel(
				masked.slice(parenIndex + 1, closeIndex),
				parenIndex + 1,
			);
			const last = pieces[pieces.length - 1];
			if (last) {
				const lead = last.text.length - last.text.trimStart().length;
				const open = last.start + lead;
				if (masked[open] === "{") {
					const close = matchBracket(masked, open);
					if (close > 0 && close < closeIndex)
						namedSpan = { start: open, end: close + 1 };
				}
			}
		}
		calls.push({
			head,
			headSpan: { start: headStart, end: headStart + m[1].length },
			parenIndex,
			closeIndex,
			args,
			namedSpan,
			resolution,
			inTemplateExpr: templateExprs.some(
				(span) => headStart >= span.start && headStart < span.end,
			),
			enclosed: false,
		});
	}
	const openCalls: number[] = [];
	for (const call of calls) {
		while (
			openCalls.length > 0 &&
			openCalls[openCalls.length - 1] < call.headSpan.start
		)
			openCalls.pop();
		call.enclosed = openCalls.length > 0;
		openCalls.push(call.closeIndex);
	}

	// Classified identifier occurrences.
	const bindingByStart = new Map<number, FlowScriptBinding>();
	const bindingsByName = new Map<string, FlowScriptBinding[]>();
	for (const binding of bindings) {
		bindingByStart.set(binding.nameSpan.start, binding);
		const list = bindingsByName.get(binding.name) ?? [];
		list.push(binding);
		bindingsByName.set(binding.name, list);
	}
	const declByStart = new Map<number, FlowScriptDeclaration>();
	const stampDecl = (decl: FlowScriptDeclaration) => {
		declByStart.set(decl.nameSpan.start, decl);
		for (const child of decl.children) stampDecl(child);
	};
	for (const decl of declarations) {
		if (decl.kind !== "variable") stampDecl(decl);
	}

	const occurrences: FlowScriptOccurrence[] = [];
	const identRe = new RegExp(IDENT, "g");
	for (let m = identRe.exec(masked); m; m = identRe.exec(masked)) {
		const start = m.index;
		if (start > 0 && /[\w$]/.test(masked[start - 1])) continue;
		const w = m[0];
		const end = start + w.length;
		const span = { start, end };
		if (start > 0 && masked[start - 1] === "@") continue;

		const useDecl = uses.find((u) => start >= u.start && end <= u.end);
		if (useDecl) {
			if (w === "use" || w === "as") continue;
			const braceIdx = masked.indexOf("{", useDecl.start);
			if (
				useDecl.kind === "members" &&
				braceIdx >= 0 &&
				start > braceIdx &&
				end < useDecl.end
			) {
				occurrences.push({
					name: w,
					span,
					kind: "catalog",
					isDeclaration: false,
					isCall: false,
				});
			} else {
				occurrences.push({
					name: w,
					span,
					kind: "namespace",
					isDeclaration: useAliasDefs.get(w)?.start === start,
					isCall: false,
					aliasHead: useAliasDefs.get(w)?.start === start,
				});
			}
			continue;
		}

		const binding = bindingByStart.get(start);
		if (binding) {
			occurrences.push({
				name: w,
				span,
				kind: binding.kind,
				binding,
				isDeclaration: true,
				isCall: false,
			});
			continue;
		}
		const decl = declByStart.get(start);
		if (decl) {
			occurrences.push({
				name: w,
				span,
				kind:
					decl.kind === "function"
						? "function"
						: decl.kind === "interface"
							? "interface"
							: "event",
				isDeclaration: true,
				isCall: false,
			});
			continue;
		}

		const prev = prevNonWs(masked, start);
		const prevPath = prev.ch === ":" && masked[prev.index - 1] === ":";
		const nextIdx = skipWs(masked, end);
		const nextCh = masked[nextIdx];
		const nextPath = nextCh === ":" && masked[nextIdx + 1] === ":";
		const nextCall = nextCh === "(";
		const nextKey = nextCh === ":" && masked[nextIdx + 1] !== ":";

		if (nextPath) {
			occurrences.push({
				name: w,
				span,
				kind: "namespace",
				isDeclaration: false,
				isCall: false,
				aliasHead: !prevPath && env.scope.namespaceAliases.has(w),
			});
			continue;
		}
		if (prevPath) {
			occurrences.push({
				name: w,
				span,
				kind: nextCall ? "catalog" : "namespace",
				isDeclaration: false,
				isCall: nextCall,
			});
			continue;
		}
		if (RESERVED_WORDS.has(w)) continue;
		if (prev.ch === ".") {
			occurrences.push({
				name: w,
				span,
				kind: nextCall
					? env.symbols.functions.has(w)
						? "function"
						: "catalog"
					: "member",
				isDeclaration: false,
				isCall: nextCall,
			});
			continue;
		}
		if (
			nextKey &&
			(prev.ch === "{" ||
				prev.ch === "," ||
				prev.ch === "(" ||
				prev.ch === "}" ||
				prev.ch === ";" ||
				prev.ch === undefined)
		) {
			occurrences.push({
				name: w,
				span,
				kind: "argKey",
				isDeclaration: false,
				isCall: false,
			});
			continue;
		}
		const resolved = resolveBindingAt(bindingsByName, w, start);
		if (resolved) {
			occurrences.push({
				name: w,
				span,
				kind: resolved.kind,
				binding: resolved,
				isDeclaration: false,
				isCall: nextCall,
			});
			continue;
		}
		if (env.symbols.functions.has(w)) {
			occurrences.push({
				name: w,
				span,
				kind: "function",
				isDeclaration: false,
				isCall: nextCall,
			});
			continue;
		}
		if (env.symbols.interfaces.has(w)) {
			occurrences.push({
				name: w,
				span,
				kind: "interface",
				isDeclaration: false,
				isCall: false,
			});
			continue;
		}
		if (nextCall) {
			occurrences.push({
				name: w,
				span,
				kind:
					index.byName.has(w) || env.scope.openMembers.has(w)
						? "catalog"
						: "unknown",
				isDeclaration: false,
				isCall: true,
			});
			continue;
		}
		if (
			env.scope.namespaceAliases.has(w) ||
			index.namespaces.has(namespaceKey(expandPath([w], env.scope)))
		) {
			occurrences.push({
				name: w,
				span,
				kind: "namespace",
				isDeclaration: false,
				isCall: false,
				aliasHead: env.scope.namespaceAliases.has(w),
			});
			continue;
		}
		occurrences.push({
			name: w,
			span,
			kind: "unknown",
			isDeclaration: false,
			isCall: false,
		});
	}

	return {
		text,
		masked,
		lineStarts,
		templates,
		templateExprs,
		index,
		env,
		uses,
		useBlock,
		useAliasDefs,
		declarations,
		bindings,
		calls,
		occurrences,
		brackets: pairs,
	};
}

function resolveBindingAt(
	bindingsByName: Map<string, FlowScriptBinding[]>,
	name: string,
	offset: number,
): FlowScriptBinding | undefined {
	const list = bindingsByName.get(name);
	if (!list) return undefined;
	let best: FlowScriptBinding | undefined;
	let bestStart = -1;
	let variable: FlowScriptBinding | undefined;
	for (const binding of list) {
		if (binding.kind === "variable") {
			variable ??= binding;
			continue;
		}
		if (offset < binding.scope.start || offset >= binding.scope.end) continue;
		if (binding.declStart <= offset && binding.declStart > bestStart) {
			best = binding;
			bestStart = binding.declStart;
		}
	}
	return best ?? variable;
}

function bindingsIndex(
	analysis: FlowScriptAnalysis,
): Map<string, FlowScriptBinding[]> {
	const map = new Map<string, FlowScriptBinding[]>();
	for (const binding of analysis.bindings) {
		const list = map.get(binding.name) ?? [];
		list.push(binding);
		map.set(binding.name, list);
	}
	return map;
}

function occurrenceAt(
	analysis: FlowScriptAnalysis,
	offset: number,
): FlowScriptOccurrence | undefined {
	return analysis.occurrences.find(
		(occ) => offset >= occ.span.start && offset <= occ.span.end,
	);
}

function referencesFor(
	analysis: FlowScriptAnalysis,
	occ: FlowScriptOccurrence,
): FlowScriptOccurrence[] {
	switch (occ.kind) {
		case "variable":
		case "local":
		case "param":
		case "loop":
			return analysis.occurrences.filter((o) => o.binding === occ.binding);
		case "function":
		case "event":
			return analysis.occurrences.filter(
				(o) =>
					o.name === occ.name &&
					(o.kind === "function" || (o.kind === "event" && o.isDeclaration)),
			);
		case "interface":
			return analysis.occurrences.filter(
				(o) => o.kind === "interface" && o.name === occ.name,
			);
		case "namespace":
			if (!occ.aliasHead) return [];
			return analysis.occurrences.filter(
				(o) => o.kind === "namespace" && o.aliasHead && o.name === occ.name,
			);
		default:
			return [];
	}
}

function findDeclaration(
	declarations: FlowScriptDeclaration[],
	name: string,
	kinds: FlowScriptDeclarationKind[],
): FlowScriptDeclaration | undefined {
	for (const decl of declarations) {
		if (kinds.includes(decl.kind) && decl.name === name) return decl;
		const nested = findDeclaration(decl.children, name, kinds);
		if (nested) return nested;
	}
	return undefined;
}

function definitionSpan(
	analysis: FlowScriptAnalysis,
	occ: FlowScriptOccurrence,
): Span | undefined {
	switch (occ.kind) {
		case "variable":
		case "local":
		case "param":
		case "loop":
			return occ.binding?.nameSpan;
		case "function":
		case "event": {
			const decl = findDeclaration(analysis.declarations, occ.name, [
				"function",
				"event",
				"handler",
			]);
			if (decl) return decl.nameSpan;
			const declared = referencesFor(analysis, occ).find(
				(o) => o.isDeclaration,
			);
			return declared?.span;
		}
		case "interface":
			return findDeclaration(analysis.declarations, occ.name, ["interface"])
				?.nameSpan;
		case "namespace":
			return occ.aliasHead ? analysis.useAliasDefs.get(occ.name) : undefined;
		default:
			return undefined;
	}
}

// ---------------------------------------------------------------------------
// Shared plumbing for the providers
// ---------------------------------------------------------------------------

interface ModelLike {
	uri: unknown;
	getValue: () => string;
	getVersionId?: () => number;
	getWordUntilPosition?: (position: Pos) => {
		word: string;
		startColumn: number;
		endColumn: number;
	};
}

type AnalysisFor = (model: ModelLike) => FlowScriptAnalysis;

interface TextEditLike {
	range: RangeLike;
	text: string;
}

function workspaceEdit(model: ModelLike, edits: TextEditLike[]) {
	return {
		edits: edits.map((edit) => ({
			resource: model.uri,
			textEdit: edit,
			versionId: model.getVersionId?.(),
		})),
	};
}

function lineStartOfOffset(
	analysis: FlowScriptEnvContext,
	offset: number,
): number {
	return analysis.text.lastIndexOf("\n", offset - 1) + 1;
}

function lineEndOfOffset(
	analysis: FlowScriptEnvContext,
	offset: number,
): number {
	const idx = analysis.text.indexOf("\n", offset);
	return idx < 0 ? analysis.text.length : idx;
}

/**
 * Edit that makes `nsKey::alias` callable bare: extends an existing `use ns::{ … }` member
 * list, or inserts a `use ns::*` line alphabetically into the leading use block.
 */
function useImportEdit(
	analysis: FlowScriptEnvContext,
	nsKey: string,
	alias?: string,
): TextEditLike | undefined {
	for (const use of analysis.uses) {
		if (use.kind !== "glob") continue;
		if (namespaceKey(expandPath(use.path, analysis.env.scope)) === nsKey)
			return undefined;
	}
	if (alias) {
		for (const use of analysis.uses) {
			if (use.kind !== "members") continue;
			if (namespaceKey(expandPath(use.path, analysis.env.scope)) !== nsKey)
				continue;
			if (use.members.includes(alias)) return undefined;
			const members = [...use.members, alias].sort();
			return {
				range: rangeOfSpan(analysis, { start: use.start, end: use.end }),
				text: `${use.path.join("::")}::{ ${members.join(", ")} }`,
			};
		}
	}
	const line = `use ${nsKey}::*`;
	if (analysis.uses.length === 0) {
		const needsBlank =
			analysis.text.length > 0 && !analysis.text.startsWith("\n");
		return {
			range: rangeOfSpan(analysis, { start: 0, end: 0 }),
			text: `${line}\n${needsBlank ? "\n" : ""}`,
		};
	}
	for (const use of analysis.uses) {
		if (use.path.join("::") > nsKey) {
			const at = lineStartOfOffset(analysis, use.start);
			return {
				range: rangeOfSpan(analysis, { start: at, end: at }),
				text: `${line}\n`,
			};
		}
	}
	const last = analysis.uses[analysis.uses.length - 1];
	const at = lineEndOfOffset(analysis, last.end);
	return {
		range: rangeOfSpan(analysis, { start: at, end: at }),
		text: `\n${line}`,
	};
}

// ---------------------------------------------------------------------------
// 1. Code actions / quick fixes
// ---------------------------------------------------------------------------

interface MarkerLike extends RangeLike {
	message: string;
}

function backtickSpans(message: string): string[] {
	return [...message.matchAll(/`([^`]+)`/g)].map((m) => m[1]);
}

/** Leading identifier-path of a backticked suggestion (`string::trim(…)` → `string::trim`). */
function suggestionPath(raw: string): string | undefined {
	const m = new RegExp(`^(${IDENT}(?:\\s*::\\s*${IDENT})*)`).exec(raw.trim());
	if (!m) return undefined;
	const rest = raw.trim().slice(m[1].length).trimStart();
	if (rest !== "" && !rest.startsWith("(")) return undefined;
	const path = m[1].replace(/\s*::\s*/g, "::");
	return PATH_ONLY_RE.test(path) ? path : undefined;
}

function pathTokenAt(
	text: string,
	from: number,
	to: number,
): { text: string; span: Span } | undefined {
	let s = from;
	if (!(s < text.length && PATH_CHAR_RE.test(text[s]))) {
		while (s < to && s < text.length && !PATH_CHAR_RE.test(text[s])) s++;
		if (s >= to || s >= text.length) return undefined;
	}
	let a = s;
	while (a > 0 && PATH_CHAR_RE.test(text[a - 1])) a--;
	let b = s;
	while (b < text.length && PATH_CHAR_RE.test(text[b])) b++;
	const token = text.slice(a, b);
	if (!PATH_ONLY_RE.test(token)) return undefined;
	return { text: token, span: { start: a, end: b } };
}

function precededByDot(text: string, offset: number): boolean {
	let i = offset - 1;
	while (i >= 0 && WS_RE.test(text[i])) i--;
	return text[i] === ".";
}

function callAt(
	analysis: FlowScriptAnalysis,
	offset: number,
): FlowScriptCallSite | undefined {
	let best: FlowScriptCallSite | undefined;
	for (const call of analysis.calls) {
		if (offset < call.headSpan.start || offset > call.closeIndex) continue;
		if (!best || call.headSpan.start > best.headSpan.start) best = call;
	}
	return best;
}

function inputPlaceholder(arg: FlowScriptArg | undefined): string {
	if (!arg) return "null";
	if (arg.enumValues && arg.enumValues.length > 0)
		return `"${arg.enumValues[0]}"`;
	if (
		arg.container === IValueType.Array ||
		arg.container === IValueType.HashSet
	)
		return "[]";
	if (arg.container === IValueType.HashMap) return "{}";
	switch (arg.dataType) {
		case IVariableType.String:
		case IVariableType.Date:
		case IVariableType.PathBuf:
		case IVariableType.Byte:
			return '""';
		case IVariableType.Integer:
			return "0";
		case IVariableType.Float:
			return "0.0";
		case IVariableType.Boolean:
			return "false";
		case IVariableType.Struct:
			return "{}";
		default:
			return "null";
	}
}

function lastNonWsBefore(text: string, offset: number): number {
	let i = offset - 1;
	while (i >= 0 && WS_RE.test(text[i])) i--;
	return i;
}

interface CodeActionLike {
	title: string;
	kind: string;
	diagnostics: MarkerLike[];
	edit: ReturnType<typeof workspaceEdit>;
	isPreferred?: boolean;
}

function markerActions(
	analysis: FlowScriptAnalysis,
	model: ModelLike,
	marker: MarkerLike,
): CodeActionLike[] {
	const actions: CodeActionLike[] = [];
	const message = marker.message;
	const ms = offsetOf(analysis, {
		lineNumber: marker.startLineNumber,
		column: marker.startColumn,
	});
	const me = Math.max(
		ms,
		offsetOf(analysis, {
			lineNumber: marker.endLineNumber,
			column: marker.endColumn,
		}),
	);
	const spans = backtickSpans(message);
	const push = (title: string, span: Span, text: string) => {
		actions.push({
			title,
			kind: "quickfix",
			diagnostics: [marker],
			edit: workspaceEdit(model, [
				{ range: rangeOfSpan(analysis, span), text },
			]),
		});
	};

	// (a) + (e): did-you-mean / write-the-qualified-form / ambiguity candidates.
	const suggests =
		/did you mean|is ambiguous|write the qualified form|write `/i.test(message);
	if (suggests && spans.length > 0) {
		const token = pathTokenAt(analysis.text, ms, Math.max(me, ms + 1));
		if (token) {
			const seen = new Set<string>();
			for (const raw of spans) {
				if (raw.startsWith("use ")) continue;
				const path = suggestionPath(raw);
				if (!path || path === token.text || seen.has(path)) continue;
				if (RESERVED_WORDS.has(path)) continue;
				// A qualified spelling cannot replace the member of a method call.
				if (
					path.includes("::") &&
					precededByDot(analysis.text, token.span.start)
				)
					continue;
				seen.add(path);
				push(`Replace with '${path}'`, token.span, path);
			}
		}
	}

	// (b) insert the `use ns::*` line the message names or implies.
	const addUse = (nsKey: string) => {
		if (!PATH_ONLY_RE.test(nsKey)) return;
		const edit = useImportEdit(analysis, nsKey);
		if (!edit) return;
		const title = `Add 'use ${nsKey}::*'`;
		if (actions.some((action) => action.title === title)) return;
		actions.push({
			title,
			kind: "quickfix",
			diagnostics: [marker],
			edit: workspaceEdit(model, [edit]),
		});
	};
	for (const raw of spans) {
		const glob = new RegExp(`^use\\s+(${IDENT}(?:::${IDENT})*)::\\*$`).exec(
			raw.trim(),
		);
		if (glob) addUse(glob[1]);
	}
	const wantsImport =
		/did you mean/i.test(message) && !/is ambiguous/i.test(message);
	if (wantsImport) {
		const token = pathTokenAt(analysis.text, ms, Math.max(me, ms + 1));
		if (token && !token.text.includes("::")) {
			for (const raw of spans) {
				const path = suggestionPath(raw);
				if (!path || !path.includes("::")) continue;
				const segments = path.split("::");
				if (segments[segments.length - 1] === token.text)
					addUse(segments.slice(0, -1).join("::"));
			}
		}
	}

	// (c) required inputs missing → insert `pin: <placeholder>` stubs.
	const missing: string[] = [];
	const listMatch = /is missing required inputs: (.+)$/.exec(message);
	if (listMatch) {
		missing.push(
			...listMatch[1]
				.split(",")
				.map((name) => name.trim().replace(/^`|`$/g, "")),
		);
	}
	const inlineMatch =
		/required inputs? ((?:`[^`]+`,? ?)+)(?:is |are )?missing/.exec(message);
	if (inlineMatch) {
		missing.push(
			...[...inlineMatch[1].matchAll(/`([^`]+)`/g)].map((m) => m[1]),
		);
	}
	if (missing.length > 0) {
		const call = callAt(analysis, ms);
		if (call) {
			const names = [
				...new Set(
					missing.map((name) =>
						toFlowScriptIdentifier(name.replace(/\[#\d+\]$/, "")),
					),
				),
			].filter((name) => IDENT_ONLY_RE.test(name));
			const present = new Set(call.args?.named?.map((arg) => arg.name) ?? []);
			const wanted = names.filter((name) => !present.has(name));
			if (wanted.length > 0) {
				const stubs = wanted
					.map(
						(name) =>
							`${name}: ${inputPlaceholder(
								call.resolution.info?.args.find((arg) => arg.name === name),
							)}`,
					)
					.join(", ");
				let at: number;
				let text: string;
				if (call.namedSpan) {
					const close = call.namedSpan.end - 1;
					const empty =
						analysis.masked.slice(call.namedSpan.start + 1, close).trim() ===
						"";
					if (empty) {
						at = call.namedSpan.start + 1;
						text = ` ${stubs} `;
					} else {
						at = lastNonWsBefore(analysis.masked, close) + 1;
						text = `, ${stubs}`;
					}
				} else {
					const empty =
						analysis.masked
							.slice(call.parenIndex + 1, call.closeIndex)
							.trim() === "";
					if (empty) {
						at = call.parenIndex + 1;
						text = `{ ${stubs} }`;
					} else {
						at = lastNonWsBefore(analysis.masked, call.closeIndex) + 1;
						text = `, { ${stubs} }`;
					}
				}
				push(
					`Add missing input${wanted.length > 1 ? "s" : ""}: ${wanted.join(", ")}`,
					{ start: at, end: at },
					text,
				);
			}
		}
	}

	// (d) receiver bound twice → remove the duplicated named argument.
	const dup =
		/Argument '([\w$]+)' is already bound by the receiver/.exec(message) ??
		/binds its receiver to `([^`]+)`, which is also given as a named argument/.exec(
			message,
		);
	if (dup) {
		const argName = toFlowScriptIdentifier(dup[1]);
		const call = callAt(analysis, ms);
		const named = call?.args?.named?.find((arg) => arg.name === argName);
		if (call?.namedSpan && named) {
			let valueEnd = named.valueStart;
			let depth = 0;
			while (valueEnd < call.namedSpan.end - 1) {
				const c = analysis.masked[valueEnd];
				if (c === "(" || c === "[" || c === "{") depth++;
				else if (c === ")" || c === "]" || c === "}") {
					if (depth === 0) break;
					depth--;
				} else if (depth === 0 && c === ",") break;
				valueEnd++;
			}
			valueEnd = lastNonWsBefore(analysis.masked, valueEnd) + 1;
			let from = named.start;
			let to = valueEnd;
			const after = skipWs(analysis.masked, to);
			if (analysis.masked[after] === ",") {
				to = skipWs(analysis.masked, after + 1);
			} else {
				const before = lastNonWsBefore(analysis.masked, from);
				if (analysis.masked[before] === ",") from = before;
			}
			push(
				`Remove duplicate argument '${argName}'`,
				{ start: from, end: to },
				"",
			);
		}
	}

	return actions;
}

function overlaps(a: RangeLike, b: RangeLike): boolean {
	const before =
		a.endLineNumber < b.startLineNumber ||
		(a.endLineNumber === b.startLineNumber && a.endColumn < b.startColumn);
	const after =
		a.startLineNumber > b.endLineNumber ||
		(a.startLineNumber === b.endLineNumber && a.startColumn > b.endColumn);
	return !(before || after);
}

function registerCodeActions(
	monaco: Monaco,
	analysisFor: AnalysisFor,
): { dispose: () => void } {
	const provider = {
		provideCodeActions: (model: ModelLike, range: RangeLike) => {
			const markers = (
				monaco.editor.getModelMarkers({
					resource: model.uri as never,
				}) as unknown as MarkerLike[]
			).filter((marker) => overlaps(marker, range));
			// No overlapping diagnostics → nothing to fix; skip the document analysis.
			if (markers.length === 0) return { actions: [], dispose: () => {} };
			const analysis = analysisFor(model);
			const actions: CodeActionLike[] = [];
			for (const marker of markers) {
				for (const action of markerActions(analysis, model, marker)) {
					if (
						actions.some(
							(existing) =>
								existing.title === action.title &&
								JSON.stringify(existing.edit) === JSON.stringify(action.edit),
						)
					)
						continue;
					actions.push(action);
				}
			}
			return { actions, dispose: () => {} };
		},
	};
	return monaco.languages.registerCodeActionProvider(
		FLOWSCRIPT_LANGUAGE_ID,
		provider as unknown as Parameters<
			Monaco["languages"]["registerCodeActionProvider"]
		>[1],
	);
}

// ---------------------------------------------------------------------------
// 2 + 5. Auto-import completions and statement snippets
// ---------------------------------------------------------------------------

function previousStatementLine(
	masked: string,
	offset: number,
): string | undefined {
	let end = masked.lastIndexOf("\n", offset - 1);
	while (end >= 0) {
		const start = masked.lastIndexOf("\n", end - 1) + 1;
		const line = masked.slice(start, end);
		if (line.trim()) return line;
		if (start === 0) return undefined;
		end = start - 1;
	}
	return undefined;
}

function isEventEntry(info: {
	nodeType: string;
	namespace?: string[];
}): boolean {
	return (
		info.nodeType.startsWith("events_") || info.namespace?.[0] === "events"
	);
}

function statementSnippetItems(
	monaco: Monaco,
	analysis: FlowScriptEnvContext,
	offset: number,
	range: RangeLike,
): unknown[] {
	const snippet = monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet;
	const kind = monaco.languages.CompletionItemKind.Snippet;
	const items: unknown[] = [
		{
			label: "for … of",
			kind,
			detail: "Loop over an array (control::forEach)",
			insertText: "for (const ${1:item} of ${2:items}) {\n\t$0\n}",
			insertTextRules: snippet,
			filterText: "for",
			range,
			sortText: "6_for",
		},
		{
			label: "function …",
			kind,
			detail: "Declare a function layer",
			insertText: "function ${1:name}(${2:param}: ${3:string}) {\n\t$0\n}",
			insertTextRules: snippet,
			filterText: "function",
			range,
			sortText: "6_function",
		},
		{
			label: "@cache",
			kind,
			detail: "Cache the following function's outputs",
			insertText: "@cache",
			filterText: "cache",
			range,
			sortText: "6_cache",
		},
	];
	for (const info of analysis.index.byName.values()) {
		if (!isEventEntry(info)) continue;
		const params = info.outputs
			.map((out) => `${out.name}: ${out.typeString}`)
			.join(", ");
		items.push({
			label: `${info.identifier} …`,
			kind,
			detail: `Event scaffold — ${info.friendlyName}`,
			documentation: { value: nodeHoverMarkdown(info) },
			insertText: `${info.identifier} \${1:onEvent}(${params}) {\n\t$0\n}`,
			insertTextRules: snippet,
			filterText: `${info.identifier} event`,
			range,
			sortText: `6_${info.identifier}`,
		});
	}

	// Exec-arm scaffold: previous statement bound an impure call with several exec outputs.
	const previous = previousStatementLine(analysis.masked, offset);
	if (previous) {
		const m = new RegExp(
			`^\\s*(?:const|let)\\s+(${IDENT})\\s*(?::[^=\\n]+)?=\\s*(.+?)\\s*$`,
		).exec(previous);
		if (m) {
			const result = evaluateExpr(m[2], analysis.env);
			const arms = result.node?.execOutputs ?? [];
			if (result.node && arms.length > 1) {
				const body = arms
					.map((arm, idx) => `\t${arm}: {\n\t\t$${idx + 1}\n\t}`)
					.join("\n");
				items.push({
					label: `${m[1]} { ${arms.join(" · ")} }`,
					kind,
					detail: `Execution arms of ${displayName(result.node)}`,
					insertText: `${m[1]} {\n${body}\n}`,
					insertTextRules: snippet,
					filterText: m[1],
					range,
					sortText: `0_${m[1]}_arms`,
				});
			}
		}
	}
	return items;
}

function autoImportItems(
	monaco: Monaco,
	analysis: FlowScriptEnvContext,
	range: RangeLike,
): unknown[] {
	const { env, index } = analysis;
	const items: unknown[] = [];
	const taken = new Set<string>(env.scope.openMembers.keys());
	for (const name of env.symbols.functions) taken.add(name);
	for (const [name] of env.symbols.variables) taken.add(name);
	for (const ns of index.namespaces.values()) {
		for (const [alias, info] of ns.members) {
			if (taken.has(alias) || index.byName.has(alias)) continue;
			const edit = useImportEdit(analysis, ns.key, alias);
			if (!edit) continue;
			items.push({
				label: { label: alias, description: info.qualified },
				kind: monaco.languages.CompletionItemKind.Function,
				detail: `use ${ns.key}::* — ${renderSignature(info)}`,
				documentation: { value: nodeHoverMarkdown(info) },
				insertText: buildCallSnippet(info, alias),
				insertTextRules:
					monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
				additionalTextEdits: [edit],
				filterText: `${alias} ${info.identifier} ${info.friendlyName}`,
				range,
				sortText: `8_${alias}`,
			});
		}
	}
	return items;
}

function registerSnippetsAndAutoImport(
	monaco: Monaco,
	analysisFor: AnalysisFor,
	getCatalogNodes: GetCatalogNodes,
): { dispose: () => void } {
	const computeItems = (
		analysis: FlowScriptEnvContext,
		model: ModelLike,
		position: Pos,
	) => {
		const offset = offsetOf(analysis, position);
		const maskedBefore = analysis.masked.slice(0, offset);
		const beforeWord = stripTrailingWord(maskedBefore);
		const trimmed = trimTrailingSpacesTabs(beforeWord);
		if (/(::|\.|@)$/.test(trimmed)) return { suggestions: [] };
		const lineStart = maskedBefore.lastIndexOf("\n") + 1;
		if (/^\s*use\b/.test(analysis.masked.slice(lineStart, offset)))
			return { suggestions: [] };
		if (analyzeCacheDecoratorContext(maskedBefore)) return { suggestions: [] };
		const context = analyzeContext(maskedBefore, analysis.env);
		if (context?.mode === "key") return { suggestions: [] };
		if (context?.info) {
			const active =
				context.mode === "value" && context.activeArg
					? context.info.args.find((arg) => arg.name === context.activeArg)
					: context.mode === "positional"
						? context.params[0]
						: undefined;
			if (active?.enumValues && active.enumValues.length > 0)
				return { suggestions: [] };
		}
		const word = model.getWordUntilPosition?.(position) ?? {
			word: "",
			startColumn: position.column,
			endColumn: position.column,
		};
		const range = {
			startLineNumber: position.lineNumber,
			endLineNumber: position.lineNumber,
			startColumn: word.startColumn,
			endColumn: word.endColumn,
		};
		const statement = trimmed === "" || /[\n;{}]$/.test(trimmed);
		const suggestions: unknown[] = [];
		if (statement)
			suggestions.push(
				...statementSnippetItems(monaco, analysis, offset, range),
			);
		suggestions.push(...autoImportItems(monaco, analysis, range));
		return { suggestions: suggestions as never[] };
	};
	const provider = {
		provideCompletionItems: (
			model: ModelLike,
			position: Pos,
			_context: unknown,
			token?: CancellationTokenLike,
		) => {
			const viaWorker = requestFlowScriptWorkerEnvDoc(
				model,
				getCatalogNodes(),
				token,
			);
			if (!viaWorker) return computeItems(analysisFor(model), model, position);
			return viaWorker.then((outcome) => {
				if (outcome.status === "cancelled") return { suggestions: [] };
				const analysis =
					outcome.status === "ok" ? outcome.value : analysisFor(model);
				return computeItems(analysis, model, position);
			});
		},
	};
	return monaco.languages.registerCompletionItemProvider(
		FLOWSCRIPT_LANGUAGE_ID,
		provider as unknown as Parameters<
			Monaco["languages"]["registerCompletionItemProvider"]
		>[1],
	);
}

// ---------------------------------------------------------------------------
// 3. Document symbols
// ---------------------------------------------------------------------------

export type FlowScriptSymbolKindTag =
	| "namespace"
	| "interface"
	| "function"
	| "variable"
	| "event";

export interface FlowScriptDocumentSymbol {
	name: string;
	detail: string;
	kind: FlowScriptSymbolKindTag;
	range: RangeLike;
	selectionRange: RangeLike;
	children: FlowScriptDocumentSymbol[];
}

/** The outline tree as plain data; the provider (or worker client) maps the kind tags. */
export function buildFlowScriptDocumentSymbols(
	analysis: FlowScriptAnalysis,
): FlowScriptDocumentSymbol[] {
	const declSymbol = (
		decl: FlowScriptDeclaration,
	): FlowScriptDocumentSymbol => ({
		name: decl.name,
		detail:
			decl.kind === "event" || decl.kind === "handler"
				? [decl.eventType, decl.detail].filter(Boolean).join(" ")
				: (decl.detail ?? ""),
		kind:
			decl.kind === "interface"
				? "interface"
				: decl.kind === "variable"
					? "variable"
					: decl.kind === "function"
						? "function"
						: "event",
		range: rangeOfSpan(analysis, decl.span),
		selectionRange: rangeOfSpan(analysis, decl.nameSpan),
		children: decl.children.map(declSymbol),
	});

	const symbols: FlowScriptDocumentSymbol[] = [];
	if (analysis.useBlock && analysis.uses.length > 0) {
		symbols.push({
			name: "use",
			detail: `${analysis.uses.length} namespace${
				analysis.uses.length === 1 ? "" : "s"
			}`,
			kind: "namespace",
			range: rangeOfSpan(analysis, analysis.useBlock),
			selectionRange: rangeOfSpan(analysis, {
				start: analysis.uses[0].start,
				end: analysis.uses[0].end,
			}),
			children: [],
		});
	}
	const groups = new Map<
		string,
		{ span: Span; children: FlowScriptDocumentSymbol[]; selection: Span }
	>();
	for (const decl of analysis.declarations) {
		if (decl.kind === "variable" && decl.category) {
			const group = groups.get(decl.category);
			if (group) {
				group.span = {
					start: Math.min(group.span.start, decl.span.start),
					end: Math.max(group.span.end, decl.span.end),
				};
				group.children.push(declSymbol(decl));
			} else {
				groups.set(decl.category, {
					span: { ...decl.span },
					selection: { ...decl.nameSpan },
					children: [declSymbol(decl)],
				});
			}
			continue;
		}
		symbols.push(declSymbol(decl));
	}
	for (const [category, group] of groups) {
		symbols.push({
			name: category,
			detail: "@category",
			kind: "namespace",
			range: rangeOfSpan(analysis, group.span),
			selectionRange: rangeOfSpan(analysis, group.selection),
			children: group.children,
		});
	}
	return symbols;
}

/** Maps plain outline symbols onto the Monaco `SymbolKind` enum. */
export function flowScriptSymbolsToMonaco(
	monaco: Monaco,
	symbols: readonly FlowScriptDocumentSymbol[],
): unknown[] {
	const kinds = monaco.languages.SymbolKind;
	const kindOf = (tag: FlowScriptSymbolKindTag) =>
		tag === "namespace"
			? kinds.Namespace
			: tag === "interface"
				? kinds.Interface
				: tag === "function"
					? kinds.Function
					: tag === "variable"
						? kinds.Variable
						: kinds.Event;
	const map = (symbol: FlowScriptDocumentSymbol): unknown => ({
		name: symbol.name,
		detail: symbol.detail,
		kind: kindOf(symbol.kind),
		tags: [],
		range: symbol.range,
		selectionRange: symbol.selectionRange,
		children: symbol.children.map(map),
	});
	return symbols.map(map);
}

function registerDocumentSymbols(
	monaco: Monaco,
	analysisFor: AnalysisFor,
	getCatalogNodes: GetCatalogNodes,
): { dispose: () => void } {
	const symbolsInThread = (model: ModelLike) =>
		flowScriptSymbolsToMonaco(
			monaco,
			buildFlowScriptDocumentSymbols(analysisFor(model)),
		);
	const provider = {
		provideDocumentSymbols: (
			model: ModelLike,
			token?: CancellationTokenLike,
		) => {
			const viaWorker = requestFlowScriptWorkerDocumentSymbols(
				model,
				getCatalogNodes(),
				token,
			);
			if (!viaWorker) return symbolsInThread(model);
			return viaWorker.then((outcome) => {
				if (outcome.status === "cancelled") return null;
				if (outcome.status === "failed") return symbolsInThread(model);
				return flowScriptSymbolsToMonaco(monaco, outcome.value);
			});
		},
	};
	return monaco.languages.registerDocumentSymbolProvider(
		FLOWSCRIPT_LANGUAGE_ID,
		provider as unknown as Parameters<
			Monaco["languages"]["registerDocumentSymbolProvider"]
		>[1],
	);
}

// ---------------------------------------------------------------------------
// 4. Folding ranges
// ---------------------------------------------------------------------------

export interface FlowScriptFoldingRange {
	start: number;
	end: number;
	imports?: boolean;
}

/** Folding ranges as plain data; the provider (or worker client) maps `imports` to the kind. */
export function buildFlowScriptFoldingRanges(
	analysis: FlowScriptAnalysis,
): FlowScriptFoldingRange[] {
	const ranges: FlowScriptFoldingRange[] = [];
	const lineOf = (offset: number) => positionOf(analysis, offset).lineNumber;
	if (analysis.useBlock) {
		const start = lineOf(analysis.useBlock.start);
		const end = lineOf(analysis.useBlock.end);
		if (end > start) ranges.push({ start, end, imports: true });
	}
	for (const template of analysis.templates) {
		const start = lineOf(template.start);
		const end = lineOf(template.end - 1) - 1;
		if (end > start) ranges.push({ start, end });
	}
	// Bracket regions: registering a provider replaces indentation folding, so every
	// multi-line block keeps folding here (bodies, branch arms, arrays, call objects).
	for (const pair of analysis.brackets) {
		const start = lineOf(pair.open);
		const end = lineOf(pair.close) - 1;
		if (end > start) ranges.push({ start, end });
	}
	return ranges;
}

/** Maps plain folding ranges to the Monaco provider shape. */
export function flowScriptFoldingToMonaco(
	monaco: Monaco,
	ranges: readonly FlowScriptFoldingRange[],
): { start: number; end: number; kind?: unknown }[] {
	return ranges.map((range) =>
		range.imports
			? {
					start: range.start,
					end: range.end,
					kind: monaco.languages.FoldingRangeKind?.Imports,
				}
			: { start: range.start, end: range.end },
	);
}

function registerFolding(
	monaco: Monaco,
	analysisFor: AnalysisFor,
	getCatalogNodes: GetCatalogNodes,
): { dispose: () => void } {
	const foldInThread = (model: ModelLike) =>
		flowScriptFoldingToMonaco(
			monaco,
			buildFlowScriptFoldingRanges(analysisFor(model)),
		);
	const provider = {
		provideFoldingRanges: (
			model: ModelLike,
			_context: unknown,
			token?: CancellationTokenLike,
		) => {
			const viaWorker = requestFlowScriptWorkerFolding(
				model,
				getCatalogNodes(),
				token,
			);
			if (!viaWorker) return foldInThread(model);
			return viaWorker.then((outcome) => {
				if (outcome.status === "cancelled") return null;
				if (outcome.status === "failed") return foldInThread(model);
				return flowScriptFoldingToMonaco(monaco, outcome.value);
			});
		},
	};
	return monaco.languages.registerFoldingRangeProvider(
		FLOWSCRIPT_LANGUAGE_ID,
		provider as unknown as Parameters<
			Monaco["languages"]["registerFoldingRangeProvider"]
		>[1],
	);
}

// ---------------------------------------------------------------------------
// 6. Inlay hints
// ---------------------------------------------------------------------------

function inferredTypeLabel(value: ValueType | null): string | undefined {
	if (!value || value.multiOutput) return undefined;
	if (value.group === "any" || value.group === "null") return undefined;
	let base: string | undefined;
	switch (value.dataType) {
		case IVariableType.String:
			base = "string";
			break;
		case IVariableType.Integer:
			base = "int";
			break;
		case IVariableType.Float:
			base = "float";
			break;
		case IVariableType.Boolean:
			base = "bool";
			break;
		case IVariableType.Date:
			base = "Date";
			break;
		case IVariableType.PathBuf:
			base = "Path";
			break;
		case IVariableType.Byte:
			base = "bytes";
			break;
		case IVariableType.Struct:
			base = value.schemaTitle ?? "Struct";
			break;
		default:
			base = undefined;
	}
	if (!base) {
		switch (value.group) {
			case "string":
				base = "string";
				break;
			case "bool":
				base = "bool";
				break;
			case "struct":
				base = value.schemaTitle ?? "Struct";
				break;
			case "date":
				base = "Date";
				break;
			case "path":
				base = "Path";
				break;
			case "bytes":
				base = "bytes";
				break;
			default:
				return undefined;
		}
	}
	if (value.container === IValueType.HashMap) return `Map<string, ${base}>`;
	if (value.container === IValueType.HashSet) return `Set<${base}>`;
	if (value.isArray) return `${base}[]`;
	return base;
}

const LITERAL_RHS_RE = /^(["'`[{]|-?\d|true\b|false\b|null\b)/;

export interface FlowScriptInlayHint {
	position: Pos;
	label: string;
	kind: "type" | "parameter";
	paddingLeft: boolean;
	paddingRight: boolean;
	tooltip?: string;
}

/** Inlay hints for a line range as plain data; kinds map to Monaco enums at the provider. */
export function buildFlowScriptInlayHints(
	analysis: FlowScriptAnalysis,
	startLine: number,
	endLine: number,
): FlowScriptInlayHint[] {
	const hints: FlowScriptInlayHint[] = [];
	const seen = new Set<string>();
	const push = (
		offset: number,
		label: string,
		kind: "type" | "parameter",
		padding: { left?: boolean; right?: boolean },
		tooltip?: string,
	) => {
		const position = positionOf(analysis, offset);
		if (position.lineNumber < startLine || position.lineNumber > endLine)
			return;
		const key = `${offset}:${label}`;
		if (seen.has(key)) return;
		seen.add(key);
		hints.push({
			position,
			label,
			kind,
			paddingLeft: padding.left === true,
			paddingRight: padding.right === true,
			tooltip,
		});
	};

	for (const call of analysis.calls) {
		const info = call.resolution.info;
		if (!info) continue;
		if (call.args && call.args.positional.length > 0) {
			const bindable = receiverIsBound(call.resolution)
				? methodParams(info)
				: info.args;
			call.args.positional.forEach((positional, idx) => {
				const pin = bindable[idx];
				if (!pin || positional.value === pin.name) return;
				push(positional.start, `${pin.name}:`, "parameter", {
					right: true,
				});
			});
		}
		if (info.impure && (call.enclosed || call.inTemplateExpr)) {
			push(
				call.closeIndex + 1,
				"impure",
				"type",
				{ left: true },
				`\`${displayName(info)}\` has side effects (execution pins).`,
			);
		}
	}
	for (const binding of analysis.bindings) {
		if (binding.kind !== "variable" && binding.kind !== "local") continue;
		if (binding.destructured || binding.typeText || !binding.rhs) continue;
		if (LITERAL_RHS_RE.test(binding.rhs.text)) continue;
		const label = inferredTypeLabel(
			evaluateExpr(binding.rhs.text, analysis.env).value,
		);
		if (!label) continue;
		push(binding.nameSpan.end, `: ${label}`, "type", {});
	}
	return hints;
}

/** Maps plain inlay hints onto the Monaco enums/shapes. */
export function flowScriptInlayHintsToMonaco(
	monaco: Monaco,
	hints: readonly FlowScriptInlayHint[],
): unknown[] {
	const kinds = monaco.languages.InlayHintKind ?? {
		Type: 1,
		Parameter: 2,
	};
	return hints.map((hint) => ({
		position: hint.position,
		label: hint.label,
		kind: hint.kind === "parameter" ? kinds.Parameter : kinds.Type,
		paddingLeft: hint.paddingLeft,
		paddingRight: hint.paddingRight,
		tooltip: hint.tooltip ? { value: hint.tooltip } : undefined,
	}));
}

function registerInlayHints(
	monaco: Monaco,
	analysisFor: AnalysisFor,
	getCatalogNodes: GetCatalogNodes,
): { dispose: () => void } {
	const hintsInThread = (model: ModelLike, range?: RangeLike) => {
		const analysis = analysisFor(model);
		const hints = buildFlowScriptInlayHints(
			analysis,
			range?.startLineNumber ?? 1,
			range?.endLineNumber ?? analysis.lineStarts.length,
		);
		return {
			hints: flowScriptInlayHintsToMonaco(monaco, hints),
			dispose: () => {},
		};
	};
	const provider = {
		provideInlayHints: (
			model: ModelLike,
			range?: RangeLike,
			token?: CancellationTokenLike,
		) => {
			// Without a range Monaco wants the whole document; the worker request
			// needs concrete bounds, so derive them from the text only when cheap.
			const viaWorker = range
				? requestFlowScriptWorkerInlayHints(
						model,
						getCatalogNodes(),
						range.startLineNumber,
						range.endLineNumber,
						token,
					)
				: null;
			if (!viaWorker) return hintsInThread(model, range);
			return viaWorker.then((outcome) => {
				if (outcome.status === "cancelled") return null;
				if (outcome.status === "failed") return hintsInThread(model, range);
				return {
					hints: flowScriptInlayHintsToMonaco(monaco, outcome.value),
					dispose: () => {},
				};
			});
		},
	};
	return monaco.languages.registerInlayHintsProvider(
		FLOWSCRIPT_LANGUAGE_ID,
		provider as unknown as Parameters<
			Monaco["languages"]["registerInlayHintsProvider"]
		>[1],
	);
}

// ---------------------------------------------------------------------------
// 7. Definition + references
// ---------------------------------------------------------------------------

function registerDefinitionAndReferences(
	monaco: Monaco,
	analysisFor: AnalysisFor,
): { dispose: () => void } {
	const definition = monaco.languages.registerDefinitionProvider(
		FLOWSCRIPT_LANGUAGE_ID,
		{
			provideDefinition: (model: ModelLike, position: Pos) => {
				const analysis = analysisFor(model);
				const occ = occurrenceAt(analysis, offsetOf(analysis, position));
				if (!occ) return null;
				const span = definitionSpan(analysis, occ);
				if (!span) return null;
				return { uri: model.uri, range: rangeOfSpan(analysis, span) };
			},
		} as unknown as Parameters<
			Monaco["languages"]["registerDefinitionProvider"]
		>[1],
	);
	const references = monaco.languages.registerReferenceProvider(
		FLOWSCRIPT_LANGUAGE_ID,
		{
			provideReferences: (
				model: ModelLike,
				position: Pos,
				context?: { includeDeclaration?: boolean },
			) => {
				const analysis = analysisFor(model);
				const occ = occurrenceAt(analysis, offsetOf(analysis, position));
				if (!occ) return [];
				return referencesFor(analysis, occ)
					.filter(
						(o) => context?.includeDeclaration !== false || !o.isDeclaration,
					)
					.map((o) => ({
						uri: model.uri,
						range: rangeOfSpan(analysis, o.span),
					}));
			},
		} as unknown as Parameters<
			Monaco["languages"]["registerReferenceProvider"]
		>[1],
	);
	return {
		dispose: () => {
			definition.dispose();
			references.dispose();
		},
	};
}

// ---------------------------------------------------------------------------
// 8. Semantic tokens
// ---------------------------------------------------------------------------

export const FLOWSCRIPT_SEMANTIC_LEGEND = {
	tokenTypes: [
		"namespace",
		"function",
		"method",
		"variable",
		"parameter",
		"local",
		"event",
		"interface",
	],
	tokenModifiers: ["declaration", "defaultLibrary"],
};

const SEMANTIC_TYPE_INDEX: Partial<Record<FlowScriptOccurrenceKind, number>> = {
	namespace: 0,
	function: 1,
	catalog: 2,
	variable: 3,
	param: 4,
	local: 5,
	loop: 5,
	event: 6,
	interface: 7,
};

const semanticTokenCache = new WeakMap<FlowScriptAnalysis, Uint32Array>();

/** Monaco semantic-token encoding of a document analysis, cached per analysis (= per version). */
export function buildFlowScriptSemanticTokens(
	analysis: FlowScriptAnalysis,
): Uint32Array {
	const cached = semanticTokenCache.get(analysis);
	if (cached) return cached;
	const data: number[] = [];
	let prevLine = 0;
	let prevChar = 0;
	for (const occ of analysis.occurrences) {
		const type = SEMANTIC_TYPE_INDEX[occ.kind];
		if (type === undefined) continue;
		const position = positionOf(analysis, occ.span.start);
		const line = position.lineNumber - 1;
		const char = position.column - 1;
		data.push(
			line - prevLine,
			line === prevLine ? char - prevChar : char,
			occ.span.end - occ.span.start,
			type,
			(occ.isDeclaration ? 1 : 0) | (occ.kind === "catalog" ? 2 : 0),
		);
		prevLine = line;
		prevChar = char;
	}
	const encoded = new Uint32Array(data);
	semanticTokenCache.set(analysis, encoded);
	return encoded;
}

function registerSemanticTokens(
	monaco: Monaco,
	analysisFor: AnalysisFor,
	getCatalogNodes: GetCatalogNodes,
): { dispose: () => void } {
	const encodeInThread = (model: ModelLike) => ({
		data: buildFlowScriptSemanticTokens(analysisFor(model)),
		resultId: undefined,
	});
	const provider = {
		getLegend: () => FLOWSCRIPT_SEMANTIC_LEGEND,
		provideDocumentSemanticTokens: (
			model: ModelLike,
			_lastResultId: unknown,
			token?: CancellationTokenLike,
		) => {
			const viaWorker = requestFlowScriptWorkerSemanticTokens(
				model,
				getCatalogNodes(),
				token,
			);
			if (!viaWorker) return encodeInThread(model);
			return viaWorker.then((outcome) => {
				if (outcome.status === "cancelled") return null;
				if (outcome.status === "failed") return encodeInThread(model);
				return { data: outcome.value, resultId: undefined };
			});
		},
		releaseDocumentSemanticTokens: () => {},
	};
	return monaco.languages.registerDocumentSemanticTokensProvider(
		FLOWSCRIPT_LANGUAGE_ID,
		provider as unknown as Parameters<
			Monaco["languages"]["registerDocumentSemanticTokensProvider"]
		>[1],
	);
}

// ---------------------------------------------------------------------------
// 9. Rename
// ---------------------------------------------------------------------------

const RENAMEABLE = new Set<FlowScriptOccurrenceKind>([
	"variable",
	"local",
	"param",
	"loop",
	"function",
	"event",
	"interface",
]);

function renameTarget(
	analysis: FlowScriptAnalysis,
	offset: number,
): FlowScriptOccurrence {
	const word = pathTokenAt(analysis.text, offset, offset + 1);
	if (word && RESERVED_WORDS.has(word.text))
		throw new Error(
			`'${word.text}' is a FlowScript keyword and cannot be renamed.`,
		);
	const occ = occurrenceAt(analysis, offset);
	if (!occ) throw new Error("There is nothing renameable here.");
	if (RENAMEABLE.has(occ.kind)) return occ;
	switch (occ.kind) {
		case "catalog":
			throw new Error(
				`'${occ.name}' is a catalog node name and cannot be renamed. Only your own bindings, functions and variables can.`,
			);
		case "namespace":
			throw new Error(`'${occ.name}' is a namespace and cannot be renamed.`);
		case "member":
		case "argKey":
			throw new Error(
				`'${occ.name}' is a pin name defined by the catalog node and cannot be renamed.`,
			);
		default:
			throw new Error(`Cannot rename '${occ.name}'.`);
	}
}

function assertRenameAllowed(
	analysis: FlowScriptAnalysis,
	occ: FlowScriptOccurrence,
	refs: FlowScriptOccurrence[],
	newName: string,
): void {
	if (!IDENT_ONLY_RE.test(newName))
		throw new Error(`'${newName}' is not a valid FlowScript identifier.`);
	if (RESERVED_WORDS.has(newName))
		throw new Error(`'${newName}' is a reserved word in FlowScript.`);
	if (newName === occ.name) throw new Error("The name is unchanged.");
	const env = analysis.env;
	if (env.symbols.functions.has(newName))
		throw new Error(
			`'${newName}' collides with a function or event declared in this document.`,
		);
	if (env.symbols.interfaces.has(newName))
		throw new Error(`'${newName}' collides with a declared interface.`);
	if (env.scope.openMembers.has(newName))
		throw new Error(
			`'${newName}' collides with a catalog member opened by \`use\`.`,
		);
	if (env.scope.namespaceAliases.has(newName))
		throw new Error(
			`'${newName}' collides with a namespace opened by \`use\`.`,
		);
	const byName = bindingsIndex(analysis);
	// Capture check: at every occurrence the new name must not resolve to another binding.
	for (const ref of refs) {
		const shadow = resolveBindingAt(byName, newName, ref.span.start);
		if (shadow && shadow !== occ.binding)
			throw new Error(
				`Renaming to '${newName}' would be captured by the ${shadow.kind} '${newName}' declared in an overlapping scope.`,
			);
	}
	if (occ.binding) {
		const scope = occ.binding.scope;
		for (const other of analysis.bindings) {
			if (other === occ.binding || other.name !== newName) continue;
			if (other.scope.start < scope.end && other.scope.end > scope.start)
				throw new Error(
					`'${newName}' collides with the ${other.kind} '${newName}' declared in an overlapping scope.`,
				);
		}
	} else if (analysis.bindings.some((binding) => binding.name === newName)) {
		if (occ.kind === "function" || occ.kind === "event")
			throw new Error(
				`'${newName}' collides with a binding declared in this document.`,
			);
	}
	if (
		(occ.kind === "function" || occ.kind === "event") &&
		analysis.index.byName.has(newName)
	)
		throw new Error(`'${newName}' collides with a catalog node name.`);
}

function registerRename(
	monaco: Monaco,
	analysisFor: AnalysisFor,
): { dispose: () => void } {
	const provider = {
		resolveRenameLocation: (model: ModelLike, position: Pos) => {
			const analysis = analysisFor(model);
			const occ = renameTarget(analysis, offsetOf(analysis, position));
			return { range: rangeOfSpan(analysis, occ.span), text: occ.name };
		},
		provideRenameEdits: (model: ModelLike, position: Pos, newName: string) => {
			const analysis = analysisFor(model);
			const occ = renameTarget(analysis, offsetOf(analysis, position));
			const refs = referencesFor(analysis, occ);
			if (refs.length === 0)
				throw new Error(`No occurrences of '${occ.name}' were found.`);
			assertRenameAllowed(analysis, occ, refs, newName);
			return workspaceEdit(
				model,
				refs.map((ref) => ({
					range: rangeOfSpan(analysis, ref.span),
					text: newName,
				})),
			);
		},
	};
	return monaco.languages.registerRenameProvider(
		FLOWSCRIPT_LANGUAGE_ID,
		provider as unknown as Parameters<
			Monaco["languages"]["registerRenameProvider"]
		>[1],
	);
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/**
 * Registers the analysis-backed language features (code actions, auto-import and snippet
 * completions, outline, folding, inlay hints, definition/references, semantic tokens and
 * rename). Called from `registerFlowScriptProviders`; returns one disposable for the set.
 */
export function registerFlowScriptFeatureProviders(
	monaco: Monaco,
	getCatalogNodes: () => INode[] | undefined,
): { dispose: () => void } {
	const analysisFor: AnalysisFor = (model) =>
		analyzeFlowScriptDocument(
			model.getValue(),
			getFlowScriptIndex(getCatalogNodes()),
		);
	const disposables = [
		registerCodeActions(monaco, analysisFor),
		registerSnippetsAndAutoImport(monaco, analysisFor, getCatalogNodes),
		registerDocumentSymbols(monaco, analysisFor, getCatalogNodes),
		registerFolding(monaco, analysisFor, getCatalogNodes),
		registerInlayHints(monaco, analysisFor, getCatalogNodes),
		registerDefinitionAndReferences(monaco, analysisFor),
		registerSemanticTokens(monaco, analysisFor, getCatalogNodes),
		registerRename(monaco, analysisFor),
	];
	return {
		dispose: () => {
			for (const disposable of disposables) disposable.dispose();
		},
	};
}
