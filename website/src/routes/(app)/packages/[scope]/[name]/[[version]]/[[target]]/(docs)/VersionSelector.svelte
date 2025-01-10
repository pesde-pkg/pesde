<script lang="ts">
	import { goto } from "$app/navigation"
	import { page } from "$app/stores"
	import Select from "$lib/components/Select.svelte"
	import { fetchRegistryJson, type PackageVersionResponse } from "$lib/registry-api"
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
		return `/packages/${encodeURIComponent(scope)}/${encodeURIComponent(name)}`
	})
</script>

<Select
	items={$page.data.versions.map((v: string) => ({ value: v, label: v }))}
	value={$page.data.pkg.version}
	contentClass={sameWidth ? "" : "w-32"}
	onValueChange={(version) => {
		disabled = true

		const path = $page.data.activeTab === "docs" ? "docs/intro" : "reference"

		fetchRegistryJson<PackageVersionResponse>(
			`packages/${encodeURIComponent($page.data.pkg.name)}/${encodeURIComponent(version)}/any`,
			fetch,
		)
			.then((pkg) => goto(`${basePath}/${version}/${pkg.targets[0].kind}/${path}`))
			.finally(() => {
				disabled = false
			})
	}}
	bind:open
	{disabled}
	{sameWidth}
	{trigger}
	{id}
/>
