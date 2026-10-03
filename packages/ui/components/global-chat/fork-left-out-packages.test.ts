import { describe, expect, test } from "bun:test";
import type { IBlockedPackage, IForkReport } from "../../lib/schema/app/fork";
import {
	leftOutPackages,
	leftOutPackagesResult,
} from "./fork-left-out-packages";

const skipped = (
	kind: string,
	sourceId: string,
): IForkReport["skipped"][number] => ({
	kind,
	source_id: sourceId,
	reason: `${sourceId} was skipped`,
});

const blocked = (
	packageId: string,
	block: IBlockedPackage["block"],
	overrides: Partial<IBlockedPackage> = {},
): IBlockedPackage => ({
	package_id: packageId,
	name: packageId,
	block,
	price: 0,
	request_pending: false,
	...overrides,
});

describe("leftOutPackages", () => {
	test("keeps only the skipped packages, with the report's reason", () => {
		const leftOut = leftOutPackages(
			[
				skipped("Secret", "api-key"),
				skipped("Package", "chart-kit"),
				skipped("Policy", "files"),
			],
			[],
			true,
		);

		expect(leftOut).toEqual([
			{ package_id: "chart-kit", reason: "chart-kit was skipped" },
		]);
	});

	test("says what the user can do about each package", () => {
		const leftOut = leftOutPackages(
			[
				skipped("Package", "chart-kit"),
				skipped("Package", "geo-tools"),
				skipped("Package", "team-utils"),
				skipped("Package", "in-house"),
			],
			[
				blocked("chart-kit", "paid", { price: 499 }),
				blocked("geo-tools", "request_access"),
				blocked("team-utils", "request_access", { request_pending: true }),
				blocked("in-house", "private"),
			],
			true,
		);

		expect(leftOut).toEqual([
			{
				package_id: "chart-kit",
				reason: "chart-kit was skipped",
				obtain: "buy",
				price: "€4.99",
			},
			{
				package_id: "geo-tools",
				reason: "geo-tools was skipped",
				obtain: "request_access",
			},
			{
				package_id: "team-utils",
				reason: "team-utils was skipped",
				obtain: "wait_for_approval",
			},
			{ package_id: "in-house", reason: "in-house was skipped" },
		]);
	});

	test("offers no purchase on a build that may not sell", () => {
		const leftOut = leftOutPackages(
			[skipped("Package", "chart-kit"), skipped("Package", "geo-tools")],
			[
				blocked("chart-kit", "paid", { price: 499 }),
				blocked("geo-tools", "request_access"),
			],
			false,
		);

		expect(leftOut).toEqual([
			{ package_id: "chart-kit", reason: "chart-kit was skipped" },
			{
				package_id: "geo-tools",
				reason: "geo-tools was skipped",
				obtain: "request_access",
			},
		]);
	});

	test("reports a package the preview did not know about by its reason alone", () => {
		const leftOut = leftOutPackages(
			[skipped("Package", "chart-kit")],
			undefined,
			true,
		);

		expect(leftOut).toEqual([
			{ package_id: "chart-kit", reason: "chart-kit was skipped" },
		]);
	});

	test("is empty for a fork that skipped no package", () => {
		expect(leftOutPackages([skipped("Secret", "api-key")], [], true)).toEqual(
			[],
		);
		expect(leftOutPackages(undefined, undefined, true)).toEqual([]);
	});
});

describe("leftOutPackagesResult", () => {
	test("adds nothing to the result of a fork that carried every package", () => {
		expect(
			leftOutPackagesResult([skipped("Secret", "api-key")], [], true),
		).toEqual({});
	});

	test("tells FlowPilot what is missing and to say so", () => {
		const result = leftOutPackagesResult(
			[skipped("Package", "chart-kit")],
			[blocked("chart-kit", "paid", { price: 499 })],
			true,
		);

		expect(result.left_out_packages).toEqual([
			{
				package_id: "chart-kit",
				reason: "chart-kit was skipped",
				obtain: "buy",
				price: "€4.99",
			},
		]);
		expect(result.left_out_packages_total).toBe(1);
		expect(result.note).toContain("Tell the user which packages are missing");
	});

	test("bounds the list and still reports how many were left out", () => {
		const many = Array.from({ length: 25 }, (_, index) =>
			skipped("Package", `package-${index}`),
		);
		const result = leftOutPackagesResult(many, [], true);

		expect(result.left_out_packages).toHaveLength(20);
		expect(result.left_out_packages_total).toBe(25);
	});
});
