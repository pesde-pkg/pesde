import { PUBLIC_REGISTRY_URL } from "$env/static/public"

export type PackageResponse = {
	name: string
	version: string
	target: TargetInfo
	description: string
	published_at: string
	license: string
}

export type TargetInfo = {
	kind: TargetKind
	lib: boolean
	bin: boolean
}

export type TargetKind = "roblox" | "lune" | "luau"

export async function fetchRegistry<T>(
	path: string,
	fetcher: typeof fetch,
	options?: RequestInit,
): Promise<T> {
	const response = await fetcher(new URL(path, PUBLIC_REGISTRY_URL), options)
	if (!response.ok) {
		throw new Error(`Failed to fetch from registry: ${response.status} ${response.statusText}`)
	}

	return response.json()
}
