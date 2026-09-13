import { expect, mock, test } from "bun:test";
import type { IBackendState } from "../../../state/backend-state";
import { fileToAttachment } from "./attachment";

test("a profile change during one attachment prevents reading the next shared file", async () => {
	let current = true;
	const fileToUrl = mock(async () => {
		await Promise.resolve();
		current = false;
		return "asset://localhost/private-file";
	});
	const backend = { helperState: { fileToUrl } } as unknown as IBackendState;
	await expect(
		fileToAttachment(
			[new File(["one"], "one.txt"), new File(["two"], "two.txt")],
			backend,
			true,
			() => {
				if (!current) throw new Error("Account changed");
			},
		),
	).rejects.toThrow("Account changed");
	expect(fileToUrl).toHaveBeenCalledTimes(1);
});

test("ordinary attachment preparation retains file ordering and metadata", async () => {
	const first = new File(["one"], "one.txt", { type: "text/plain" });
	const fileToUrl = mock(
		async (file: File) => `asset://localhost/${file.name}`,
	);
	const backend = { helperState: { fileToUrl } } as unknown as IBackendState;
	expect(
		await fileToAttachment(
			[first, new File(["two"], "two.txt")],
			backend,
			true,
		),
	).toEqual([
		{
			name: "one.txt",
			type: first.type,
			size: 3,
			url: "asset://localhost/one.txt",
		},
		{ name: "two.txt", type: "", size: 3, url: "asset://localhost/two.txt" },
	]);
});
