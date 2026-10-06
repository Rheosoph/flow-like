"use client";

import { useTranslation } from "@flow-like/locales";
import { Eraser, FlaskConical, Send, Square } from "lucide-react";
import {
	type KeyboardEvent,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import type { HostedModel } from "../../../../../lib/device-management/models";
import { holdForModelUse } from "../../../../../lib/device-management/workspace/keys";
import type { BrowserFetch } from "../../../../../lib/service-runtime/transport";
import { gateView } from "../../device/use-device-page";
import { type DevicesT, useAreaTime } from "../../primitives/area-context";
import { DvButton } from "../../primitives/dv-button";
import { DvSheet } from "../../primitives/dv-sheet";
import { DvSelect, DvTextarea, Field } from "../../primitives/form-fields";
import { GatedAction } from "../../primitives/gate-notice";
import { InlineResult } from "../../primitives/inline-result";
import { StateView } from "../../primitives/state-view";
import { useAttentionState, useGate } from "../../workspace";
import { stateLabel } from "../models-copy";
import {
	type ChatMeasure,
	type ChatTurn,
	type EmbedMeasure,
	embedText,
	gatewayFetch,
	invokeSystemOne,
	modelGatewayOf,
	streamChat,
} from "./gateway-client";
import {
	chatMeasureText,
	embedMeasureText,
	failureText,
	vectorPreview,
} from "./playground-copy";

/*
 * The playground (plan §3.7): chat and embedding requests to a hosted model
 * through the device's model gateway, over the encrypted tunnel from this
 * browser or the desktop app. It shows what a caller gets: time to the first
 * token, tokens per second and the counts. While a request runs the keys'
 * idle lock waits; the requests count as the viewer's in the statistics.
 */

/** Models that can answer: everything but models still downloading their files. */
export function playgroundModels(models: readonly HostedModel[]) {
	const usable: HostedModel[] = [];
	for (const model of models)
		if (model.state !== "acquiring") usable.push(model);
	return usable;
}

const chatsNow = (model: HostedModel) =>
	model.state === "loaded" &&
	(model.kind === "chat" || model.kind === "vision");

/** The model to start with: the asked one, else a loaded chat model, else the first. */
function firstModel(models: readonly HostedModel[], asked?: string) {
	let loadedChat: HostedModel | undefined;
	for (const model of models) {
		if (model.id === asked) return model;
		if (!loadedChat && chatsNow(model)) loadedChat = model;
	}
	return loadedChat ?? models[0];
}

function useGatewayFetch(deviceId: string) {
	const { workspace } = useAttentionState();
	const { live, keys } = workspace;
	return useMemo(() => {
		const open = modelGatewayOf(live, deviceId);
		return open
			? gatewayFetch(open, () => holdForModelUse(keys, deviceId))
			: undefined;
	}, [live, keys, deviceId]);
}

/** Aborts what runs when the panel goes away. */
function useRunning() {
	const running = useRef<AbortController | undefined>(undefined);
	useEffect(() => () => running.current?.abort(), []);
	return running;
}

function Failure({
	error,
	device,
	onDismiss,
}: Readonly<{ error: unknown; device: string; onDismiss: () => void }>) {
	const { t } = useTranslation("devices");
	return (
		<InlineResult tone="critical" onDismiss={onDismiss}>
			{failureText(t, error, device)}
		</InlineResult>
	);
}

interface PanelProps {
	fetcher: BrowserFetch;
	model: HostedModel;
	device: string;
}

interface LiveAnswer {
	content: string;
	reasoning: string;
}

function Turn({
	who,
	text,
	reasoning,
	pending,
}: Readonly<{
	who: string;
	text: string;
	reasoning?: string;
	pending?: string;
}>) {
	const { t } = useTranslation("devices");
	return (
		<li data-turn={who} className="flex min-w-0 flex-col gap-1">
			<span className="text-xs font-medium text-ink-2">{who}</span>
			{reasoning ? (
				<p className="border-l-2 border-hairline pl-2 text-xs whitespace-pre-wrap text-muted-foreground">
					<span className="font-medium">
						{t("devices:models.playground.chat.thinking", "Thinking")}
					</span>{" "}
					{reasoning}
				</p>
			) : null}
			{text ? (
				<p className="text-sm whitespace-pre-wrap wrap-anywhere">{text}</p>
			) : (
				<p className="text-xs text-muted-foreground">{pending}</p>
			)}
		</li>
	);
}

/** The live answer's text before its first token: a model that isn't loaded loads first. */
function pendingText(t: DevicesT, model: HostedModel, device: string) {
	return model.state === "loaded"
		? t(
				"devices:models.playground.chat.waiting",
				"Waiting for the first token…",
			)
		: t(
				"devices:models.playground.chat.loading",
				"Waiting for {{device}} to load {{model}}. The first answer can take a few minutes.",
				{ device, model: model.display_name },
			);
}

/** Announced once a turn is complete: `aria-busy` holds the streaming answer back. */
function Transcript({
	model,
	device,
	turns,
	live,
}: Readonly<{
	model: HostedModel;
	device: string;
	turns: readonly ChatTurn[];
	live: LiveAnswer | null;
}>) {
	const { t } = useTranslation("devices");
	const you = t("devices:models.playground.chat.you", "You");
	if (!turns.length && !live)
		return (
			<p className="rounded-md border border-dashed border-hairline px-3 py-6 text-center text-xs text-muted-foreground">
				{t(
					"devices:models.playground.chat.empty",
					"Send a message to see how fast {{model}} answers.",
					{ model: model.display_name },
				)}
			</p>
		);
	return (
		<ol
			data-transcript=""
			aria-live="polite"
			aria-busy={live ? true : undefined}
			className="flex max-h-[45vh] flex-col gap-3 overflow-auto rounded-md border border-hairline bg-surface-sunken p-3"
		>
			{turns.map((turn, index) => (
				<Turn
					// biome-ignore lint/suspicious/noArrayIndexKey: turns only append or drop the last one
					key={index}
					who={turn.role === "user" ? you : model.display_name}
					text={turn.content}
				/>
			))}
			{live ? (
				<Turn
					who={model.display_name}
					text={live.content}
					reasoning={live.reasoning}
					pending={pendingText(t, model, device)}
				/>
			) : null}
		</ol>
	);
}

function MeasureLine({ text }: Readonly<{ text: string }>) {
	return (
		<p data-measure="" className="text-xs text-ink-2 tabular-nums">
			{text}
		</p>
	);
}

/** Cmd/Ctrl+Enter sends. */
const sendsOn = (event: KeyboardEvent<HTMLTextAreaElement>) =>
	event.key === "Enter" && (event.metaKey || event.ctrlKey);

/**
 * The conversation alternates user and model turns, as chat templates
 * require: a message that got no answer (refused, stopped, broken off before
 * the first token) leaves the transcript, and `send` hands its text back.
 */
function useChat(fetcher: BrowserFetch, model: HostedModel) {
	const running = useRunning();
	const [turns, setTurns] = useState<ChatTurn[]>([]);
	const [live, setLive] = useState<LiveAnswer | null>(null);
	const [measure, setMeasure] = useState<ChatMeasure>();
	const [failure, setFailure] = useState<unknown>();
	const [stopped, setStopped] = useState(false);
	const send = async (text: string): Promise<string | undefined> => {
		if (running.current) return undefined;
		const asked: ChatTurn = { role: "user", content: text };
		const messages: ChatTurn[] = [...turns, asked];
		const abort = new AbortController();
		running.current = abort;
		setTurns(messages);
		setLive({ content: "", reasoning: "" });
		setMeasure(undefined);
		setFailure(undefined);
		setStopped(false);
		const answer: LiveAnswer = { content: "", reasoning: "" };
		try {
			const result = await streamChat(fetcher, {
				model: model.id,
				messages,
				signal: abort.signal,
				onDelta: (delta) => {
					answer.content += delta.content ?? "";
					answer.reasoning += delta.reasoning ?? "";
					setLive({ ...answer });
				},
			});
			setMeasure(result);
		} catch (error) {
			if (abort.signal.aborted) setStopped(true);
			else setFailure(error);
		} finally {
			running.current = undefined;
			setLive(null);
		}
		const content = answer.content;
		setTurns((previous) => {
			if (content) return [...previous, { role: "assistant", content }];
			return previous.at(-1) === asked ? previous.slice(0, -1) : previous;
		});
		return content ? undefined : text;
	};
	const clear = () => {
		setTurns([]);
		setMeasure(undefined);
		setFailure(undefined);
		setStopped(false);
	};
	return {
		turns,
		live,
		measure,
		failure,
		stopped,
		busy: live !== null,
		send,
		stop: () => running.current?.abort(),
		clear,
		dismiss: () => setFailure(undefined),
	};
}

function ChatPanel({ fetcher, model, device }: Readonly<PanelProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const id = useId();
	const chat = useChat(fetcher, model);
	const [draft, setDraft] = useState("");
	const submit = () => {
		const text = draft.trim();
		if (!text || chat.busy) return;
		setDraft("");
		void chat.send(text).then((unanswered) => {
			if (unanswered)
				setDraft((typed) =>
					typed.trim() ? `${unanswered}\n\n${typed}` : unanswered,
				);
		});
	};
	return (
		<div data-playground="chat" className="flex min-w-0 flex-col gap-3">
			<Transcript
				model={model}
				device={device}
				turns={chat.turns}
				live={chat.live}
			/>
			{chat.measure ? (
				<MeasureLine text={chatMeasureText(t, time.locale, chat.measure)} />
			) : null}
			{chat.stopped ? (
				<p className="text-xs text-muted-foreground">
					{t("devices:models.playground.chat.stopped", "Stopped.")}
				</p>
			) : null}
			{chat.failure ? (
				<Failure
					error={chat.failure}
					device={device}
					onDismiss={chat.dismiss}
				/>
			) : null}
			<Field
				id={`${id}-message`}
				label={t("devices:models.playground.chat.message", "Message")}
			>
				<DvTextarea
					rows={3}
					value={draft}
					placeholder={t(
						"devices:models.playground.chat.placeholder",
						"Ask {{model}} something",
						{ model: model.display_name },
					)}
					onChange={(event) => setDraft(event.target.value)}
					onKeyDown={(event) => {
						if (!sendsOn(event)) return;
						event.preventDefault();
						submit();
					}}
				/>
			</Field>
			<div className="flex flex-wrap items-center justify-end gap-2">
				<DvButton
					size="sm"
					variant="ghost"
					icon={Eraser}
					disabled={chat.busy || !chat.turns.length}
					onClick={chat.clear}
				>
					{t("devices:models.playground.chat.clear", "Clear")}
				</DvButton>
				{chat.busy ? (
					<DvButton size="sm" icon={Square} onClick={chat.stop}>
						{t("devices:models.playground.chat.stop", "Stop")}
					</DvButton>
				) : (
					<DvButton
						size="sm"
						variant="primary"
						icon={Send}
						disabled={!draft.trim()}
						onClick={submit}
					>
						{t("devices:models.playground.chat.send", "Send")}
					</DvButton>
				)}
			</div>
		</div>
	);
}

function SystemOnePanel({ fetcher, model, device }: Readonly<PanelProps>) {
	const id = useId();
	const running = useRunning();
	const [request, setRequest] = useState(
		JSON.stringify(
			{
				state: "The order arrived two days late.",
				questions: {
					sentiment: {
						type: "choice",
						instructions: "What is the customer's sentiment?",
						criteria: { positive: null, neutral: null, negative: null },
					},
				},
			},
			null,
			2,
		),
	);
	const [result, setResult] = useState<Record<string, unknown>>();
	const [failure, setFailure] = useState<unknown>();
	const [pending, setPending] = useState(false);
	const run = async () => {
		if (running.current) return;
		const abort = new AbortController();
		running.current = abort;
		setPending(true);
		setFailure(undefined);
		setResult(undefined);
		try {
			setResult(
				await invokeSystemOne(fetcher, {
					model: model.id,
					request: JSON.parse(request),
					signal: abort.signal,
				}),
			);
		} catch (error) {
			if (!abort.signal.aborted) setFailure(error);
		} finally {
			running.current = undefined;
			setPending(false);
		}
	};
	return (
		<div data-playground="systemone" className="flex min-w-0 flex-col gap-3">
			<Field id={`${id}-request`} label="State and questions (JSON)">
				<DvTextarea
					rows={12}
					value={request}
					onChange={(event) => setRequest(event.target.value)}
					className="font-mono text-xs"
				/>
			</Field>
			<p className="text-xs text-muted-foreground">
				Use choice, score, or noul questions. Each question needs instructions.
				Choice and score questions also need criteria.
			</p>
			{failure ? (
				<Failure
					error={failure}
					device={device}
					onDismiss={() => setFailure(undefined)}
				/>
			) : null}
			{result && (
				<pre
					aria-label="SystemOne answers"
					className="max-h-96 overflow-auto whitespace-pre-wrap rounded-md border p-3 text-xs"
				>
					{JSON.stringify(result, null, 2)}
				</pre>
			)}
			<div className="flex justify-end">
				<DvButton
					size="sm"
					variant="primary"
					icon={Send}
					busy={pending}
					disabled={!request.trim()}
					onClick={() => void run()}
				>
					Answer questions
				</DvButton>
			</div>
		</div>
	);
}

function EmbedPanel({ fetcher, model, device }: Readonly<PanelProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const id = useId();
	const running = useRunning();
	const [text, setText] = useState("");
	const [result, setResult] = useState<EmbedMeasure>();
	const [failure, setFailure] = useState<unknown>();
	const [pending, setPending] = useState(false);
	const run = async () => {
		const input = text.trim();
		if (!input || running.current) return;
		const abort = new AbortController();
		running.current = abort;
		setPending(true);
		setFailure(undefined);
		try {
			setResult(
				await embedText(fetcher, {
					model: model.id,
					text: input,
					signal: abort.signal,
				}),
			);
		} catch (error) {
			if (!abort.signal.aborted) setFailure(error);
		} finally {
			running.current = undefined;
			setPending(false);
		}
	};
	return (
		<div data-playground="embedding" className="flex min-w-0 flex-col gap-3">
			<Field
				id={`${id}-text`}
				label={t("devices:models.playground.embed.text", "Text")}
			>
				<DvTextarea
					rows={4}
					value={text}
					placeholder={t(
						"devices:models.playground.embed.placeholder",
						"Text to turn into a vector",
					)}
					onChange={(event) => setText(event.target.value)}
				/>
			</Field>
			{failure ? (
				<Failure
					error={failure}
					device={device}
					onDismiss={() => setFailure(undefined)}
				/>
			) : null}
			{result ? (
				<div data-embedding="" className="flex min-w-0 flex-col gap-1.5">
					<MeasureLine text={embedMeasureText(t, time.locale, result)} />
					<p className="font-mono text-xs wrap-anywhere text-muted-foreground">
						{vectorPreview(t, result)}
					</p>
				</div>
			) : null}
			<div className="flex justify-end">
				<DvButton
					size="sm"
					variant="primary"
					icon={Send}
					busy={pending}
					disabled={!text.trim()}
					onClick={() => void run()}
				>
					{t("devices:models.playground.embed.run", "Embed")}
				</DvButton>
			</div>
		</div>
	);
}

function ModelPicker({
	models,
	value,
	onChange,
}: Readonly<{
	models: readonly HostedModel[];
	value: string;
	onChange: (modelId: string) => void;
}>) {
	const { t } = useTranslation("devices");
	const id = useId();
	const options = models.map((model) => ({
		value: model.id,
		label: t("devices:models.playground.modelOption", "{{model}} · {{state}}", {
			model: model.display_name,
			state: stateLabel(t, model.state),
		}),
	}));
	return (
		<Field id={id} label={t("devices:models.playground.model", "Model")}>
			<DvSelect value={value} onValueChange={onChange} options={options} />
		</Field>
	);
}

export interface PlaygroundSheetProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	deviceId: string;
	device: string;
	models: readonly HostedModel[];
	/** The model to start with; a loaded chat model otherwise. */
	modelId?: string;
}

