import {
	fetchRegistryJson,
	RegistryHttpError,
	type PackageVersionsResponse,
} from "$lib/registry-api"
import { error } from "@sveltejs/kit"
import type { PageServerLoad } from "./$types"

export const load: PageServerLoad = async ({ params, fetch }) => {
	const { scope, name } = params

	try {
		const versions = await fetchRegistryJson<PackageVersionsResponse>(
			`packages/${encodeURIComponent(`${scope}/${name}`)}`,
			fetch,
		)

		versions.reverse()

		return {
			versions,
		}
	} catch (e) {
		if (e instanceof RegistryHttpError && e.response.status === 404) {
			error(404, "Package not found")
		}
		throw e
	}
}
