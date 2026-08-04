"use client";
import { ExternalLink, Plus, Sparkles, Target } from "lucide-react";
import { useMemo } from "react";
import type { LessonAction, LessonAppRef } from "../../lib/learn/types";
import { Button } from "../ui/button";

export type LessonActionDispatcher = (
	action: LessonAction,
) => Promise<void> | void;

interface LessonActionButtonProps {
	readonly appRef: LessonAppRef;
	readonly resolveAppId: (
		appAlias: string | null,
		fallbackAppId: string | null,
	) => string | null;
	readonly dispatch: LessonActionDispatcher;
}

const kindIcon = {
	NAVIGATE: ExternalLink,
	FOCUS_NODE: Target,
	ADD_NODE: Plus,
	CREATE_EVENT: Sparkles,
	OPEN_OR_CLONE_APP: ExternalLink,
} as const;

const defaultLabel: Record<string, string> = {
	NAVIGATE: "Open in app",
	FOCUS_NODE: "Show me the node",
	ADD_NODE: "Add this node",
	CREATE_EVENT: "Create event",
	OPEN_OR_CLONE_APP: "Open shared app",
};

export function buildLessonAction(
	appRef: LessonAppRef,
	resolveAppId: (
		appAlias: string | null,
		fallbackAppId: string | null,
	) => string | null,
): LessonAction | null {
	const appAlias = appRef.app_alias ?? undefined;
	const linkedAppId = resolveAppId(appRef.app_alias, null);
	const appId = linkedAppId ?? (appAlias ? null : appRef.app_id);
	const target = appRef.target as unknown as Record<string, unknown>;
	switch (appRef.kind) {
		case "NAVIGATE":
			if (!appId && !appAlias) return null;
			return {
				kind: "NAVIGATE",
				appId,
				appAlias,
				subpath: String(target.subpath ?? "config"),
				params: (target.params as Record<string, string>) ?? undefined,
			};
		case "FOCUS_NODE":
			if (!appId && !appAlias) return null;
			return {
				kind: "FOCUS_NODE",
				appId,
				appAlias,
				boardId: String(target.boardId ?? ""),
				nodeId: String(target.nodeId ?? ""),
			};
		case "ADD_NODE":
			if (!appId && !appAlias) return null;
			return {
				kind: "ADD_NODE",
				appId,
				appAlias,
				boardId: String(target.boardId ?? ""),
				nodeTypeId: String(target.nodeTypeId ?? ""),
				coords: target.coords as readonly [number, number] | undefined,
			};
		case "CREATE_EVENT":
			if (!appId && !appAlias) return null;
			return {
				kind: "CREATE_EVENT",
				appId,
				appAlias,
				template: (target.template as Record<string, unknown>) ?? {},
			};
		case "OPEN_OR_CLONE_APP":
			return {
				kind: "OPEN_OR_CLONE_APP",
				sharedAppId:
					typeof target.sharedAppId === "string"
						? target.sharedAppId
						: (appRef.app_id ?? null),
				alias: appAlias,
			};
	}
}

export function LessonActionButton({
	appRef,
	resolveAppId,
	dispatch,
}: LessonActionButtonProps) {
	const Icon = kindIcon[appRef.kind] ?? ExternalLink;
	const label = appRef.label ?? defaultLabel[appRef.kind] ?? "Open";

	const action = useMemo<LessonAction | null>(
		() => buildLessonAction(appRef, resolveAppId),
		[appRef, resolveAppId],
	);

	if (!action) {
		return (
			<Button variant="outline" size="sm" disabled>
				{label} (app not linked)
			</Button>
		);
	}

	return (
		<Button
			variant="outline"
			size="sm"
			onClick={() => {
				void dispatch(action);
			}}
		>
			<Icon className="h-3.5 w-3.5 mr-1.5" />
			{label}
		</Button>
	);
}
