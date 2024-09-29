<script>
	import GitHub from "$lib/components/GitHub.svelte"
	import Logo from "$lib/components/Logo.svelte"
	import Hamburger from "./Hamburger.svelte"
	import Search from "./Search.svelte"
</script>

<header class="pt-14 sm:pt-16">
	<div class="bg-background/80 fixed inset-x-0 top-0 z-50 backdrop-blur">
		<div class="mx-auto flex h-14 max-w-screen-lg items-center justify-between px-4 sm:h-16">
			<a href="/">
				<Logo class="text-primary h-7 sm:h-9" />
			</a>
			<div class="hidden w-full max-w-80 sm:flex">
				<Search />
			</div>
			<div class="[&_a:hover]:text-heading hidden items-center space-x-6 sm:flex [&_a]:transition">
				<nav class="flex items-center space-x-6 font-medium">
					<a href="/docs">Docs</a>
					<a href="/policies">Policies</a>
				</nav>

				<a href="https://github.com/daimond113/pesde" target="_blank" rel="noreferrer noopener">
					<GitHub class="size-6" />
				</a>
			</div>
			<div class="flex items-center sm:hidden">
				<Hamburger />
			</div>
		</div>
	</div>
	<div class="px-4 py-1 sm:hidden">
		<Search />
	</div>
</header>
