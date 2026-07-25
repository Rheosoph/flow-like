import common from "./en/common";
import comparison from "./en/comparison";
import cta from "./en/cta";
import dataLayer from "./en/data-layer";
import engine from "./en/engine";
import faq from "./en/faq";
import hardware from "./en/hardware";
import hero from "./en/hero";
import iface from "./en/interface";
import packages from "./en/packages";
import practice from "./en/practice";
import rules from "./en/rules";
import sprawl from "./en/sprawl";
import systemMap from "./en/system-map";
import teams from "./en/teams";

export const v4en = {
	...common,
	...hero,
	...sprawl,
	...systemMap,
	...engine,
	...dataLayer,
	...iface,
	...packages,
	...rules,
	...hardware,
	...teams,
	...comparison,
	...practice,
	...faq,
	...cta,
} as const;
