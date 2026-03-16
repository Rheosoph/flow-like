"use client";

import { useCallback, useMemo, useRef, useState } from "react";
import { A2UIRenderer } from "./A2UIRenderer";
import type {
	A2UIClientMessage,
	A2UIServerMessage,
	Surface,
	SurfaceComponent,
} from "./types";

export interface SurfaceManagerProps {
	onSendMessage?: (message: A2UIClientMessage) => void;
	className?: string;
	appId?: string;
	renderSurface?: (
		surface: Surface,
		renderer: React.ReactNode,
	) => React.ReactNode;
	enableOptimisticUpdates?: boolean;
}

export interface OptimisticUpdate {
	surfaceId: string;
	componentId: string;
	changes: Partial<SurfaceComponent["component"]>;
	timestamp: number;
	rollback?: SurfaceComponent["component"];
}

export function useSurfaceManager() {
	const [surfaces, setSurfaces] = useState<Map<string, Surface>>(new Map());
	const [pendingUpdates, setPendingUpdates] = useState<
		Map<string, OptimisticUpdate>
	>(new Map());
	const updateTimeoutRef = useRef<Map<string, ReturnType<typeof setTimeout>>>(
		new Map(),
	);

	const handleServerMessage = useCallback((message: A2UIServerMessage) => {
		setSurfaces((prev) => {
			const next = new Map(prev);

			switch (message.type) {
				case "beginRendering": {
					const componentsMap: Record<string, SurfaceComponent> = {};
					for (const comp of message.components) {
						componentsMap[comp.id] = comp;
					}
					next.set(message.surfaceId, {
						id: message.surfaceId,
						rootComponentId: message.rootComponentId,
						components: componentsMap,
						catalogId: message.catalogId,
					});
					break;
				}

				case "setCanvasSettings": {
					const existing = next.get(message.surfaceId);
					if (!existing) break;

					next.set(message.surfaceId, {
						...existing,
						canvasSettings: {
							...existing.canvasSettings,
							...message.canvasSettings,
						},
					});
					break;
				}

				case "surfaceUpdate": {
					const existing = next.get(message.surfaceId);
					if (existing) {
						const updatedComponents = { ...existing.components };
						for (const comp of message.components) {
							updatedComponents[comp.id] = comp;
							// Clear any pending optimistic updates for this component
							const updateKey = `${message.surfaceId}/${comp.id}`;
							setPendingUpdates((p) => {
								const updated = new Map(p);
								updated.delete(updateKey);
								return updated;
							});
						}
						next.set(message.surfaceId, {
							...existing,
							components: updatedComponents,
						});
					}
					break;
				}

				case "dataModelUpdate": {
					// Handle data model updates if needed
					break;
				}

				case "deleteSurface": {
					next.delete(message.surfaceId);
					// Clear any pending updates for this surface
					setPendingUpdates((p) => {
						const updated = new Map(p);
						for (const key of updated.keys()) {
							if (key.startsWith(`${message.surfaceId}/`)) {
								updated.delete(key);
							}
						}
						return updated;
					});
					break;
				}

				case "upsertElement": {
					// Handle element updates from workflows
					// elementId format: "surfaceId/componentId" or just "componentId"
					const { element_id, value } = message;
					const [surfaceId, componentId] = element_id.includes("/")
						? element_id.split("/", 2)
						: [Array.from(next.keys())[0], element_id];

					if (!surfaceId) break;

					const surface = next.get(surfaceId);
					if (!surface) break;

					const component = surface.components[componentId];

					// Create new component if it doesn't exist and value has createComponent type
					if (!component) {
						const updateValue = value as Record<string, unknown>;
						if (updateValue?.type === "createComponent") {
							const newComponent: SurfaceComponent = {
								id: componentId,
								component: updateValue.component as SurfaceComponent["component"],
								style: updateValue.style as SurfaceComponent["style"],
							};
							next.set(surfaceId, {
								...surface,
								components: {
									...surface.components,
									[componentId]: newComponent,
								},
							});
						}
						break;
					}

					const updateValue = value as Record<string, unknown>;
					const updateType = updateValue?.type as string;

					let updatedComponent: SurfaceComponent = { ...component };

					switch (updateType) {
						case "setText": {
							const text = updateValue.text as string;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							updatedComponent = {
								...component,
								component: {
									...componentData,
									content: text,
									text: text,
									label: text,
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "setValue": {
							const value = updateValue.value as string;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							updatedComponent = {
								...component,
								component: {
									...componentData,
									value,
									defaultValue: value,
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "setStyle": {
							const newStyle = updateValue.style as Partial<
								SurfaceComponent["style"]
							>;
							updatedComponent = {
								...component,
								style: {
									...component.style,
									...newStyle,
								},
							};
							break;
						}
						case "setVisibility": {
							const visible = updateValue.visible as boolean;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							updatedComponent = {
								...component,
								component: {
									...componentData,
									hidden: !visible,
								} as unknown as SurfaceComponent["component"],
								style: {
									...component.style,
									opacity: visible ? undefined : 0,
								},
							};
							break;
						}
						case "setDisabled": {
							const disabled = updateValue.disabled as boolean;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							updatedComponent = {
								...component,
								component: {
									...componentData,
									disabled,
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "setLoading": {
							const loading = updateValue.loading as boolean;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							updatedComponent = {
								...component,
								component: {
									...componentData,
									loading,
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "setAction": {
							const action = updateValue.action as {
								name: string;
								context: Record<string, unknown>;
							} | null;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							updatedComponent = {
								...component,
								component: {
									...componentData,
									actions: action ? [action] : undefined,
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "setPlaceholder": {
							const placeholder = updateValue.placeholder as string;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							updatedComponent = {
								...component,
								component: {
									...componentData,
									placeholder,
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "setChecked": {
							const checked = updateValue.checked as boolean;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							updatedComponent = {
								...component,
								component: {
									...componentData,
									checked,
									value: checked,
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "setGeoMapViewport": {
							const viewport = updateValue.viewport as
								| { literalJson?: string }
								| undefined;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							updatedComponent = {
								...component,
								component: {
									...componentData,
									viewport,
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "setChartData": {
							const data = updateValue.data;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							updatedComponent = {
								...component,
								component: {
									...componentData,
									data,
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "setChartLayout": {
							const layout = updateValue.layout;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							updatedComponent = {
								...component,
								component: {
									...componentData,
									layout,
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "setProgress": {
							const value = updateValue.value as number;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							updatedComponent = {
								...component,
								component: {
									...componentData,
									value,
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "setImageSrc": {
							const src = updateValue.src as string;
							const alt = updateValue.alt as string | undefined;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							updatedComponent = {
								...component,
								component: {
									...componentData,
									url: src,
									alt: alt ?? componentData.alt,
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "setSpeakerName": {
							const name = updateValue.name as string;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							updatedComponent = {
								...component,
								component: {
									...componentData,
									speakerName: name,
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "setSpeakerPortrait": {
							const portraitId = updateValue.portraitId as string;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							updatedComponent = {
								...component,
								component: {
									...componentData,
									speakerPortraitId: portraitId,
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "setTypewriter": {
							const enabled = updateValue.enabled as boolean;
							const speed = updateValue.speed as number | undefined;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							updatedComponent = {
								...component,
								component: {
									...componentData,
									typewriter: enabled,
									...(speed !== undefined && { typewriterSpeed: speed }),
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "setTableData": {
							const data = updateValue.data as unknown[];
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							updatedComponent = {
								...component,
								component: {
									...componentData,
									data: { literalOptions: data },
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "setTableColumns": {
							const columns = updateValue.columns as unknown[];
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							updatedComponent = {
								...component,
								component: {
									...componentData,
									columns: { literalOptions: columns },
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "addTableRow": {
							const row = updateValue.row as Record<string, unknown>;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							const dataValue = componentData.data as
								| { literalOptions?: unknown[] }
								| undefined;
							const existingData = dataValue?.literalOptions || [];
							updatedComponent = {
								...component,
								component: {
									...componentData,
									data: { literalOptions: [...existingData, row] },
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "removeTableRow": {
							const index = updateValue.index as number;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							const dataValue = componentData.data as
								| { literalOptions?: unknown[] }
								| undefined;
							const existingData = [...(dataValue?.literalOptions || [])];
							if (index >= 0 && index < existingData.length) {
								existingData.splice(index, 1);
							}
							updatedComponent = {
								...component,
								component: {
									...componentData,
									data: { literalOptions: existingData },
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "updateTableCell": {
							const rowIndex = updateValue.rowIndex as number;
							const column = updateValue.column as string;
							const cellValue = updateValue.value;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							const dataValue = componentData.data as
								| { literalOptions?: unknown[] }
								| undefined;
							const existingData = [
								...(dataValue?.literalOptions || []),
							] as Record<string, unknown>[];
							if (rowIndex >= 0 && rowIndex < existingData.length) {
								existingData[rowIndex] = {
									...existingData[rowIndex],
									[column]: cellValue,
								};
							}
							updatedComponent = {
								...component,
								component: {
									...componentData,
									data: { literalOptions: existingData },
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "pushChild": {
							const childId = updateValue.childId as string;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							const childrenData = componentData.children as
								| { explicitList?: string[] }
								| undefined;
							const existingChildren = childrenData?.explicitList || [];
							updatedComponent = {
								...component,
								component: {
									...componentData,
									children: { explicitList: [...existingChildren, childId] },
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "insertChildAt": {
							const childId = updateValue.childId as string;
							const index = updateValue.index as number;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							const childrenData = componentData.children as
								| { explicitList?: string[] }
								| undefined;
							const existingChildren = [...(childrenData?.explicitList || [])];
							const insertIndex = Math.max(
								0,
								Math.min(index, existingChildren.length),
							);
							existingChildren.splice(insertIndex, 0, childId);
							updatedComponent = {
								...component,
								component: {
									...componentData,
									children: { explicitList: existingChildren },
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "removeChildAt": {
							const index = updateValue.index as number;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							const childrenData = componentData.children as
								| { explicitList?: string[] }
								| undefined;
							const existingChildren = [...(childrenData?.explicitList || [])];
							if (index >= 0 && index < existingChildren.length) {
								existingChildren.splice(index, 1);
							}
							updatedComponent = {
								...component,
								component: {
									...componentData,
									children: { explicitList: existingChildren },
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "clearChildren": {
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							updatedComponent = {
								...component,
								component: {
									...componentData,
									children: { explicitList: [] },
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "setIframeSrc": {
							const src = updateValue.src as string;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							updatedComponent = {
								...component,
								component: {
									...componentData,
									src: { literalString: src },
									srcdoc: undefined,
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "setIframeSrcdoc": {
							const srcdoc = updateValue.srcdoc as string;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							updatedComponent = {
								...component,
								component: {
									...componentData,
									srcdoc: { literalString: srcdoc },
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						case "setProps": {
							const props = updateValue.props as Record<string, unknown>;
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							updatedComponent = {
								...component,
								component: {
									...componentData,
									...props,
								} as unknown as SurfaceComponent["component"],
							};
							break;
						}
						default: {
							// Generic update - merge value into component
							const componentData = component.component as unknown as Record<
								string,
								unknown
							>;
							updatedComponent = {
								...component,
								component: {
									...componentData,
									...updateValue,
								} as unknown as SurfaceComponent["component"],
							};
						}
					}

					next.set(surfaceId, {
						...surface,
						components: {
							...surface.components,
							[componentId]: updatedComponent,
						},
					});
					break;
				}

				case "createElement": {
					const { surfaceId, parentId, component, index } = message;
					const surface = next.get(surfaceId);
					if (!surface) break;

					// Add the new component to the surface
					const updatedComponents = {
						...surface.components,
						[component.id]: component,
					};

					// Update parent's children array inside component.children.explicitList
					const parent = surface.components[parentId];
					if (parent) {
						const parentComp = parent.component as unknown as Record<
							string,
							unknown
						>;
						const childrenData = parentComp.children as
							| { explicitList?: string[] }
							| undefined;
						const existingChildren = childrenData?.explicitList || [];
						const newChildren = [...existingChildren];

						if (
							index !== undefined &&
							index >= 0 &&
							index <= newChildren.length
						) {
							newChildren.splice(index, 0, component.id);
						} else {
							newChildren.push(component.id);
						}

						updatedComponents[parentId] = {
							...parent,
							component: {
								...parentComp,
								children: { explicitList: newChildren },
							} as SurfaceComponent["component"],
						};
					}

					next.set(surfaceId, {
						...surface,
						components: updatedComponents,
					});
					break;
				}

				case "removeElement": {
					const { surfaceId, elementId } = message;
					const surface = next.get(surfaceId);
					if (!surface) break;

					const updatedComponents = { ...surface.components };

					// Remove from any parent's children.explicitList array
					for (const [compId, comp] of Object.entries(updatedComponents)) {
						const compData = comp.component as unknown as Record<
							string,
							unknown
						>;
						const childrenData = compData.children as
							| { explicitList?: string[] }
							| undefined;
						if (childrenData?.explicitList?.includes(elementId)) {
							updatedComponents[compId] = {
								...comp,
								component: {
									...compData,
									children: {
										explicitList: childrenData.explicitList.filter(
											(id: string) => id !== elementId,
										),
									},
								} as SurfaceComponent["component"],
							};
						}
					}

					// Delete the component itself
					delete updatedComponents[elementId];

					next.set(surfaceId, {
						...surface,
						components: updatedComponents,
					});
					break;
				}
			}

			return next;
		});
	}, []);

	// Apply optimistic update immediately, auto-rollback after timeout
	const applyOptimisticUpdate = useCallback(
		(
			surfaceId: string,
			componentId: string,
			changes: Partial<SurfaceComponent["component"]>,
			rollbackMs = 5000,
		) => {
			const updateKey = `${surfaceId}/${componentId}`;

			setSurfaces((prev) => {
				const surface = prev.get(surfaceId);
				if (!surface) return prev;

				const component = surface.components[componentId];
				if (!component) return prev;

				// Store rollback data
				const update: OptimisticUpdate = {
					surfaceId,
					componentId,
					changes,
					timestamp: Date.now(),
					rollback: component.component,
				};

				setPendingUpdates((p) => new Map(p).set(updateKey, update));

				// Apply optimistic update
				const next = new Map(prev);
				next.set(surfaceId, {
					...surface,
					components: {
						...surface.components,
						[componentId]: {
							...component,
							component: {
								...component.component,
								...changes,
							} as SurfaceComponent["component"],
						},
					},
				});

				// Set auto-rollback timeout
				const existingTimeout = updateTimeoutRef.current.get(updateKey);
				if (existingTimeout) clearTimeout(existingTimeout);

				const timeout = setTimeout(() => {
					setPendingUpdates((p) => {
						const current = p.get(updateKey);
						if (current && current.timestamp === update.timestamp) {
							// Rollback if server hasn't confirmed
							setSurfaces((s) => {
								const surf = s.get(surfaceId);
								if (!surf || !current.rollback) return s;

								const updated = new Map(s);
								updated.set(surfaceId, {
									...surf,
									components: {
										...surf.components,
										[componentId]: {
											...surf.components[componentId],
											component: current.rollback,
										},
									},
								});
								return updated;
							});

							const newPending = new Map(p);
							newPending.delete(updateKey);
							return newPending;
						}
						return p;
					});
				}, rollbackMs);

				updateTimeoutRef.current.set(updateKey, timeout);

				return next;
			});
		},
		[],
	);

	const getSurface = useCallback(
		(surfaceId: string): Surface | undefined => surfaces.get(surfaceId),
		[surfaces],
	);

	const getAllSurfaces = useCallback(
		(): Surface[] => Array.from(surfaces.values()),
		[surfaces],
	);

	const clearSurfaces = useCallback(() => {
		setSurfaces(new Map());
		setPendingUpdates(new Map());
		for (const timeout of updateTimeoutRef.current.values()) {
			clearTimeout(timeout);
		}
		updateTimeoutRef.current.clear();
	}, []);

	const hasPendingUpdate = useCallback(
		(surfaceId: string, componentId: string) =>
			pendingUpdates.has(`${surfaceId}/${componentId}`),
		[pendingUpdates],
	);

	return {
		surfaces,
		handleServerMessage,
		getSurface,
		getAllSurfaces,
		clearSurfaces,
		applyOptimisticUpdate,
		hasPendingUpdate,
		pendingUpdates,
	};
}

export function SurfaceManager({
	onSendMessage,
	className,
	appId,
	renderSurface,
	enableOptimisticUpdates = true,
}: SurfaceManagerProps) {
	const {
		surfaces,
		handleServerMessage,
		applyOptimisticUpdate,
		hasPendingUpdate,
	} = useSurfaceManager();

	const handleClientMessage = useCallback(
		(message: A2UIClientMessage) => {
			onSendMessage?.(message);
		},
		[onSendMessage],
	);

	const surfaceElements = useMemo(() => {
		const elements: React.ReactNode[] = [];

		surfaces.forEach((surface) => {
			const renderer = (
				<A2UIRenderer
					key={surface.id}
					surface={surface}
					onMessage={handleClientMessage}
					onA2UIMessage={handleServerMessage}
					className={className}
					appId={appId}
					isPreviewMode={true}
				/>
			);

			elements.push(
				renderSurface ? renderSurface(surface, renderer) : renderer,
			);
		});

		return elements;
	}, [
		surfaces,
		handleClientMessage,
		handleServerMessage,
		className,
		appId,
		renderSurface,
	]);

	return <>{surfaceElements}</>;
}
