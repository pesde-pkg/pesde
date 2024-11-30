import { sveltekit } from "@sveltejs/kit/vite"
import { readFile } from "node:fs/promises"
import path from "node:path"
import { defineConfig, type Plugin, type ResolvedConfig } from "vite"

export default defineConfig({
	plugins: [sveltekit(), cloudflareWasmImport()],
})

// This plugin allows us to import WebAssembly modules and have them work in
// both the browser, Node.js, and Cloudflare Workers.
function cloudflareWasmImport(): Plugin {
	const wasmPostfix = ".wasm"
	const importMetaPrefix = "___WASM_IMPORT_PATH___"

	let config: ResolvedConfig

	return {
		name: "cloudflare-wasm-import",
		configResolved(resolvedConfig) {
			config = resolvedConfig
		},
		async load(id) {
			if (!id.endsWith(wasmPostfix)) return

			if (config.command === "serve") {
				// Running dev server

				// We generate a module that on the browser will fetch the WASM file
				// (through a Vite `?url` import), and on the server will read the file
				// from the file system.

				return `
					import WASM_URL from ${JSON.stringify(`${id}?url`)}

					let promise
					export default function() {
						if (import.meta.env.SSR) {
							return promise ?? (promise = import("node:fs/promises")
								.then(({ readFile }) => readFile(${JSON.stringify(id)})))
						} else {
							return promise ?? (promise = fetch(WASM_URL).then(r => r.arrayBuffer()))
						}
					}
				`
			}

			// When building, we emit the WASM file as an asset and generate a module
			// that will fetch the asset in the browser, import the WASM file when in
			// a Cloudflare Worker, and read the file from the file system when in
			// Node.js.

			const wasmSource = await readFile(id)

			const refId = this.emitFile({
				type: "asset",
				name: path.basename(id),
				source: wasmSource,
			})

			return `
				import WASM_URL from ${JSON.stringify(`${id}?url`)}

				let promise
				export default function() {
					if (import.meta.env.SSR) {
						if (typeof navigator !== "undefined" && navigator.userAgent === "Cloudflare-Workers") {
							return promise ?? (promise = import(import.meta.${importMetaPrefix}${refId}))
						} else {
							return promise ?? (promise = import(\`\${"node:fs/promises"}\`)
								.then(({ readFile }) => readFile(new URL(import.meta.ROLLUP_FILE_URL_${refId}))))
						}
					} else {
						return promise ?? (promise = fetch(WASM_URL).then(r => r.arrayBuffer()))
					}
				}
			`
		},
		resolveImportMeta(property, { chunkId }) {
			if (!property?.startsWith(importMetaPrefix)) return

			const refId = property.slice(importMetaPrefix.length)
			const fileName = this.getFileName(refId)
			const relativePath = path.relative(path.dirname(chunkId), fileName)

			return JSON.stringify(relativePath)
		},
	}
}
