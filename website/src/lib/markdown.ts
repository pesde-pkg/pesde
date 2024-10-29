import rehypeShikiFromHighlighter from "@shikijs/rehype/core"
import type { Nodes } from "hast"
import { heading } from "hast-util-heading"
import { headingRank } from "hast-util-heading-rank"
import { toText } from "hast-util-to-text"
import rehypeInferDescriptionMeta from "rehype-infer-description-meta"
import rehypeRaw from "rehype-raw"
import rehypeSanitize from "rehype-sanitize"
import rehypeSlug from "rehype-slug"
import rehypeStringify from "rehype-stringify"
import remarkFrontmatter from "remark-frontmatter"
import remarkGemoji from "remark-gemoji"
import remarkGfm from "remark-gfm"
import remarkParse from "remark-parse"
import remarkRehype from "remark-rehype"
import { createCssVariablesTheme, createHighlighter } from "shiki"
import { unified } from "unified"
import type { Node } from "unist"
import { map } from "unist-util-map"

const highlighter = createHighlighter({
	themes: [],
	langs: [],
})

export const markdown = (async () => {
	return unified()
		.use(remarkParse)
		.use(remarkFrontmatter)
		.use(remarkGfm)
		.use(remarkGemoji)
		.use(remarkRehype, { allowDangerousHtml: true })
		.use(rehypeRaw)
		.use(rehypeSanitize)
		.use(rehypeShikiFromHighlighter, await highlighter, {
			lazy: true,
			theme: createCssVariablesTheme({
				name: "css-variables",
				variablePrefix: "--shiki-",
				variableDefaults: {},
				fontStyle: true,
			}),
			fallbackLanguage: "text",
		})
		.use(rehypeStringify)
		.freeze()
})()

export type TocItem = {
	id: string
	title: string
	level: number
}

export const docsMarkdown = (async () => {
	return unified()
		.use(remarkParse)
		.use(remarkFrontmatter)
		.use(remarkGfm)
		.use(remarkGemoji)
		.use(remarkRehype, { allowDangerousHtml: true, clobberPrefix: "" })
		.use(rehypeSlug)
		.use(() => (node, file) => {
			const toc: TocItem[] = []
			file.data.toc = toc

			return map(node as Nodes, (node) => {
				if (node.type === "element" && node.tagName === "a") {
					const fullUrl = new URL(node.properties.href as string, `file://${file.path}`)

					let href = node.properties.href as string
					if (fullUrl.protocol === "file:") {
						href = file.data.basePath + fullUrl.pathname.replace(/\.mdx?$/, "") + fullUrl.hash
					}

					return {
						...node,
						properties: {
							...node.properties,
							href,
						},
					}
				}

				if (heading(node)) {
					const rank = headingRank(node)
					if (rank && typeof node.properties.id === "string" && rank >= 2 && rank <= 3) {
						toc.push({
							id: node.properties.id,
							title: toText(node),
							level: rank,
						})
					}
				}

				return node
			}) as Node
		})
		.use(rehypeRaw)
		.use(rehypeSanitize)
		.use(rehypeShikiFromHighlighter, await highlighter, {
			lazy: true,
			theme: createCssVariablesTheme({
				name: "css-variables",
				variablePrefix: "--shiki-",
				variableDefaults: {},
				fontStyle: true,
			}),
			fallbackLanguage: "text",
		})
		.use(rehypeInferDescriptionMeta, {
			selector: "p",
		})
		.use(rehypeStringify)
		.freeze()
})()
