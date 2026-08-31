"use client";

import { usePathname } from "next/navigation";
import {
	type ReactNode,
	createContext,
	useCallback,
	useContext,
	useState,
} from "react";
import { toast } from "sonner";
import {
	resolveEventBoardVersion,
	withBoardVersion,
} from "../../lib/schema/flow/board-version";
import {
	mayDispatchRawPageBoardAction,
	pageTriggerFromAction,
} from "../../lib/schema/flow/page-trigger";
import { useBackend } from "../../state/backend-state";
import { useExecutionServiceOptional } from "../../state/execution-service-context";
import {
	buildFrontendContextPayload,
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
	const executionService = useExecutionServiceOptional();
	const pathname = usePathname();
	const runtimeActionContext = useActionContext();
	const collectInputValues = useEventRelevantValues({
		instanceId: instance.instanceId,
	});
	const collectElements = useCollectEventElements({
		instanceId: instance.instanceId,
	});
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
					const pageAction = binding.pageAction;
					if (
						!pageAction &&
						!mayDispatchRawPageBoardAction(runtimeActionContext.isGovernedPage)
					) {
						console.warn(
							"[WidgetAction] Refusing a raw workflow binding on a governed Page.",
							{ actionId, surfaceId, triggeringComponentId },
						);
						toast.error(
							"This widget action is missing its execution authorization. Reload the Page.",
						);
						return;
					}

					const inputValues = collectInputValues();
					const elements = await collectElements();
					const payload: Record<string, unknown> = {
						_action_id: actionId,
						_widget_instance_id: instance.instanceId,
						_widget_id: instance.widgetId,
						_surface_id: surfaceId,
						_input_values: inputValues,
						_elements: elements,
						...buildFrontendContextPayload(
							pathname,
							runtimeActionContext.globalState,
							runtimeActionContext.pageState,
						),
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

						const baseRunPayload = {
							id: pageAction?.actionId ?? "widget_action",
							payload: compactPayload,
						};

						if (pageAction) {
							const eventId = runtimeActionContext.eventId;
							if (!eventId) {
								throw new Error(
									"Governed widget action is missing its Event id.",
								);
							}
							await (
								executionService?.executeEvent ??
								backend.eventState.executeEvent.bind(backend.eventState)
							)(
								appId,
								eventId,
								baseRunPayload,
								false,
								undefined,
								onA2UIEvents,
								undefined,
								pageTriggerFromAction(pageAction),
							);
						} else {
							const runPayload = withBoardVersion(
								baseRunPayload,
								resolveEventBoardVersion(
									runtimeActionContext.boardId,
									runtimeActionContext.boardVersion,
									flowId,
								),
							);

							await (
								executionService?.executeBoard ??
								backend.boardState.executeBoard
							)(appId, flowId, runPayload, false, undefined, onA2UIEvents);
						}
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
			backend,
			executionService,
			appId,
			surfaceId,
			instance,
			widgetActions,
			getBinding,
			onA2UIEvents,
			collectInputValues,
			collectElements,
			markComponentTriggering,
			pathname,
			runtimeActionContext.boardId,
			runtimeActionContext.boardVersion,
			runtimeActionContext.eventId,
			runtimeActionContext.isGovernedPage,
			runtimeActionContext.globalState,
			runtimeActionContext.pageState,
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
