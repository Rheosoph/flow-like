import type { ShortWords } from "../contracts";
import { formatDate } from "../model/date-text";

/** English `ShortWords` for tests (what `shortWordsOf(t, "en-GB")` gives, written out). */
export function shortWords(locale = "en-GB"): ShortWords {
	return {
		none: "none",
		empty: "empty",
		on: "On",
		off: "Off",
		files: (count) => (count === 1 ? "1 file" : `${count} files`),
		entries: (count) => (count === 1 ? "1 entry" : `${count} entries`),
		date: (iso) => formatDate(iso, locale),
	};
}
