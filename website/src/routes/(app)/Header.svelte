<script lang="ts">
	import GitHub from "$lib/components/GitHub.svelte"
	import Logo from "$lib/components/Logo.svelte"
	import type { Action } from "svelte/action"
	import Hamburger from "./Hamburger.svelte"
	import Search from "./Search.svelte"

	let hideSearch = $state(false)
	let headerHidden = $state(false)

	const handleScroll = () => {
		hideSearch = window.scrollY > 0
	}

	$effect(() => {
		handleScroll()
	})

	const headerIntersection: Action = (node) => {
		$effect(() => {
			const callback: IntersectionObserverCallback = (entries) => {
				for (const entry of entries) {
					headerHidden = !entry.isIntersecting
				}
			}

			const observer = new IntersectionObserver(callback, {
				threshold: 0,
				rootMargin: "-57px 0px 0px 0px",
			})

			observer.observe(node)

			return () => {
				observer.disconnect()
			}
		})
	}
</script>

<svelte:window on:scroll={handleScroll} />

<header
	use:headerIntersection
	data-hidden={headerHidden ? true : null}
	data-hide-search={hideSearch ? true : null}
	class="bg-header group border-b pt-14 sm:!bg-transparent"
>
	<div
		class="bg-header fixed inset-x-0 top-0 z-50 bg-opacity-100 backdrop-blur-lg transition-[background] group-data-[hidden]:border-b group-data-[hidden]:bg-opacity-80 sm:!border-b sm:!bg-opacity-80"
	>
		<div class="mx-auto flex h-14 max-w-screen-lg items-center justify-between px-4">
			<a href="/">
				<Logo class="text-primary h-7" />
			</a>
			<div class="hidden w-full max-w-80 sm:flex">
				<Search />
			</div>
			<div class="[&_a:hover]:text-heading hidden items-center space-x-6 sm:flex [&_a]:transition">
				<nav class="flex items-center space-x-6 font-medium">
					<a href="https://docs.pesde.daimond113.com/">Docs</a>
					<a href="https://docs.pesde.daimond113.com/registry/policies">Policies</a>
				</nav>

				<a href="https://github.com/pesde-pkg/pesde" target="_blank" rel="noreferrer noopener">
					<GitHub class="size-6" />
				</a>
			</div>
			<div class="flex items-center sm:hidden">
				<Hamburger />
			</div>
		</div>
	</div>
	<div class="overflow-hidden px-4 pb-2 pt-1 sm:hidden">
		<div
			class="transition duration-300 group-data-[hide-search]:-translate-y-1 group-data-[hide-search]:opacity-0"
		>
			<Search />
		</div>
	</div>
</header>
