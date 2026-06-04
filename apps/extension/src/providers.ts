import * as vscode from "vscode";
import {
	type FlowSymbol,
	type FlowVariable,
	analyzeFlowDocument,
	identifierOccurrences,
} from "./flowDocument";
import {
	type NodeSignature,
	type SignatureRegistry,
	signatureLabel,
	signatureMarkdown,
} from "./signatures";
import {
	findVariable,
	parseChain,
	variableShape,
	walkAccessors,
} from "./typeResolver";

const WORD_RE = /[A-Za-z_$][\w$]*/;

/** Variable decorators recognized by the FlowScript parser. Mirrors
 * `render::var_decorators_of` / `parser::apply_var_decorators` in flow-like-ast. */
export interface DecoratorDef {
	readonly name: string;
	readonly hasArg: boolean;
	readonly detail: string;
	readonly doc: string;
}

export const FLOW_DECORATORS: readonly DecoratorDef[] = [
	{
		name: "description",
		hasArg: true,
		detail: '@description("…")',
		doc: "Human-facing description of the variable, shown in the UI.",
	},
	{
		name: "category",
		hasArg: true,
		detail: '@category("…")',
		doc: "UI grouping category for the variable.",
	},
	{
		name: "schema",
		hasArg: true,
		detail: '@schema("…")',
		doc: "JSON schema for a struct-typed variable, preserved verbatim.",
	},
	{
		name: "secret",
		hasArg: false,
		detail: "@secret",
		doc: "Marks the variable as a secret (masked and stored securely).",
	},
	{
		name: "readonly",
		hasArg: false,
		detail: "@readonly",
		doc: "Marks the variable as not user-editable.",
	},
	{
		name: "runtime",
		hasArg: false,
		detail: "@runtime",
		doc: "Variable is configured per-user at runtime.",
	},
];

const DECORATOR_BY_NAME = new Map(FLOW_DECORATORS.map((d) => [d.name, d]));

export function getDecorator(name: string): DecoratorDef | undefined {
	return DECORATOR_BY_NAME.get(name);
}

function wordAt(
	document: vscode.TextDocument,
	position: vscode.Position,
): { word: string; range: vscode.Range } | undefined {
	const range = document.getWordRangeAtPosition(position, WORD_RE);
	if (!range) {
		return undefined;
	}
	return { word: document.getText(range), range };
}

/** Resolve a variable's type for hover display. */
function resolveVariableType(
	variable: FlowVariable,
	registry: SignatureRegistry,
): string | undefined {
	if (variable.typeText) {
		return variable.typeText;
	}
	const shape = variableShape(variable, registry);
	return shape?.text;
}

function variableHoverMarkdown(
	variable: FlowVariable,
	registry: SignatureRegistry,
): vscode.MarkdownString {
	const md = new vscode.MarkdownString();
	const type = resolveVariableType(variable, registry);
	md.appendCodeblock(
		`${variable.keyword} ${variable.name}${type ? `: ${type}` : ""}`,
		"flowscript",
	);
	if (variable.initCall && registry.has(variable.initCall)) {
		md.appendMarkdown(`\n\nReturned by \`${variable.initCall}\`.`);
	}
	const shape = variableShape(variable, registry);
	const objectShape =
		shape?.kind === "object"
			? shape
			: shape?.kind === "array" && shape.element?.kind === "object"
				? shape.element
				: undefined;
	if (objectShape && objectShape.fields.size > 0) {
		const fields = [...objectShape.fields.entries()]
			.map(([name, field]) => `  ${name}: ${field.text}`)
			.join("\n");
		md.appendMarkdown("\n\nFields:");
		md.appendCodeblock(`{\n${fields}\n}`, "flowscript");
	}
	return md;
}

export class FlowCompletionProvider implements vscode.CompletionItemProvider {
	constructor(private readonly registry: SignatureRegistry) {}

	provideCompletionItems(
		document: vscode.TextDocument,
	): vscode.CompletionItem[] {
		const items: vscode.CompletionItem[] = [];
		const local = analyzeFlowDocument(document);

		for (const sig of this.registry.all()) {
			items.push(this.signatureItem(sig));
		}
		for (const sym of local.symbols) {
			if (this.registry.has(sym.name)) {
				continue;
			}
			const item = new vscode.CompletionItem(
				sym.name,
				sym.kind === "variable"
					? vscode.CompletionItemKind.Variable
					: vscode.CompletionItemKind.Function,
			);
			item.detail = `(${sym.detail}) ${sym.name}`;
			items.push(item);
		}
		return items;
	}

