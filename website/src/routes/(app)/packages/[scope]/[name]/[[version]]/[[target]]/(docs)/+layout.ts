import {
	fetchRegistryJson,
	isTargetKind,
	RegistryHttpError,
	type PackageVersionResponse,
	type PackageVersionsResponse,
	type TargetKind,
} from "$lib/registry-api"
import { error, redirect } from "@sveltejs/kit"
import type { LayoutLoad } from "./$types"

type FetchPackageOptions = {
	scope: string
	name: string
	version: string
	target: TargetKind
}

const fetchPackageAndVersions = async (fetcher: typeof fetch, options: FetchPackageOptions) => {
	const { scope, name, version, target } = options

	try {
		const versionsResponse = await fetchRegistryJson<PackageVersionsResponse>(
			`packages/${encodeURIComponent(`${scope}/${name}`)}`,
			fetcher,
		)

		const versions = Object.keys(versionsResponse.versions).reverse()

		return {
			pkg: {
				name: versionsResponse.name,
				version,
				targets: versionsResponse.versions[version].targets,
				description: versionsResponse.versions[version].description,
				...versionsResponse.versions[version].targets[target],
			},
			versions,
		}
	} catch (e) {
		if (e instanceof RegistryHttpError && e.response.status === 404) {
			error(404, "This package does not exist.")
		}
		throw e
	}
}

export const load: LayoutLoad = async ({ params, url, fetch }) => {
	const { scope, name, version, target } = params

	if (version !== undefined && target === undefined) {
		error(404, "Not Found")
	}

	if (version === undefined || version === "latest" || !isTargetKind(target)) {
		const pkg = await fetchRegistryJson<PackageVersionResponse>(
			`packages/${encodeURIComponent(`${scope}/${name}`)}/${version ?? "latest"}/${target ?? "any"}`,
			fetch,
		)

		const path = url.pathname.split("/").slice(6).join("/")

		return redirect(303, `/packages/${scope}/${name}/${pkg.version}/${pkg.targets[0].kind}/${path}`)
	}

	const { pkg, versions } = await fetchPackageAndVersions(fetch, { scope, name, version, target })

	return {
		pkg,
		versions,
	}
}
