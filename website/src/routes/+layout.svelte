<script>
	import Header from "./Header.svelte"

	import "@fontsource-variable/nunito-sans"
	import "../app.css"
	import Footer from "./Footer.svelte"
</script>

<Header />
<main class="mt-16 min-h-screen">
	<slot></slot>
</main>
<Footer />
