import type { A2UIServerMessage } from "../a2ui/types";

/**
 * A page with an onLoad workflow renders its static layout behind a loading indicator. Clear it as
 * soon as the authorized run produces a renderable mutation, then let later messages fill the
 * page in place. Request, navigation, and destructive-only messages do not prove that fresh
 * content is ready.
 */
export function shouldRevealProgressively(message: A2UIServerMessage): boolean {
	switch (message.type) {
		case "surfaceUpdate":
			return message.components.length > 0;
		case "createElement":
		case "upsertElement":
			return true;
		case "dataModelUpdate":
			return message.contents.length > 0;
		default:
			return false;
	}
}

/** Only the onLoad run's own output reveals the page; action and interval output does not. */
export function revealsPageLoad(message: A2UIServerMessage): boolean {
	return message.type === "showScreen" || shouldRevealProgressively(message);
}
