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
		const versionsResponse = await fetchRegistryJson<PackageVersionsResponse>(
			`packages/${encodeURIComponent(`${scope}/${name}`)}`,
			fetch,
		)

		const versions = Object.entries(versionsResponse.versions)
			.map(([version, data]) => ({
				version,
				description: data.description,
				targets: data.targets,
				published_at: Object.values(data.targets)
					.map(({ published_at }) => new Date(published_at))
					.sort()
					.reverse()[0]
					.toISOString(),
			}))
			.reverse()

		return {
			name: versionsResponse.name,
			versions,

			meta: {
				title: `${versionsResponse.name} - versions`,
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
