import { afterEach, expect, test } from "bun:test";
import { ApiResponseError } from "../lib/api-error";
import type { QuotaDetail } from "../lib/quota";
import {
	handleUpgradeRequiredError,
	openUpgradeDialogIfEnabled,
	useUpgradeDialogStore,
} from "./upgrade-dialog-state";

const original = useUpgradeDialogStore.getState();
afterEach(() => useUpgradeDialogStore.setState(original));

const quota: QuotaDetail = {
	resource: "hosted_ai_calls",
	scope: "bot",
	payerId: "owner",
	plan: "enterprise_policy",
	used: 99,
	reserved: 1,
	requested: 1,
	limit: 100,
	unit: "operations",
	periodEnd: "2026-12-01T00:00:00Z",
	actions: ["contact_admin"],
};

test("monthly policy failures still show details when subscription conversion is disabled", () => {
	useUpgradeDialogStore.setState({ enabled: false, isOpen: false });
	for (const scope of ["app", "bot", "bot_group"]) {
		const detail = { ...quota, scope };
		const error = new ApiResponseError({
			status: 402,
			code: "PLAN_LIMIT_EXCEEDED",
			message: "Monthly budget reached",
			quota: detail,
		});
		expect(handleUpgradeRequiredError(error)).toBe(true);
		expect(useUpgradeDialogStore.getState().isOpen).toBe(true);
		expect(useUpgradeDialogStore.getState().trigger?.quota).toEqual(detail);
		useUpgradeDialogStore.getState().close();
		expect(
			openUpgradeDialogIfEnabled({ reason: "ai-calls", quota: detail }),
		).toBe(true);
		useUpgradeDialogStore.getState().close();
	}
});

test("account quotas still obey the disabled subscription-conversion setting", () => {
	useUpgradeDialogStore.setState({ enabled: false, isOpen: false });
	const detail = {
		...quota,
		scope: "account",
		plan: "FREE",
		actions: ["upgrade"],
	};
	const error = new ApiResponseError({
		status: 402,
		code: "PLAN_LIMIT_EXCEEDED",
		message: "Account quota reached",
		quota: detail,
	});
	expect(handleUpgradeRequiredError(error)).toBe(false);
	expect(
		openUpgradeDialogIfEnabled({ reason: "ai-calls", quota: detail }),
	).toBe(false);
	expect(useUpgradeDialogStore.getState().isOpen).toBe(false);
});
