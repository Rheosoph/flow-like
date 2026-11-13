/** `<prefix>-YYYY-MM-DD-HHMMSS[-index]<extension>` in local time: readable and sorts by time. */
export function captureFileName(
	prefix: string,
	at: Date,
	extension: string,
	index?: number,
): string {
	const pad = (value: number) => String(value).padStart(2, "0");
	const date = `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}`;
	const time = `${pad(at.getHours())}${pad(at.getMinutes())}${pad(at.getSeconds())}`;
	return `${prefix}-${date}-${time}${index ? `-${index}` : ""}${extension}`;
}