	private signatureItem(sig: NodeSignature): vscode.CompletionItem {
		const item = new vscode.CompletionItem(
			sig.name,
			vscode.CompletionItemKind.Function,
		);
		item.detail = signatureLabel(sig);
		item.documentation = signatureMarkdown(sig);
		const args = sig.params
			.map((p, idx) => `${p.name}: \${${idx + 1}:${p.name}}`)
			.join(", ");
		item.insertText = new vscode.SnippetString(
			sig.params.length > 0 ? `${sig.name}({ ${args} })` : `${sig.name}()`,
		);
		item.sortText = sig.impure ? `1_${sig.name}` : `0_${sig.name}`;
		return item;
	}
}

/** Completion for `@` variable decorators (`@description`, `@secret`, …). */
export class FlowDecoratorCompletionProvider
	implements vscode.CompletionItemProvider
{
	provideCompletionItems(
		document: vscode.TextDocument,
		position: vscode.Position,
	): vscode.CompletionItem[] | undefined {
		const prefix = document
			.lineAt(position.line)
			.text.slice(0, position.character);
		const match = /@(\w*)$/.exec(prefix);
		if (!match) {
			return undefined;
		}
		const range = new vscode.Range(
			position.translate(0, -match[0].length),
			position,
		);
		return FLOW_DECORATORS.map((dec) => {
			const item = new vscode.CompletionItem(
				`@${dec.name}`,
				vscode.CompletionItemKind.Keyword,
			);
			item.detail = dec.detail;
			item.documentation = new vscode.MarkdownString(dec.doc);
			item.range = range;
			item.insertText = dec.hasArg
				? new vscode.SnippetString(`@${dec.name}("$1")`)
				: `@${dec.name}`;
			return item;
		});
	}
}

/** Completion for member access (`base.` / `base.field.`): lists the fields of
 * the resolved object shape — node return structs and inferred literal shapes. */
export class FlowMemberCompletionProvider
	implements vscode.CompletionItemProvider
{
	constructor(private readonly registry: SignatureRegistry) {}

	provideCompletionItems(
		document: vscode.TextDocument,
		position: vscode.Position,
	): vscode.CompletionItem[] | undefined {
		const before = document
			.lineAt(position.line)
			.text.slice(0, position.character);
		if (!/\.\s*$/.test(before)) {
			return undefined;
		}
		const chain = parseChain(before.replace(/\.\s*$/, ""));
		if (!chain) {
			return undefined;
		}
		const model = analyzeFlowDocument(document);
		const variable = findVariable(model.variables, chain.base, position);
		if (!variable) {
			return undefined;
		}
		const baseShape = variableShape(variable, this.registry);
		if (!baseShape) {
			return undefined;
		}
		const target = walkAccessors(baseShape, chain.accessors);
		if (target?.kind !== "object") {
			return undefined;
		}
		const items: vscode.CompletionItem[] = [];
		for (const [name, shape] of target.fields) {
			const item = new vscode.CompletionItem(
				name,
				vscode.CompletionItemKind.Field,
			);
			item.detail = `${name}: ${shape.text}`;
			const doc = target.docs?.get(name);
			if (doc) {
				item.documentation = new vscode.MarkdownString(doc);
			}
			items.push(item);
		}
		return items;
	}
}

export class FlowHoverProvider implements vscode.HoverProvider {
	constructor(private readonly registry: SignatureRegistry) {}

