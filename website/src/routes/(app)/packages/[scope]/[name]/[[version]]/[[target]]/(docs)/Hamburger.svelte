<script lang="ts">
	import { navigating, page } from "$app/stores"
	import Logomark from "$lib/components/Logomark.svelte"
	import { Dialog, Label, useId } from "bits-ui"
	import { Menu, X } from "lucide-svelte"
	import { fade, fly } from "svelte/transition"
	import SidebarItem from "./SidebarItem.svelte"
	import TargetSelector from "./TargetSelector.svelte"
	import VersionSelector from "./VersionSelector.svelte"

	let dialogOpen = $state(false)
	const [scope, name] = $page.data.pkg.name.split("/")

	$effect(() => {
		if ($navigating) {
			dialogOpen = false
		}
	})

	let versionOpen = $state(false)
	let targetOpen = $state(false)

	const versionId = useId()
	const targetId = useId()
</script>

<Dialog.Root bind:open={dialogOpen}>
	<Dialog.Trigger>
		<span class="sr-only">open menu</span>
		<Menu aria-hidden="true" />
	</Dialog.Trigger>
	<Dialog.Portal>
		<Dialog.Content forceMount>
			{#snippet child({ props, open })}
				{#if open}
					<div {...props} class="fixed inset-0 top-0 z-50 flex flex-col">
						<Dialog.Title class="sr-only">Menu</Dialog.Title>
						<div transition:fade={{ duration: 200 }} class="bg-header">
							<div
								class="relative z-50 flex h-14 flex-shrink-0 items-center justify-between border-b px-4"
							>
								<a
									class="flex items-center truncate"
									href={`/packages/${encodeURIComponent(scope)}/${encodeURIComponent(name)}/${encodeURIComponent($page.params.version ?? "latest")}/${encodeURIComponent($page.params.target ?? "any")}`}
								>
									{#snippet separator()}
										<span class="text-body/60 px-2 text-xl">/</span>
									{/snippet}

									<span class="text-primary mr-2">
										<Logomark class="h-7" />
									</span>
									<span class="truncate">{scope}</span>
									{@render separator()}
									<span class="text-heading truncate font-medium">{name}</span>
								</a>
								<Dialog.Close>
									<span class="sr-only">close menu</span>
									<X aria-hidden="true" />
								</Dialog.Close>
							</div>
						</div>
						<div
							class="bg-header flex flex-grow flex-col overflow-hidden"
							transition:fade={{ duration: 200 }}
						>
							<nav
								class="flex h-full flex-col overflow-y-auto p-4"
								transition:fly={{ y: "-2%", duration: 200 }}
							>
								<div
									class="mb-4 flex flex-col space-y-4 border-b pb-4 sm:flex-row sm:space-x-4 sm:space-y-0"
								>
									<div class="w-full">
										<div class="text-heading mb-1 text-sm font-semibold">
											<Label.Root for={versionId}>Version</Label.Root>
										</div>
										<VersionSelector id={versionId} bind:open={versionOpen} />
									</div>
									<div class="w-full">
										<div class="text-heading mb-1 text-sm font-semibold">
											<Label.Root for={targetId}>Target</Label.Root>
										</div>
										<TargetSelector id={targetId} bind:open={targetOpen} />
									</div>
								</div>
								<ul>
									{#each $page.data.sidebar ?? [] as item}
										<li>
											<SidebarItem {item} />
										</li>
									{/each}
								</ul>
							</nav>
						</div>
					</div>
				{/if}
			{/snippet}
		</Dialog.Content>
	</Dialog.Portal>
</Dialog.Root>
