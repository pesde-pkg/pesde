import {
	fetchRegistry,
	type PackageVersionsResponse,
	type PackageVersionResponse,
} from "$lib/registry-api"
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

	if ("version" in options) {
		const { version, target } = options
		return fetchRegistry<PackageVersionResponse>(
			`packages/${encodeURIComponent(`${scope}/${name}`)}/${version}/${target}`,
			fetcher,
		)
	}

	const versions = await fetchRegistry<PackageVersionsResponse>(
		`packages/${encodeURIComponent(`${scope}/${name}`)}`,
		fetcher,
	)

	const latestVersion = versions.at(-1)
	if (latestVersion === undefined) throw new Error("package has no versions *blows up*")

	return latestVersion
}

export const load: LayoutServerLoad = async ({ params }) => {
	const { scope, name, version, target } = params

	const options = version ? { scope, name, version, target } : { scope, name }

	const pkg = await fetchPackage(fetch, options)

	return {
		pkg,
	}
}
