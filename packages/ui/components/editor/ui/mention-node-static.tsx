"use client";

import * as React from "react";

import type { TMentionElement } from "platejs";
import type { SlateElementProps } from "platejs/static";

import { KEYS } from "platejs";
import { SlateElement } from "platejs/static";

import { cn } from "../../../lib/utils";
import { IdentityHoverCard } from "../../identity/identity-hover-card";

/** A mention written as `[@Name](mention://sub)` names a principal, which Plate keeps as `//sub`. */
export function mentionedSubject(key: unknown): string | undefined {
	if (typeof key !== "string" || !key.startsWith("//") || key.length < 3) {
		return undefined;
	}
	try {
		return decodeURIComponent(key.slice(2));
	} catch {
		return undefined;
	}
}

/** Where set, an `@word` that names nobody in the conversation stays plain text instead of a chip. */
export const PlainUnresolvedMentionsContext = React.createContext(false);

export const SUBJECT_MENTION_CLASS =
	"inline-block rounded-md bg-primary/15 px-1.5 py-0.5 align-baseline text-sm font-medium text-primary";

export function SubjectMentionLabel({
	subject,
	label,
}: Readonly<{ subject: string; label: string }>) {
	return (
		<IdentityHoverCard subject={subject}>
			<button
				type="button"
				className="cursor-default rounded-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
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
				className={SUBJECT_MENTION_CLASS}
				attributes={{
					...props.attributes,
					"data-slate-value": element.value,
					"data-mention-subject": subject,
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
