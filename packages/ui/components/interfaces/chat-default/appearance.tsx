"use client";

import { useTranslation } from "@flow-like/locales";
import { type CSSProperties, type ReactNode, useId } from "react";
import { useAssetSource } from "../../../hooks/use-asset-source";
import { isStorageAssetPath } from "../../../lib/asset-url-cache";
import {
	createChatBackgroundImage,
	escapeCssAttributeValue,
	resolveChatColorScheme,
} from "../../../lib/chat-appearance";
import type { IEventPayloadChat } from "../../../lib/schema/flow/event-payload-chat";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { ScopedCustomCss } from "../../scoped-custom-css";

interface ChatAppearanceProps {
	appId?: string;
	eventId: string;
	config?: Partial<IEventPayloadChat>;
	children: ReactNode;
}

export function ChatAppearance({
	appId,
	eventId,
	config = {},
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
			<ScopedCustomCss
				css={config.custom_css}
				scopeSelector={customCssScope}
				options={{ scopeRoot: true }}
			/>
			{children}
		</div>
	);
}
