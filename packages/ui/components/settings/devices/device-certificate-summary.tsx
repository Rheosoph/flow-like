"use client";

import { useQuery } from "@tanstack/react-query";
import { readPublicCertificates } from "../../../lib/device-management/certificates";
import type { DeviceAccountScope } from "../../../lib/device-management/storage";
import { useBackend } from "../../../state/backend-state";
import type { IProfile } from "../../../types";
import { CertificateWarnings } from "./device-certificates";

export function DeviceCertificateSummary({
	deviceId,
	profile,
	scope,
}: {
	deviceId: string;
	profile: IProfile;
	scope: DeviceAccountScope;
}) {
	const backend = useBackend();
	const inventory = useQuery({
		queryKey: [
			"device-certificate-inventory",
			scope.apiOrigin,
			scope.issuer,
			scope.account,
			profile.id,
			deviceId,
		],
		queryFn: () => readPublicCertificates(backend.apiState, profile, deviceId),
		staleTime: 30_000,
		gcTime: 0,
		refetchInterval: (query) => (query.state.error ? false : 60_000),
		refetchIntervalInBackground: false,
		retry: false,
		meta: { persist: false },
	});
	if (inventory.isPending)
		return (
			<p className="text-xs text-muted-foreground">
				Checking certificate expiry…
			</p>
		);
	if (inventory.isError)
		return (
			<p className="text-xs text-muted-foreground">
				Certificate expiry information is unavailable. Unlock management to
				inspect the device.
			</p>
		);
	if (!inventory.data.certificates.length) return null;
	return (
		<div className="space-y-1">
			<CertificateWarnings certificates={inventory.data.certificates} />
			<p className="text-xs text-muted-foreground">
				{inventory.data.certificates.length} service certificate
				{inventory.data.certificates.length === 1 ? "" : "s"}. Earliest expiry:{" "}
				{new Date(
					Math.min(
						...inventory.data.certificates.map(
							(certificate) => certificate.not_after,
						),
					) * 1000,
				).toLocaleString()}
				. Unlock management for details.
			</p>
			{inventory.data.updated_at !== null && (
				<p className="text-xs text-muted-foreground">
					Certificate inventory received{" "}
					{new Date(inventory.data.updated_at * 1000).toLocaleString()}.
				</p>
			)}
		</div>
	);
}
