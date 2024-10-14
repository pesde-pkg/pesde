<script lang="ts">
	import { page } from "$app/stores"
	import { formatDistanceToNow } from "date-fns"
	import { onMount, setContext, untrack } from "svelte"
	import Tab from "./Tab.svelte"
	import TargetSelector from "./TargetSelector.svelte"

	let { children, data } = $props()

	const [scope, name] = $derived(data.pkg.name.split("/"))

	let currentPkg = $state(data.pkg)
	let currentTarget = $state(data.pkg.targets[0])

	setContext("currentPkg", {
		get value() {
			return currentPkg
		},
		set value(v) {
			currentPkg = v
		},
	})

	setContext("currentTarget", {
		get value() {
			return currentTarget
		},
		set value(v) {
			currentTarget = v
		},
	})

	const getReadme = () => {
		if ("target" in $page.params) {
			return `${$page.params.version}/${$page.params.target}`
		}
		return ""
	}

	const pkgVersion = $derived(currentPkg.version)
	const pkgDescription = $derived(currentPkg.description)

	let pkgDate = $state<null | string>(null)
	let readme = $state(getReadme())

	$effect(() => {
		pkgDate = formatDistanceToNow(new Date(currentPkg.published_at), { addSuffix: true })
		readme = untrack(getReadme)
	})

	onMount(() => {
		return page.subscribe((page) => {
			if (pkgDate === null || page.params.target !== undefined) {
				currentTarget =
					data.pkg.targets.find((target) => target.kind === page.params.target) ??
					data.pkg.targets[0]
				currentPkg = data.pkg
			}
		})
	})
</script>

<div class="mx-auto max-w-prose px-4 py-16 lg:max-w-screen-lg">
	<h1 class="text-3xl font-bold">
		<span class="text-heading">{scope}/</span><span class="text-light">{name}</span>
	</h1>
	<div class="text-primary mb-2 font-semibold" class:invisible={pkgDate === null}>
		v{pkgVersion} ·
		<time datetime={data.pkg.published_at} title={new Date(data.pkg.published_at).toLocaleString()}>
			published {pkgDate ?? "..."}
		</time>
	</div>
	<p class="mb-6 max-w-prose">{pkgDescription}</p>

	<div class="mb-8 lg:hidden">
		<TargetSelector id="mobile-target-selector" />
	</div>

	<nav class="flex w-full border-b-2">
		<Tab tab={readme}>Readme</Tab>
		<Tab tab={`${pkgVersion}/${currentTarget.kind}/dependencies`}>Dependencies</Tab>
		<Tab tab="versions">Versions</Tab>
	</nav>

	{@render children()}
</div>
