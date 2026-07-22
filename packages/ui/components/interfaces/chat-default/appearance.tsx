"use client";

import {
	type CSSProperties,
	type ReactNode,
	useEffect,
	useId,
	useState,
} from "react";
import {
	createChatBackgroundImage,
	escapeCssAttributeValue,
	resolveChatColorScheme,
} from "../../../lib/chat-appearance";
import {
	isStoragePrefix,
	presignSinglePath,
} from "../../../lib/presign-assets";
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
	const backend = useBackend();
	const instanceId = useId();
	const scopeKey = `${eventId || "default"}-${instanceId}`;
	const colorScheme = resolveChatColorScheme(config.color_scheme);
	const configuredBackgroundImage =
		typeof config.background_image === "string"
			? config.background_image.trim()
			: "";
	const storageBackedBackground =
		configuredBackgroundImage.length > 0 &&
		isStoragePrefix(configuredBackgroundImage);
	const [presignedBackground, setPresignedBackground] = useState<{
		appId: string;
		source: string;
		url?: string;
	} | null>(null);

	useEffect(() => {
		if (!storageBackedBackground || !appId) return;

		let cancelled = false;
		void presignSinglePath(
			appId,
			configuredBackgroundImage,
			backend.storageState,
		).then((url) => {
			if (cancelled) return;
			setPresignedBackground({
				appId,
				source: configuredBackgroundImage,
				url: url && !isStoragePrefix(url) ? url : undefined,
			});
		});

		return () => {
			cancelled = true;
		};
	}, [
		appId,
		backend.storageState,
		configuredBackgroundImage,
		storageBackedBackground,
	]);

	const resolvedBackgroundImage = storageBackedBackground
		? presignedBackground &&
			presignedBackground.appId === appId &&
			presignedBackground.source === configuredBackgroundImage
			? presignedBackground.url
			: undefined
		: configuredBackgroundImage || undefined;
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
