"use client";

import { useCallback, useMemo, useRef, useState } from "react";
import { A2UIRenderer } from "./A2UIRenderer";
import { applyA2UIMessage } from "./apply-a2ui-message";
import { resolveElementUpdateSurfaceId } from "./fold-surfaces";
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
						dataModel: message.dataModel,
						catalogId: message.catalogId,
					});
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
				case "surfaceUpdate": {
					// Delegate the component merge, but keep clearing the optimistic
					// pending updates that only this surface manager tracks.
					const existing = next.get(message.surfaceId);
					if (existing) {
						for (const comp of message.components) {
							const updateKey = `${message.surfaceId}/${comp.id}`;
							setPendingUpdates((p) => {
								const updated = new Map(p);
								updated.delete(updateKey);
								return updated;
							});
						}
						next.set(message.surfaceId, applyA2UIMessage(existing, message));
					}
					break;
				}
				case "upsertElement": {
					const surfaceId = resolveElementUpdateSurfaceId(
						next,
						message.element_id,
					);
					if (!surfaceId) break;
					const existing = next.get(surfaceId);
					if (!existing) break;
					next.set(surfaceId, applyA2UIMessage(existing, message));
					break;
				}
				case "setCanvasSettings":
				case "dataModelUpdate":
				case "createElement":
				case "removeElement": {
					const existing = next.get(message.surfaceId);
					if (!existing) break;
					next.set(message.surfaceId, applyA2UIMessage(existing, message));
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
