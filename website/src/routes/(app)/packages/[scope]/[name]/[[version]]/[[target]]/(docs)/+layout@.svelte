<script lang="ts">
	import { page } from "$app/stores"
	import Logo from "$lib/components/Logo.svelte"
	import Logomark from "$lib/components/Logomark.svelte"
	import type { TocItem } from "$lib/markdown"
	import { ChevronsUpDown } from "lucide-svelte"
	import type { Action } from "svelte/action"
	import Hamburger from "./Hamburger.svelte"
	import TocMobile from "./MobileNavbar.svelte"
	import Sidebar from "./Sidebar.svelte"
	import Tab from "./Tab.svelte"
	import TargetSelector from "./TargetSelector.svelte"
	import Toc from "./Toc.svelte"
	import TocObserver from "./TocObserver.svelte"
	import VersionSelector from "./VersionSelector.svelte"

	const { children, data } = $props()
	const [scope, name] = data.pkg.name.split("/")

	let hideNavigation = $state(false)
	let headerHidden = $state(false)

	const handleScroll = () => {
		hideNavigation = window.scrollY > 0
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

	const toc: TocItem[] = $page.error
		? [
				{
					id: "_top",
					title: "Overview",
					level: 2,
				},
			]
		: ($page.data.toc ?? [])
</script>

<svelte:window on:scroll={handleScroll} />

<TocObserver toc={$page.data.toc ?? []} />

<div class="min-h-screen">
	<header
		class="bg-header group w-full border-b pt-14"
		use:headerIntersection
		data-hide-navigation={hideNavigation ? true : null}
		data-hidden={headerHidden ? true : null}
	>
		<div
			class="bg-header fixed top-0 z-10 w-full backdrop-blur-lg transition-[background] group-data-[hidden]:border-b xl:group-data-[hidden]:bg-opacity-80"
		>
			<div class="mx-auto flex h-14 max-w-screen-2xl items-center px-4">
				{#snippet separator()}
					<span class="text-body/60 px-2 text-xl">/</span>
				{/snippet}

				<span class="flex min-w-0 items-center">
					<a
						class="flex min-w-0 items-center"
						href={`/packages/${scope}/${name}/${$page.params.version ?? "latest"}/${$page.params.target ?? "any"}`}
					>
						<span class="text-primary mr-2">
							<Logomark class="h-7" />
						</span>
						<span class="min-w-0 truncate">{scope}</span>
						{@render separator()}
						<span class="text-heading min-w-0 truncate font-medium">{name}</span>
					</a>
					<span class="hidden items-center lg:flex">
						{#snippet trigger(props: Record<string, unknown>, label: string)}
							<button
								{...props}
								class="flex items-center transition-opacity data-[disabled]:opacity-50"
							>
								{label}
								<ChevronsUpDown class="ml-1 size-4" />
							</button>
						{/snippet}

						{@render separator()}
						<VersionSelector {trigger} sameWidth={false} />
						{@render separator()}
						<TargetSelector {trigger} sameWidth={false} />
					</span>
				</span>

				<div class="ml-auto flex items-center lg:hidden">
					<Hamburger />
				</div>
			</div>
		</div>
		<div class="group-data-[hidden]:invisible">
			<div class="-mb-px overflow-hidden pb-px">
				<nav
					class="transition duration-300 group-data-[hide-navigation]:-translate-y-1 group-data-[hide-navigation]:opacity-0"
				>
					<div class="mx-auto flex max-w-screen-2xl px-4">
						<Tab tab="docs" active={$page.data.activeTab == "docs"}>Docs</Tab>
						<Tab tab="reference" active={$page.data.activeTab == "reference"}>Reference</Tab>
					</div>
				</nav>
			</div>
		</div>
	</header>

	<TocMobile {toc} />

	<div class="mx-auto flex max-w-screen-2xl">
		<Sidebar items={$page.data.sidebar ?? []} />
		<main class="mx-auto w-full max-w-screen-md px-4 md:px-6">
			<div id="_top"></div>
			{@render children()}
		</main>
		<Toc {toc} />
	</div>
</div>

<footer class="mx-auto max-w-screen-2xl px-4">
	<div class="border-t py-16">
		<p class="text-center font-semibold">
			Documentation powered by<br />
			<span class="mt-2 inline-block">
				<a href="/"><Logo class="inline h-8" /></a>
				<span class="mx-2 text-lg font-semibold">+</span>
				<a href="https://eryn.io/moonwave">
					<svg
						class="inline h-6"
						viewBox="0 0 76 36"
						fill="none"
						xmlns="http://www.w3.org/2000/svg"
					>
						<title>Moonwave</title>
						<path
							d="M0.212789 31.8083C-0.187211 31.6083 0.012789 31.0083 0.512789 31.1083C6.01279 32.7083 15.9128 33.8083 21.4128 24.5083C28.8128 12.1083 31.8128 -5.59168 58.0128 1.70832C58.5128 1.80832 58.5128 2.50832 58.1128 2.60832C54.5128 4.10832 45.3128 9.20832 49.9128 19.5083C49.9128 19.5083 53.3128 8.50832 66.0128 11.8083C66.3128 11.9083 66.3128 12.3083 66.0128 12.5083C63.5128 13.6083 56.5128 17.6083 59.1128 25.8083C61.4128 33.0083 71.8128 32.5083 75.1128 32.1083C75.6128 32.0083 75.7128 32.6083 75.1128 32.9083C72.2128 34.3083 56.7128 40.1083 50.3128 26.9083C50.3128 26.9083 41.9128 32.5083 37.5128 21.1083C37.3128 21.2083 25.7128 45.0083 0.212789 31.8083Z"
							fill="currentColor"
						/>
					</svg>
				</a>
			</span>
		</p>
	</div>
</footer>
