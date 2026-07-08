import { normalizeBoxes, resolveBoxesField } from "./bbox-utils";
import { applyMediaSourceUpdate } from "./media-source";
import { applyCalendarUpdate, applyGanttUpdate } from "./planning-updates";
import { applyStyleUpdate } from "./style-updates";
import type { A2UIServerMessage, Surface, SurfaceComponent } from "./types";

type ComponentData = Record<string, unknown>;

function getComponentData(component: SurfaceComponent): ComponentData {
	return component.component as unknown as ComponentData;
}

function withComponentData(
	component: SurfaceComponent,
	next: ComponentData,
): SurfaceComponent {
	return {
		...component,
		component: next as unknown as SurfaceComponent["component"],
	};
}

/**
 * Apply a single `upsertElement` update payload to a component and return the
 * updated component. Pure and shared by every a2ui surface reducer so the case
 * set can never drift between the runtime page, the surface manager and the
 * builder preview. New element-update message types belong here.
 */
export function applyElementUpdate(
	component: SurfaceComponent,
	updateValue: Record<string, unknown>,
): SurfaceComponent {
	const updateType = updateValue?.type as string;
	const data = getComponentData(component);

	switch (updateType) {
		case "setText": {
			const text = updateValue.text as string;
			return withComponentData(component, {
				...data,
				content: text,
				text,
				label: text,
			});
		}
		case "setValue": {
			const raw = updateValue.value;
			const bound =
				typeof raw === "string"
					? { literalString: raw }
					: typeof raw === "number"
						? { literalNumber: raw }
						: typeof raw === "boolean"
							? { literalBool: raw }
							: raw;
			return withComponentData(component, {
				...data,
				value: bound,
				defaultValue: bound,
			});
		}
		case "setStyle":
			return {
				...component,
				style: applyStyleUpdate(component.style, updateValue.style),
			};
		case "setVisibility": {
			const visible = updateValue.visible as boolean;
			return {
				...component,
				component: {
					...data,
					hidden: { literalBool: !visible },
				} as unknown as SurfaceComponent["component"],
				style: visible
					? applyStyleUpdate(component.style, { opacity: null })
					: component.style,
			};
		}
		case "setDisabled":
			return withComponentData(component, {
				...data,
				disabled: updateValue.disabled as boolean,
			});
		case "setLoading":
			return withComponentData(component, {
				...data,
				loading: updateValue.loading as boolean,
			});
		case "setAction": {
			const action = updateValue.action as {
				name: string;
				context: Record<string, unknown>;
			} | null;
			return withComponentData(component, {
				...data,
				actions: action ? [action] : undefined,
			});
		}
		case "setPlaceholder":
			return withComponentData(component, {
				...data,
				placeholder: updateValue.placeholder as string,
			});
		case "setChecked": {
			const checked = updateValue.checked as boolean;
			return withComponentData(component, { ...data, checked, value: checked });
		}
		case "setGeoMapViewport":
			return withComponentData(component, {
				...data,
				viewport: updateValue.viewport as { literalJson?: string } | undefined,
			});
		case "setChartData":
		case "setNivoData":
			return withComponentData(component, {
				...data,
				data: { literalJson: JSON.stringify(updateValue.data) },
			});
		case "setChartLayout":
		case "setNivoConfig": {
			const configOrLayout = updateValue.layout ?? updateValue.config;
			return withComponentData(component, {
				...data,
				...(updateValue.layout !== undefined && {
					layout: { literalJson: JSON.stringify(configOrLayout) },
				}),
				...(updateValue.config !== undefined && {
					config: { literalJson: JSON.stringify(configOrLayout) },
				}),
			});
		}
		case "setProgress":
			return withComponentData(component, {
				...data,
				value: updateValue.value as number,
			});
		case "setImageSrc": {
			const src = String(updateValue.src ?? updateValue.url ?? "");
			const alt = updateValue.alt as string | undefined;
			return withComponentData(component, {
				...data,
				src: { literalString: src },
				url: src,
				...(alt !== undefined && { alt: { literalString: alt } }),
			});
		}
		case "setMediaSource":
			return applyMediaSourceUpdate(component, updateValue);
		case "setSpeakerName":
			return withComponentData(component, {
				...data,
				speakerName: updateValue.name as string,
			});
		case "setSpeakerPortrait":
			return withComponentData(component, {
				...data,
				speakerPortraitId: updateValue.portraitId as string,
			});
		case "setTypewriter": {
			const enabled = updateValue.enabled as boolean;
			const speed = updateValue.speed as number | undefined;
			return withComponentData(component, {
				...data,
				typewriter: enabled,
				...(speed !== undefined && { typewriterSpeed: speed }),
			});
		}
		case "setTableData":
			return withComponentData(component, {
				...data,
				data: { literalOptions: updateValue.data as unknown[] },
			});
		case "setTableColumns":
			return withComponentData(component, {
				...data,
				columns: { literalOptions: updateValue.columns as unknown[] },
			});
		case "addTableRow": {
			const row = updateValue.row as Record<string, unknown>;
			const dataValue = data.data as { literalOptions?: unknown[] } | undefined;
			const existing = dataValue?.literalOptions || [];
			return withComponentData(component, {
				...data,
				data: { literalOptions: [...existing, row] },
			});
		}
		case "removeTableRow": {
			const index = updateValue.index as number;
			const dataValue = data.data as { literalOptions?: unknown[] } | undefined;
			const existing = [...(dataValue?.literalOptions || [])];
			if (index >= 0 && index < existing.length) existing.splice(index, 1);
			return withComponentData(component, {
				...data,
				data: { literalOptions: existing },
			});
		}
		case "updateTableCell": {
			const rowIndex = updateValue.rowIndex as number;
			const column = updateValue.column as string;
			const cellValue = updateValue.value;
			const dataValue = data.data as { literalOptions?: unknown[] } | undefined;
			const existing = [...(dataValue?.literalOptions || [])] as Record<
				string,
				unknown
			>[];
			if (rowIndex >= 0 && rowIndex < existing.length) {
				existing[rowIndex] = { ...existing[rowIndex], [column]: cellValue };
			}
			return withComponentData(component, {
				...data,
				data: { literalOptions: existing },
			});
		}
		case "setCalendarEvents":
		case "addCalendarEvent":
		case "updateCalendarEvent":
		case "removeCalendarEvent":
		case "setCalendarView":
		case "setCalendarDate":
		case "setCalendarConfig":
			return applyCalendarUpdate(component, updateValue);
		case "setGanttTasks":
		case "addGanttTask":
		case "updateGanttTask":
		case "removeGanttTask":
		case "setGanttProgress":
		case "addGanttDependency":
		case "removeGanttDependency":
		case "setGanttView":
		case "setGanttConfig":
			return applyGanttUpdate(component, updateValue);
		case "pushChild": {
			const childId = updateValue.childId as string;
			const childrenData = data.children as
				| { explicitList?: string[] }
				| undefined;
			const existing = childrenData?.explicitList || [];
			return withComponentData(component, {
				...data,
				children: { explicitList: [...existing, childId] },
			});
		}
		case "insertChildAt": {
			const childId = updateValue.childId as string;
			const index = updateValue.index as number;
			const childrenData = data.children as
				| { explicitList?: string[] }
				| undefined;
			const existing = [...(childrenData?.explicitList || [])];
			const insertIndex = Math.max(0, Math.min(index, existing.length));
			existing.splice(insertIndex, 0, childId);
			return withComponentData(component, {
				...data,
				children: { explicitList: existing },
			});
		}
		case "removeChildAt": {
			const index = updateValue.index as number;
			const childrenData = data.children as
				| { explicitList?: string[] }
				| undefined;
			const existing = [...(childrenData?.explicitList || [])];
			if (index >= 0 && index < existing.length) existing.splice(index, 1);
			return withComponentData(component, {
				...data,
				children: { explicitList: existing },
			});
		}
		case "clearChildren":
			return withComponentData(component, {
				...data,
				children: { explicitList: [] },
			});
		case "setIframeSrc":
			return withComponentData(component, {
				...data,
				src: { literalString: updateValue.src as string },
				srcdoc: undefined,
			});
		case "setIframeSrcdoc":
			return withComponentData(component, {
				...data,
				srcdoc: { literalString: updateValue.srcdoc as string },
			});
		case "setProps": {
			const props = updateValue.props as Record<string, unknown>;
			return withComponentData(component, { ...data, ...props });
		}
		case "setOverlayBoxes":
		case "setLabelerBoxes":
			return withComponentData(component, {
				...data,
				boxes: { literalOptions: normalizeBoxes(updateValue.boxes) },
			});
		case "addOverlayBox":
		case "addLabelerBox": {
			const existing = resolveBoxesField(data.boxes);
			const added = normalizeBoxes([updateValue.box]);
			return withComponentData(component, {
				...data,
				boxes: { literalOptions: [...existing, ...added] },
			});
		}
		case "clearOverlayBoxes":
			return withComponentData(component, {
				...data,
				boxes: { literalOptions: [] },
			});
		case "removeLabelerBox": {
			const boxId = updateValue.boxId as string;
			const remaining = resolveBoxesField(data.boxes).filter(
				(box) => box.id !== boxId,
			);
			return withComponentData(component, {
				...data,
				boxes: { literalOptions: remaining },
			});
		}
		case "updateLabelerBoxLabel": {
			const boxId = updateValue.boxId as string;
			const label = updateValue.label as string;
			const updated = resolveBoxesField(data.boxes).map((box) =>
				box.id === boxId ? { ...box, label } : box,
			);
			return withComponentData(component, {
				...data,
				boxes: { literalOptions: updated },
			});
		}
		case "setLabelerImage": {
			const src = updateValue.src as string;
			const alt = updateValue.alt as string | undefined;
			return withComponentData(component, {
				...data,
				src: { literalString: src },
				...(alt !== undefined ? { alt: { literalString: alt } } : {}),
			});
		}
		default: {
			const { type: _type, ...rest } = updateValue;
			return withComponentData(component, { ...data, ...rest });
		}
	}
}

