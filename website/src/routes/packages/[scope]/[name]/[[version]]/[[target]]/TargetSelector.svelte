<script lang="ts">
	import { goto } from "$app/navigation"
	import { page } from "$app/stores"
	import { ChevronDownIcon } from "lucide-svelte"

	const { id }: { id: string } = $props()

	const defaultTarget = $derived(
		"target" in $page.params && $page.params.target !== "any"
			? $page.params.target
			: $page.data.pkg.targets[0].kind,
	)

	const basePath = $derived.by(() => {
		const { scope, name } = $page.params
		if ("target" in $page.params) {
			const { version } = $page.params
			return `/packages/${scope}/${name}/${version}`
		}
		return `/packages/${scope}/${name}/latest`
	})
</script>

<div class="mb-1 text-lg font-semibold text-heading">
	<label for={id}>Target</label>
</div>
<div
	class="relative mb-6 flex h-11 w-full items-center rounded border border-input-border bg-input-bg ring-0 ring-primary-bg/20 transition focus-within:border-primary focus-within:ring-4 has-[:disabled]:opacity-50"
>
	<select
		class="absolute inset-0 appearance-none bg-transparent px-4 outline-none"
		{id}
		onchange={(e) => {
			const select = e.currentTarget

			select.disabled = true
			goto(`${basePath}/${e.currentTarget.value}`).finally(() => {
				select.disabled = false
			})
		}}
	>
		{#each $page.data.pkg.targets as target}
			<option value={target.kind} class="bg-card" selected={target.kind === defaultTarget}>
				{target.kind[0].toUpperCase() + target.kind.slice(1)}
			</option>
		{/each}
	</select>
	<ChevronDownIcon class="pointer-events-none absolute right-4 h-5 w-5" />
</div>
