"use client";

import {
	BoldRules,
	CodeRules,
	HighlightRules,
	ItalicRules,
	StrikethroughRules,
	SubscriptRules,
	SuperscriptRules,
	UnderlineRules,
} from "@platejs/basic-nodes";
import {
	BoldPlugin,
	CodePlugin,
	HighlightPlugin,
	ItalicPlugin,
	KbdPlugin,
	StrikethroughPlugin,
	SubscriptPlugin,
	SuperscriptPlugin,
	UnderlinePlugin,
} from "@platejs/basic-nodes/react";
import { KEYS, createMarkInputRule } from "platejs";

import { CodeLeaf } from "../ui/code-node";
import { HighlightLeaf } from "../ui/highlight-node";
import { KbdLeaf } from "../ui/kbd-node";
import { notInside } from "./input-rule-guards";

const enabled = notInside(KEYS.codeBlock);

/** Closed by the mirrored opening run: `***x***`, `__*x*__`, `___***x***___`. */
const markCombo = (start: string, marks: string[]) =>
	createMarkInputRule({
		enabled,
		end: [...start].reverse().join("").slice(0, -1),
		marks,
		start,
		trigger: start.charAt(0),
	});

export const BasicMarksKit = [
	BoldPlugin.configure({
		inputRules: [
			markCombo("***", [KEYS.bold, KEYS.italic]),
			BoldRules.markdown({ enabled, variant: "*" }),
		],
	}),
	ItalicPlugin.configure({
		inputRules: [
			ItalicRules.markdown({ enabled, variant: "*" }),
			ItalicRules.markdown({ enabled, variant: "_" }),
		],
	}),
	UnderlinePlugin.configure({
		inputRules: [
			markCombo("___***", [KEYS.underline, KEYS.bold, KEYS.italic]),
			markCombo("__**", [KEYS.underline, KEYS.bold]),
			markCombo("__*", [KEYS.underline, KEYS.italic]),
			UnderlineRules.markdown({ enabled }),
		],
	}),
	CodePlugin.configure({
		inputRules: [CodeRules.markdown({ enabled })],
		node: { component: CodeLeaf },
		shortcuts: { toggle: { keys: "mod+e" } },
	}),
	StrikethroughPlugin.configure({
		inputRules: [StrikethroughRules.markdown({ enabled })],
		shortcuts: { toggle: { keys: "mod+shift+x" } },
	}),
	SubscriptPlugin.configure({
		inputRules: [SubscriptRules.markdown({ enabled })],
		shortcuts: { toggle: { keys: "mod+comma" } },
	}),
	SuperscriptPlugin.configure({
		inputRules: [SuperscriptRules.markdown({ enabled })],
		shortcuts: { toggle: { keys: "mod+period" } },
	}),
	HighlightPlugin.configure({
		inputRules: [
			HighlightRules.markdown({ enabled, variant: "==" }),
			HighlightRules.markdown({ enabled, variant: "≡" }),
		],
		node: { component: HighlightLeaf },
		shortcuts: { toggle: { keys: "mod+shift+h" } },
	}),
	KbdPlugin.withComponent(KbdLeaf),
];
