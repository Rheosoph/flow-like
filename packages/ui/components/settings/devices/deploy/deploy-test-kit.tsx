import type { ReactNode } from "react";
import {
	APPS,
	VISITOR_PLAN_APP,
} from "../../../../lib/device-management/model/__fixtures__/apps";
import { SAMPLE_IDS } from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import {
	type DeployDraft,
	type PlanApp,
	makePlan,
} from "../../../../lib/device-management/model/deploy-plan";
import type {
	DeployRoute,
	DevicesScope,
} from "../../../../lib/device-management/model/types";
import { useDevicesRoute } from "../routing/use-devices-route";
import type { FakeWorkspace } from "../testing/fake-workspace";
import type {
	MountDevicesOptions,
	MountedDevices,
} from "../testing/mount-devices";
import { DeployWizard } from "./deploy-wizard";
import { deployDraftKey, deployStorageKey } from "./use-deploy-draft";

/*
 * Shared by the deploy DOM tests. Import it dynamically, after `installDom()`:
 *   const kit = await import("../deploy-test-kit");
 */

export const EDGE = SAMPLE_IDS.edge;
export const STUDIO = SAMPLE_IDS.studio;
export const LAB = SAMPLE_IDS.lab;
export const WAREHOUSE = SAMPLE_IDS.warehouse;
export const COLD = SAMPLE_IDS.cold;

export const VISITOR = "app_visitor_checkin";
export const CRM = "app_crm_sync";
export const INVOICE = "app_invoice_ai";

/** The wizard as the screen switch mounts it: route and scope come from the area's routing. */
export function WizardHost() {
	const { route, scope } = useDevicesRoute();
	return <DeployWizard route={route} scope={scope} />;
}

export const text = (root: ParentNode = document.body) =>
	(root as HTMLElement).textContent?.replace(/\s+/g, " ") ?? "";

/** Rendered copy without ids: what R3 (no machine vocabulary) is checked against. */
export function copyOf(root: HTMLElement): string {
	const clone = root.cloneNode(true) as HTMLElement;
	for (const el of clone.querySelectorAll("[data-idref], .font-mono, input"))
		el.remove();
	return text(clone);
}

/** Banned words (APP §7) and anything that looks like a wire code. */
export const MACHINE_WORDS =
	/\b(placements?|grants?|replicas?|standalone)\b|\b[a-z]+_[a-z_]+\b/i;

export const primaries = (root: ParentNode = document.body) =>
	root.querySelectorAll("[data-dv-primary]").length;

export function wizardSearch(
	scope: "app" | "account",
	params: Record<string, string | string[] | undefined>,
): string {
	const search = new URLSearchParams();
	const add = (key: string, value: string | string[] | undefined) => {
		for (const item of [value].flat())
			if (item !== undefined) search.append(key, item);
	};
	if (scope === "app") add("id", params.id);
	search.set("flow", "deploy");
	for (const [key, value] of Object.entries(params))
		if (key !== "id") add(key, value);
	return search.toString();
}

export type Mount = (
	node: ReactNode,
	options?: MountDevicesOptions,
) => Promise<MountedDevices>;

/** Mount the wizard on an app page (`/library/config/devices?id=…&flow=deploy…`). */
export function mountApp(
	mount: Mount,
	appId: string,
	params: Record<string, string | string[] | undefined> = {},
	options: MountDevicesOptions = {},
) {
	return mount(<WizardHost />, {
		host: "app",
		search: wizardSearch("app", { id: appId, mode: "new", ...params }),
		...options,
	});
}

/** Mount the wizard in the account area (`/settings/devices?flow=deploy&device=…`). */
export function mountAccount(
	mount: Mount,
	params: Record<string, string | string[] | undefined>,
	options: MountDevicesOptions = {},
) {
	return mount(<WizardHost />, {
		host: "account",
		search: wizardSearch("account", params),
		...options,
	});
}

export const PLAN_APPS: Record<string, PlanApp> = {
	[VISITOR]: VISITOR_PLAN_APP,
	[CRM]: APPS.app_crm_sync,
	[INVOICE]: APPS.app_invoice_ai,
};

interface SeedOptions {
	route: Omit<DeployRoute, "screen">;
	scope: DevicesScope;
	appId: string;
	reached?: number;
	change?(draft: DeployDraft): DeployDraft;
}

/** Saved progress as an earlier visit left it; the wizard resumes from it. */
export function seedDraft(fake: FakeWorkspace, options: SeedOptions) {
	const route: DeployRoute = { screen: "deploy", ...options.route };
	const draft = makePlan({
		scope: options.scope,
		route,
		app: PLAN_APPS[options.appId] ?? null,
		deploymentId: "0d0d0d0d-0000-4000-8000-000000000001",
		now: fake.seed.now,
	});
	const key = deployStorageKey(
		fake.workspace.scopeKey,
		deployDraftKey(route, options.scope),
	);
	globalThis.sessionStorage.setItem(
		key,
		JSON.stringify({
			v: 1,
			draft: options.change ? options.change(draft) : draft,
			reached: options.reached ?? 0,
			deployed: false,
			seeded: true,
			touched: true,
		}),
	);
	return key;
}

/** Everything the wizard keeps in `sessionStorage`, as one string. */
export function savedText(): string {
	const store = globalThis.sessionStorage;
	return Array.from({ length: store.length }, (_, index) =>
		store.getItem(store.key(index) ?? ""),
	).join("\n");
}

export const footBlocking = (root: ParentNode = document.body) =>
	root.querySelector("[data-foot-blocking]")?.textContent ?? null;
