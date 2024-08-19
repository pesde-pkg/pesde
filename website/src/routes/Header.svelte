<script>
	import Logo from "$lib/components/Logo.svelte"
	import Github from "$lib/components/Github.svelte"
	import Hamburger from "./Hamburger.svelte"
	import Search from "./Search.svelte"
</script>

<header class="pt-14 sm:pt-16">
	<div class="fixed inset-x-0 top-0 z-50 bg-background/80 backdrop-blur">
		<div class="mx-auto flex h-14 max-w-screen-lg items-center justify-between px-4 sm:h-16">
			<a href="/">
				<Logo class="h-7 text-primary md:h-9" />
			</a>
			<div class="hidden w-full max-w-80 sm:flex">
				<Search />
			</div>
			<div class="hidden items-center space-x-6 md:flex [&_a:hover]:text-heading [&_a]:transition">
				<nav class="flex items-center space-x-6 font-medium">
					<a href="/docs">Docs</a>
					<a href="/policies">Policies</a>
				</nav>

				<a href="https://github.com/daimond113/pesde" target="_blank" rel="noreferrer noopener">
					<Github class="size-6" />
				</a>
			</div>
			<div class="flex items-center md:hidden">
				<Hamburger />
			</div>
		</div>
	</div>
	<div class="px-4 py-1 sm:hidden">
		<Search />
	</div>
</header>