function PlaygroundBody({
	deviceId,
	device,
	models,
	modelId,
}: Readonly<Omit<PlaygroundSheetProps, "open" | "onOpenChange">>) {
	const { t } = useTranslation("devices");
	const fetcher = useGatewayFetch(deviceId);
	const usable = useMemo(() => playgroundModels(models), [models]);
	const [picked, setPicked] = useState(() => firstModel(usable, modelId)?.id);
	const model = firstModel(usable, picked);
	if (!fetcher)
		return (
			<StateView
				kind="unsupported"
				title={t(
					"devices:models.playground.unsupported",
					"This version of Flow-Like can't send requests to models on devices yet.",
				)}
			/>
		);
	if (!model)
		return (
			<StateView
				kind="empty"
				title={t(
					"devices:models.playground.noModels",
					"No model on {{device}} can answer yet. Wait until its files are downloaded.",
					{ device },
				)}
			/>
		);
	const Panel =
		model.kind === "systemone"
			? SystemOnePanel
			: model.kind === "embedding"
				? EmbedPanel
				: ChatPanel;
	return (
		<>
			<ModelPicker models={usable} value={model.id} onChange={setPicked} />
			<Panel key={model.id} fetcher={fetcher} model={model} device={device} />
		</>
	);
}

/** Chat with or embed text on a model the device hosts, and see how fast it answers. */
export function PlaygroundSheet({
	open,
	onOpenChange,
	...body
}: Readonly<PlaygroundSheetProps>) {
	const { t } = useTranslation("devices");
	return (
		<DvSheet
			open={open}
			onOpenChange={onOpenChange}
			icon={FlaskConical}
			wide
			title={t("devices:models.playground.title", "Try a model on {{device}}", {
				device: body.device,
			})}
			sub={t(
				"devices:models.playground.intro",
				"Requests travel to the device through the encrypted tunnel and count as yours in its statistics.",
			)}
		>
			{open ? <PlaygroundBody {...body} /> : null}
		</DvSheet>
	);
}

/** "Try a model…": opens the playground; gated on using the device's models (`models_use`). */
export function PlaygroundButton({
	deviceId,
	device,
	models,
	modelId,
}: Readonly<Omit<PlaygroundSheetProps, "open" | "onOpenChange">>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const view = gateView(t, time, useGate("models_use", deviceId));
	const [open, setOpen] = useState(false);
	return (
		<>
			<GatedAction gate={view?.gate}>
				<DvButton size="sm" icon={FlaskConical} onClick={() => setOpen(true)}>
					{t("devices:models.playground.open", "Try a model…")}
				</DvButton>
			</GatedAction>
			<PlaygroundSheet
				open={open}
				onOpenChange={setOpen}
				deviceId={deviceId}
				device={device}
				models={models}
				{...(modelId ? { modelId } : {})}
			/>
		</>
	);
}
