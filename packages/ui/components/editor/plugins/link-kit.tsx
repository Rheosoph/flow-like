"use client";

import { LinkRules, validateUrl } from "@platejs/link";
import { LinkPlugin } from "@platejs/link/react";
import {
	type AnyInputRule,
	KEYS,
	type SelectionInputRuleContext,
} from "platejs";

import { LinkElement } from "../ui/link-node";
import { LinkFloatingToolbar } from "../ui/link-toolbar";
import { notInside } from "./input-rule-guards";

type AutolinkResolve = (
	context: SelectionInputRuleContext,
) => { url: string } | undefined;

/**
 * The stock rule selects the word, then gives up when `validateUrl` rejects it
 * (`ftp://x.io`) while still consuming the key, so the next keystroke replaced
 * the URL. Only match what it will link.
 */
const typedAutolink = (variant: "break" | "space"): AnyInputRule => {
	const rule = LinkRules.autolink({
		enabled: notInside(KEYS.codeBlock),
		variant,
	});
	const resolve = rule.resolve as AutolinkResolve | undefined;
	return {
		...rule,
		resolve: (context: SelectionInputRuleContext) => {
			const match = resolve?.(context);
			return match && validateUrl(context.editor, match.url)
				? match
				: undefined;
		},
	} as AnyInputRule;
};

export const LinkKit = [
	LinkPlugin.configure({
		inputRules: [
			LinkRules.autolink({ variant: "paste" }),
			typedAutolink("space"),
			typedAutolink("break"),
		],
		render: {
			node: LinkElement,
			afterEditable: () => <LinkFloatingToolbar />,
		},
	}),
];
