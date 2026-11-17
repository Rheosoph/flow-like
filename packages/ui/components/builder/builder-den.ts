import { createContext } from "react";
import {
	DEN_KEY_ATTRIBUTE,
	DEN_TARGET_ATTRIBUTE,
	denTargetProps,
} from "../../lib/bot-den/targets";
import type { IAppPointedAt } from "../../lib/pointed-at";

export interface BuilderDenScope {
	appId: string;
	pageId?: string;
	widgetId?: string;
	surfaceLabel: string;
}

export const BuilderDenContext = createContext<BuilderDenScope | null>(null);

export function builderSurfaceTarget(scope: BuilderDenScope): IAppPointedAt {
	const isPage = !!scope.pageId;
	return {
		kind: isPage ? "page" : "widget",
		app_id: scope.appId,
		id: isPage ? scope.pageId : scope.widgetId,
		label: scope.surfaceLabel,
	};
}

export function builderComponentTarget(
	scope: BuilderDenScope,
	componentId: string,
	label: string,
): IAppPointedAt {
	return {
		kind: "component",
		app_id: scope.appId,
		ids: [componentId],
		...(scope.pageId
			? { page_id: scope.pageId }
			: scope.widgetId
				? { widget_id: scope.widgetId }
				: {}),
		label,
	};
}

export function componentDisplayName(
	component: { type: string } & Record<string, unknown>,
	getWidgetRefName: (instanceId: string) => string | undefined,
): string {
	if (component.type === "widgetInstance") {
		const instanceId = component.instanceId;
		const name =
			typeof instanceId === "string" ? getWidgetRefName(instanceId) : undefined;
		return name ?? "Widget";
	}
	if (component.type === "microWidgetInstance") {
		return typeof component.widgetId === "string"
			? component.widgetId
			: "Package Widget";
	}
	return component.type;
}

export function setDenTargetAttributes(
	element: Element,
	target: IAppPointedAt | null,
): void {
	if (!target) {
		element.removeAttribute(DEN_TARGET_ATTRIBUTE);
		element.removeAttribute(DEN_KEY_ATTRIBUTE);
		return;
	}
	for (const [name, value] of Object.entries(denTargetProps(target))) {
		element.setAttribute(name, value);
	}
}
