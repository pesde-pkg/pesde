import {
	fetchRegistryJson,
	RegistryHttpError,
	type PackageVersionsResponse,
	type PackageVersionResponse,
} from "$lib/registry-api"
import { error } from "@sveltejs/kit"
import type { LayoutServerLoad } from "./$types"

type FetchPackageOptions =
	| {
			scope: string
			name: string
	  }
	| {
			scope: string
			name: string
			version: string
			target: string
	  }

const fetchPackage = async (fetcher: typeof fetch, options: FetchPackageOptions) => {
	const { scope, name } = options

	try {
		if ("version" in options) {
			if (options.target === undefined) {
				error(404, "Not Found")
			}

			const { version, target } = options
			return fetchRegistryJson<PackageVersionResponse>(
				`packages/${encodeURIComponent(`${scope}/${name}`)}/${version}/${target}`,
				fetcher,
			)
		}

		const versions = await fetchRegistryJson<PackageVersionsResponse>(
			`packages/${encodeURIComponent(`${scope}/${name}`)}`,
			fetcher,
		)

		const latestVersion = versions.at(-1)
		if (latestVersion === undefined) throw new Error("package has no versions *blows up*")

		return latestVersion
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
