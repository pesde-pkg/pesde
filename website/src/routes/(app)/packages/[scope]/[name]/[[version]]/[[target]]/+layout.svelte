<script lang="ts">
	import { page } from "$app/stores"
	import GitHub from "$lib/components/GitHub.svelte"
	import { makeRegistryUrl, type TargetInfo } from "$lib/registry-api"
	import { BinaryIcon, Globe, Icon, LibraryIcon, Mail, ScrollIcon } from "lucide-svelte"
	import type { ComponentType } from "svelte"
	import TargetSelector from "../../TargetSelector.svelte"
	import Command from "./Command.svelte"

	let { children, data } = $props()

	const installCommand = $derived(`pesde add ${data.pkg.name}`)
	const xCommand = $derived(`pesde x ${data.pkg.name}`)

	const defaultTarget = $derived(
		"target" in $page.params && $page.params.target !== "any"
			? $page.params.target
			: data.pkg.targets[0].kind,
	)
	const currentTarget = $derived(
		data.pkg.targets.find((target: TargetInfo) => target.kind === defaultTarget),
	)

	const repositoryUrl = $derived(
		data.pkg.repository !== undefined ? new URL(data.pkg.repository) : undefined,
	)
	const isGitHub = $derived(repositoryUrl?.hostname === "github.com")
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
		scripts: "Scripts",
	}

	const exportIcons: Partial<Record<keyof TargetInfo, ComponentType<Icon>>> = {
		lib: LibraryIcon,
		bin: BinaryIcon,
		scripts: ScrollIcon,
	}

	const exportEntries = $derived(
		Object.entries(exportNames).filter(([key]) => !!currentTarget?.[key as keyof TargetInfo]),
	)

	const downloadUrl = $derived(
		makeRegistryUrl(
			`packages/${encodeURIComponent(data.pkg.name)}/${encodeURIComponent(data.pkg.version)}/${encodeURIComponent(currentTarget?.kind ?? defaultTarget)}/archive`,
		).toString(),
	)
</script>

<div class="flex flex-col lg:flex-row">
	<div class="flex-grow lg:pr-4">
		{@render children()}
	</div>
	<aside
		class="w-full flex-shrink-0 border-t pt-16 lg:ml-auto lg:max-w-[22rem] lg:border-l lg:border-t-0 lg:pl-4 lg:pt-6"
	>
		<h2 class="text-heading mb-1 text-lg font-semibold">Install</h2>
		<Command command={installCommand} class="mb-2" />
		<p class="mb-4 text-sm">
			Or, download the archive <a class="text-primary underline" href={downloadUrl}> here </a>.
		</p>

		<div class="hidden lg:block">
			<TargetSelector />
		</div>

		{#if data.pkg.license !== undefined}
			<h2 class="text-heading mb-1 text-lg font-semibold">License</h2>
			<div class="mb-6">{data.pkg.license}</div>
		{/if}

		{#if data.pkg.repository !== undefined}
			<h2 class="text-heading mb-1 text-lg font-semibold">Repository</h2>
			<div class="mb-6">
				<a
					href={data.pkg.repository}
					class="inline-flex items-center space-x-2 underline"
					target="_blank"
					rel="noreferrer noopener"
				>
					{#if isGitHub}
						<GitHub class="text-primary size-5" />
						<span>
							{githubRepo}
						</span>
					{:else}
						{data.pkg.repository}
					{/if}
				</a>
			</div>
		{/if}

		<h2 class="text-heading mb-1 text-lg font-semibold">Exports</h2>
		<ul class="mb-6 space-y-0.5">
			{#each exportEntries as [exportKey, exportName]}
				{@const Icon = exportIcons[exportKey as keyof TargetInfo]}
				<li>
					<div class="flex items-center">
						<Icon aria-hidden="true" class="text-primary mr-2 size-5" />
						{exportName}
					</div>
					{#if exportKey === "bin"}
						<p class="text-body/80 mb-4 mt-3 text-sm">
							This package provides a binary that can be executed after installation, or globally
							via:
						</p>
						<Command command={xCommand} class="mb-6" />
					{:else if exportKey === "scripts"}
						<div class="text-body/80 mt-3 flex flex-wrap gap-2 text-sm">
							{#each currentTarget?.scripts ?? [] as script}
								<div class="bg-card text-heading w-max truncate rounded px-3 py-2" title={script}>
									{script}
								</div>
							{/each}
						</div>
					{/if}
				</li>
			{/each}
		</ul>

		{#if data.pkg.authors && data.pkg.authors.length > 0}
			<h2 class="text-heading mb-2 text-lg font-semibold">Authors</h2>
			<ul>
				{#each data.pkg.authors as author}
					{@const [, name] = author.match(/^(.*?)\s*(<|\(|$)/) ?? []}
					{@const [, email] = author.match(/<(.*)>/) ?? []}
					{@const [, website] = author.match(/\((.*)\)/) ?? []}

					<li class="mb-2 flex items-center">
						{name}
						<div class="ml-auto flex items-center space-x-2">
							{#if email}
								<a href={`mailto:${email}`} class="text-primary ml-1" title={`Email: ${email}`}>
									<Mail class="text-primary size-5" aria-hidden="true" />
								</a>
							{/if}
							{#if website}
								<a href={website} class="text-primary ml-1" title={`Website: ${website}`}>
									<Globe class="text-primary size-5" aria-hidden="true" />
								</a>
							{/if}
						</div>
					</li>
				{/each}
			</ul>
		{/if}
	</aside>
</div>
