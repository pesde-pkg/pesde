import { markdown } from "$lib/markdown"
import { fetchRegistry, RegistryHttpError } from "$lib/registry-api"
import type { PageLoad } from "./$types"

const fetchReadme = async (
	fetcher: typeof fetch,
	name: string,
	version: string,
	target: string,
) => {
	try {
		const res = await fetchRegistry(
			`packages/${encodeURIComponent(name)}/${encodeURIComponent(version)}/${encodeURIComponent(target)}/readme`,
			fetcher,
		)

		return res.text()
	} catch (e) {
		if (e instanceof RegistryHttpError && e.response.status === 404) {
			return "*No README provided*"
		}
		throw e
	}
}

export const load: PageLoad = async ({ parent, fetch }) => {
	const { pkg } = await parent()
	const { name, version, targets } = pkg

	const readmeText = await fetchReadme(fetch, name, version, targets[0].kind)

	const file = await (await markdown)
		.process(readmeText)

	const readmeHtml = file.value

	return {
		readmeHtml,
		pkg,

		meta: {
			title: `${name} - ${version}`,
			description: pkg.description,
		},
	}
}
