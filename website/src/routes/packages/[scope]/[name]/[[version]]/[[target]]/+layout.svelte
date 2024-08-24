<script lang="ts">
	import { BinaryIcon, Globe, Icon, LibraryIcon, Mail } from "lucide-svelte"
	import { page } from "$app/stores"
	import type { TargetInfo } from "$lib/registry-api"
	import type { ComponentType } from "svelte"
	import Command from "./Command.svelte"
	import TargetSelector from "./TargetSelector.svelte"
	import Github from "$lib/components/Github.svelte"

	let { children, data } = $props()

	const installCommand = $derived(`pesde add ${data.pkg.name}`)
	const xCommand = $derived(`pesde x ${data.pkg.name}`)

	const defaultTarget = $derived(
		"target" in $page.params && $page.params.target !== "any"
			? $page.params.target
			: data.pkg.targets[0].kind,
	)
	const currentTarget = $derived(data.pkg.targets.find((target) => target.kind === defaultTarget))

	const repositoryUrl = $derived(
		data.pkg.repository !== undefined ? new URL(data.pkg.repository) : undefined,
	)
	const isGithub = $derived(repositoryUrl?.hostname === "github.com")
	const githubRepo = $derived(
		repositoryUrl?.pathname
			.split("/")
			.slice(1, 3)
			.join("/")
			.replace(/\.git$/, ""),
	)

	const exportNames: Partial<Record<keyof TargetInfo, string>> = {
		lib: "Library",
		bin: "Binary",
	}

	const exportIcons: Partial<Record<keyof TargetInfo, ComponentType<Icon>>> = {
		lib: LibraryIcon,
		bin: BinaryIcon,
	}

	const exportEntries = $derived(
		Object.entries(exportNames).filter(([key]) => !!currentTarget?.[key as keyof TargetInfo]),
	)
</script>

<div class="flex flex-col lg:flex-row">
	<div class="lg:pr-4">
		{@render children()}
	</div>
	<aside
		class="w-full flex-shrink-0 border-t pt-16 lg:ml-auto lg:max-w-[22rem] lg:border-l lg:border-t-0 lg:pl-4 lg:pt-6"
	>
		<h2 class="mb-1 text-lg font-semibold text-heading">Install</h2>
		<Command command={installCommand} class="mb-4" />

		<div class="hidden lg:block">
			<TargetSelector id="target-selector-sidebar" />
		</div>

		{#if data.pkg.license !== undefined}
			<h2 class="mb-1 text-lg font-semibold text-heading">License</h2>
			<div class="mb-6">{data.pkg.license}</div>
		{/if}

		{#if data.pkg.repository !== undefined}
			<h2 class="mb-1 text-lg font-semibold text-heading">Repository</h2>
			<div class="mb-6">
				<a
					href={data.pkg.repository}
					class="inline-flex items-center space-x-2 underline"
					target="_blank"
					rel="noreferrer noopener"
				>
					{#if isGithub}
						<Github class="size-5 text-primary" />
						<span>
							{githubRepo}
						</span>
					{:else}
						{data.pkg.repository}
					{/if}
				</a>
			</div>
		{/if}

		<h2 class="mb-1 text-lg font-semibold text-heading">Exports</h2>
		<ul class="mb-6 space-y-0.5">
			{#each exportEntries as [exportKey, exportName]}
				{@const Icon = exportIcons[exportKey as keyof TargetInfo]}
				<li class="flex items-center">
					<Icon class="mr-2 size-5 text-primary" />
					{exportName}
				</li>
			{/each}
		</ul>

		{#if currentTarget?.bin}
			<p class="-mt-3 mb-4 text-sm text-body/80">
				This package provides a binary that can be executed after installation, or globally via:
			</p>
			<Command command={xCommand} class="mb-6" />
		{/if}

		{#if data.pkg.authors && data.pkg.authors.length > 0}
			<h2 class="mb-2 text-lg font-semibold text-heading">Authors</h2>
			<ul>
				{#each data.pkg.authors as author}
					{@const [, name] = author.match(/^(.*?)(<|\()/) ?? []}
					{@const [, email] = author.match(/<(.*)>/) ?? []}
					{@const [, website] = author.match(/\((.*)\)/) ?? []}

					<li class="mb-2 flex items-center">
						{name}
						<div class="ml-auto flex items-center space-x-2">
							{#if email}
								<a href={`mailto:${email}`} class="ml-1 text-primary" title={`Email: ${email}`}>
									<Mail class="size-5 text-primary" aria-hidden="true" />
								</a>
							{/if}
							{#if website}
								<a href={website} class="ml-1 text-primary" title={`Website: ${website}`}>
									<Globe class="size-5 text-primary" aria-hidden="true" />
								</a>
							{/if}
						</div>
					</li>
				{/each}
			</ul>
		{/if}
	</aside>
</div>
