import { createHomeWidget } from "../../packages/ui/components/home/catalog";
import type { IHomeLayout } from "../../packages/ui/components/home/types";

export function personalFixtureLayout(): IHomeLayout {
	const widget = (
		preset: string,
		columns: number,
		title?: string,
		config?: Record<string, unknown>,
	) => {
		const item = createHomeWidget(preset);
		return {
			...item,
			title: title ?? item.title,
			size: { ...item.size, columns },
			config: { ...item.config, ...config },
		};
	};
	return {
		version: 1,
		widgets: [
			widget("greeting", 12),
			widget("flowpilot-bar", 8),
			widget("flowpilot-orb", 4),
			widget("flowpilot-hero", 8, "Turn a good idea into an app"),
			widget("flowpilot-card", 4, "Make room for your next idea"),
			widget("quick-actions", 12),
			widget("quick-links", 4, "Useful destinations", {
				links: [
					{
						title: "Your library",
						description: "Keep your projects close.",
						href: "/library",
					},
					{
						title: "Learning paths",
						description: "Learn something you can use today.",
						href: "/learn",
					},
				],
			}),
			widget("checklist", 4, "A thoughtful start", {
				items: [
					{ id: "one", title: "Choose the tools you need", checked: true },
					{ id: "two", title: "Try your first app", checked: false },
					{ id: "three", title: "Make your home your own", checked: false },
				],
			}),
			widget("countdown", 4, "Our next milestone", {
				date: new Date(Date.now() + 12 * 86_400_000).toISOString(),
				body: "A little progress, every day.",
			}),
			widget("resource-directory", 6, "Team resources", {
				items: [
					{
						title: "Start here",
						label: "GUIDE",
						body: "Find the **right app** for the task in front of you.",
						href: "/library",
					},
					{
						title: "Explore packages",
						label: "BUILD",
						body: "Add a useful capability to your next flow.",
						href: "/store/packages",
					},
				],
			}),
			widget("faq", 6, "A few useful answers", {
				items: [
					{
						title: "Can I change this later?",
						body: "You can add, move, and resize widgets whenever you need.",
					},
					{
						title: "Where do I find my apps?",
						body: "Open your [library](/library) to see your projects.",
					},
				],
			}),
			widget("guided-steps", 6, "From idea to useful app", {
				items: [
					{
						title: "Choose a starting point",
						body: "Start small, with one task you want to improve.",
					},
					{
						title: "Try it with real work",
						body: "Use a familiar example to see what needs refining.",
					},
				],
			}),
			widget("updates-feed", 6, "What changed", {
				items: [
					{
						label: "TODAY",
						title: "A home that fits your work",
						body: "Bring your apps, context, and useful links together.",
					},
					{
						label: "THIS WEEK",
						title: "A better place to start",
						body: "Browse your team's recommended apps.",
					},
				],
			}),
			widget("banner", 8, "Space for what matters next", {
				eyebrow: "YOUR WORKSPACE",
				body: "Keep your most useful apps and ideas within reach.",
				actionHref: "/library",
				actionLabel: "Open your library",
			}),
			widget("notes", 4, "A note for later", {
				body: "## Make the next step clear\nKeep a short plan here.\n\n- One useful task\n- A link to the context\n- Something to come back to",
			}),
			widget("facts-card", 6, "A few helpful facts", {
				items: [
					{
						label: "YOUR HOME",
						title: "Yours to arrange",
						body: "Widgets follow the profile you choose.",
					},
					{
						label: "START AGAIN",
						title: "Always reversible",
						body: "Reset to your administrator's current default.",
					},
				],
			}),
			widget("announcement", 6, "A useful update", {
				body: "You can now bring a specific app page into your home.",
				actionHref: "/library",
				actionLabel: "Explore your apps",
			}),
			widget("info", 4, "An empty note", { body: "" }),
			widget("app-embed", 8, "An app, close at hand", {
				appId: "fixture-app-0",
				route: "/reports",
				target: "route",
				query: "period=month",
			}),
		],
	};
}
