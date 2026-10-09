import type { HttpClient, QueryParams } from "./client.js";
import { appPath, segment } from "./paths.js";
import type { JsonObject } from "./types.js";
const devicePath = (id: string) => `/devices/${segment(id)}`;

/** Hub registry and signed/encrypted document transport. Live device commands require an encrypted controller session. */
export function createDeviceMethods(http: HttpClient) {
	return {
		listDevices(): Promise<unknown> {
			return http.request("GET", "/devices");
		},
		getDeviceSetup(): Promise<JsonObject> {
			return http.request("GET", "/devices/setup");
		},
		getDeviceUsage(): Promise<JsonObject> {
			return http.request("GET", "/devices/usage");
		},
		getDevice(id: string): Promise<JsonObject> {
			return http.request("GET", devicePath(id));
		},
		renameDevice(id: string, displayName: string | null): Promise<JsonObject> {
			return http.request("PATCH", devicePath(id), {
				body: { display_name: displayName },
			});
		},
		revokeDevice(id: string): Promise<unknown> {
			return http.request("DELETE", devicePath(id));
		},
		listDeviceEnrollments(state?: string): Promise<unknown> {
			return http.request("GET", "/devices/enrollments", { query: { state } });
		},
		createDeviceEnrollment(body: JsonObject): Promise<JsonObject> {
			return http.request("POST", "/devices/enrollments", { body });
		},
		cancelDeviceEnrollment(id: string): Promise<unknown> {
			return http.request("DELETE", `/devices/enrollments/${segment(id)}`);
		},
		getDeviceIdentity(id: string): Promise<JsonObject> {
			return http.request("GET", `${devicePath(id)}/identity`);
		},
		getDeviceManagementPolicy(id: string): Promise<JsonObject> {
			return http.request("GET", `${devicePath(id)}/management/policy`);
		},
		putDeviceManagementPolicy(
			id: string,
			policyJws: string,
		): Promise<JsonObject> {
			return http.request("PUT", `${devicePath(id)}/management/policy`, {
				body: { policy_jws: policyJws },
			});
		},
		getDeviceAccess(id: string): Promise<JsonObject> {
			return http.request("GET", `${devicePath(id)}/management/my-access`);
		},
		createControllerSignaling(
			id: string,
			participantId: string,
		): Promise<JsonObject> {
			return http.request("POST", `${devicePath(id)}/signaling/controller`, {
				body: { participant_id: participantId },
			});
		},
		listControllerVaults(): Promise<unknown> {
			return http.request("GET", "/devices/controller-vaults");
		},
		getControllerVault(id: string): Promise<JsonObject> {
			return http.request("GET", `/devices/controller-vaults/${segment(id)}`);
		},
		putControllerVault(id: string, body: JsonObject): Promise<JsonObject> {
			return http.request("PUT", `/devices/controller-vaults/${segment(id)}`, {
				body,
			});
		},
		getDeviceInventory(
			id: string,
			key: string,
			query: QueryParams = {},
		): Promise<JsonObject> {
			return http.request(
				"GET",
				`${devicePath(id)}/inventory/${segment(key)}`,
				{ query },
			);
		},
		putDeviceInventory(
			id: string,
			key: string,
			envelope: JsonObject,
		): Promise<JsonObject> {
			return http.request(
				"PUT",
				`${devicePath(id)}/inventory/${segment(key)}`,
				{ body: envelope },
			);
		},
		getDeviceResourceSummary(): Promise<unknown> {
			return http.request("GET", "/devices/resource-summary");
		},
		listDeviceResourceGrants(id: string): Promise<unknown> {
			return http.request("GET", `${devicePath(id)}/resource-grants`);
		},
		createDeviceResourceGrant(
			id: string,
			body: JsonObject,
		): Promise<JsonObject> {
			return http.request("POST", `${devicePath(id)}/resource-grants`, {
				body,
			});
		},
		getDeviceResourceGrant(id: string, grant: string): Promise<JsonObject> {
			return http.request(
				"GET",
				`${devicePath(id)}/resource-grants/${segment(grant)}`,
			);
		},
		revokeDeviceResourceGrant(id: string, grant: string): Promise<unknown> {
			return http.request(
				"DELETE",
				`${devicePath(id)}/resource-grants/${segment(grant)}`,
			);
		},
		getDeviceResourceGrantBilling(id: string, grant: string): Promise<unknown> {
			return http.request(
				"GET",
				`${devicePath(id)}/resource-grants/${segment(grant)}/billing`,
			);
		},
		approveDeviceBillingGrant(
			id: string,
			grant: string,
			body: JsonObject,
		): Promise<JsonObject> {
			return http.request(
				"POST",
				`${devicePath(id)}/resource-grants/${segment(grant)}/billing`,
				{ body },
			);
		},
		getDeviceBillingEligibility(
			id: string,
			grant: string,
		): Promise<JsonObject> {
			return http.request(
				"GET",
				`${devicePath(id)}/resource-grants/${segment(grant)}/billing/eligibility`,
			);
		},
		listDeviceBillingGrants(id: string): Promise<unknown> {
			return http.request("GET", `${devicePath(id)}/billing-grants`);
		},
		getDeviceBillingGrant(id: string, billing: string): Promise<JsonObject> {
			return http.request(
				"GET",
				`${devicePath(id)}/billing-grants/${segment(billing)}`,
			);
		},
		revokeDeviceBillingGrant(id: string, billing: string): Promise<unknown> {
			return http.request(
				"DELETE",
				`${devicePath(id)}/billing-grants/${segment(billing)}`,
			);
		},
		getDeviceBillingUsage(id: string, billing: string): Promise<JsonObject> {
			return http.request(
				"GET",
				`${devicePath(id)}/billing-grants/${segment(billing)}/usage`,
			);
		},
		listDeviceInstances(id: string): Promise<unknown> {
			return http.request("GET", `${devicePath(id)}/instances`);
		},
		getAppDeviceMetadata(app: string): Promise<JsonObject> {
			return http.request("GET", `${appPath(app)}/device-metadata`);
		},
		getAppDevicePlacements(app: string): Promise<unknown> {
			return http.request("GET", `${appPath(app)}/device-placements`);
		},
		releaseDeviceSchedule(
			app: string,
			event: string,
			deviceId: string,
			placementId: string,
		): Promise<JsonObject> {
			return http.request(
				"PUT",
				`${appPath(app)}/device-schedules/${segment(event)}`,
				{ body: { device_id: deviceId, placement_id: placementId } },
			);
		},
		giveBackDeviceSchedule(app: string, event: string): Promise<JsonObject> {
			return http.request(
				"DELETE",
				`${appPath(app)}/device-schedules/${segment(event)}`,
			);
		},
		getFleetReader(id: string, key: string): Promise<JsonObject> {
			return http.request(
				"GET",
				`${devicePath(id)}/fleet/readers/${segment(key)}`,
			);
		},
		putFleetReader(
			id: string,
			key: string,
			readerJws: string,
		): Promise<JsonObject> {
			return http.request(
				"PUT",
				`${devicePath(id)}/fleet/readers/${segment(key)}`,
				{ body: { reader_jws: readerJws } },
			);
		},
		deleteFleetReader(
			id: string,
			key: string,
			revision: number,
		): Promise<unknown> {
			return http.request(
				"DELETE",
				`${devicePath(id)}/fleet/readers/${segment(key)}`,
				{ body: { revision } },
			);
		},
		getFleetSnapshot(id: string, key: string): Promise<JsonObject> {
			return http.request(
				"GET",
				`${devicePath(id)}/fleet/snapshots/${segment(key)}`,
			);
		},
		getDeviceArchiveUsage(): Promise<JsonObject> {
			return http.request("GET", "/devices/archive-usage");
		},
		listDeviceArchives(
			id: string,
			query: QueryParams = {},
		): Promise<JsonObject> {
			return http.request("GET", `${devicePath(id)}/archives`, { query });
		},
		getDeviceArchive(id: string, archive: string): Promise<JsonObject> {
			return http.request(
				"GET",
				`${devicePath(id)}/archives/${segment(archive)}`,
			);
		},
		getFleetCertificateInventory(): Promise<unknown> {
			return http.request("GET", "/devices/certificate-inventory");
		},
		getDeviceCertificateInventory(id: string): Promise<JsonObject> {
			return http.request("GET", `${devicePath(id)}/certificate-inventory`);
		},
		listDeviceCertificateNotices(
			id: string,
			query: QueryParams = {},
		): Promise<unknown> {
			return http.request("GET", `${devicePath(id)}/certificate-notices`, {
				query,
			});
		},
		getDeviceCertificateNoticeMutes(id: string): Promise<unknown> {
			return http.request("GET", `${devicePath(id)}/certificate-notices/mute`);
		},
		muteDeviceCertificateNotices(
			id: string,
			body: JsonObject,
		): Promise<unknown> {
			return http.request("PUT", `${devicePath(id)}/certificate-notices/mute`, {
				body,
			});
		},
		unmuteDeviceCertificateNotices(
			id: string,
			query: QueryParams,
		): Promise<unknown> {
			return http.request(
				"DELETE",
				`${devicePath(id)}/certificate-notices/mute`,
				{ query },
			);
		},
		testDeviceCertificateNotice(
			id: string,
			body: JsonObject,
		): Promise<unknown> {
			return http.request(
				"POST",
				`${devicePath(id)}/certificate-notices/test`,
				{ body },
			);
		},
	};
}
