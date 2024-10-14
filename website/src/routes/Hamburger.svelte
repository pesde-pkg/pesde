<script lang="ts">
	import { navigating } from "$app/stores"
	import GitHub from "$lib/components/GitHub.svelte"
	import Logo from "$lib/components/Logo.svelte"
	import { Dialog } from "bits-ui"
	import { Menu, X } from "lucide-svelte"
	import { fade, fly } from "svelte/transition"
	import Search from "./Search.svelte"

	let dialogOpen = $state(false)

	$effect(() => {
		if ($navigating) {
			dialogOpen = false
		}
	})
</script>

<Dialog.Root bind:open={dialogOpen}>
	<Dialog.Trigger>
		<span class="sr-only">open menu</span>
		<Menu aria-hidden="true" />
	</Dialog.Trigger>
	<Dialog.Portal>
		<Dialog.Content class="fixed inset-0 top-0 z-50 flex flex-col">
			<Dialog.Title class="sr-only">Menu</Dialog.Title>
			<div transition:fade={{ duration: 200 }} class="bg-background">
				<div class="relative z-50 flex h-14 flex-shrink-0 items-center justify-between px-4">
					<a href="/">
						<Logo class="text-primary h-7" />
					</a>
					<Dialog.Close>
						<span class="sr-only">close menu</span>
						<X aria-hidden="true" />
					</Dialog.Close>
				</div>
				<div class="px-4 py-1">
					<Search />
				</div>
			</div>
			<div
				class="bg-background flex flex-grow flex-col overflow-hidden"
				transition:fade={{ duration: 200 }}
			>
				<nav class="flex h-full flex-col px-4 pt-2" transition:fly={{ y: "-2%", duration: 200 }}>
					<div class="flex flex-grow flex-col border-y py-3">
						{#snippet item(href: string, text: string)}
							<a {href} class="hover:bg-card/50 flex h-10 items-center rounded px-3">{text}</a>
						{/snippet}

						{@render item("https://docs.pesde.daimond113.com/", "Documentation")}
						{@render item("https://docs.pesde.daimond113.com/registry/policies", "Policies")}
					</div>
					<div class="flex items-center py-5">
						<a href="https://github.com/daimond113/pesde" target="_blank" rel="noreferrer noopener">
							<GitHub class="size-6" />
						</a>
					</div>
				</nav>
			</div>
		</Dialog.Content>
	</Dialog.Portal>
</Dialog.Root>
