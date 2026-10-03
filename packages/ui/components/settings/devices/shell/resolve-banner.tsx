"use client";

import { useTranslation } from "@flow-like/locales";
import type {
	CopyParams,
	CopyRef,
} from "../../../../lib/device-management/model/types";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Banner } from "../primitives/banner";
import { DvButton } from "../primitives/dv-button";
import type { ResolveCode } from "../routing/resolve-target";

type Copy = (
	t: DevicesT,
	params: CopyParams,
	at: (s: number) => string,
) => string;

/** One sentence per reason a deep link could not be opened (SPEC §3.11, IA §6.1.4). */
const RESOLVE_COPY = {
	device_not_found: (t, { device, hub }) =>
		hub === undefined
			? t(
					"devices:shell.resolve.deviceNotFound",
					"No device with the ID {{device}} on this hub. It may have been set up again or removed.",
					{ device },
				)
			: t(
					"devices:shell.resolve.deviceNotFoundOnHub",
					"No device with the ID {{device}} on {{hub}}. It may have been set up again or removed.",
					{ device, hub },
				),
	device_not_shared: (t, { device }) =>
		t(
			"devices:shell.resolve.deviceNotShared",
			"The device {{device}} isn't shared with you any more. Ask its owner to share it again.",
			{ device },
		),
	device_other_account: (t, { device }) =>
		t(
			"devices:shell.resolve.deviceOtherAccount",
			"The device {{device}} belongs to another account on this computer. Sign in with that account to open it.",
			{ device },
		),
	device_revoked: (t, { device, revokedAt }, at) =>
		typeof revokedAt === "number"
			? t(
					"devices:shell.resolve.deviceRevokedAt",
					"{{device}} was revoked on {{date}}, so this can't be done any more.",
					{ device, date: at(revokedAt) },
				)
			: t(
					"devices:shell.resolve.deviceRevoked",
					"{{device}} was revoked, so this can't be done any more.",
					{ device },
				),
	service_not_found: (t, { device, service }) =>
		t(
			"devices:shell.resolve.serviceNotFound",
			"{{device}} has no service {{service}}. It may have been removed.",
			{ device, service },
		),
	certificate_not_found: (t, { device, certificate }) =>
		t(
			"devices:shell.resolve.certificateNotFound",
			"{{device}} has no certificate {{certificate}}. It may have been replaced or deleted.",
			{ device, certificate },
		),
	app_not_found: (t, { app }) =>
		t(
			"devices:shell.resolve.appNotFound",
			"No app with the ID {{app}} on this hub or on this computer.",
			{ app },
		),
	setup_not_found: (t) =>
		t(
			"devices:shell.resolve.setupNotFound",
			"That setup isn't waiting any more. It was finished, cancelled or it expired.",
		),
} satisfies Record<ResolveCode, Copy>;

export function resolveCopy(
	t: DevicesT,
	banner: CopyRef<ResolveCode>,
	at: (unixSeconds: number) => string,
): string {
	return RESOLVE_COPY[banner.code](t, banner.params ?? {}, at);
}

/** Says why a link could not be opened; the nearest valid view is already showing. */
export function ResolveBanner({
	banner,
	onDismiss,
	className,
}: Readonly<{
	banner: CopyRef<ResolveCode>;
	onDismiss(): void;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	return (
		<Banner
			tone="warning"
			className={className}
			actions={
				<DvButton size="sm" onClick={onDismiss}>
					{t("shell.resolve.dismiss", "Dismiss")}
				</DvButton>
			}
		>
			<span data-resolve={banner.code}>{resolveCopy(t, banner, time.abs)}</span>
		</Banner>
	);
}
