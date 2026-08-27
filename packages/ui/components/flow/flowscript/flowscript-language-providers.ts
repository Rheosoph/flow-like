import type { Monaco } from "@monaco-editor/react";
import type { INode } from "../../../lib/schema/flow/node";
import { registerFlowScriptCoreProviders } from "./flowscript-language";
import { registerFlowScriptFeatureProviders } from "./flowscript-language-features";
import { flowScriptWorkerRequests } from "./flowscript-worker-client";

/**
 * Main-thread composition root for every FlowScript language provider.
 * Worker-backed requests are injected here so worker analysis modules never import
 * the client that constructs the worker itself.
 */
export function registerFlowScriptProviders(
	monaco: Monaco,
	getCatalogNodes: () => INode[] | undefined,
): { dispose: () => void } {
	const core = registerFlowScriptCoreProviders(
		monaco,
		getCatalogNodes,
		flowScriptWorkerRequests,
	);
	const features = registerFlowScriptFeatureProviders(
		monaco,
		getCatalogNodes,
		flowScriptWorkerRequests,
	);

	return {
		dispose: () => {
			core.dispose();
			features.dispose();
		},
	};
}
