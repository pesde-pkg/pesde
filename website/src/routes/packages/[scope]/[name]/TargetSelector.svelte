<script lang="ts">
	import { goto } from "$app/navigation"
	import { page } from "$app/stores"
	import { TARGET_KIND_DISPLAY_NAMES, type TargetInfo, type TargetKind } from "$lib/registry-api"
	import { ChevronDownIcon } from "lucide-svelte"
	import { getContext } from "svelte"

	const { id }: { id: string } = $props()

	const currentTarget = getContext<{ value: TargetInfo }>("currentTarget")

	const basePath = $derived.by(() => {
		const { scope, name } = $page.params
		if ("target" in $page.params) {
			const { version } = $page.params
			return `/packages/${scope}/${name}/${version}`
		}
		return `/packages/${scope}/${name}/latest`
	})
</script>

<div class="text-heading mb-1 text-lg font-semibold">
	<label for={id}>Target</label>
</div>
<div
	class="border-input-border bg-input-bg ring-primary-bg/20 focus-within:border-primary relative mb-6 flex h-11 w-full items-center rounded border ring-0 transition focus-within:ring-4 has-[:disabled]:opacity-50"
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
			<option
				value={target.kind}
				class="bg-card"
				selected={target.kind === currentTarget.value.kind}
			>
				{TARGET_KIND_DISPLAY_NAMES[target.kind as TargetKind]}
			</option>
		{/each}
	</select>
	<ChevronDownIcon aria-hidden="true" class="pointer-events-none absolute right-4 h-5 w-5" />
</div>
