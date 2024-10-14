<script lang="ts">
	import { page } from "$app/stores"
	import { DEPENDENCY_KIND_DISPLAY_NAMES, type DependencyKind } from "$lib/registry-api.js"

	const { data } = $props()

	let groupedDeps = $derived(
		Object.groupBy(
			Object.entries(data.pkg.dependencies).map(([alias, dependency]) => ({ alias, dependency })),
			(entry) => entry.dependency[1],
		),
	)
</script>

{#if Object.keys(groupedDeps).length === 0}
	<p class="py-24 text-center">This package doesn't have any dependencies.</p>
{:else}
	<div class="space-y-8 py-8">
		{#each Object.entries(groupedDeps).sort( (a, b) => b[0].localeCompare(a[0]), ) as [dependencyKind, group]}
			<section>
				<h2 class="text-heading mb-4 text-xl font-medium">
					{DEPENDENCY_KIND_DISPLAY_NAMES[dependencyKind as DependencyKind]}
				</h2>

				<div class="space-y-4">
					{#each group as { dependency: [dependencyInfo] }}
						{@const isWally = "wally" in dependencyInfo}
						{@const [scope, name] = (isWally ? dependencyInfo.wally : dependencyInfo.name).split(
							"/",
						)}
						{@const target = isWally
							? undefined
							: (dependencyInfo.target ?? $page.params.target ?? data.pkg.targets[0].kind)}
						{@const isOfficialRegistry = isWally
							? dependencyInfo.index.toLowerCase() === "https://github.com/upliftgames/wally-index"
							: dependencyInfo.index.toLowerCase() === "https://github.com/daimond113/pesde-index"}

						<article
							class={`bg-card relative overflow-hidden rounded px-5 py-4 transition ${
								isOfficialRegistry ? "hover:bg-card-hover" : ""
							}`}
						>
							<h3 class="font-semibold">
								<svelte:element
									this={isOfficialRegistry ? "a" : "svelte:fragment"}
									{...isOfficialRegistry
										? {
												href: isWally
													? `https://wally.run/package/${dependencyInfo.wally}`
													: `/packages/${dependencyInfo.name}/latest/${target}`,
											}
										: {}}
									class="after:absolute after:inset-0 after:content-['']"
								>
									<span class="text-heading">{scope}/</span><span class="text-light">{name}</span>
									{#if isWally}
										<span class="text-red-400">(wally)</span>
									{/if}
								</svelte:element>
							</h3>
							<div class="text-primary text-sm font-semibold">
								{dependencyInfo.version}
								{#if !isWally}
									·
									{target}
								{/if}
							</div>
						</article>
					{/each}
				</div>
			</section>
		{/each}
	</div>
{/if}
