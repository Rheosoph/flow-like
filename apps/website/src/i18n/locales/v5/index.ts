import de from "./de";
import en from "./en";
import es from "./es";
import fr from "./fr";
import it from "./it";
import ja from "./ja";
import ko from "./ko";
import nl from "./nl";
import pt from "./pt";
import sv from "./sv";
import zh from "./zh";

export const v5en = en;

export const v5 = {
	en,
	de,
	es,
	fr,
	zh,
	ja,
	ko,
	pt,
	it,
	nl,
	sv,
} as Record<string, Record<string, string>>;
