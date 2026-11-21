"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import type { GuideTag, GuideTourId } from "../../lib/guide/guide-model";

export interface IGuideStepCopy {
	readonly title: string;
	readonly body: string;
}

type StepCopy = Record<string, IGuideStepCopy>;

/** Every guide string as a literal `t()` call, so extraction keeps the keys alive. */
export function useGuideCopy() {
	const { t } = useTranslation("guide");
	return useMemo(() => {
		const use: StepCopy = {
			bots: {
				title: t("tours.use.bots.title", "Click Bots in the sidebar."),
				body: t(
					"tours.use.bots.body",
					"Bots are helpers that do tasks for you. They have their own tab.",
				),
			},
			crew: {
				title: t("tours.use.crew.title", "Pick a bot from your crew."),
				body: t(
					"tours.use.crew.body",
					"Assistant helps with anything, Flux builds flows with you. The chat opens next to your crew.",
				),
			},
			send: {
				title: t("tours.use.send.title", "Send it a message."),
				body: t(
					"tours.use.send.body",
					"Ask for something real, like “What can you do for me?” or “Plan a three-day trip to Lisbon.”",
				),
			},
			watch: {
				title: t("tours.use.watch.title", "Watch it work."),
				body: t(
					"tours.use.watch.body",
					"Each step shows up as it happens. If it needs something sensitive, it asks you first.",
				),
			},
			approve: {
				title: t(
					"tours.use.approve.title",
					"It's asking first. Answer the request.",
				),
				body: t(
					"tours.use.approve.body",
					"Bots ask before they use your location, send messages or spend money. Allow it once, always, or say no.",
				),
			},
			details: {
				title: t(
					"tours.use.details.title",
					"Click the bot's name above the chat.",
				),
				body: t(
					"tours.use.details.body",
					"It opens the details: what the bot can do and what it has to ask you about.",
				),
			},
			explore: {
				title: t("tours.use.explore.title", "Now apps: open Explore."),
				body: t(
					"tours.use.explore.body",
					"Explore has ready-made apps from Flow-Like and the community.",
				),
			},
			app: {
				title: t("tours.use.app.title", "Open an app that looks useful."),
				body: t(
					"tours.use.app.body",
					"Add it and it shows up in My Apps, ready to use. No building needed.",
				),
			},
		};
		const build: StepCopy = {
			library: {
				title: t("tours.build.library.title", "Open My Apps."),
				body: t(
					"tours.build.library.body",
					"Apps hold your flows, data, pages and chats.",
				),
			},
			newApp: {
				title: t("tours.build.newApp.title", "Create a new app."),
				body: t(
					"tours.build.newApp.body",
					"You'll build a small one in a few minutes.",
				),
			},
			createApp: {
				title: t("tours.build.createApp.title", "Name it, then create it."),
				body: t(
					"tours.build.createApp.body",
					"Any name works. You can change it later.",
				),
			},
			newFlow: {
				title: t("tours.build.newFlow.title", "Add the app's first flow."),
				body: t(
					"tours.build.newFlow.body",
					"A flow is the logic: nodes that do the work. Click New flow.",
				),
			},
			createFlow: {
				title: t("tours.build.createFlow.title", "Create the flow."),
				body: t(
					"tours.build.createFlow.body",
					"It opens a short form for the flow's name.",
				),
			},
			confirmFlow: {
				title: t("tours.build.confirmFlow.title", "Name it and confirm."),
				body: t(
					"tours.build.confirmFlow.body",
					"Something like “Main flow” is fine.",
				),
			},
			openFlow: {
				title: t("tours.build.openFlow.title", "Open the flow."),
				body: t(
					"tours.build.openFlow.body",
					"The board is where you build. Click your new flow.",
				),
			},
			addEvent: {
				title: t(
					"tours.build.addEvent.title",
					"Right-click the board and add an Event.",
				),
				body: t(
					"tours.build.addEvent.body",
					"Every flow starts with an event: the moment it runs. Pick Event in the palette. On a touch screen, press and hold.",
				),
			},
			connect: {
				title: t(
					"tours.build.connect.title",
					"Drag from the event's white pin into empty space.",
				),
				body: t(
					"tours.build.connect.body",
					"The palette opens with only the nodes that fit. Pick one, for example Log. It gets wired for you.",
				),
			},
			run: {
				title: t("tours.build.run.title", "Press play on the event."),
				body: t(
					"tours.build.run.body",
					"That runs your flow once, right here on the board.",
				),
			},
			wait: {
				title: t("tours.build.wait.title", "Watch it run."),
				body: t("tours.build.wait.body", "Each node lights up while it works."),
			},
			logs: {
				title: t("tours.build.logs.title", "That was your first run."),
				body: t(
					"tours.build.logs.body",
					"Logs show what each node did, step by step. Open them any time from the board's side rail.",
				),
			},
		};
		const steps: Record<GuideTourId, StepCopy> = { use, build };
		const tags: Record<GuideTag, string> = {
			click: t("tag.click", "Click"),
			type: t("tag.type", "Type here"),
			rightClick: t("tag.rightClick", "Right-click"),
			drag: t("tag.drag", "Drag a wire"),
			pick: t("tag.pick", "Pick one"),
			open: t("tag.open", "Open"),
		};
		return {
			step: (tour: GuideTourId, id: string): IGuideStepCopy =>
				steps[tour][id] ?? { title: id, body: "" },
			tag: (tag: GuideTag | undefined) => (tag ? tags[tag] : undefined),
		};
	}, [t]);
}
