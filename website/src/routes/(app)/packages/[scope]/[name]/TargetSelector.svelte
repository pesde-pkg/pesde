<script lang="ts">
	import { goto } from "$app/navigation"
	import { page } from "$app/stores"
	import Select from "$lib/components/Select.svelte"
	import { TARGET_KIND_DISPLAY_NAMES, type TargetInfo, type TargetKind } from "$lib/registry-api"
	import { Label, useId } from "bits-ui"
	import { getContext } from "svelte"
	import { TriangleAlert } from "lucide-svelte"

	const currentTarget = getContext<{ value: TargetInfo }>("currentTarget")

	const basePath = $derived.by(() => {
		const { scope, name } = $page.params
		if ("target" in $page.params) {
			const { version } = $page.params
			return `/packages/${encodeURIComponent(scope)}/${encodeURIComponent(name)}/${encodeURIComponent(version)}`
		}
		return `/packages/${encodeURIComponent(scope)}/${encodeURIComponent(name)}/latest`
	})

	const items = ($page.data.pkg.targets as TargetInfo[]).map((target) => {
		return {
			value: target.kind,
			label: TARGET_KIND_DISPLAY_NAMES[target.kind as TargetKind],
		}
	})

	const id = useId()

	let disabled = $state(false)
	let open = $state(false)
</script>

<div class="text-heading mb-1 text-lg font-semibold">
	<Label.Root for={id} onclick={() => (open = true)}>Target</Label.Root>
	{#if currentTarget.value.yanked}
		<span
			class="ml-1 inline-flex items-center rounded bg-yellow-600/10 px-2 py-1 text-sm text-yellow-950 dark:bg-yellow-500/10 dark:text-yellow-100"
		>
			<TriangleAlert class="mr-1 inline-block size-4" />
			<span class="-mb-0.5">Yanked</span>
		</span>
	{/if}
</div>

<Select
	{items}
	{disabled}
	{id}
	bind:open
	name="target-selector"
	allowDeselect={false}
	value={currentTarget.value.kind}
	triggerClass="mb-6"
	onValueChange={(selected) => {
		disabled = true
		goto(`${basePath}/${selected}`).finally(() => {
			disabled = false
		})
	}}
/>
