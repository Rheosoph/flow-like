export class FlowLikeError extends Error {
	constructor(
		message: string,
		public readonly statusCode?: number,
		public readonly body?: unknown,
	) {
		super(message);
		this.name = "FlowLikeError";
	}
}

export class AuthError extends FlowLikeError {
	constructor(message: string, statusCode = 401, body?: unknown) {
		super(message, statusCode, body);
		this.name = "AuthError";
	}
}

export class NotFoundError extends FlowLikeError {
	constructor(message: string, body?: unknown) {
		super(message, 404, body);
		this.name = "NotFoundError";
	}
}

export class ValidationError extends FlowLikeError {
	constructor(message: string) {
		super(message, 400);
		this.name = "ValidationError";
	}
}
