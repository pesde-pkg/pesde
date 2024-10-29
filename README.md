<br>

<div align="center">
    <img src="https://raw.githubusercontent.com/daimond113/pesde/0.5/assets/logotype.svg" alt="pesde logo" width="200" />
</div>

<br>

pesde is a package manager for the Luau programming language, supporting
multiple runtimes including Roblox and Lune. pesde has its own registry, however
it can also use Wally, and Git repositories as package sources. It has been
designed with multiple targets in mind, namely Roblox, Lune, and Luau.

## Installation

pesde can be installed from GitHub Releases. You can find the latest release
[here](https://github.com/daimond113/pesde/releases). Once you have downloaded
the binary, run the following command to install it:

```sh
pesde self-install
```

Note that pesde manages its own versions, so you can update it by running the
following command:

```sh
pesde self-upgrade
```

## Documentation

For more information about its usage, you can check the
[documentation](https://docs.pesde.daimond113.com).

## Registry

The main pesde registry is hosted on [fly.io](https://fly.io). You can find it
at https://registry.pesde.daimond113.com.

### Self-hosting

The registry tries to require no modifications to be self-hosted. Please refer
to the
[documentation](http://docs.pesde.daimond113.com/guides/self-hosting-registries)
for more information.

## Previous art

pesde is heavily inspired by [npm](https://www.npmjs.com/),
[pnpm](https://pnpm.io/), [Wally](https://wally.run), and
[Cargo](https://doc.rust-lang.org/cargo/).