	provideHover(
		document: vscode.TextDocument,
		position: vscode.Position,
	): vscode.Hover | undefined {
		const hit = wordAt(document, position);
		if (!hit) {
			return undefined;
		}
		// `@decorator` — the `@` sits just before the word range.
		if (hit.range.start.character > 0) {
			const at = new vscode.Range(
				hit.range.start.translate(0, -1),
				hit.range.start,
			);
			if (document.getText(at) === "@") {
				const dec = getDecorator(hit.word);
				if (dec) {
					const md = new vscode.MarkdownString();
					md.appendCodeblock(dec.detail, "flowscript");
					md.appendMarkdown(`\n\n${dec.doc}`);
					return new vscode.Hover(
						md,
						new vscode.Range(at.start, hit.range.end),
					);
				}
			}
		}
		const sig = this.registry.get(hit.word);
		if (sig) {
			return new vscode.Hover(signatureMarkdown(sig), hit.range);
		}
		const model = analyzeFlowDocument(document);
		// Member access `base.field` — resolve the field from the base's node return.
		const member = this.memberHover(document, position, hit, model);
		if (member) {
			return member;
		}
		const variable = findVariable(model.variables, hit.word, position);
		if (variable) {
			return new vscode.Hover(
				variableHoverMarkdown(variable, this.registry),
				hit.range,
			);
		}
		const local = model.symbols.find((s) => s.name === hit.word);
		if (local) {
			const md = new vscode.MarkdownString();
			md.appendCodeblock(`(${local.detail}) ${local.name}`, "flowscript");
			return new vscode.Hover(md, hit.range);
		}
		return undefined;
	}

	/** Hover for a member access chain: resolve the type at the hovered segment,
	 * walking through node return structs and inferred struct-literal shapes. */
	private memberHover(
		document: vscode.TextDocument,
		position: vscode.Position,
		hit: { word: string; range: vscode.Range },
		model: ReturnType<typeof analyzeFlowDocument>,
	): vscode.Hover | undefined {
		const start = hit.range.start;
		if (start.character === 0) {
			return undefined;
		}
		const dotRange = new vscode.Range(start.translate(0, -1), start);
		if (document.getText(dotRange) !== ".") {
			return undefined;
		}
		const before = document
			.lineAt(position.line)
			.text.slice(0, hit.range.end.character);
		const chain = parseChain(before);
		if (!chain || chain.accessors.length === 0) {
			return undefined;
		}
		const variable = findVariable(model.variables, chain.base, position);
		if (!variable) {
			return undefined;
		}
		const baseShape = variableShape(variable, this.registry);
		if (!baseShape) {
			return undefined;
		}
		const resolved = walkAccessors(baseShape, chain.accessors);
		if (!resolved) {
			return undefined;
		}
		// The parent object that owns the hovered field (for docs / origin).
		const parent = walkAccessors(baseShape, chain.accessors.slice(0, -1));
		const md = new vscode.MarkdownString();
		md.appendCodeblock(`${hit.word}: ${resolved.text}`, "flowscript");
		if (parent?.kind === "object") {
			const doc = parent.docs?.get(hit.word);
			if (doc) {
				md.appendMarkdown(`\n\n${doc}`);
			}
			if (parent.origin) {
				md.appendMarkdown(`\n\n_Output of \`${parent.origin}\`._`);
			}
		}
		return new vscode.Hover(md, hit.range);
	}
}

export class FlowSignatureHelpProvider implements vscode.SignatureHelpProvider {
	constructor(private readonly registry: SignatureRegistry) {}

	provideSignatureHelp(
		document: vscode.TextDocument,
		position: vscode.Position,
	): vscode.SignatureHelp | undefined {
		const ctx = findEnclosingCall(document, position);
		if (!ctx) {
			return undefined;
		}
		const sig = this.registry.get(ctx.name);
		if (!sig) {
			return undefined;
		}
		const info = new vscode.SignatureInformation(
			signatureLabel(sig),
			signatureMarkdown(sig),
		);
		info.parameters = sig.params.map(
			(p) =>
				new vscode.ParameterInformation(
					`${p.name}${p.optional ? "?" : ""}: ${p.type}`,
					p.doc,
				),
		);
		const help = new vscode.SignatureHelp();
		help.signatures = [info];
		help.activeSignature = 0;
		help.activeParameter = Math.min(
			ctx.activeParam,
			Math.max(0, sig.params.length - 1),
		);
		return help;
	}
}

interface CallContext {
	readonly name: string;
	readonly activeParam: number;
}

