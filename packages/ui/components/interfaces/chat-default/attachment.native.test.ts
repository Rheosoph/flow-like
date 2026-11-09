import { expect, mock, test } from "bun:test";
import type { IBackendState } from "../../../state/backend-state";
import { fileToAttachment, fitsInlineUnchanged } from "./attachment";

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

function sized(name: string, type: string, bytes: number) {
	return new File([new Uint8Array(bytes)], name, { type });
}

test("an image stays inline only while it fits its share unchanged", () => {
	const photo = sized("photo.jpg", "image/jpeg", 3_000_000);
	const scan = sized("scan.png", "image/png", 3_000_000);
	const report = sized("report.pdf", "application/pdf", 50_000_000);

	expect(fitsInlineUnchanged(photo, [photo])).toBe(true);
	expect(
		fitsInlineUnchanged(sized("raw.jpg", "image/jpeg", 4_000_000), []),
	).toBe(false);
	expect(fitsInlineUnchanged(photo, [photo, scan])).toBe(false);
	expect(fitsInlineUnchanged(report, [report, photo])).toBe(true);
});

test("a per-file choice decides where each attachment is kept", async () => {
	const fileToUrl = mock(async (file: File, offline?: boolean) =>
		offline ? `asset://localhost/${file.name}` : `https://store/${file.name}`,
	);
	const backend = { helperState: { fileToUrl } } as unknown as IBackendState;
	const small = sized("small.png", "image/png", 10);
	const large = sized("large.jpg", "image/jpeg", 5_000_000);
	const attachments = await fileToAttachment([small, large], backend, (file) =>
		fitsInlineUnchanged(file, [small, large]),
	);
	expect(attachments).toMatchObject([
		{ url: "asset://localhost/small.png" },
		{ url: "https://store/large.jpg" },
	]);
});
