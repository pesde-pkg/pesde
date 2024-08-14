import {
	fetchRegistryJson,
	RegistryHttpError,
	type PackageVersionResponse,
} from "$lib/registry-api"
import { error } from "@sveltejs/kit"
import type { LayoutServerLoad } from "./$types"

type FetchPackageOptions = {
	scope: string
	name: string
	version?: string
	target?: string
}

const fetchPackage = async (fetcher: typeof fetch, options: FetchPackageOptions) => {
	const { scope, name, version = "latest", target = "any" } = options

	try {
		return await fetchRegistryJson<PackageVersionResponse>(
			`packages/${encodeURIComponent(`${scope}/${name}`)}/${version}/${target}`,
			fetcher,
		)
	} catch (e) {
		if (e instanceof RegistryHttpError && e.response.status === 404) {
			error(404, "This package does not exist.")
		}
		throw e
	}
}

export const load: LayoutServerLoad = async ({ params }) => {
	const { scope, name, version, target } = params

	const options = version ? { scope, name, version, target } : { scope, name }

	const pkg = await fetchPackage(fetch, options)

	return {
		pkg,
	}
}