/** Find the innermost call whose argument list contains `position`. */
function findEnclosingCall(
	document: vscode.TextDocument,
	position: vscode.Position,
): CallContext | undefined {
	const offset = document.offsetAt(position);
	const text = document.getText();
	let depth = 0;
	let commas = 0;
	let inString = false;

	for (let i = offset - 1; i >= 0; i--) {
		const ch = text[i];
		if (inString) {
			if (ch === '"' && text[i - 1] !== "\\") {
				inString = false;
			}
			continue;
		}
		if (ch === '"') {
			inString = true;
			continue;
		}
		if (ch === ")" || ch === "}") {
			depth++;
		} else if (ch === "(") {
			if (depth === 0) {
				const name = identifierBefore(text, i);
				if (!name) {
					return undefined;
				}
				return { name, activeParam: commas };
			}
			depth--;
		} else if (ch === "{") {
			depth--;
		} else if (ch === "," && depth === 0) {
			commas++;
		}
	}
	return undefined;
}

function identifierBefore(
	text: string,
	parenIndex: number,
): string | undefined {
	let end = parenIndex;
	while (end > 0 && /\s/.test(text[end - 1])) {
		end--;
	}
	let start = end;
	while (start > 0 && /[A-Za-z0-9_$]/.test(text[start - 1])) {
		start--;
	}
	const name = text.slice(start, end);
	return WORD_RE.test(name) ? name : undefined;
}

export class FlowDocumentSymbolProvider
	implements vscode.DocumentSymbolProvider
{
	provideDocumentSymbols(
		document: vscode.TextDocument,
	): vscode.DocumentSymbol[] {
		return analyzeFlowDocument(document).symbols.map(toDocumentSymbol);
	}
}

function toDocumentSymbol(sym: FlowSymbol): vscode.DocumentSymbol {
	const kind =
		sym.kind === "variable"
			? vscode.SymbolKind.Variable
			: sym.kind === "event"
				? vscode.SymbolKind.Event
				: vscode.SymbolKind.Function;
	return new vscode.DocumentSymbol(
		sym.name,
		sym.detail,
		kind,
		sym.fullRange,
		sym.selectionRange,
	);
}

export class DeclarationSymbolProvider
	implements vscode.DocumentSymbolProvider
{
	constructor(private readonly registry: SignatureRegistry) {}

	provideDocumentSymbols(
		document: vscode.TextDocument,
	): vscode.DocumentSymbol[] {
		const out: vscode.DocumentSymbol[] = [];
		for (const sig of this.registry.all()) {
			if (sig.source.toString() !== document.uri.toString()) {
				continue;
			}
			out.push(
				new vscode.DocumentSymbol(
					sig.name,
					signatureLabel(sig),
					vscode.SymbolKind.Function,
					sig.nameRange,
					sig.nameRange,
				),
			);
		}
		return out;
	}
}

/** Jump from a call to its declaration: nodes resolve to their `.flow.d`,
 * local variables/functions/events resolve within the current document. */
export class FlowDefinitionProvider implements vscode.DefinitionProvider {
	constructor(private readonly registry: SignatureRegistry) {}

	provideDefinition(
		document: vscode.TextDocument,
		position: vscode.Position,
	): vscode.Location | undefined {
		const hit = wordAt(document, position);
		if (!hit) {
			return undefined;
		}
		const sig = this.registry.get(hit.word);
		if (sig) {
			return new vscode.Location(sig.source, sig.nameRange);
		}
		const model = analyzeFlowDocument(document);
		const local = model.symbols.find((s) => s.name === hit.word);
		if (local) {
			return new vscode.Location(document.uri, local.selectionRange);
		}
		const variable = findVariable(model.variables, hit.word, position);
		if (variable) {
			return new vscode.Location(document.uri, variable.range);
		}
		return undefined;
	}
}

/** All occurrences of the identifier under the cursor in the current document. */
export class FlowReferenceProvider implements vscode.ReferenceProvider {
	provideReferences(
		document: vscode.TextDocument,
		position: vscode.Position,
	): vscode.Location[] {
		const hit = wordAt(document, position);
		if (!hit) {
			return [];
		}
		return identifierOccurrences(document, hit.word).map(
			(range) => new vscode.Location(document.uri, range),
		);
	}
}

/** Highlight every occurrence of the identifier under the cursor. */
export class FlowDocumentHighlightProvider
	implements vscode.DocumentHighlightProvider
{
	provideDocumentHighlights(
		document: vscode.TextDocument,
		position: vscode.Position,
	): vscode.DocumentHighlight[] {
		const hit = wordAt(document, position);
		if (!hit) {
			return [];
		}
		return identifierOccurrences(document, hit.word).map(
			(range) => new vscode.DocumentHighlight(range),
		);
	}
}

