"use client";

import { useTranslation } from "@flow-like/locales";
import { type CSSProperties, type ReactNode, useId, useState } from "react";
import { useAssetSource } from "../../../hooks/use-asset-source";
import { isStorageAssetPath } from "../../../lib/asset-url-cache";
import {
	createChatBackgroundImage,
	escapeCssAttributeValue,
	resolveChatColorScheme,
} from "../../../lib/chat-appearance";
import { createSanitizedStyleProps } from "../../../lib/css-utils";
import type { IEventPayloadChat } from "../../../lib/schema/flow/event-payload-chat";
import { useScopedCss } from "../../../lib/use-scoped-css";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";

const CUSTOM_CSS_OPTIONS = { scopeRoot: true } as const;

interface ChatAppearanceProps {
	appId?: string;
	eventId: string;
	config?: Partial<IEventPayloadChat>;
	/** Shown in place of the chat until the event's custom stylesheet is attached. */
	fallback?: ReactNode;
	children: ReactNode;
}

export function ChatAppearance({
	appId,
	eventId,
	config = {},
	fallback = null,
	children,
}: Readonly<ChatAppearanceProps>) {
	const { t } = useTranslation("chat");
	const backend = useBackend();
	const instanceId = useId();
	const scopeKey = `${eventId || "default"}-${instanceId}`;
	const colorScheme = resolveChatColorScheme(config.color_scheme);
	const configuredBackgroundImage =
		typeof config.background_image === "string"
			? config.background_image.trim()
			: "";
	const { src: resolvedBackground } = useAssetSource(
		appId,
		configuredBackgroundImage || undefined,
	);
	// A storage path that came back unresolved has nothing to paint.
	const resolvedBackgroundImage = isStorageAssetPath(resolvedBackground)
		? undefined
		: resolvedBackground;
	const backgroundImage = createChatBackgroundImage(resolvedBackgroundImage);
	const customCssScope = `[data-fl-chat-root="${escapeCssAttributeValue(scopeKey)}"]`;
	const { css: customCss, pending: customCssPending } = useScopedCss(
		config.custom_css,
		customCssScope,
		CUSTOM_CSS_OPTIONS,
	);
	// Only the first paint waits for the stylesheet. A later edit swaps the sheet
	// under the live conversation rather than unmounting it.
	const [stylesheetAttached, setStylesheetAttached] = useState(
		!customCssPending,
	);
	if (!customCssPending && !stylesheetAttached) setStylesheetAttached(true);

	const style: CSSProperties = {
		backgroundImage,
		backgroundPosition: "var(--fl-chat-background-position)",
		backgroundSize: "var(--fl-chat-background-size)",
		backgroundRepeat: "no-repeat",
		colorScheme: colorScheme === "system" ? undefined : colorScheme,
	};

	return (
		<div
			className={cn(
				"fl-chat-root relative flex h-full min-h-0 w-full flex-1 flex-col overflow-hidden bg-background text-foreground",
				colorScheme === "dark" && "dark",
			)}
			data-fl-chat-root={scopeKey}
			data-fl-chat-color-scheme={
				colorScheme === "system" ? undefined : colorScheme
			}
			data-fl-chat-has-background={backgroundImage ? "true" : undefined}
			style={style}
		>
			{customCss ? <style {...createSanitizedStyleProps(customCss)} /> : null}
			{stylesheetAttached ? children : fallback}
		</div>
	);
}
