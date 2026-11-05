import { describe, expect, it } from "vitest";
import { createDefaultHomeLayout } from "../../../home/catalog";
import { homeLayoutFingerprint } from "../../../home/home-layout-json";
import { getHomeWidgetCatalog } from "./catalog";
import contracts from "./generated/native-contracts.json";

describe("native Home contracts", () => {
	it("ships the same catalog and default layout to the native host", () => {
		const layout = createDefaultHomeLayout();
		expect(contracts.catalog).toEqual(getHomeWidgetCatalog({}));
		expect(contracts.default_layout).toEqual(layout);
		expect(contracts.default_fingerprint).toEqual(
			homeLayoutFingerprint(layout),
		);
	});
});
