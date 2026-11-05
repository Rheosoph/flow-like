"use client";

import { useTranslation } from "@flow-like/locales";
import { motion } from "framer-motion";
import {
	KeyRoundIcon,
	LayersIcon,
	type LucideIcon,
	MessageCircleQuestionIcon,
	ShieldQuestionIcon,
} from "lucide-react";
import { usePathname } from "next/navigation";
import { useCallback, useState } from "react";
import { Button, Checkbox, useBackend, useInvoke } from "../../index";
import {
	type AskUserForm,
	askUserAnswerPayload,
	initialAskUserDrafts,
	isAskUserFormComplete,
} from "../../lib/ask-user";
import {
	type GlobalToolPrompt,
	type GlobalToolSecret,
	useGlobalChatStore,
} from "../../state/global-chat/global-chat-store";
import { restoreGlobalChatConversation } from "../../state/global-chat/global-chat-stream";
import { BOTS_ROUTE } from "../bots-tab/dock-selection";
import { SecretRequestForm } from "../bots-tab/secret-request-form";
import { BotAvatar } from "../bots/bot-avatar";
import {
	RequestCard,
	RequestCardBody,
	RequestCardFooter,
	RequestCardHeader,
	RequestDetails,
	RequestMark,
	type RequestTone,
	requestButtonClass,
} from "../interfaces/chat-default/request-card";
import { AskUserQuestions } from "./ask-user-questions";
import {
	type GlobalChatBotView,
	useGlobalChatBot,
} from "./use-global-chat-bot";

const EMPTY_ASK: AskUserForm = { questions: [], batched: false };

/** The face of the bot whose run asks, badged with what it asks for. */
function AskingBotMark({
	view,
	icon,
	tone,
	size = "md",
}: Readonly<{
	view: GlobalChatBotView;
	icon: LucideIcon;
	tone: RequestTone;
	size?: "sm" | "md";
}>) {
	return (
		<RequestMark icon={icon} tone={tone} size={size}>
			<BotAvatar
				appearance={view.appearance}
				size={size === "sm" ? 32 : 36}
				animated={false}
				decorative
			/>
		</RequestMark>
	);
}

/**
 * Resolves an approval's target app id to its name + icon so the card shows the app's identity
 * instead of the opaque id.
 */
function ApprovalAppIdentity({
	appId,
	description,
}: {
	appId: string;
	description?: string;
}) {
	const backend = useBackend();
	const meta = useInvoke(
		backend.appState.getAppMeta,
		backend.appState,
		[appId],
		!!appId,
	);
	const name = meta.data?.name || appId;
	const icon = meta.data?.icon ?? meta.data?.thumbnail;
	const resolvedDescription = description?.replace(appId, name);

	return (
		<>
			<div className="inline-flex max-w-full items-center gap-2 rounded-lg border border-border/70 bg-background/60 py-1 pr-2.5 pl-1">
				{icon ? (
					<img
						src={icon}
						alt=""
						className="size-6 shrink-0 rounded-md object-cover"
					/>
				) : (
					<span className="flex size-6 shrink-0 items-center justify-center rounded-md bg-primary/12 text-primary">
						<LayersIcon className="size-3.5" />
					</span>
				)}
				<span className="min-w-0 truncate text-[13px] font-medium">{name}</span>
			</div>
			{resolvedDescription && <RequestDetails text={resolvedDescription} />}
		</>
	);
}

function RememberedInApp({
	name,
	appId,
}: Readonly<{ name: string; appId: string }>) {
	const { t } = useTranslation("chat");
	const backend = useBackend();
	const meta = useInvoke(backend.appState.getAppMeta, backend.appState, [
		appId,
	]);
	return t(
		"dontAskAgainForBotInApp",
		"Don't ask again for {{name}} in {{app}}",
		{
			name,
			app: meta.data?.name || t("thisAppInApproval", "this app"),
		},
	);
}

/**
 * "Don't ask again" for what it covers: the asking run's bot (every Flux chat shares one scope) and,
 * for a tool that acts in one app, that app.
 */
