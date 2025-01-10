import { docsMarkdown, type TocItem } from "$lib/markdown"
import { fetchRegistry, RegistryHttpError, type DocEntry } from "$lib/registry-api"
import { error } from "@sveltejs/kit"
import { VFile } from "vfile"
import type { PageLoad } from "./$types"

const findDocTitle = (docs: DocEntry[], name: string): string | undefined => {
	for (const doc of docs) {
		if ("name" in doc && doc.name === name) return doc.label
		if ("items" in doc && doc.items) {
			const title = findDocTitle(doc.items, name)
			if (title) return title
		}
	}
	return undefined
}

export const load: PageLoad = async ({ params, parent, fetch }) => {
	try {
		const page = await fetchRegistry(
			`packages/${encodeURIComponent(`${params.scope}/${params.name}`)}/${encodeURIComponent(params.version ?? "latest")}/${encodeURIComponent(params.target ?? "any")}?doc=${encodeURIComponent(params.doc)}`,
			fetch,
		).then((r) => r.text())

		const inputFile = new VFile({
			path: `/docs/${params.doc}`,
			value: page,
			data: {
				basePath: `/packages/${encodeURIComponent(params.scope)}/${encodeURIComponent(params.name)}/${encodeURIComponent(params.version ?? "latest")}/${encodeURIComponent(params.target ?? "any")}`,
			},
		})

		const file = await (await docsMarkdown).process(inputFile)

		const html = file.value

		const parentData = await parent()
		const docTitle = findDocTitle(parentData.pkg.docs ?? [], params.doc) ?? params.doc

		return {
			html,
			toc: [
				{
					id: "_top",
					title: "Overview",
					level: 2,
				},
				...(file.data.toc as TocItem[]),
			],
			meta: {
				siteName: `${parentData.pkg.name} - pesde`,
				title: docTitle,
				description: (file.data.meta as { description: string }).description,
			},
		}
	} catch (e) {
		if (e instanceof RegistryHttpError && e.response.status === 404) {
			error(404, "Page not found")
		}
		throw e
	}
}
