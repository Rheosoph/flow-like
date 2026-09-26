"use client";

import { useBackend, useInvoke } from "@flow-like/flow-like-ui";
import type {
	PackageInspection,
	WasmExecutionResult,
	WasmNodeDefinition,
} from "@flow-like/flow-like-ui/lib/schema/developer";
import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import { countBySeverity, lintNodes } from "../../../../lib/validate-nodes";
import { inspectPackage } from "../../local-projects";
import {
	MODEL_BIT_TYPES,
	applySpecialInputDefaults,
	buildInitialInputValues,
	isDataPin,
	isModelBitPin,
	isSelectedBitValue,
} from "./schema";

/** A linked checkout, or a bare `.wasm` file (the `/developer/debug` picker). */
export interface NodeDebuggerTarget {
	projectPath?: string | null;
	wasmPath?: string | null;
}

export function nodeInspectionKey(target: NodeDebuggerTarget) {
	return target.projectPath
		? (["developer-inspection", "project", target.projectPath] as const)
		: (["developer-inspection", "wasm", target.wasmPath ?? ""] as const);
}

async function inspectTarget(
	target: NodeDebuggerTarget,
): Promise<PackageInspection> {
	if (target.projectPath) return inspectPackage(target.projectPath);
	const wasmPath = target.wasmPath ?? "";
	const nodes = await invoke<WasmNodeDefinition[]>("developer_inspect_node", {
		wasmPath,
	});
	return {
		nodes,
		manifest: null,
		isPackage: nodes.length > 1,
		wasmPath,
		widgets: [],
		widgetBundlePath: null,
	};
}

interface NodeInputs {
	node: string | null;
	values: Record<string, unknown>;
}

const NO_NODES: WasmNodeDefinition[] = [];
const NO_INPUTS: Record<string, unknown> = {};
const NO_SELECTION: NodeInputs = { node: null, values: NO_INPUTS };

export function useNodeDebugger({ projectPath, wasmPath }: NodeDebuggerTarget) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const target = useMemo<NodeDebuggerTarget>(
		() => (projectPath ? { projectPath } : { wasmPath }),
		[projectPath, wasmPath],
	);
	const hasTarget = Boolean(projectPath || wasmPath);

	const inspection = useQuery({
		queryKey: nodeInspectionKey(target),
		queryFn: () => inspectTarget(target),
		enabled: hasTarget,
		retry: false,
		meta: { persist: false },
	});
	const profileBits = useInvoke(
		backend.bitState.getProfileBits,
		backend.bitState,
		[],
	);
	const availableModelBits = useMemo(
		() =>
			(profileBits.data ?? []).filter((bit) => MODEL_BIT_TYPES.has(bit.type)),
		[profileBits.data],
	);

	const nodes = inspection.data?.nodes ?? NO_NODES;
	const resolvedWasmPath = inspection.data?.wasmPath ?? "";
	const [selectedName, setSelectedName] = useState<string | null>(null);
	const selectedIndex = Math.max(
		0,
		nodes.findIndex((node) => node.name === selectedName),
	);
	const selectedNode = nodes[selectedIndex] ?? null;
	const [inputs, setInputs] = useState<NodeInputs>(NO_SELECTION);
	const [result, setResult] = useState<WasmExecutionResult | null>(null);
	const [running, setRunning] = useState(false);

	const [shownTarget, setShownTarget] = useState(target);
	if (shownTarget !== target) {
		setShownTarget(target);
		setSelectedName(null);
		setInputs(NO_SELECTION);
		setResult(null);
	}

	useEffect(() => {
		if (!selectedNode) return;
		setInputs((prev) => {
			if (prev.node !== selectedNode.name) {
				return {
					node: selectedNode.name,
					values: buildInitialInputValues(selectedNode, availableModelBits),
				};
			}
			const values = applySpecialInputDefaults(
				selectedNode,
				prev.values,
				availableModelBits,
			);
			return values === prev.values ? prev : { node: prev.node, values };
		});
	}, [selectedNode, availableModelBits]);

	const inputValues =
		selectedNode && inputs.node === selectedNode.name
			? inputs.values
			: NO_INPUTS;

	const selectNode = useCallback(
		(index: number) => {
			const node = nodes[index];
			if (!node) return;
			setSelectedName(node.name);
			setResult(null);
			setInputs({
				node: node.name,
				values: buildInitialInputValues(node, availableModelBits),
			});
		},
		[nodes, availableModelBits],
	);

	const setInputValue = useCallback((name: string, value: unknown) => {
		setInputs((prev) => ({
			...prev,
			values: { ...prev.values, [name]: value },
		}));
	}, []);

	const inputPins = useMemo(
		() => selectedNode?.pins.filter((pin) => isDataPin(pin, "Input")) ?? [],
		[selectedNode],
	);
	const outputPins = useMemo(
		() => selectedNode?.pins.filter((pin) => isDataPin(pin, "Output")) ?? [],
		[selectedNode],
	);
	const lintIssues = useMemo(() => lintNodes(nodes), [nodes]);
	const lintCounts = useMemo(() => countBySeverity(lintIssues), [lintIssues]);
	const missingModelInput = useMemo(
		() =>
			inputPins.some(
				(pin) =>
					isModelBitPin(pin) && !isSelectedBitValue(inputValues[pin.name]),
			),
		[inputPins, inputValues],
	);

	const runNode = useCallback(async () => {
		if (!resolvedWasmPath || !selectedNode) return;
		setRunning(true);
		try {
			const res = await invoke<WasmExecutionResult>("developer_run_node", {
				input: {
					wasmPath: resolvedWasmPath,
					inputs: inputValues,
					nodeName: selectedNode.name,
				},
			});
			setResult(res);
			toast[res.error ? "error" : "success"](
				res.error
					? t("nodeErrorError", "Node error: {{error}}", { error: res.error })
					: t("nodeExecutedSuccessfully", "Node executed successfully"),
			);
		} catch (err) {
			toast.error(`Execution failed: ${err}`);
		} finally {
			setRunning(false);
		}
	}, [resolvedWasmPath, selectedNode, inputValues, t]);

	const { refetch } = inspection;
	const reinspect = useCallback(() => {
		setResult(null);
		void refetch();
	}, [refetch]);

	return {
		hasTarget,
		inspection: inspection.data,
		nodes,
		manifest: inspection.data?.manifest ?? null,
		isPackage: inspection.data?.isPackage ?? false,
		wasmPath: resolvedWasmPath,
		isInspecting: inspection.isLoading,
		isFetching: inspection.isFetching,
		error: inspection.error,
		inspectedAt: inspection.dataUpdatedAt,
		reinspect,
		selectedIndex,
		selectedNode,
		selectNode,
		inputPins,
		outputPins,
		inputValues,
		setInputValue,
		availableModelBits,
		missingModelInput,
		running,
		result,
		runNode,
		lintIssues,
		lintCounts,
	};
}

export type NodeDebuggerSession = ReturnType<typeof useNodeDebugger>;
