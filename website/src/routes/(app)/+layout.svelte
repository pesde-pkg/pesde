<script>
	import Footer from "./Footer.svelte"
	import Header from "./Header.svelte"

	const { children } = $props()
</script>

<Header />

<main class="min-h-screen">
	{@render children()}
</main>

<Footer />
