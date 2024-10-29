<script lang="ts">
	import { goto } from "$app/navigation"
	import { page } from "$app/stores"
	import Select from "$lib/components/Select.svelte"
	import { TARGET_KIND_DISPLAY_NAMES, type TargetInfo } from "$lib/registry-api"
	import type { Snippet } from "svelte"

	let disabled = $state(false)

	type Props = {
		trigger?: Snippet<[Record<string, unknown>, string]>
		sameWidth?: boolean
		open?: boolean
		id?: string
	}

	let { trigger, sameWidth = true, open = $bindable(false), id }: Props = $props()

	const basePath = $derived.by(() => {
		const { scope, name } = $page.params
		return `/packages/${scope}/${name}`
	})
</script>

<Select
	items={$page.data.pkg.targets.map((target: TargetInfo) => ({
		value: target.kind,
		label: TARGET_KIND_DISPLAY_NAMES[target.kind],
	}))}
	value={$page.params.target ?? $page.data.pkg.targets[0].kind}
	contentClass={sameWidth ? "" : "w-32"}
	onValueChange={(target) => {
		disabled = true

		const path = $page.data.activeTab === "docs" ? "docs/intro" : "reference"

		goto(`${basePath}/${$page.data.pkg.version}/${target}/${path}`).finally(() => {
			disabled = false
		})
	}}
	bind:open
	{disabled}
	{sameWidth}
	{trigger}
	{id}
/>
