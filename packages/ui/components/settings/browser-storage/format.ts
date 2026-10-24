import { i18n } from "@flow-like/locales";

export function formatStorageDate(timestamp: number): string {
	try {
		return new Intl.DateTimeFormat(i18n.language || undefined, {
			dateStyle: "medium",
			timeStyle: "short",
		}).format(new Date(timestamp));
	} catch {
		return new Date(timestamp).toLocaleString();
	}
}
