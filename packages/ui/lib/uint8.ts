export function parseUint8ArrayToJson(value: number[] | undefined | null): any {
	if (value === undefined || value === null) return undefined;
	try {
		if (value.length === 0) return undefined;
		const decoder = new TextDecoder("utf-8");
		const uint8Array = new Uint8Array(value);
		const jsonString = decoder.decode(uint8Array);
		if (jsonString.trim() === "") return undefined;
		return JSON.parse(jsonString);
	} catch (error) {
		console.error("Error parsing Uint8Array to JSON:", error);
		return null;
	}
}

export function convertJsonToUint8Array(jsonObject: any): number[] | undefined {
	try {
		const jsonString = JSON.stringify(jsonObject);
		const encoder = new TextEncoder();
		const encoded = encoder.encode(jsonString);
		return Array.from(encoded);
	} catch (error) {
		console.error("Error converting JSON to Uint8Array:", error);
		return undefined;
	}
}
