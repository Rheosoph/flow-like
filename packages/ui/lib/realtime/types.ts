export interface IRealtimeIceServer {
	urls: string | string[];
	username?: string;
	credential?: string;
}

export interface IRealtimeAccess {
	jwt: string;
	encryption_key: string;
	key_id: string;
	ice_servers?: IRealtimeIceServer[];
	/** Unix timestamp in seconds. */
	ice_expires_at?: number;
}

export interface IJwk {
	kty: string;
	kid?: string;
	alg?: string;
	use?: string;
	// EC (P-256)
	crv?: string;
	x?: string;
	y?: string;
	// RSA (optional fields, not used currently)
	n?: string;
	e?: string;
}

export interface IJwks {
	keys: IJwk[];
}
