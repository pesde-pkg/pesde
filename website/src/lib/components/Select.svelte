<script lang="ts">
	import { Select, type SelectSingleRootProps, type WithoutChildren } from "bits-ui"
	import { Check, ChevronsUpDown } from "lucide-svelte"
	import type { Snippet } from "svelte"

	type Props = Omit<WithoutChildren<SelectSingleRootProps>, "type"> & {
		placeholder?: string
		items: { value: string; label: string; disabled?: boolean }[]
		contentProps?: WithoutChildren<Select.ContentProps>
		contentClass?: string
		triggerClass?: string
		trigger?: Snippet<[Record<string, unknown>, string]>
		sameWidth?: boolean
	}

	let {
		value = $bindable(""),
		items,
		trigger,
		contentProps,
		contentClass = "",
		triggerClass = "",
		placeholder,
		sameWidth = true,
		id,
		open = $bindable(false),
		...restProps
	}: Props = $props()

	const selectedLabel = $derived(items.find((item) => item.value === value)?.label)
	const triggerLabel = $derived(selectedLabel ?? placeholder ?? "")
</script>

<Select.Root type="single" bind:value bind:open {...restProps}>
	<Select.Trigger {id}>
		{#snippet child({ props })}
			{#if trigger}
				{@render trigger(props, triggerLabel)}
			{:else}
				<button
					{...props}
					class={`border-input-border bg-input-bg ring-primary-bg/20 focus:border-primary relative flex h-11 w-full items-center rounded border px-4 outline-none ring-0 transition focus:ring-4 data-[disabled]:opacity-50 ${triggerClass}`}
				>
					{triggerLabel}
					<ChevronsUpDown class="ml-auto size-5" />
				</button>
			{/if}
		{/snippet}
	</Select.Trigger>
	<Select.Portal>
		<Select.Content
			sideOffset={8}
			collisionPadding={8}
			{...contentProps}
			class={`bg-card border-input-border z-50 max-h-[var(--bits-floating-available-height)] origin-top overflow-y-auto rounded-lg border p-1 shadow-lg ${sameWidth ? "w-[var(--bits-select-anchor-width)]" : ""} ${contentClass}`}
		>
			{#each items as { value, label, disabled } (value)}
				<Select.Item
					{value}
					{label}
					{disabled}
					class="data-[highlighted]:bg-card-hover flex h-10 flex-shrink-0 items-center truncate rounded-sm px-3"
				>
					{#snippet children({ selected })}
						{label}
						{#if selected}
							<Check class="ml-auto size-4" />
						{/if}
					{/snippet}
				</Select.Item>
			{/each}
		</Select.Content>
	</Select.Portal>
</Select.Root>
