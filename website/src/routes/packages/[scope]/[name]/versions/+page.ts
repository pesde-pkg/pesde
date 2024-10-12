import {
	fetchRegistryJson,
	RegistryHttpError,
	type PackageVersionsResponse,
} from "$lib/registry-api"
import { error } from "@sveltejs/kit"
import type { PageLoad } from "./$types"

export const load: PageLoad = async ({ params, fetch }) => {
	const { scope, name } = params

	try {
		const versions = await fetchRegistryJson<PackageVersionsResponse>(
			`packages/${encodeURIComponent(`${scope}/${name}`)}`,
			fetch,
		)

		versions.reverse()

		return {
			versions,

			meta: {
				title: `${versions[0].name} - versions`,
				description: versions[0].description,
			},
		}
	} catch (e) {
		if (e instanceof RegistryHttpError && e.response.status === 404) {
			error(404, "Package not found")
		}
		throw e
	}
}
