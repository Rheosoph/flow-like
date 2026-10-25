"use client";

import { useTranslation } from "@flow-like/locales";
import { CheckIcon, CodeIcon, CopyIcon, EyeIcon } from "lucide-react";
import {
	type CSSProperties,
	memo,
	useCallback,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import {
	type AssetSourceResolver,
	AssetSourceResolverContext,
} from "../../../hooks/use-asset-source";
import {
	ExecutionServiceContext,
	type ExecutionServiceContextValue,
} from "../../../state/execution-service-context-value";
import { A2UIRenderer } from "../../a2ui/A2UIRenderer";
import {
	CHAT_EXTERNAL_LINK_ACTION,
	CHAT_SEND_MESSAGE_ACTION,
	type ChatSurfaceBuilder,
	type ChatSurfaceState,
	authoredInputValue,
	changedInputValue,
	createChatSurfaceBuilder,
	formatChatInputValue,
	interpolateChatMessage,
	isAllowedChatAssetSource,
	openExternalUrl,
} from "../../a2ui/authoring";
import type { A2UIClientMessage, Surface } from "../../a2ui/types";
import { useChatFollowUp } from "../../interfaces/chat-default/chat-follow-up";
import { Button } from "../../ui/button";
import { Skeleton } from "../../ui/skeleton";
import { IsStreamingMarkdownContext } from "../renderer-presentation";
import { A2UIFenceContext } from "./a2ui-fence-context";

/** Fixed positioning, z-index and overflow from model-authored classes stay inside the bubble. */
const SANDBOX_STYLE: CSSProperties = {
	position: "relative",
	contain: "layout paint",
	overflow: "clip",
	isolation: "isolate",
};

/** A double click must not post the same follow-up twice. */
const DUPLICATE_SEND_WINDOW_MS = 800;

const rejectExecution = () =>
	Promise.reject(new Error("A chat reply's interface cannot run workflows."));

/** Defense in depth: workflow actions are stripped, and nothing below may start a run. */
const NO_EXECUTION: ExecutionServiceContextValue = {
	executeBoard: rejectExecution,
	executeBoardRemote: rejectExecution,
	executeEvent: rejectExecution,
	executeBoardDirect: rejectExecution,
	executeEventDirect: rejectExecution,
};

const resolveChatAsset: AssetSourceResolver = (source) =>
	isAllowedChatAssetSource(source)
		? Promise.resolve(source.trim())
		: Promise.reject(
				new Error(
					"A chat reply's interface loads media from https or inline images only.",
				),
			);

const ignoreNavigation = () => {};

const ChatSurface = memo(function ChatSurface({
	surface,
	onMessage,
}: Readonly<{
	surface: Surface;
	onMessage: (message: A2UIClientMessage) => void;
}>) {
	return (
		<IsStreamingMarkdownContext.Provider value={false}>
			<A2UIFenceContext.Provider value={true}>
				<ExecutionServiceContext.Provider value={NO_EXECUTION}>
					<AssetSourceResolverContext.Provider value={resolveChatAsset}>
						<div data-fl-a2ui-surface="" style={SANDBOX_STYLE}>
							<A2UIRenderer
								surface={surface}
								isPreviewMode
								onMessage={onMessage}
								onNavigationMessage={ignoreNavigation}
							/>
						</div>
					</AssetSourceResolverContext.Provider>
				</ExecutionServiceContext.Provider>
			</A2UIFenceContext.Provider>
		</IsStreamingMarkdownContext.Provider>
	);
});

function FenceSkeleton({ label }: Readonly<{ label: string }>) {
	return (
		<div data-slot="a2ui-fence-skeleton" className="space-y-2">
			<output className="sr-only">{label}</output>
			<Skeleton aria-hidden="true" className="h-5 w-2/5 bg-muted" />
			<Skeleton aria-hidden="true" className="h-16 w-full bg-muted" />
		</div>
	);
}

function FenceActions({
	content,
	showSource,
	canToggle,
	onToggle,
}: Readonly<{
	content: string;
	showSource: boolean;
	canToggle: boolean;
	onToggle: () => void;
}>) {
	const { t } = useTranslation("common");
	const [copied, setCopied] = useState(false);
	useEffect(() => {
		if (!copied) return;
		const timer = setTimeout(() => setCopied(false), 2000);
		return () => clearTimeout(timer);
	}, [copied]);
	const copy = useCallback(() => {
		const clipboard =
			typeof navigator === "undefined" ? undefined : navigator.clipboard;
		if (!clipboard) return;
		void clipboard.writeText(content).then(
			() => setCopied(true),
			() => undefined,
		);
	}, [content]);
	return (
		<div data-slot="a2ui-fence-actions" className="mt-1 flex justify-end gap-1">
			{canToggle && (
				<Button
					type="button"
					variant="ghost"
					size="sm"
					className="h-7 gap-1.5 px-2 text-xs text-muted-foreground"
					aria-pressed={showSource}
					onClick={onToggle}
				>
					{showSource ? (
						<EyeIcon aria-hidden="true" className="size-3.5" />
					) : (
						<CodeIcon aria-hidden="true" className="size-3.5" />
					)}
					{showSource
						? t("showPreview", "Show preview")
						: t("viewSource", "View source")}
				</Button>
			)}
			<Button
				type="button"
				variant="ghost"
				size="sm"
				className="h-7 gap-1.5 px-2 text-xs text-muted-foreground"
				onClick={copy}
			>
				{copied ? (
					<CheckIcon aria-hidden="true" className="size-3.5" />
				) : (
					<CopyIcon aria-hidden="true" className="size-3.5" />
				)}
				{copied ? t("copied", "Copied!") : t("copy", "Copy")}
			</Button>
		</div>
	);
}

/**
 * A model-authored, sandboxed a2ui surface inside a chat reply (```` ```a2ui ````).
 * Re-rendered per token while the fence streams; each closed object appears as
 * soon as it arrives.
 */
export default function A2UICodeBlock({
	content,
	isStreaming = false,
}: Readonly<{ content: string; isStreaming?: boolean }>) {
	const { t } = useTranslation("common");
	const surfaceId = `a2ui-chat-${useId()}`;
	const followUp = useChatFollowUp();
	const builderRef = useRef<ChatSurfaceBuilder | null>(null);
	builderRef.current ??= createChatSurfaceBuilder(surfaceId);
	const builder = builderRef.current;
	const canSendMessages = followUp !== null;
	const state = useMemo(
		() => builder.build(content, { streaming: isStreaming, canSendMessages }),
		[builder, content, isStreaming, canSendMessages],
	);
	const [showSource, setShowSource] = useState(false);

	const stateRef = useRef<ChatSurfaceState>(state);
	const followUpRef = useRef(followUp);
	useEffect(() => {
		stateRef.current = state;
		followUpRef.current = followUp;
	}, [state, followUp]);
	const inputValuesRef = useRef(new Map<string, unknown>());
	const lastSendRef = useRef({ text: "", at: 0 });

	const sendMessage = useCallback((template: unknown) => {
		const send = followUpRef.current;
		if (!send || typeof template !== "string") return;
		const { surface, data } = stateRef.current;
		const values = inputValuesRef.current;
		const text = interpolateChatMessage(template, (id) => {
			const component = surface?.components[id];
			if (!component) return undefined;
			return formatChatInputValue(
				values.has(id) ? values.get(id) : authoredInputValue(component, data),
			);
		}).trim();
		const now = Date.now();
		const last = lastSendRef.current;
		if (
			!text ||
			(last.text === text && now - last.at < DUPLICATE_SEND_WINDOW_MS)
		)
			return;
		lastSendRef.current = { text, at: now };
		send(text);
	}, []);

	const handleMessage = useCallback(
		(message: A2UIClientMessage) => {
			const context = message.context ?? {};
			switch (message.name) {
				case "change":
					if (message.sourceComponentId) {
						inputValuesRef.current.set(
							message.sourceComponentId,
							changedInputValue(context),
						);
					}
					return;
				case CHAT_SEND_MESSAGE_ACTION:
					sendMessage(context.message);
					return;
				case CHAT_EXTERNAL_LINK_ACTION:
					openExternalUrl(context.url);
					return;
			}
		},
		[sendMessage],
	);
	const toggleSource = useCallback(() => setShowSource((open) => !open), []);

	const { surface, pending } = state;
	const busy = isStreaming && (pending || !surface);
	const unrenderable = !surface && !isStreaming;
	return (
		<section
			data-fl-a2ui-fence=""
			data-source-open={showSource || unrenderable || undefined}
			aria-busy={busy || undefined}
			aria-label={t("interactiveAnswer", "Interactive answer")}
			className="my-3 min-w-0"
		>
			{unrenderable && (
				<p
					data-slot="a2ui-fence-error"
					className="mb-2 text-sm text-muted-foreground"
				>
					{t(
						"thisInterfaceCouldNotBeShown",
						"This interface could not be shown.",
					)}
				</p>
			)}
			{showSource || unrenderable ? (
				<pre
					data-slot="a2ui-fence-source"
					className="max-h-96 overflow-auto rounded-lg bg-muted/40 p-4 text-sm"
				>
					<code>{content}</code>
				</pre>
			) : surface ? (
				<ChatSurface surface={surface} onMessage={handleMessage} />
			) : (
				<FenceSkeleton label={t("buildingPreview", "Building preview")} />
			)}
			{busy && surface && !showSource && (
				<output className="sr-only">
					{t("buildingPreview", "Building preview")}
				</output>
			)}
			<FenceActions
				content={content}
				showSource={showSource}
				canToggle={!unrenderable}
				onToggle={toggleSource}
			/>
		</section>
	);
}
