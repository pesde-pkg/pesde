<script lang="ts">
	import { formatDistanceToNow } from "date-fns"
	import { BinaryIcon, Icon, LibraryIcon } from "lucide-svelte"
	import Tab from "./Tab.svelte"
	import { page } from "$app/stores"
	import type { TargetInfo } from "$lib/registry-api"
	import type { ComponentType } from "svelte"
	import Command from "./Command.svelte"
	import TargetSelector from "./TargetSelector.svelte"

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
						<svg
							class="size-5 text-primary"
							role="img"
							viewBox="0 0 24 24"
							xmlns="http://www.w3.org/2000/svg"
						>
							<title>GitHub</title>
							<path
								d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12"
								fill="currentColor"
							/>
						</svg>
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
