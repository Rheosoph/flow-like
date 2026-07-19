"use client";

import {
	type ReactNode,
	createContext,
	useCallback,
	useContext,
	useState,
} from "react";
import {
	resolveEventBoardVersion,
	withBoardVersion,
} from "../../lib/schema/flow/board-version";
import { useBackend } from "../../state/backend-state";
import {
	compactWorkflowPayload,
	useActionContext,
	useCollectEventElements,
	useEventRelevantValues,
	useMarkComponentTriggering,
} from "./ActionHandler";
import type {
	ActionBinding,
	BoundValue,
	WidgetAction,
	WidgetInstance,
} from "./types";

export interface WidgetActionContextValue {
	instance: WidgetInstance | null;
	widgetActions: WidgetAction[];
	triggerAction: (
		actionId: string,
		context?: Record<string, unknown>,
		triggeringComponentId?: string,
	) => Promise<void>;
	getBinding: (actionId: string) => ActionBinding | null;
}

const WidgetActionContext = createContext<WidgetActionContextValue | null>(
	null,
);

export interface WidgetActionProviderProps {
	instance: WidgetInstance;
	widgetActions: WidgetAction[];
	appId: string;
	surfaceId: string;
	children: ReactNode;
	onA2UIEvents?: (events: unknown[]) => void;
}

function resolveBoundValue(
	mapping: BoundValue,
	context: Record<string, unknown>,
	fieldName: string,
): unknown {
	if ("literalString" in mapping) {
		return mapping.literalString;
	}
	if ("literalNumber" in mapping) {
		return mapping.literalNumber;
	}
	if ("literalBool" in mapping) {
		return mapping.literalBool;
	}
	if ("literalJson" in mapping) {
		try {
			return JSON.parse(mapping.literalJson);
		} catch {
			return mapping.literalJson;
		}
	}
	if ("literalOptions" in mapping) {
		return mapping.literalOptions;
	}
	if ("path" in mapping) {
		const path = mapping.path;
		if (path.startsWith("context.")) {
			return context[path.slice(8)];
		}
		if (path.startsWith("data.")) {
			return context[`data.${path.slice(5)}`];
		}
		if (path.startsWith("state.")) {
			return context[`state.${path.slice(6)}`];
		}
		return context[path] ?? context[fieldName];
	}
	return context[fieldName];
}

export function WidgetActionProvider({
	instance,
	widgetActions,
	appId,
	surfaceId,
	children,
	onA2UIEvents,
}: WidgetActionProviderProps) {
	const backend = useBackend();
	const runtimeActionContext = useActionContext();
	const collectInputValues = useEventRelevantValues();
	const collectElements = useCollectEventElements();
	const markComponentTriggering = useMarkComponentTriggering();

	const getBinding = useCallback(
		(actionId: string): ActionBinding | null => {
			return instance.actionBindings[actionId] ?? null;
		},
		[instance.actionBindings],
	);

	const triggerAction = useCallback(
		async (
			actionId: string,
			context: Record<string, unknown> = {},
			triggeringComponentId?: string,
		) => {
			const binding = getBinding(actionId);
			if (!binding) {
				console.warn(`[WidgetAction] No binding found for action: ${actionId}`);
				return;
			}

			const action = widgetActions.find((a) => a.id === actionId);
			if (!action) {
				console.warn(`[WidgetAction] Unknown action: ${actionId}`);
				return;
			}

			if (triggeringComponentId) {
				markComponentTriggering?.(triggeringComponentId, true);
			}

			try {
				if ("workflow" in binding) {
					const { flowId, inputMappings } = binding.workflow;

					const inputValues = collectInputValues();
					const elements = await collectElements();
					const payload: Record<string, unknown> = {
						_action_id: actionId,
						_widget_instance_id: instance.instanceId,
						_widget_id: instance.widgetId,
						_surface_id: surfaceId,
						_input_values: inputValues,
						_elements: elements,
					};

					for (const field of action.contextSchema) {
						const mapping = inputMappings?.[field.name];
						if (mapping) {
							payload[field.name] = resolveBoundValue(
								mapping,
								context,
								field.name,
							);
						} else {
							payload[field.name] = context[field.name];
						}
					}

					try {
						console.log("[WidgetAction] Executing workflow:", {
							appId,
							flowId,
							payload,
						});

						const compactPayload = compactWorkflowPayload(payload) as Record<
							string,
							unknown
						>;

						const runPayload = withBoardVersion(
							{
								id: "widget_action",
								payload: compactPayload,
							},
							resolveEventBoardVersion(
								runtimeActionContext.boardId,
								runtimeActionContext.boardVersion,
								flowId,
							),
						);

						await backend.boardState.executeBoard(
							appId,
							flowId,
							runPayload,
							false,
							undefined,
							onA2UIEvents,
						);
					} catch (error) {
						console.error("[WidgetAction] Failed to execute workflow:", error);
					}
				} else if ("command" in binding) {
					const { commandName, args } = binding.command;
					const resolvedArgs: Record<string, unknown> = {};
					for (const [key, value] of Object.entries(args)) {
						resolvedArgs[key] = resolveBoundValue(value, context, key);
					}
					console.log("[WidgetAction] Executing command:", {
						command: commandName,
						args: resolvedArgs,
					});
					window.dispatchEvent(
						new CustomEvent("a2ui:command", {
							detail: {
								command: commandName,
								args: resolvedArgs,
								context,
							},
						}),
					);
				}
			} finally {
				if (triggeringComponentId) {
					markComponentTriggering?.(triggeringComponentId, false);
				}
			}
		},
		[
			backend.boardState,
			appId,
			surfaceId,
			instance,
			widgetActions,
			getBinding,
			onA2UIEvents,
			collectInputValues,
			collectElements,
			markComponentTriggering,
			runtimeActionContext.boardId,
			runtimeActionContext.boardVersion,
		],
	);

	return (
		<WidgetActionContext.Provider
			value={{
				instance,
				widgetActions,
				triggerAction,
				getBinding,
			}}
		>
			{children}
		</WidgetActionContext.Provider>
	);
}

export function useWidgetActions(): WidgetActionContextValue {
	const context = useContext(WidgetActionContext);
	if (!context) {
		return {
			instance: null,
			widgetActions: [],
			triggerAction: async () => {},
			getBinding: () => null,
		};
	}
	return context;
}

export function useWidgetAction(actionId: string, componentId?: string) {
	const { triggerAction, getBinding, widgetActions } = useWidgetActions();
	const action = widgetActions.find((a) => a.id === actionId);
	const binding = getBinding(actionId);
	const [isLoading, setIsLoading] = useState(false);

	const trigger = useCallback(
		async (context?: Record<string, unknown>) => {
			setIsLoading(true);
			try {
				await triggerAction(actionId, context, componentId);
			} finally {
				setIsLoading(false);
			}
		},
		[triggerAction, actionId, componentId],
	);

	return {
		action,
		binding,
		trigger,
		isBound: !!binding,
		isLoading,
	};
}
