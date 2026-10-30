import { writeFile } from "node:fs/promises";
import { getHomeWidgetCatalog } from "../components/global-chat/tools/home/catalog";
import { createDefaultHomeLayout } from "../components/home/catalog";
import { homeLayoutFingerprint } from "../components/home/home-layout-json";

const defaultLayout = createDefaultHomeLayout();
const contracts = {
	catalog: getHomeWidgetCatalog({}),
	default_layout: defaultLayout,
	default_fingerprint: homeLayoutFingerprint(defaultLayout),
};
await writeFile(
	new URL(
		"../components/global-chat/tools/home/generated/native-contracts.json",
		import.meta.url,
	),
	`${JSON.stringify(contracts, null, 2)}\n`,
);
