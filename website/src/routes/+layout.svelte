<script>
	import { page } from "$app/stores"
	import Header from "./Header.svelte"

	import "@fontsource-variable/nunito-sans"
	import "../app.css"
	import Footer from "./Footer.svelte"

	const { children } = $props()

	const title = $derived($page.data.meta?.title)
	const description = $derived(
		$page.data.meta?.description ??
			"A package manager for the Luau programming language, supporting multiple runtimes including Roblox and Lune.",
	)
</script>

<svelte:head>
	<title>{title ? `${title} - pesde` : "pesde"}</title>
	<meta name="description" content={description} />
	<meta name="theme-color" content="#F19D1E" />

	<meta property="og:site_name" content="pesde" />
	<meta property="og:type" content="website" />
	<meta property="og:title" content={title ?? "Manage your packages for Luau"} />
	<meta property="og:description" content={description} />

	<link rel="icon" type="image/png" href="/favicon-48x48.png" sizes="48x48" />
	<link rel="icon" type="image/svg+xml" href="/favicon.svg" />
	<link rel="shortcut icon" href="/favicon.ico" />
	<link rel="apple-touch-icon" sizes="180x180" href="/apple-touch-icon.png" />
	<meta name="apple-mobile-web-app-title" content="pesde" />
	<link rel="manifest" href="/site.webmanifest" />
</svelte:head>

<Header />

<main class="min-h-screen">
	{@render children()}
</main>

<Footer />
