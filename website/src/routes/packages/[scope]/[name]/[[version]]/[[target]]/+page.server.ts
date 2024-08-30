import { fetchRegistry, RegistryHttpError } from "$lib/registry-api"
import { unified } from "unified"
import type { PageServerLoad } from "./$types"
import remarkParse from "remark-parse"
import remarkRehype from "remark-rehype"
import remarkGfm from "remark-gfm"
import remarkGemoji from "remark-gemoji"
import rehypeSanitize from "rehype-sanitize"
import rehypeStringify from "rehype-stringify"
import rehypeRaw from "rehype-raw"
import rehypeShiki from "@shikijs/rehype"
import { createCssVariablesTheme } from "shiki"

const fetchReadme = async (
	fetcher: typeof fetch,
	name: string,
	version: string,
	target: string,
) => {
	try {
		const res = await fetchRegistry(
			`packages/${encodeURIComponent(name)}/${version}/${target}`,
			fetcher,
			{
				headers: {
					Accept: "text/plain",
				},
			},
		)

		return res.text()
	} catch (e) {
		if (e instanceof RegistryHttpError && e.response.status === 404) {
			return "*No README provided*"
		}
		throw e
	}
}

export const load: PageServerLoad = async ({ parent }) => {
	const { pkg } = await parent()
	const { name, version, targets } = pkg

	const readmeText = await fetchReadme(fetch, name, version, targets[0].kind)

	const file = await unified()
		.use(remarkParse)
		.use(remarkGfm)
		.use(remarkGemoji)
		.use(remarkRehype, { allowDangerousHtml: true })
		.use(rehypeRaw)
		.use(rehypeSanitize)
		.use(rehypeShiki, {
			theme: createCssVariablesTheme({
				name: "css-variables",
				variablePrefix: "--shiki-",
				variableDefaults: {},
				fontStyle: true,
			}),
			defaultLanguage: "text",
		})
		.use(rehypeStringify)
		.process(readmeText)

	const readmeHtml = file.value

	return {
		readmeHtml,
		pkg,
	}
}
