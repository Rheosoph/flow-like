/**
 * Saves a signed URL through a blob, since browsers ignore `download` on
 * cross-origin links and would navigate to the object instead.
 */
export async function saveUrlAsFile(url: string, fileName: string) {
	const response = await fetch(url);
	if (!response.ok) {
		throw new Error(
			`Downloading ${fileName} failed with HTTP ${response.status}`,
		);
	}
	const blob = new Blob([await response.blob()], {
		type: "application/octet-stream",
	});
	const objectUrl = URL.createObjectURL(blob);
	const anchor = document.createElement("a");
	anchor.href = objectUrl;
	anchor.download = fileName;
	document.body.appendChild(anchor);
	anchor.click();
	document.body.removeChild(anchor);
	URL.revokeObjectURL(objectUrl);
}
