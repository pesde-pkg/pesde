import { PUBLIC_REGISTRY_URL } from "$env/static/public"

export type SearchResponse = {
	count: number
	data: PackageResponse[]
}

export type PackageVersionsResponse = {
	name: string
	deprecated?: string
	versions: Record<
		string,
		{
			description?: string
			targets: Record<
				TargetKind,
				{ target: TargetInfoInner; yanked?: boolean } & PackageResponseInner
			>
		}
	>
}

export type PackageVersionResponse = PackageResponse

export type PackageResponseInner = {
	published_at: string
	license?: string
	authors?: string[]
	repository?: string
	docs?: DocEntry[]
	dependencies?: Record<string, DependencyEntry>
}

export type PackageResponse = {
	name: string
	version: string
	targets: TargetInfo[]
	description?: string
	deprecated?: string
} & PackageResponseInner

export type TargetInfoInner = {
	lib: boolean
	bin: boolean
	scripts?: string[]
}

export type TargetInfo = {
	yanked?: boolean
	kind: TargetKind
} & TargetInfoInner

export type TargetKind = "roblox" | "roblox_server" | "lune" | "luau"

export const isTargetKind = (value: string | undefined): value is TargetKind => {
	return value === "roblox" || value === "roblox_server" || value === "lune" || value === "luau"
}

export type DependencyEntry = [DependencyInfo, DependencyKind]

export type DependencyInfo =
	| {
			index: string
			name: string
			target?: string
			version: string
	  }
	| {
			index: string
			wally: string
			version: string
	  }

export type DependencyKind = "standard" | "peer" | "dev"

export type DocEntry = DocEntryCategory | DocEntryPage

export type DocEntryBase = {
	label: string
	position: number
}

export type DocEntryCategory = DocEntryBase & {
	items?: DocEntry[]
	collapsed?: boolean
}

export type DocEntryPage = DocEntryBase & {
	name: string
}

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
