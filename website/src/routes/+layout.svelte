<script lang="ts">
	import { page } from "$app/stores"

	import "@fontsource-variable/nunito-sans"
	import "../app.css"

	const { children } = $props()

	const siteName = $derived($page.data.meta?.siteName ?? "pesde")
	const title = $derived($page.data.meta?.title)
	const description = $derived(
		$page.data.meta?.description ??
			"A package manager for the Luau programming language, supporting multiple runtimes including Roblox and Lune.",
	)

	let themeColor = $state("#F19D1E")
	$effect(() => {
		const query = window.matchMedia("(prefers-color-scheme: dark)")

		const updateColor = (dark: boolean) => {
			themeColor = dark ? "#14100C" : "#FAEAD7"
		}

		const listener = (e: MediaQueryListEvent) => {
			updateColor(e.matches)
		}

		query.addEventListener("change", listener)
		updateColor(query.matches)

		return () => query.removeEventListener("change", listener)
	})

	function hashChange() {
		let hash

		try {
			hash = decodeURIComponent(location.hash.slice(1)).toLowerCase()
		} catch {
			return
		}

		const id = "user-content-" + hash
		const target = document.getElementById(id)

		if (target) {
			target.scrollIntoView()
		}
	}

	$effect(() => {
		hashChange()
	})
</script>

<svelte:head>
	<title>{title ? `${title} - ${siteName}` : siteName}</title>
	<meta name="description" content={description} />
	<meta name="theme-color" content={themeColor} />

	<meta property="og:site_name" content={siteName} />
	<meta property="og:type" content="website" />
	<meta property="og:title" content={title ?? "Manage your packages for Luau"} />
	<meta property="og:description" content={description} />
	<meta property="og:image" content="/favicon-48x48.png" />
	<meta name="twitter:card" content="summary" />

	<link rel="icon" type="image/png" href="/favicon-48x48.png" sizes="48x48" />
	<link rel="icon" type="image/svg+xml" href="/favicon.svg" />
	<link rel="shortcut icon" href="/favicon.ico" />
	<link rel="apple-touch-icon" sizes="180x180" href="/apple-touch-icon.png" />
	<meta name="apple-mobile-web-app-title" content="pesde" />
	<link rel="manifest" href="/site.webmanifest" />
</svelte:head>

<svelte:window onhashchange={hashChange} />

{@render children()}
