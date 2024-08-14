import { PUBLIC_REGISTRY_URL } from "$env/static/public"

export type SearchResponse = {
	count: number
	data: PackageResponse[]
}

export type PackageVersionsResponse = PackageResponse[]

export type PackageVersionResponse = PackageResponse

export type PackageResponse = {
	name: string
	version: string
	targets: TargetInfo[]
	description: string
	published_at: string
	license: string
	authors?: string[]
	repository?: string
}

export type TargetInfo = {
	kind: TargetKind
	lib: boolean
	bin: boolean
}

export type TargetKind = "roblox" | "lune" | "luau"

export class RegistryHttpError extends Error {
	name = "RegistryError"
	constructor(
		message: string,
		public response: Response,
	) {
		super(message)
	}
}

export async function fetchRegistryJson<T>(
	path: string,
	fetcher: typeof fetch,
	options?: RequestInit,
): Promise<T> {
	const response = await fetchRegistry(path, fetcher, options)
	return response.json()
}

export async function fetchRegistry(path: string, fetcher: typeof fetch, options?: RequestInit) {
	const response = await fetcher(new URL(path, PUBLIC_REGISTRY_URL), options)
	if (!response.ok) {
		throw new RegistryHttpError(`Failed to fetch ${response.url}: ${response.statusText}`, response)
	}

	return response
}
