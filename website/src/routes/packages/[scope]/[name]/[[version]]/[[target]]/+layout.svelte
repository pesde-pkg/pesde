<script lang="ts">
	import { formatDistanceToNow } from "date-fns"
	import { BinaryIcon, Icon, LibraryIcon } from "lucide-svelte"
	import Tab from "./Tab.svelte"
	import { page } from "$app/stores"
	import type { TargetInfo } from "$lib/registry-api"
	import type { ComponentType } from "svelte"
	import Command from "./Command.svelte"
	import TargetSelector from "./TargetSelector.svelte"
	import Github from "$lib/components/Github.svelte"

	let { children, data } = $props()

	const [scope, name] = $derived(data.pkg.name.split("/"))

	const installCommand = $derived(`pesde add ${data.pkg.name}`)
	const xCommand = $derived(`pesde x ${data.pkg.name}`)

	const defaultTarget = $derived(
		"target" in $page.params ? $page.params.target : data.pkg.targets[0].kind,
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
</script>

<div class="mx-auto flex max-w-prose flex-col px-4 py-16 lg:max-w-screen-lg lg:flex-row">
	<div class="flex-grow lg:pr-4">
		<h1 class="text-3xl font-bold">
			<span class="text-heading">{scope}/</span><span class="text-light">{name}</span>
		</h1>
		<div class="mb-2 font-semibold text-primary">
			v{data.pkg.version} ·
			<time
				datetime={data.pkg.published_at}
				title={new Date(data.pkg.published_at).toLocaleString()}
			>
				published {formatDistanceToNow(new Date(data.pkg.published_at), {
					addSuffix: true,
				})}
			</time>
		</div>
		<p class="mb-6 max-w-prose">{data.pkg.description}</p>

		<div class="mb-12 lg:hidden">
			<TargetSelector id="target-selector-sidebar" />
		</div>

		<nav class="flex w-full border-b-2">
			<Tab tab="">Readme</Tab>
			<Tab tab="versions">Versions</Tab>
		</nav>

		{@render children()}
	</div>
	<aside
		class="w-full flex-shrink-0 border-t pt-16 lg:ml-auto lg:max-w-[22rem] lg:border-l lg:border-t-0 lg:pl-4 lg:pt-0"
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
			{#each Object.entries(exportNames).filter(([key]) => !!currentTarget?.[key as keyof TargetInfo]) as [exportKey, exportName]}
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
	</aside>
</div>
