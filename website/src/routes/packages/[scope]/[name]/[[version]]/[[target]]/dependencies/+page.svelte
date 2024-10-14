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
		{#each Object.entries(groupedDeps).sort( (a, b) => a[0].localeCompare(b[0]), ) as [dependencyKind, group]}
			<section>
				<h2 class="text-heading mb-4 text-xl font-medium">
					{DEPENDENCY_KIND_DISPLAY_NAMES[dependencyKind as DependencyKind]}
				</h2>

				<div class="space-y-4">
					{#each group as { dependency: [dependencyInfo] }}
						{@const [scope, name] = dependencyInfo.name.split("/")}
						{@const target =
							dependencyInfo.target ?? $page.params.target ?? data.pkg.targets[0].kind}

						<article
							class="bg-card hover:bg-card-hover relative overflow-hidden rounded px-5 py-4 transition"
						>
							<h3 class="font-semibold">
								<a
									href={`/packages/${dependencyInfo.name}/latest/${target}`}
									class="after:absolute after:inset-0 after:content-['']"
								>
									<span class="text-heading">{scope}/</span><span class="text-light">{name}</span>
								</a>
							</h3>
							<div class="text-primary text-sm font-semibold">
								{dependencyInfo.version}
								·
								{target}
							</div>
						</article>
					{/each}
				</div>
			</section>
		{/each}
	</div>
{/if}
