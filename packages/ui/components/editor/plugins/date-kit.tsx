"use client";

import { DatePlugin } from "@platejs/date/react";

import { DateElement } from "../ui/date-node";

export const DateKit = [
	DatePlugin.configure({
		node: { component: DateElement, isSelectable: false },
	}),
];
