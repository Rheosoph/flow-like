"use client";

import {
	BulletedListRules,
	OrderedListRules,
	TaskListRules,
} from "@platejs/list";
import { ListPlugin } from "@platejs/list/react";
import { KEYS } from "platejs";

import { BlockList } from "../ui/block-list";
import { IndentKit } from "./indent-kit";

export const ListKit = [
	...IndentKit,
	ListPlugin.configure({
		inputRules: [
			BulletedListRules.markdown({ variant: "-" }),
			BulletedListRules.markdown({ variant: "*" }),
			OrderedListRules.markdown({ variant: "." }),
			OrderedListRules.markdown({ variant: ")" }),
			TaskListRules.markdown({ checked: false }),
			TaskListRules.markdown({ checked: true }),
		],
		inject: {
			targetPlugins: [
				...KEYS.heading,
				KEYS.p,
				KEYS.blockquote,
				KEYS.codeBlock,
				KEYS.toggle,
			],
		},
		render: {
			belowNodes: BlockList,
		},
	}),
];
