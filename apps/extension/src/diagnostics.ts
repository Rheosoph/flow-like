import * as vscode from "vscode";
import { analyzeFlowDocument } from "./flowDocument";
import { getDecorator } from "./providers";
import type { SignatureRegistry } from "./signatures";

const CONTROL_NAMES = new Set([
	"if",
	"else",
	"for",
	"of",
	"return",
	"const",
	"let",
	"function",
	"interface",
]);

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
		for (const call of model.calls) {
			if (CONTROL_NAMES.has(call.name)) {
				continue;
			}
			if (this.registry.has(call.name) || model.localNames.has(call.name)) {
				continue;
			}
			const diag = new vscode.Diagnostic(
				call.range,
				`Unknown function '${call.name}' — not declared in any .flow.d file or locally.`,
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
		let inString = false;
		let stringStart = 0;
		let inComment = false;

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
				} else if (ch === '"') {
					inString = false;
				} else if (ch === "\n") {
					this.pushAt(
						document,
						stringStart,
						i,
						"Unterminated string literal.",
						diagnostics,
					);
					inString = false;
				}
				continue;
			}
			if (ch === '"') {
				inString = true;
				stringStart = i;
			} else if (ch === "/" && text[i + 1] === "/") {
				inComment = true;
			} else if (ch === "(" || ch === "[" || ch === "{") {
				stack.push({ ch, offset: i });
			} else if (ch === ")" || ch === "]" || ch === "}") {
				const open = stack.pop();
				if (!open || open.ch !== pairs[ch]) {
					this.pushAt(
						document,
						i,
						i + 1,
						`Unmatched closing '${ch}'.`,
						diagnostics,
					);
				}
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
