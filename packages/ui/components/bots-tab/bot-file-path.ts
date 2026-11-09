/** Whether a chat reference names a file relative to the bot's workspace. */
export function isBotFileReference(url: string): boolean {
	return /^(?:storage:\/\/|\.\/)?bot\//.test(url);
}

/** Resolve URL-encoded names without allowing a reference to leave the bot folder. */
export function botFilePath(url: string): string {
	if (!isBotFileReference(url)) throw new Error("Not a bot file reference.");
	const path = url.replace(/^(?:storage:\/\/|\.\/)/, "");
	const segments = path
		.split("/")
		.map((segment) => decodeURIComponent(segment));
	if (
		path.length > 1024 ||
		/[?#]/.test(path) ||
		segments.length < 2 ||
		segments.some(
			(segment) =>
				!segment ||
				segment === "." ||
				segment === ".." ||
				/[\\/\p{Cc}]/u.test(segment),
		)
	) {
		throw new Error("Invalid bot file reference.");
	}
	// The storage backend decodes URL segments when constructing its object key.
	return path;
}
