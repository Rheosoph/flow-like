import { afterAll, afterEach, expect, test } from "bun:test";
import type {
	CopyParams,
	CopyRef,
} from "../../../../lib/device-management/model/types";
import type { DevicesT } from "../primitives/area-context";
import type { ResolveCode } from "../routing/resolve-target";
import { byRole, click, installDom } from "../testing/dom-harness";

const dom = installDom();
const { getI18n } = await import("@flow-like/locales");
const { ResolveBanner, resolveCopy } = await import("./resolve-banner");

afterEach(dom.cleanup);
afterAll(dom.restore);

const t = getI18n().getFixedT("en", "devices") as DevicesT;
const at = () => "12 Sept, 09:30 CEST";

const CASES: Record<ResolveCode, { params: CopyParams; says: string[] }> = {
	device_not_found: {
		params: { device: "6f1c2a3b", hub: "api.flow-like.com" },
		says: ["6f1c2a3b", "api.flow-like.com"],
	},
	device_not_shared: {
		params: { device: "6f1c2a3b" },
		says: ["6f1c2a3b", "isn't shared with you any more"],
	},
	device_other_account: {
		params: { device: "6f1c2a3b" },
		says: ["6f1c2a3b", "another account"],
	},
	device_revoked: {
		params: { device: "old-kiosk", revokedAt: 1_789_000_000 },
		says: ["old-kiosk", "revoked on 12 Sept, 09:30 CEST"],
	},
	service_not_found: {
		params: { device: "edge-berlin-01", service: "invoice-extractor" },
		says: ["edge-berlin-01", "invoice-extractor"],
	},
	certificate_not_found: {
		params: { device: "edge-berlin-01", certificate: "cert_01" },
		says: ["edge-berlin-01", "cert_01"],
	},
	app_not_found: { params: { app: "app_gone" }, says: ["app_gone"] },
	setup_not_found: { params: { enrollment: "enr_1" }, says: ["setup"] },
};

test("every reason is one sentence that names the object", () => {
	for (const [code, { params, says }] of Object.entries(CASES)) {
		const banner = { code, params } as CopyRef<ResolveCode>;
		const text = resolveCopy(t, banner, at);
		for (const part of says) expect(text).toContain(part);
		expect(text).not.toContain(code);
		expect(text).not.toMatch(/\{\{|\}\}/);
	}
});

test("a hub or revocation date that isn't known is left out", () => {
	expect(
		resolveCopy(t, { code: "device_not_found", params: { device: "abc" } }, at),
	).toContain("on this hub");
	expect(
		resolveCopy(t, { code: "device_revoked", params: { device: "abc" } }, at),
	).toBe("abc was revoked, so this can't be done any more.");
});

test("the banner states the reason and can be dismissed", async () => {
	let dismissed = 0;
	const view = await dom.render(
		<ResolveBanner
			banner={{
				code: "service_not_found",
				params: { device: "edge-berlin-01", service: "invoice-extractor" },
			}}
			onDismiss={() => dismissed++}
		/>,
	);
	const banner = byRole("status");
	expect(banner.getAttribute("data-tone")).toBe("warning");
	expect(view.container.textContent).toContain(
		"edge-berlin-01 has no service invoice-extractor. It may have been removed.",
	);
	await click(byRole("button", "Dismiss"));
	expect(dismissed).toBe(1);
});
