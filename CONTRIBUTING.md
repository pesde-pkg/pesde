# How to contribute

Thank you for wanting to contribute to pesde! We're glad to have your help
improving the project.

We'd love to have everyone on board, but unfortunately we can't help with the
basics of the tooling involved, so please make sure you can build the project
yourself before starting work on an issue.

To avoid wasting anyone's time, please first confirm whether your change will be
accepted. This includes, for example, opening a feature request or claiming an
issue, either on GitHub or through another channel (such as Discord). Small
touchups such as typos or documentation changes don't necessarily need to go
through this process.

## Version

The `main` branch contains unreleased changes for an unspecified version. Look
for a branch such as `0.8` if you want to contribute to a specific version.

## Setup

This codebase uses a pretty conventional setup, so you shouldn't need to do
anything particularly extraordinary. For contributing to the Rust code, e.g.
the CLI or registry, [rustup](https://rustup.rs/) gives you all the necessary
tools. For contributing to the documentation, [pnpm](https://pnpm.io/) alongside
a JavaScript runtime, such as [Node.js](https://nodejs.org), will get you up and
running.

### SQL

If you're editing any SQL queries, be sure to unset `SQLX_OFFLINE=1` in the
crate's `.env` file. You may run `cargo sqlx database setup` to initialise the
database and apply all migrations.

#### Registry

If you're changing a backend of the registry, you may use `docker compose up` to
start the database servers.

## Style

Our code style is mostly what formatters and linters produce with our configs.
We try not to be pedantic in this regard during reviews, so as long as
you run the appropriate tooling, you should be good to go.

## Testing

While we understand that writing tests is a chore, it is important nonetheless.
We'd greatly appreciate it if your change included them, but we won't block your
contribution on this.

## Submitting your changes

This project uses [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/),
so you should attempt to follow this style for your commits. Most pull requests
end up squash-merged, so this isn't a rigid requirement.

The title of your pull request should *not* use that format. Use a normal
sentence describing the key idea of your changes.

An example of a pull request title and commits:

```
Add stats endpoint to registry v2 api

- feat: add stats endpoint to registry v2 api
- chore: remove outdated TODO comment
```

If your contribution changes code, please add your change to the
`CHANGELOG.md` in the `Unreleased` header under one of these categories:
1. `Added`: your contribution adds new functionality
2. `Changed`: your contribution neither adds nor removes functionality (most
commonly, bug fixes)
3. `Removed`: your contribution removes something

Additionally, mention your GitHub username at the end of the note so that you
can be properly attributed in release notes; a suffix of
`by @yourusernamehere` is great.

Documentation changes, typos, and stylistic touch-ups are not part of the
changelog.

Remember: if you've changed any SQLx queries, you must run `cargo sqlx prepare`
in the crate's directory.