import * as vscode from "vscode";
import { FlowLinter } from "./diagnostics";
import {
	DeclarationSymbolProvider,
	FlowCompletionProvider,
	FlowDecoratorCompletionProvider,
	FlowDefinitionProvider,
	FlowDocumentHighlightProvider,
	FlowDocumentSymbolProvider,
	FlowHoverProvider,
	FlowMemberCompletionProvider,
	FlowQuickFixProvider,
	FlowReferenceProvider,
	FlowRenameProvider,
	FlowSignatureHelpProvider,
	FlowWorkspaceSymbolProvider,
} from "./providers";
import { SignatureRegistry } from "./signatures";

const FLOW = "flowscript";
const FLOW_D = "flowscript-declaration";

export async function activate(
	context: vscode.ExtensionContext,
): Promise<void> {
	const registry = new SignatureRegistry();
	const linter = new FlowLinter(registry);
	context.subscriptions.push(linter);

	await loadDeclarations(registry);

	const lintAll = () => {
		for (const doc of vscode.workspace.textDocuments) {
			if (doc.languageId === FLOW) {
				linter.lint(doc);
			}
		}
	};
	lintAll();

	context.subscriptions.push(
		vscode.languages.registerCompletionItemProvider(
			FLOW,
			new FlowCompletionProvider(registry),
		),
		vscode.languages.registerCompletionItemProvider(
			FLOW,
			new FlowDecoratorCompletionProvider(),
			"@",
		),
		vscode.languages.registerCompletionItemProvider(
			FLOW,
			new FlowMemberCompletionProvider(registry),
			".",
		),
		vscode.languages.registerHoverProvider(
			FLOW,
			new FlowHoverProvider(registry),
		),
		vscode.languages.registerSignatureHelpProvider(
			FLOW,
			new FlowSignatureHelpProvider(registry),
			"(",
			",",
		),
		vscode.languages.registerDocumentSymbolProvider(
			FLOW,
			new FlowDocumentSymbolProvider(),
		),
		vscode.languages.registerDocumentSymbolProvider(
			FLOW_D,
			new DeclarationSymbolProvider(registry),
		),
		vscode.languages.registerDefinitionProvider(
			FLOW,
			new FlowDefinitionProvider(registry),
		),
		vscode.languages.registerReferenceProvider(
			FLOW,
			new FlowReferenceProvider(),
		),
		vscode.languages.registerDocumentHighlightProvider(
			FLOW,
			new FlowDocumentHighlightProvider(),
		),
		vscode.languages.registerRenameProvider(
			FLOW,
			new FlowRenameProvider(registry),
		),
		vscode.languages.registerWorkspaceSymbolProvider(
			new FlowWorkspaceSymbolProvider(registry),
		),
		vscode.languages.registerCodeActionsProvider(
			FLOW,
			new FlowQuickFixProvider(registry),
			{ providedCodeActionKinds: FlowQuickFixProvider.providedKinds },
		),
	);

	// Keep declarations and diagnostics fresh as files change.
	context.subscriptions.push(
		vscode.workspace.onDidChangeTextDocument((e) => {
			if (e.document.languageId === FLOW) {
				linter.lint(e.document);
			} else if (e.document.languageId === FLOW_D) {
				registry.ingest(e.document.uri, e.document.getText());
				lintAll();
			}
		}),
		vscode.workspace.onDidOpenTextDocument((doc) => {
			if (doc.languageId === FLOW) {
				linter.lint(doc);
			}
		}),
		vscode.workspace.onDidCloseTextDocument((doc) => {
			if (doc.languageId === FLOW) {
				linter.clear(doc.uri);
			}
		}),
	);

	const watcher = vscode.workspace.createFileSystemWatcher("**/*.flow.d");
	const refresh = async (uri: vscode.Uri) => {
		await ingestFile(registry, uri);
		lintAll();
	};
	watcher.onDidCreate(refresh);
	watcher.onDidChange(refresh);
	watcher.onDidDelete((uri) => {
		registry.removeSource(uri);
		lintAll();
	});
	context.subscriptions.push(watcher);

	const schemaWatcher = vscode.workspace.createFileSystemWatcher(
		"**/*.flow.schemas.json",
	);
	const refreshSchemas = async () => {
		await loadSchemaSidecars(registry);
		lintAll();
	};
	schemaWatcher.onDidCreate(refreshSchemas);
	schemaWatcher.onDidChange(refreshSchemas);
	schemaWatcher.onDidDelete(refreshSchemas);
	context.subscriptions.push(schemaWatcher);

	context.subscriptions.push(
		vscode.commands.registerCommand(
			"flow-like.reloadDeclarations",
			async () => {
				registry.clear();
				await loadDeclarations(registry);
				lintAll();
				vscode.window.showInformationMessage(
					`Flow-Like: loaded ${registry.size} node declarations.`,
				);
			},
		),
	);
}

async function loadDeclarations(registry: SignatureRegistry): Promise<void> {
	const files = await vscode.workspace.findFiles(
		"**/*.flow.d",
		"**/node_modules/**",
	);
	await Promise.all(files.map((uri) => ingestFile(registry, uri)));
	await loadSchemaSidecars(registry);
}

async function loadSchemaSidecars(registry: SignatureRegistry): Promise<void> {
	const files = await vscode.workspace.findFiles(
		"**/*.flow.schemas.json",
		"**/node_modules/**",
	);
	await Promise.all(files.map((uri) => ingestSchemaFile(registry, uri)));
}

async function ingestSchemaFile(
	registry: SignatureRegistry,
	uri: vscode.Uri,
): Promise<void> {
	try {
		const bytes = await vscode.workspace.fs.readFile(uri);
		registry.ingestSchemas(new TextDecoder().decode(bytes));
	} catch {
		// Ignore unreadable files.
	}
}

async function ingestFile(
	registry: SignatureRegistry,
	uri: vscode.Uri,
): Promise<void> {
	try {
		const bytes = await vscode.workspace.fs.readFile(uri);
		registry.ingest(uri, new TextDecoder().decode(bytes));
	} catch {
		// Ignore unreadable files.
	}
}

export function deactivate(): void {}
