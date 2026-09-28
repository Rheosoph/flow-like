import type { IEvent } from "./schema/flow/event";

/**
 * The one line that says how this event is reached — the URL a caller hits, the
 * schedule it runs on, the mailbox it polls. UI events answer this with their
 * route, which is editable and therefore rendered separately; everything else
 * derives it from the saved trigger config.
 */
export interface IEventEntry {
	/** Rendered in the entry column. Monospace, single line. */
	text: string;
	/** Full value for the title attribute when the text is likely to truncate. */
	title?: string;
	/**
	 * Muted styling for entries that are a description rather than an address —
	 * "2 channels" is not something you can paste anywhere.
	 */
	muted?: boolean;
}

const str = (value: unknown): string =>
	typeof value === "string" ? value.trim() : "";

const count = (value: unknown): number =>
	Array.isArray(value) ? value.length : 0;

function plural(n: number, one: string, many: string): string {
	return `${n} ${n === 1 ? one : many}`;
}

/** Read a configured local date and time; the runner owns timezone and firing state. */
export function getCronScheduledTime(
	config: Record<string, unknown>,
): string | null {
	const scheduled = config.scheduled_for;
	if (!scheduled || typeof scheduled !== "object") return null;
	const { date, time } = scheduled as { date?: unknown; time?: unknown };
	if (
		typeof date !== "string" ||
		typeof time !== "string" ||
		!/^\d{4}-\d{2}-\d{2}$/.test(date) ||
		!/^\d{2}:\d{2}$/.test(time)
	)
		return null;
	const timestamp = `${date}T${time}:00.000Z`;
	const parsed = new Date(timestamp);
	if (Number.isNaN(parsed.getTime()) || parsed.toISOString() !== timestamp)
		return null;
	return `${date} ${time}`;
}

export function describeEventEntry(
	event: IEvent,
	config: Record<string, unknown>,
): IEventEntry | null {
	switch (event.event_type) {
		case "api": {
			const method = str(config.method).toUpperCase() || "GET";
			const path = str(config.path) || "/";
			const text = `${method} ${path}`;
			return { text, title: text };
		}
		case "cron": {
			const schedule = str(config.expression) || getCronScheduledTime(config);
			if (!schedule) return { text: "No schedule", muted: true };
			const timezone = str(config.timezone);
			return {
				text: timezone ? `${schedule} · ${timezone}` : schedule,
				title: timezone ? `${schedule} (${timezone})` : schedule,
			};
		}
		case "deeplink": {
			const route = str(config.route);
			if (!route) return { text: "No route", muted: true };
			const text = `flow-like://${route}`;
			return { text, title: text };
		}
		case "email": {
			const mailbox = str(config.mail) || str(config.username);
			if (!mailbox) return { text: "No mailbox", muted: true };
			return { text: mailbox, title: mailbox };
		}
		case "teams":
			return { text: "Microsoft Teams bot", muted: true };
		case "inbound_email":
			return { text: "Server email address", muted: true };
		case "discord": {
			const allowed = count(config.channel_whitelist);
			return {
				text: allowed
					? plural(allowed, "channel", "channels")
					: "Every invited channel",
				muted: true,
			};
		}
		case "telegram": {
			const allowed = count(config.chat_whitelist);
			return {
				text: allowed ? plural(allowed, "chat", "chats") : "Every chat",
				muted: true,
			};
		}
		case "rest": {
			const routes = count(config.routes);
			const exposure = event.exposure === "INTERNAL" ? "internal" : "public";
			return {
				text: `${routes ? plural(routes, "route", "routes") : "No routes"} · ${exposure}`,
				muted: true,
			};
		}
		case "mcp": {
			const tools = count(config.tools);
			const exposure = event.exposure === "INTERNAL" ? "internal" : "public";
			return {
				text: `${tools ? plural(tools, "tool", "tools") : "No tools"} · ${exposure}`,
				muted: true,
			};
		}
		case "daemon": {
			const policy = str(config.restart_policy) || "on_failure";
			return { text: `Restart ${policy.replace(/_/g, " ")}`, muted: true };
		}
		default:
			return null;
	}
}
