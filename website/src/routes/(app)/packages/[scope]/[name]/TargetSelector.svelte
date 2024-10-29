<script lang="ts">
	import { goto } from "$app/navigation"
	import { page } from "$app/stores"
	import Select from "$lib/components/Select.svelte"
	import { TARGET_KIND_DISPLAY_NAMES, type TargetInfo, type TargetKind } from "$lib/registry-api"
	import { Label, useId } from "bits-ui"
	import { getContext } from "svelte"

	const currentTarget = getContext<{ value: TargetInfo }>("currentTarget")

	const basePath = $derived.by(() => {
		const { scope, name } = $page.params
		if ("target" in $page.params) {
			const { version } = $page.params
			return `/packages/${scope}/${name}/${version}`
		}
		return `/packages/${scope}/${name}/latest`
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
