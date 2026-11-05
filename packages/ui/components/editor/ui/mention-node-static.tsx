"use client";

import { useTranslation } from "@flow-like/locales";
import * as React from "react";

import type { TMentionElement } from "platejs";
import type { SlateElementProps } from "platejs/static";

import { KEYS } from "platejs";
import { SlateElement } from "platejs/static";

import { cn } from "../../../lib/utils";
import { isLocalUserSub } from "../../../state/backend-state/user-state";
import { IdentityHoverCard } from "../../identity/identity-hover-card";

/** A mention written as `[@Name](mention://sub)` names a principal, which Plate keeps as `//sub`. */
export function mentionedSubject(key: unknown): string | undefined {
	if (typeof key !== "string" || !key.startsWith("//") || key.length < 3) {
		return undefined;
	}
	// Plate has already decoded the link destination when it creates the mention.
	return key.slice(2);
}

/** Where set, an `@word` that names nobody in the conversation stays plain text instead of a chip. */
export const PlainUnresolvedMentionsContext = React.createContext(false);

/** The reader of a conversation, so only their own mentions get the stronger highlight. */
export const MentionReaderContext = React.createContext<string | null>(null);

export function useIsSelfMention(subject: string | undefined): boolean {
	const reader = React.useContext(MentionReaderContext);
	return Boolean(subject && (subject === reader || isLocalUserSub(subject)));
}

export const SUBJECT_MENTION_CLASS =
	"inline-block max-w-full rounded-md bg-primary/15 px-1.5 py-0.5 align-baseline text-[0.95em] font-medium text-primary";

export const SELF_SUBJECT_MENTION_CLASS =
	"bg-primary/20 font-semibold text-foreground ring-2 ring-inset ring-primary/70";

export function SubjectMentionLabel({
	subject,
	label,
}: Readonly<{ subject: string; label: string }>) {
	const { t } = useTranslation("bots");
	const isSelf = useIsSelfMention(subject);
	return (
		<IdentityHoverCard subject={subject}>
			<button
				type="button"
				className="cursor-default rounded-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
				aria-label={isSelf ? `${label} (${t("crew.you", "You")})` : undefined}
			>
				{label}
			</button>
		</IdentityHoverCard>
	);
}

export function MentionElementStatic(
	props: SlateElementProps<TMentionElement> & {
		prefix?: string;
	},
) {
	const { prefix } = props;
	const element = props.element;
	const subject = mentionedSubject(element.key);
	const plainUnresolved = React.useContext(PlainUnresolvedMentionsContext);
	const isSelf = useIsSelfMention(subject);

	if (!subject && plainUnresolved) {
		return (
			<SlateElement
				{...props}
				as="span"
				attributes={{
					...props.attributes,
					"data-slate-value": element.value,
				}}
			>
				{props.children}
				{prefix ?? "@"}
				{element.value}
			</SlateElement>
		);
	}

	if (subject) {
		return (
			<SlateElement
				{...props}
				as="span"
				className={cn(
					SUBJECT_MENTION_CLASS,
					isSelf && SELF_SUBJECT_MENTION_CLASS,
				)}
				attributes={{
					...props.attributes,
					"data-slate-value": element.value,
					"data-mention-subject": subject,
					"data-mention-self": isSelf || undefined,
				}}
			>
				{props.children}
				<SubjectMentionLabel
					subject={subject}
					label={`${prefix ?? ""}${element.value}`}
				/>
			</SlateElement>
		);
	}

	return (
		<SlateElement
			{...props}
			className={cn(
				"inline-block rounded-md bg-muted px-1.5 py-0.5 align-baseline text-sm font-medium",
				element.children[0][KEYS.bold] === true && "font-bold",
				element.children[0][KEYS.italic] === true && "italic",
				element.children[0][KEYS.underline] === true && "underline",
			)}
			attributes={{
				...props.attributes,
				"data-slate-value": element.value,
			}}
		>
			<React.Fragment>
				{props.children}
				{prefix}
				{element.value}
			</React.Fragment>
		</SlateElement>
	);
}