/** Rename locally declared variables, functions and event handlers.
 * Node declarations live in `.flow.d` files and are rejected here. */
export class FlowRenameProvider implements vscode.RenameProvider {
	constructor(private readonly registry: SignatureRegistry) {}

	prepareRename(
		document: vscode.TextDocument,
		position: vscode.Position,
	): vscode.Range {
		const hit = wordAt(document, position);
		if (!hit) {
			throw new Error("You cannot rename this element.");
		}
		if (this.registry.has(hit.word)) {
			throw new Error(
				"Node declarations come from .flow.d files and cannot be renamed here.",
			);
		}
		if (!analyzeFlowDocument(document).localNames.has(hit.word)) {
			throw new Error("Only locally declared names can be renamed.");
		}
		return hit.range;
	}

	provideRenameEdits(
		document: vscode.TextDocument,
		position: vscode.Position,
		newName: string,
	): vscode.WorkspaceEdit {
		const edit = new vscode.WorkspaceEdit();
		const hit = wordAt(document, position);
		if (!hit) {
			return edit;
		}
		for (const range of identifierOccurrences(document, hit.word)) {
			edit.replace(document.uri, range, newName);
		}
		return edit;
	}
}

/** Search all catalog nodes across loaded `.flow.d` declarations. */
export class FlowWorkspaceSymbolProvider
	implements vscode.WorkspaceSymbolProvider
{
	constructor(private readonly registry: SignatureRegistry) {}

	provideWorkspaceSymbols(query: string): vscode.SymbolInformation[] {
		const q = query.toLowerCase();
		const out: vscode.SymbolInformation[] = [];
		for (const sig of this.registry.all()) {
			if (q && !sig.name.toLowerCase().includes(q)) {
				continue;
			}
			out.push(
				new vscode.SymbolInformation(
					sig.name,
					vscode.SymbolKind.Function,
					signatureLabel(sig),
					new vscode.Location(sig.source, sig.nameRange),
				),
			);
		}
		return out;
	}
}

/** Quick fix for `unknown-function` warnings: offer the closest known node. */
export class FlowQuickFixProvider implements vscode.CodeActionProvider {
	static readonly providedKinds = [vscode.CodeActionKind.QuickFix];

	constructor(private readonly registry: SignatureRegistry) {}

	provideCodeActions(
		document: vscode.TextDocument,
		_range: vscode.Range,
		context: vscode.CodeActionContext,
	): vscode.CodeAction[] {
		const actions: vscode.CodeAction[] = [];
		for (const diag of context.diagnostics) {
			if (diag.code !== "unknown-function") {
				continue;
			}
			const word = document.getText(diag.range);
			for (const suggestion of this.closestNames(word)) {
				const action = new vscode.CodeAction(
					`Replace with '${suggestion}'`,
					vscode.CodeActionKind.QuickFix,
				);
				action.diagnostics = [diag];
				action.edit = new vscode.WorkspaceEdit();
				action.edit.replace(document.uri, diag.range, suggestion);
				actions.push(action);
			}
		}
		return actions;
	}

	private closestNames(word: string): string[] {
		const target = word.toLowerCase();
		const scored: Array<{ name: string; distance: number }> = [];
		for (const sig of this.registry.all()) {
			const distance = levenshtein(target, sig.name.toLowerCase());
			if (distance <= Math.max(2, Math.floor(word.length / 3))) {
				scored.push({ name: sig.name, distance });
			}
		}
		scored.sort((a, b) => a.distance - b.distance);
		return scored.slice(0, 3).map((s) => s.name);
	}
}

function levenshtein(a: string, b: string): number {
	const rows = a.length + 1;
	const cols = b.length + 1;
	const dp: number[] = new Array(cols);
	for (let j = 0; j < cols; j++) {
		dp[j] = j;
	}
	for (let i = 1; i < rows; i++) {
		let prev = dp[0];
		dp[0] = i;
		for (let j = 1; j < cols; j++) {
			const temp = dp[j];
			dp[j] =
				a[i - 1] === b[j - 1] ? prev : 1 + Math.min(prev, dp[j], dp[j - 1]);
			prev = temp;
		}
	}
	return dp[cols - 1];
}