/**
 * Apply a single a2ui server message to one surface and return the next surface
 * (unchanged if the message targets a different surface or is a non-surface
 * side effect such as navigation/dialogs). Map-level messages — `beginRendering`
 * and `deleteSurface` — and caller-specific concerns (navigation, optimistic
 * updates) stay in the individual reducers.
 */
export function applyA2UIMessage(
	surface: Surface,
	message: A2UIServerMessage,
): Surface {
	switch (message.type) {
		case "setCanvasSettings": {
			if (message.surfaceId !== surface.id) return surface;
			const filtered = Object.fromEntries(
				Object.entries(message.canvasSettings).filter(([, v]) => v != null),
			);
			return {
				...surface,
				canvasSettings: { ...surface.canvasSettings, ...filtered },
			};
		}
		case "surfaceUpdate": {
			if (message.surfaceId !== surface.id) return surface;
			const components = { ...surface.components };
			for (const comp of message.components) {
				components[comp.id] = comp;
			}
			return { ...surface, components };
		}
		case "dataModelUpdate": {
			if (message.surfaceId !== surface.id) return surface;
			const entries = new Map(
				(surface.dataModel ?? []).map((entry) => [entry.path, entry]),
			);
			for (const entry of message.contents) {
				entries.set(entry.path, entry);
			}
			return { ...surface, dataModel: Array.from(entries.values()) };
		}
		case "createElement": {
			if (message.surfaceId !== surface.id) return surface;
			const { parentId, component, index } = message;
			const components: Record<string, SurfaceComponent> = {
				...surface.components,
				[component.id]: component,
			};
			const parent = surface.components[parentId];
			if (parent) {
				const parentData = getComponentData(parent);
				const childrenData = parentData.children as
					| { explicitList?: string[] }
					| undefined;
				const newChildren = [...(childrenData?.explicitList || [])];
				if (index !== undefined && index >= 0 && index <= newChildren.length) {
					newChildren.splice(index, 0, component.id);
				} else {
					newChildren.push(component.id);
				}
				components[parentId] = {
					...parent,
					component: {
						...parentData,
						children: { explicitList: newChildren },
					} as unknown as SurfaceComponent["component"],
				};
			}
			return { ...surface, components };
		}
		case "removeElement": {
			if (message.surfaceId !== surface.id) return surface;
			const { elementId } = message;
			const components = { ...surface.components };
			for (const [compId, comp] of Object.entries(components)) {
				const compData = getComponentData(comp);
				const childrenData = compData.children as
					| { explicitList?: string[] }
					| undefined;
				if (childrenData?.explicitList?.includes(elementId)) {
					components[compId] = {
						...comp,
						component: {
							...compData,
							children: {
								explicitList: childrenData.explicitList.filter(
									(id) => id !== elementId,
								),
							},
						} as unknown as SurfaceComponent["component"],
					};
				}
			}
			delete components[elementId];
			return { ...surface, components };
		}
		case "upsertElement": {
			const { element_id, value } = message;
			if (!element_id) return surface;
			const [msgSurfaceId, componentId] = element_id.includes("/")
				? element_id.split("/", 2)
				: [surface.id, element_id];
			if (msgSurfaceId !== surface.id) return surface;

			const updateValue = (value ?? {}) as Record<string, unknown>;
			const component = surface.components[componentId];

			// createComponent creates the element if missing and replaces it if
			// present, so it is handled here rather than in applyElementUpdate.
			if (updateValue.type === "createComponent") {
				const newComponent: SurfaceComponent = {
					id: componentId,
					component: updateValue.component as SurfaceComponent["component"],
					style:
						(updateValue.style as SurfaceComponent["style"]) ??
						component?.style,
				};
				return {
					...surface,
					components: { ...surface.components, [componentId]: newComponent },
				};
			}

			if (!component) return surface;

			return {
				...surface,
				components: {
					...surface.components,
					[componentId]: applyElementUpdate(component, updateValue),
				},
			};
		}
		default:
			return surface;
	}
}