function RememberApproval({
	prompt,
	name,
	remember,
	onRememberChange,
}: Readonly<{
	prompt: GlobalToolPrompt;
	name: string;
	remember: boolean;
	onRememberChange: (remember: boolean) => void;
}>) {
	const { t } = useTranslation("chat");
	return (
		<label
			htmlFor="inline-tool-prompt-remember"
			className="flex w-fit cursor-pointer items-center gap-2 text-xs text-muted-foreground transition-colors hover:text-foreground"
		>
			<Checkbox
				id="inline-tool-prompt-remember"
				checked={remember}
				onCheckedChange={(checked) => onRememberChange(checked === true)}
			/>
			{prompt.rememberAppId ? (
				<RememberedInApp name={name} appId={prompt.rememberAppId} />
			) : (
				t("dontAskAgainForBot", "Don't ask again for {{name}}", { name })
			)}
		</label>
	);
}

/**
 * In place of a card that belongs to another chat: who waits for an answer, and the way to the
 * chat whose run asks. On the Bots tab its dock follows; elsewhere the overlay shows it.
 */
export function ToolPromptElsewhere({
	conversationId,
}: Readonly<{ conversationId: string }>) {
	const { t } = useTranslation("chat");
	const asking = useGlobalChatBot(conversationId);
	const pathname = usePathname();
	const openAskingChat = useCallback(async () => {
		const shown = await restoreGlobalChatConversation(conversationId).catch(
			() => false,
		);
		if (shown && pathname === BOTS_ROUTE) {
			useGlobalChatStore.getState().requestReveal();
		}
	}, [conversationId, pathname]);
	return (
		<RequestCard tone="warning" live className="shadow-floating">
			<div className="flex items-center gap-3 px-4 py-3">
				<AskingBotMark
					view={asking}
					icon={ShieldQuestionIcon}
					tone="warning"
					size="sm"
				/>
				<p className="min-w-0 flex-1 text-sm leading-snug">
					{t(
						"toolPromptInOtherChat",
						"{{name}} is waiting for your answer in another chat.",
						{ name: asking.name },
					)}
				</p>
				<Button
					type="button"
					variant="outline"
					size="sm"
					className={requestButtonClass}
					onClick={() => void openAskingChat()}
				>
					{t("openAskingChat", "Open chat")}
				</Button>
			</div>
		</RequestCard>
	);
}

/**
 * Inline replacement for the former modal tool dialogs: renders the pending approval or question
 * from the global assistant directly in the chat surface, pinned above the input. A question is
 * either a single freeform/choice prompt or the batched BUILD intake form, whose gaps are all
 * answered in this one card before the build starts.
 */
export function InlineToolPrompt({ prompt }: { prompt: GlobalToolPrompt }) {
	if (prompt.kind === "secret" && prompt.secret)
		return <InlineSecretPrompt prompt={prompt} secret={prompt.secret} />;
	return <InlineQuestionPrompt prompt={prompt} />;
}

/** A bot's request for a secret: the value goes to the store, the resolution only says it was saved. */
function InlineSecretPrompt({
	prompt,
	secret,
}: Readonly<{ prompt: GlobalToolPrompt; secret: GlobalToolSecret }>) {
	const { t } = useTranslation(["chat", "bots"]);
	const asking = useGlobalChatBot(prompt.conversationId);
	return (
		<motion.div
			initial={{ opacity: 0, y: 8, scale: 0.98 }}
			animate={{ opacity: 1, y: 0, scale: 1 }}
			transition={{ type: "spring", stiffness: 380, damping: 32 }}
			className="flex max-h-[70vh] w-full"
		>
			<RequestCard
				tone="warning"
				live
				aria-label={prompt.title}
				className="max-h-full shadow-floating"
			>
				<RequestCardHeader
					mark={
						<AskingBotMark
							view={asking}
							icon={KeyRoundIcon}
							tone="warning"
							size="sm"
						/>
					}
					tone="warning"
					live
					eyebrow={`${asking.name} · ${t("bots:crew.approval.secret.eyebrow", "Needs a secret")}`}
					title={t(
						"bots:crew.approval.secret.title",
						"{{name}} needs a secret",
						{
							name: asking.name,
						},
					)}
					className="pb-3"
				/>
				<SecretRequestForm
					secret={secret}
					botName={asking.name}
					onSave={async (value) => {
						await secret.store(value);
						prompt.respond({ approved: true, remember: false });
					}}
					onDecline={() => prompt.respond({ approved: false, remember: false })}
				/>
			</RequestCard>
		</motion.div>
	);
}

