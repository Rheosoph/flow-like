import { describe, expect, test } from "bun:test";
import type { TFunction } from "i18next";
import type { ILockedItem } from "../../state/backend-state/locked-storage-state";
import {
	isLockedEntry,
	lockedBrowserEntries,
	lockedFolderForPrefix,
	nextRetentionDay,
	retentionFromDay,
	retentionLabel,
	shortSha256,
} from "./locked-storage-entries";

const NAME = "Übersicht (2)#1.pdf";

const lockedItem = { id: "item-1", name: NAME, live_size: 42 } as ILockedItem;

describe("locked browser entries", () => {
	test("the locked API gets display folder names", () => {
		expect(lockedFolderForPrefix("/Akten/%C3%9Cbersicht/")).toBe(
			"Akten/Übersicht",
		);
		expect(lockedFolderForPrefix("")).toBe("");
	});

	test("folders navigate by the listed segment and skip storage duplicates", () => {
		const entries = lockedBrowserEntries(
			"Akten",
			{ items: [lockedItem], folders: ["Übersicht", "Verträge"] },
			[
				{
					location: "apps/app/upload/Akten/%C3%9Cbersicht",
					is_dir: true,
					size: 0,
					last_modified: "",
				},
			],
		);
		expect(
			entries.filter((entry) => entry.is_dir).map((entry) => entry.location),
		).toEqual(["Akten/Vertr%C3%A4ge"]);
		const [row] = entries.filter(isLockedEntry);
		expect(row?.lockedItem).toBe(lockedItem);
		expect(row?.size).toBe(42);
		expect(row?.location).toBe("locked:item-1/%C3%9Cbersicht (2)%231.pdf");
	});

	test("no listing yields no entries", () => {
		expect(lockedBrowserEntries("", undefined, [])).toEqual([]);
	});
});

describe("retention extension dates", () => {
	test("the earliest day is the next UTC day and keeps the time of day", () => {
		const current = "2036-10-21T12:30:00.000Z";
		expect(nextRetentionDay(current)).toBe("2036-10-22");
		expect(retentionFromDay("2036-10-22", current)).toBe(
			"2036-10-22T12:30:00.000Z",
		);
		expect(
			new Date(retentionFromDay(nextRetentionDay(current), current)) >
				new Date(current),
		).toBe(true);
	});

	test("short checksums keep both ends", () => {
		expect(shortSha256(`${"a".repeat(56)}12345678`)).toBe("aaaaaaaa…12345678");
		expect(shortSha256("abc")).toBe("abc");
	});
});

describe("retention labels", () => {
	const t = ((key: string) => key) as unknown as TFunction<"common">;
	const label = (
		retain_until: string,
		delete_after_retention: boolean,
		legal_hold = false,
	) =>
		retentionLabel(
			{ retain_until, delete_after_retention, legal_hold },
			"en",
			t,
		);
	const future = "2999-01-01T00:00:00Z";
	const past = "2000-01-01T00:00:00Z";

	test("kept versions name the retention date only", () => {
		expect(label(future, false)).toBe("lockedStorage.retainedUntil");
		expect(label(past, false)).toBe("lockedStorage.retentionEndedOn");
	});

	test("versions marked for deletion name the deletion", () => {
		expect(label(future, true)).toBe("lockedStorage.deletedOn");
		expect(label(past, true)).toBe("lockedStorage.deletionDue");
	});

	test("a legal hold keeps a version marked for deletion", () => {
		expect(label(future, true, true)).toBe("lockedStorage.retainedUntil");
		expect(label(past, true, true)).toBe("lockedStorage.retentionEndedOn");
	});
});
