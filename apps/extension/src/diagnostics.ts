import * as vscode from "vscode";
import {
	type FlowCall,
	type FlowDocumentModel,
	analyzeFlowDocument,
	expandUsePath,
	scanCode,
} from "./flowDocument";
import { getDecorator } from "./providers";
import type { NodeSignature, SignatureRegistry } from "./signatures";
import {
	type ResolveContext,
	contextOf,
	expressionShape,
	openedMembers,
	resolveMethod,
} from "./typeResolver";

const CONTROL_NAMES = new Set([
	"if",
	"else",
	"for",
	"of",
	"while",
	"return",
	"const",
	"let",
	"function",
	"interface",
	"use",
	"as",
]);

export interface UnknownCallIssue {
	readonly call: FlowCall;
	readonly message: string;
}

/** "Did you mean …" from nodes sharing the alias or flat name. */
function spellingHint(member: string, registry: SignatureRegistry): string {
	const wanted = member.toLowerCase();
	const hints: string[] = [];
	for (const sig of registry.all()) {
		if (sig.alias?.toLowerCase() === wanted || sig.flat.toLowerCase() === wanted) {
			const spelling = `${sig.name}(…)`;
			if (!hints.includes(spelling)) {
				hints.push(spelling);
			}
		}
	}
	return hints.length > 0
		? ` Did you mean ${hints
				.slice(0, 3)
				.map((hint) => `\`${hint}\``)
				.join(" or ")}?`
		: "";
}

/**
 * Calls that resolve to no declared node: bare names (flat, or opened by `use`), `ns::member`
 * paths (after `use` expansion) and `receiver.method()` calls dispatched on the receiver's
 * class. User functions, event handlers and UFCS calls on user functions never count.
 */
export function unknownCallIssues(
	model: FlowDocumentModel,
	registry: SignatureRegistry,
): UnknownCallIssue[] {
	const ctx = contextOf(registry, model);
	const issues: UnknownCallIssue[] = [];
	for (const call of model.calls) {
		if (call.inTemplate || (call.kind === "bare" && CONTROL_NAMES.has(call.name))) {
			continue;
		}
		if (model.localNames.has(call.name)) {
			continue;
		}
		const message = unknownCallMessage(call, ctx);
		if (message) {
			issues.push({ call, message });
		}
	}
	return issues;
}

function unknownCallMessage(call: FlowCall, ctx: ResolveContext): string | undefined {
	const { registry } = ctx;
	switch (call.kind) {
		case "path": {
			const path = expandUsePath(call.path ?? [], ctx.uses);
			const ns = registry.namespace(path);
			if (!ns) {
				return `Unknown function '${call.display}' — namespace '${(call.path ?? []).join("::")}' is not declared in any .flow.d file.${spellingHint(call.name, registry)}`;
			}
			return ns.members.has(call.name)
				? undefined
				: `Unknown function '${call.display}' — '${call.name}' is not a member of namespace '${ns.key}'.${spellingHint(call.name, registry)}`;
		}
		case "method": {
			if (registry.methodCount === 0) {
				return undefined; // No method tables (legacy declarations without names.json).
			}
			const receiver = expressionShape(call.receiverText ?? "", ctx, call.range.start);
			const { candidates, cls } = resolveMethod(receiver, call.name, ctx);
			if (candidates.length > 0) {
				return undefined;
			}
			// `ns.member(...)` with an unbound namespace root is accepted (namespace walk).
			const walked = (call.receiverText ?? "").split(".");
			if (
				walked.every((segment) => /^[A-Za-z_$][\w$]*$/.test(segment)) &&
				registry.member(expandUsePath(walked, ctx.uses), call.name)
			) {
				return undefined;
			}
			return cls
				? `Unknown method '${call.name}' on ${cls}.${spellingHint(call.name, registry)}`
				: `Unknown method '${call.name}' — no declared node is callable as .${call.name}().${spellingHint(call.name, registry)}`;
		}
		default: {
			if (registry.get(call.name)) {
				return undefined;
			}
			const opened: NodeSignature[] = openedMembers(call.name, ctx);
			if (opened.length === 1) {
				return undefined;
			}
			if (opened.length > 1) {
				return `'${call.name}' is ambiguous: ${opened.map((sig) => `\`${sig.name}\``).join(", ")}. Write the qualified name.`;
			}
			return `Unknown function '${call.name}' — not declared in any .flow.d file or locally.${spellingHint(call.name, registry)}`;
		}
	}
}

export interface DecoratorValidationIssue {
	readonly message: string;
	readonly code: "unknown-decorator" | "decorator-arg";
}

/** Validate a decorator argument captured with its surrounding parentheses. */
export function validateDecoratorArgument(
	name: string,
	argGroup: string | undefined,
): DecoratorValidationIssue | undefined {
	const def = getDecorator(name);
	if (!def) {
		return {
			message: `Unknown decorator '@${name}'.`,
			code: "unknown-decorator",
		};
	}

	const hasArg = argGroup !== undefined;
	switch (def.argumentKind) {
		case "required-string":
			return hasArg
				? undefined
				: {
						message: `Decorator '@${name}' requires a string argument, e.g. @${name}("…").`,
						code: "decorator-arg",
					};
		case "none":
			return hasArg
				? {
						message: `Decorator '@${name}' does not take an argument.`,
						code: "decorator-arg",
					}
				: undefined;
		case "optional-cache-settings": {
			const argument = argGroup?.slice(1, -1).trim();
			return argument === undefined ||
				(argument.startsWith("{") && argument.endsWith("}"))
				? undefined
				: {
						message:
							'Decorator \'@cache\' takes an optional settings object, e.g. @cache({ namespace: "global", ttlSeconds: 300, scope: "user" }). Use ttlSeconds: 0 for no expiry.',
						code: "decorator-arg",
					};
		}
	}
}

export class FlowLinter {
	private readonly collection: vscode.DiagnosticCollection;

	constructor(private readonly registry: SignatureRegistry) {
		this.collection = vscode.languages.createDiagnosticCollection("flowscript");
	}

	dispose(): void {
		this.collection.dispose();
	}

	clear(uri: vscode.Uri): void {
		this.collection.delete(uri);
	}

	lint(document: vscode.TextDocument): void {
		const config = vscode.workspace.getConfiguration("flowLike");
		if (!config.get<boolean>("lint.enable", true)) {
			this.collection.delete(document.uri);
			return;
		}

		const diagnostics: vscode.Diagnostic[] = [];
		const text = document.getText();

		this.checkStringsAndBrackets(document, text, diagnostics);
		this.checkDecorators(document, diagnostics);

		if (config.get<boolean>("lint.unknownFunctions", true)) {
			this.checkUnknownFunctions(document, diagnostics);
		}

		this.collection.set(document.uri, diagnostics);
	}

	/** Validate `@decorator` lines against the parser's known set and arg rules. */
	private checkDecorators(
		document: vscode.TextDocument,
		diagnostics: vscode.Diagnostic[],
	): void {
		const lineRe = /^(\s*)@([A-Za-z_$][\w$]*)\s*(\([\s\S]*\))?\s*$/;
		for (let line = 0; line < document.lineCount; line++) {
			const textLine = document.lineAt(line).text;
			const m = lineRe.exec(textLine);
			if (!m) {
				continue;
			}
			const [, indent, name, argGroup] = m;
			const nameStart = indent.length + 1; // skip the `@`
			const range = new vscode.Range(
				new vscode.Position(line, nameStart),
				new vscode.Position(line, nameStart + name.length),
			);
			const issue = validateDecoratorArgument(name, argGroup);
			if (issue) {
				this.pushWarning(range, issue.message, issue.code, diagnostics);
			}
		}
	}

	private pushWarning(
		range: vscode.Range,
		message: string,
		code: string,
		diagnostics: vscode.Diagnostic[],
	): void {
		const diag = new vscode.Diagnostic(
			range,
			message,
			vscode.DiagnosticSeverity.Warning,
		);
		diag.source = "flowscript";
		diag.code = code;
		diagnostics.push(diag);
	}

	private checkUnknownFunctions(
		document: vscode.TextDocument,
		diagnostics: vscode.Diagnostic[],
	): void {
		if (this.registry.size === 0) {
			return; // No declarations loaded — avoid false positives.
		}
		const model = analyzeFlowDocument(document);
		for (const use of model.uses) {
			if (use.kind === "invalid") {
				this.pushWarning(use.range, use.error, "invalid-use", diagnostics);
				continue;
			}
			if (this.registry.namespaceCount === 0) {
				continue;
			}
			const ns = this.registry.namespace(expandUsePath(use.path, model.uses));
			if (!ns) {
				this.pushWarning(
					use.range,
					`Unknown namespace '${use.path.join("::")}' — not declared in any .flow.d file.`,
					"unknown-namespace",
					diagnostics,
				);
				continue;
			}
			if (use.kind === "members") {
				for (const member of use.members) {
					if (!ns.members.has(member)) {
						this.pushWarning(
							use.range,
							`'${member}' is not a member of namespace '${ns.key}'.`,
							"unknown-namespace-member",
							diagnostics,
						);
					}
				}
			}
		}
		for (const issue of unknownCallIssues(model, this.registry)) {
			const diag = new vscode.Diagnostic(
				issue.call.kind === "path" ? issue.call.headRange : issue.call.range,
				issue.message,
				vscode.DiagnosticSeverity.Warning,
			);
			diag.source = "flowscript";
			diag.code = "unknown-function";
			diagnostics.push(diag);
		}
	}

	private checkStringsAndBrackets(
		document: vscode.TextDocument,
		text: string,
		diagnostics: vscode.Diagnostic[],
	): void {
		const stack: Array<{ ch: string; offset: number }> = [];
		const pairs: Record<string, string> = { ")": "(", "]": "[", "}": "{" };

		// Unterminated `"…"` / `'…'` strings (template literals may span lines).
		let inString: string | undefined;
		let stringStart = 0;
		let inComment = false;
		let inTemplate = 0;
		for (let i = 0; i < text.length; i++) {
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
				} else if (ch === inString) {
					inString = undefined;
				} else if (ch === "\n") {
					this.pushAt(
						document,
						stringStart,
						i,
						"Unterminated string literal.",
						diagnostics,
					);
					inString = undefined;
				}
				continue;
			}
			if (inTemplate > 0) {
				if (ch === "\\") {
					i++;
				} else if (ch === "`") {
					inTemplate--;
				}
				continue;
			}
			if (ch === '"' || ch === "'") {
				inString = ch;
				stringStart = i;
			} else if (ch === "`") {
				inTemplate++;
			} else if (ch === "/" && text[i + 1] === "/") {
				inComment = true;
			}
		}
		if (inString) {
			this.pushAt(
				document,
				stringStart,
				text.length,
				"Unterminated string literal.",
				diagnostics,
			);
		}

		scanCode(text, (ch, offset) => {
			if (ch === "(" || ch === "[" || ch === "{") {
				stack.push({ ch, offset });
			} else if (ch === ")" || ch === "]" || ch === "}") {
				const open = stack.pop();
				if (!open || open.ch !== pairs[ch]) {
					this.pushAt(
						document,
						offset,
						offset + 1,
						`Unmatched closing '${ch}'.`,
						diagnostics,
					);
				}
			}
		});
		for (const open of stack) {
			this.pushAt(
				document,
				open.offset,
				open.offset + 1,
				`Unmatched opening '${open.ch}'.`,
				diagnostics,
			);
		}
	}

	private pushAt(
		document: vscode.TextDocument,
		startOffset: number,
		endOffset: number,
		message: string,
		diagnostics: vscode.Diagnostic[],
	): void {
		const range = new vscode.Range(
			document.positionAt(startOffset),
			document.positionAt(endOffset),
		);
		const diag = new vscode.Diagnostic(
			range,
			message,
			vscode.DiagnosticSeverity.Error,
		);
		diag.source = "flowscript";
		diagnostics.push(diag);
	}
}