function InlineQuestionPrompt({ prompt }: { prompt: GlobalToolPrompt }) {
	const { t } = useTranslation(["chat", "bots", "settings"]);
	const asking = useGlobalChatBot(prompt.conversationId);
	const [remember, setRemember] = useState(false);
	const ask = prompt.ask ?? EMPTY_ASK;
	const [drafts, setDrafts] = useState(() => initialAskUserDrafts(ask));
	const isApproval = prompt.kind === "approval";
	const tone: RequestTone = isApproval ? "warning" : "primary";

	const canSend = isAskUserFormComplete(ask, drafts);
	const isSingleFreeform =
		ask.questions.length === 1 && ask.questions[0].mode === "freeform";
	// A lone question is the card's subject; its generic "needs input" title would only repeat
	// what the eyebrow already says.
	const title =
		!isApproval && prompt.description ? prompt.description : prompt.title;

	const submit = () => {
		if (!canSend) return;
		prompt.respond({ answer: askUserAnswerPayload(ask, drafts) });
	};

	const hasBody =
		Boolean(prompt.appId) ||
		(isApproval && Boolean(prompt.description)) ||
		(!isApproval && ask.questions.length > 0);

	return (
		<motion.div
			initial={{ opacity: 0, y: 8, scale: 0.98 }}
			animate={{ opacity: 1, y: 0, scale: 1 }}
			transition={{ type: "spring", stiffness: 380, damping: 32 }}
			className="flex max-h-[70vh] w-full"
		>
			<RequestCard
				tone={tone}
				live
				aria-label={prompt.title}
				className="max-h-full shadow-floating"
			>
				<RequestCardHeader
					mark={
						<AskingBotMark
							view={asking}
							icon={isApproval ? ShieldQuestionIcon : MessageCircleQuestionIcon}
							tone={tone}
						/>
					}
					tone={tone}
					live
					eyebrow={`${asking.name} · ${
						isApproval
							? t("needsYourApproval", "Needs your approval")
							: t("needsAnAnswer", "Needs an answer")
					}`}
					title={title}
					className={hasBody ? undefined : "pb-4"}
				/>
				{hasBody && (
					<RequestCardBody className="overflow-y-auto overscroll-contain">
						{prompt.appId ? (
							<ApprovalAppIdentity
								appId={prompt.appId}
								description={isApproval ? prompt.description : undefined}
							/>
						) : (
							isApproval &&
							prompt.description && <RequestDetails text={prompt.description} />
						)}
						{!isApproval && (
							<AskUserQuestions
								form={ask}
								drafts={drafts}
								onDraftsChange={setDrafts}
								onSubmit={isSingleFreeform ? submit : undefined}
							/>
						)}
					</RequestCardBody>
				)}
				<RequestCardFooter
					start={
						isApproval &&
						prompt.rememberable !== false && (
							<RememberApproval
								prompt={prompt}
								name={asking.name}
								remember={remember}
								onRememberChange={setRemember}
							/>
						)
					}
				>
					<Button
						variant="outline"
						size="sm"
						className={requestButtonClass}
						onClick={() =>
							isApproval
								? prompt.respond({ approved: false, remember: false })
								: prompt.respond(null)
						}
					>
						{isApproval
							? t("bots:crew.approval.deny", "Deny")
							: t("cancel", "Cancel")}
					</Button>
					<Button
						size="sm"
						disabled={!isApproval && !canSend}
						className={requestButtonClass}
						onClick={() => {
							if (isApproval) {
								prompt.respond({
									approved: true,
									remember: prompt.rememberable !== false && remember,
								});
							} else {
								submit();
							}
						}}
					>
						{isApproval ? t("settings:approve", "Approve") : t("send", "Send")}
					</Button>
				</RequestCardFooter>
			</RequestCard>
		</motion.div>
	);
}
