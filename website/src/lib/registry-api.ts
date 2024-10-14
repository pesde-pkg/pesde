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
	license?: string
	authors?: string[]
	repository?: string
	dependencies: Record<string, DependencyEntry>
}

export type TargetInfo = {
	kind: TargetKind
	lib: boolean
	bin: boolean
}

export type TargetKind = "roblox" | "roblox_server" | "lune" | "luau"

export type DependencyEntry = [DependencyInfo, DependencyKind]

export type DependencyInfo = {
	index: string
	name: string
	target: string
	version: string
}

export type DependencyKind = "standard" | "peer" | "dev"

export const TARGET_KIND_DISPLAY_NAMES: Record<TargetKind, string> = {
	roblox: "Roblox",
	roblox_server: "Roblox (server)",
	lune: "Lune",
	luau: "Luau",
}

export const DEPENDENCY_KIND_DISPLAY_NAMES: Record<DependencyKind, string> = {
	standard: "Dependencies",
	peer: "Peer Dependencies",
	dev: "Dev Dependencies",
}

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
